# Memory Leak Indicators -- C#

## Overview
Memory leaks in C# occur when `IDisposable` objects are not disposed, event handlers are added without removal, or static collections grow without bounds. Although .NET has a garbage collector, it cannot reclaim objects referenced by live event handlers, static fields, or undisposed native resources.

## Why It's a Scalability Concern
ASP.NET Core services handle thousands of concurrent requests. Leaked event subscriptions, undisposed `HttpClient` instances, or growing static dictionaries accumulate over the service's lifetime. This increases Gen2 GC collections, causes LOH fragmentation, and eventually leads to `OutOfMemoryException` or degraded response times from GC pressure.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: .NET Core, ASP.NET Core, Entity Framework, System.Timers
- **Existing pipeline**: `disposable_not_disposed.rs` in `src/audit/pipelines/csharp/` — extends with additional patterns

---

## Pattern 1: IDisposable Not in using Statement

### Description
Creating an `IDisposable` object (e.g., `HttpClient`, `StreamReader`, `SqlConnection`) without wrapping it in a `using` statement or block, risking resource leaks.

### Bad Code (Anti-pattern)
```csharp
public async Task<string> FetchData(string url)
{
    var client = new HttpClient();
    var response = await client.GetStringAsync(url);
    return response;
    // HttpClient not disposed — socket leak
}
```

### Good Code (Fix)
```csharp
public async Task<string> FetchData(string url)
{
    using var client = new HttpClient();
    var response = await client.GetStringAsync(url);
    return response;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `local_declaration_statement`, `variable_declaration`, `object_creation_expression`, `using_statement`
- **Detection approach**: Find `local_declaration_statement` with `object_creation_expression` for known `IDisposable` types that are NOT inside a `using_statement` or `using_declaration`.
- **S-expression query sketch**:
```scheme
(local_declaration_statement
  (variable_declaration
    (variable_declarator
      (equals_value_clause
        (object_creation_expression
          type: (identifier) @type)))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `disposable_not_in_using`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Event += Without Event -=

### Description
Subscribing to an event with `+=` without a corresponding `-=` unsubscription, causing the subscriber to be retained as long as the event source lives.

### Bad Code (Anti-pattern)
```csharp
public class OrderProcessor
{
    public void Initialize(EventBus bus)
    {
        bus.OrderReceived += OnOrderReceived;
        // never unsubscribes — OrderProcessor leaks if bus outlives it
    }

    private void OnOrderReceived(object sender, OrderEventArgs e) { }
}
```

### Good Code (Fix)
```csharp
public class OrderProcessor : IDisposable
{
    private EventBus _bus;

    public void Initialize(EventBus bus)
    {
        _bus = bus;
        _bus.OrderReceived += OnOrderReceived;
    }

    public void Dispose()
    {
        _bus.OrderReceived -= OnOrderReceived;
    }

    private void OnOrderReceived(object sender, OrderEventArgs e) { }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression`, `identifier`, `operator`
- **Detection approach**: Find `assignment_expression` with `+=` operator where the left side is an event (member access). Search the class for a corresponding `-=` on the same event. Flag if no unsubscription exists.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (member_access_expression
    name: (identifier) @event_name)
  operator: "+=")
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `event_subscribe_no_unsubscribe`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: Dictionary/List Growth Without Removal

### Description
Calling `.Add()`, `[key] = value`, or `.TryAdd()` on a collection without any `.Remove()`, `.Clear()`, or size check, causing unbounded memory growth.

### Bad Code (Anti-pattern)
```csharp
private readonly Dictionary<string, CachedResult> _cache = new();

public CachedResult GetOrCompute(string key)
{
    if (!_cache.ContainsKey(key))
    {
        _cache[key] = ComputeExpensive(key);
    }
    return _cache[key];
}
```

### Good Code (Fix)
```csharp
private readonly ConcurrentDictionary<string, CachedResult> _cache = new();

public CachedResult GetOrCompute(string key)
{
    return _cache.GetOrAdd(key, k =>
    {
        if (_cache.Count > 10000)
        {
            var oldest = _cache.Keys.First();
            _cache.TryRemove(oldest, out _);
        }
        return ComputeExpensive(k);
    });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `element_access_expression`
- **Detection approach**: Find `invocation_expression` calling `.Add()`, `.TryAdd()` or `element_access_expression` with assignment (indexer set) on a collection field. Check the class for `.Remove()`, `.TryRemove()`, `.Clear()` on the same field. Flag if no removal exists.
- **S-expression query sketch**:
```scheme
(invocation_expression
  (member_access_expression
    expression: (identifier) @collection
    name: (identifier) @method)
  (#match? @method "^(Add|TryAdd)$"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `collection_growth_no_removal`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 4: Static ConcurrentDictionary Growth

### Description
A `static` `ConcurrentDictionary` with `TryAdd` or `GetOrAdd` calls but no `TryRemove` or cleanup mechanism. Static collections persist for the entire application lifetime.

### Bad Code (Anti-pattern)
```csharp
public static class ConnectionTracker
{
    private static readonly ConcurrentDictionary<string, DateTime> _connections = new();

    public static void Track(string connectionId)
    {
        _connections.TryAdd(connectionId, DateTime.UtcNow);
    }
}
```

### Good Code (Fix)
```csharp
public static class ConnectionTracker
{
    private static readonly ConcurrentDictionary<string, DateTime> _connections = new();

    public static void Track(string connectionId)
    {
        _connections.TryAdd(connectionId, DateTime.UtcNow);
        CleanupExpired();
    }

    public static void Untrack(string connectionId)
    {
        _connections.TryRemove(connectionId, out _);
    }

    private static void CleanupExpired()
    {
        var cutoff = DateTime.UtcNow.AddMinutes(-30);
        foreach (var kvp in _connections.Where(x => x.Value < cutoff))
        {
            _connections.TryRemove(kvp.Key, out _);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration`, `modifier`, `invocation_expression`
- **Detection approach**: Find `field_declaration` with `static` modifier and `ConcurrentDictionary` type. Search the class for `TryAdd`/`GetOrAdd` calls. Flag if no `TryRemove`/`Clear` exists.
- **S-expression query sketch**:
```scheme
(field_declaration
  (modifier) @mod
  type: (generic_name) @type
  (#eq? @mod "static")
  (#match? @type "ConcurrentDictionary"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `static_concurrent_dict_growth`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 5: Timer Without Dispose

### Description
Creating a `System.Timers.Timer` or `System.Threading.Timer` without calling `.Dispose()` or wrapping in `using`, causing it to keep firing and holding references to its callback's closure.

### Bad Code (Anti-pattern)
```csharp
public void StartMonitoring()
{
    var timer = new System.Timers.Timer(5000);
    timer.Elapsed += (s, e) => CheckHealth();
    timer.Start();
    // timer never disposed — keeps firing and leaks
}
```

### Good Code (Fix)
```csharp
private System.Timers.Timer _timer;

public void StartMonitoring()
{
    _timer = new System.Timers.Timer(5000);
    _timer.Elapsed += (s, e) => CheckHealth();
    _timer.Start();
}

public void Dispose()
{
    _timer?.Stop();
    _timer?.Dispose();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `local_declaration_statement`, `object_creation_expression`, `identifier`
- **Detection approach**: Find `local_declaration_statement` creating a `Timer` object that is not stored in a field and has no `Dispose()` call in the same method or enclosing scope.
- **S-expression query sketch**:
```scheme
(local_declaration_statement
  (variable_declaration
    (variable_declarator
      (equals_value_clause
        (object_creation_expression
          type: (qualified_name) @type)))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `timer_no_dispose`
- **Severity**: warning
- **Confidence**: medium
