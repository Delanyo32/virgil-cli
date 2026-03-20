# Sync Blocking in Async -- C#

## Overview
Synchronous blocking in C# async contexts occurs when code blocks on `Task` results using `.Result`, `.Wait()`, or uses `Thread.Sleep()` inside `async` methods. These patterns cause thread-pool starvation and deadlocks in ASP.NET Core applications.

## Why It's a Scalability Concern
The .NET thread pool has a limited number of threads. Blocking on async operations via `.Result` or `.Wait()` holds a thread hostage while waiting, preventing it from processing other requests. In ASP.NET Core, this leads to thread-pool starvation — the server stops accepting new requests even though it's not doing real work. With `SynchronizationContext` in older ASP.NET, it causes deadlocks.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: ASP.NET Core, Entity Framework, HttpClient, System.Threading.Tasks
- **Existing pipeline**: `sync_over_async.rs` in `src/audit/pipelines/csharp/` — extends with additional patterns

---

## Pattern 1: .Result on Task

### Description
Accessing `.Result` on a `Task<T>` synchronously blocks the calling thread until the task completes. In async contexts, this can deadlock or starve the thread pool.

### Bad Code (Anti-pattern)
```csharp
public async Task<UserDto> GetUserAsync(int id)
{
    var user = _httpClient.GetAsync($"/api/users/{id}").Result;
    var content = user.Content.ReadAsStringAsync().Result;
    return JsonSerializer.Deserialize<UserDto>(content);
}
```

### Good Code (Fix)
```csharp
public async Task<UserDto> GetUserAsync(int id)
{
    var user = await _httpClient.GetAsync($"/api/users/{id}");
    var content = await user.Content.ReadAsStringAsync();
    return JsonSerializer.Deserialize<UserDto>(content);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `member_access_expression`, `identifier`
- **Detection approach**: Find `member_access_expression` where the member name is `Result` and the object is a `Task`-returning expression (invocation ending in `Async` or known async methods). Check if the enclosing method has `async` modifier.
- **S-expression query sketch**:
```scheme
(member_access_expression
  name: (identifier) @prop
  (#eq? @prop "Result"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `task_result_blocking`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: .Wait() / .WaitAll() on Task

### Description
Calling `.Wait()` on a `Task` or `Task.WaitAll()` / `Task.WaitAny()` synchronously blocks the calling thread, similar to `.Result`.

### Bad Code (Anti-pattern)
```csharp
public void ProcessBatch(List<int> ids)
{
    var tasks = ids.Select(id => ProcessItemAsync(id)).ToArray();
    Task.WaitAll(tasks);
}
```

### Good Code (Fix)
```csharp
public async Task ProcessBatch(List<int> ids)
{
    var tasks = ids.Select(id => ProcessItemAsync(id)).ToArray();
    await Task.WhenAll(tasks);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `identifier`
- **Detection approach**: Find `invocation_expression` calling `.Wait()` on a task variable, or `Task.WaitAll()`, `Task.WaitAny()` static calls. Also check for `GetAwaiter().GetResult()` chains.
- **S-expression query sketch**:
```scheme
(invocation_expression
  (member_access_expression
    name: (identifier) @method
    (#match? @method "^(Wait|WaitAll|WaitAny)$")))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `task_wait_blocking`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: async void Methods

### Description
Methods declared as `async void` instead of `async Task` cannot be awaited, swallow exceptions, and cause fire-and-forget behavior that is unpredictable under load.

### Bad Code (Anti-pattern)
```csharp
public async void SendNotification(string userId, string message)
{
    var user = await _userService.GetUserAsync(userId);
    await _emailService.SendAsync(user.Email, message);
}
```

### Good Code (Fix)
```csharp
public async Task SendNotification(string userId, string message)
{
    var user = await _userService.GetUserAsync(userId);
    await _emailService.SendAsync(user.Email, message);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `predefined_type`
- **Detection approach**: Find `method_declaration` nodes with both `async` modifier and `void` return type (via `predefined_type`). Exclude event handlers (methods with `object sender, EventArgs e` parameters).
- **S-expression query sketch**:
```scheme
(method_declaration
  (modifier) @mod
  type: (predefined_type) @return_type
  (#eq? @mod "async")
  (#eq? @return_type "void"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `async_void_method`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: Thread.Sleep() in Async Method

### Description
Using `Thread.Sleep()` inside an async method blocks the thread-pool thread instead of using `await Task.Delay()`.

### Bad Code (Anti-pattern)
```csharp
public async Task<string> RetryWithBackoff(Func<Task<string>> operation)
{
    for (int i = 0; i < 3; i++)
    {
        try { return await operation(); }
        catch { Thread.Sleep(1000 * (i + 1)); }
    }
    throw new Exception("All retries failed");
}
```

### Good Code (Fix)
```csharp
public async Task<string> RetryWithBackoff(Func<Task<string>> operation)
{
    for (int i = 0; i < 3; i++)
    {
        try { return await operation(); }
        catch { await Task.Delay(1000 * (i + 1)); }
    }
    throw new Exception("All retries failed");
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`
- **Detection approach**: Find `invocation_expression` calling `Thread.Sleep` inside a method with `async` modifier.
- **S-expression query sketch**:
```scheme
(invocation_expression
  (member_access_expression
    expression: (identifier) @class
    name: (identifier) @method)
  (#eq? @class "Thread")
  (#eq? @method "Sleep"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `thread_sleep_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 5: Console.ReadLine() in Async Method

### Description
Using `Console.ReadLine()` or `Console.Read()` inside an async method, which blocks the thread waiting for user input.

### Bad Code (Anti-pattern)
```csharp
public async Task RunInteractiveAsync()
{
    while (true)
    {
        var input = Console.ReadLine();
        await ProcessCommandAsync(input);
    }
}
```

### Good Code (Fix)
```csharp
public async Task RunInteractiveAsync()
{
    using var reader = new StreamReader(Console.OpenStandardInput());
    while (true)
    {
        var input = await reader.ReadLineAsync();
        await ProcessCommandAsync(input);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`
- **Detection approach**: Find `invocation_expression` calling `Console.ReadLine` or `Console.Read` inside a method with `async` modifier.
- **S-expression query sketch**:
```scheme
(invocation_expression
  (member_access_expression
    expression: (identifier) @class
    name: (identifier) @method)
  (#eq? @class "Console")
  (#eq? @method "ReadLine"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `console_read_in_async`
- **Severity**: info
- **Confidence**: high
