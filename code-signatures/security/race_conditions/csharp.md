# Race Conditions -- C#

## Overview
C# applications commonly use multi-threading via `Task`, `Thread`, `async/await`, and parallel LINQ. Race conditions arise when shared state is accessed from multiple threads without proper synchronization, and when file system operations follow a check-then-act pattern without atomicity. The .NET runtime's memory model permits instruction reordering that can make unsynchronized shared access behave unexpectedly even on x86 architectures.

## Why It's a Security Concern
Race conditions in C# can corrupt shared data structures, produce incorrect authorization decisions in ASP.NET request pipelines, allow double-processing of payments, and enable cache poisoning in singleton services. TOCTOU races in file operations allow attackers to exploit the window between a `File.Exists()` check and a subsequent file write to redirect writes to sensitive locations via symlinks or junction points, particularly on Windows where NTFS junctions are easy to create.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Threading, System.IO, ASP.NET Core, Entity Framework, System.Collections.Concurrent

---

## Pattern 1: Race Condition in Shared State Access

### Description
Reading and writing shared fields or static variables from multiple threads (or async continuations) without `lock`, `Interlocked`, `SemaphoreSlim`, or concurrent collections. Common patterns include unsynchronized lazy initialization, shared counters without `Interlocked`, and accessing non-thread-safe collections like `Dictionary<K,V>` or `List<T>` from concurrent tasks.

### Bad Code (Anti-pattern)
```csharp
public class SessionTracker
{
    private static Dictionary<string, int> activeSessions = new();

    // Called from multiple ASP.NET request threads
    public void TrackSession(string userId)
    {
        // RACE: Dictionary is not thread-safe for concurrent writes
        if (!activeSessions.ContainsKey(userId))
        {
            activeSessions[userId] = 0;
        }
        activeSessions[userId]++;  // read-modify-write race
    }
}
```

### Good Code (Fix)
```csharp
using System.Collections.Concurrent;

public class SessionTracker
{
    private static readonly ConcurrentDictionary<string, int> activeSessions = new();

    public void TrackSession(string userId)
    {
        activeSessions.AddOrUpdate(userId, 1, (_, count) => count + 1);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `element_access_expression`, `assignment_expression`, `if_statement`
- **Detection approach**: Find class-level field declarations of `Dictionary<,>` or `List<>` (non-concurrent types) that are accessed inside methods not wrapped in `lock` statements. Specifically, detect `if (!dict.ContainsKey(...)) { dict[...] = ... }` patterns or `dict[key]++` compound operations on shared collections without enclosing `lock` blocks.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (prefix_unary_expression
    (invocation_expression
      (member_access_expression
        expression: (identifier) @dict
        name: (identifier) @method)
      (#eq? @method "ContainsKey")))
  (block
    (expression_statement
      (assignment_expression
        left: (element_access_expression
          expression: (identifier) @dict_assign)))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `shared_state_no_sync`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: TOCTOU in File Operations

### Description
Using `File.Exists()`, `Directory.Exists()`, or `FileInfo` to check file system state before performing an operation on the same path. Between the check and the operation, the target can be replaced with a symlink, junction, or modified by another process, making the check unreliable for security decisions.

### Bad Code (Anti-pattern)
```csharp
using System.IO;

public class ConfigWriter
{
    public void WriteConfig(string path, string content)
    {
        if (!File.Exists(path))
        {
            // RACE: file or symlink can be created between check and write
            File.WriteAllText(path, content);
        }
    }
}
```

### Good Code (Fix)
```csharp
using System.IO;

public class ConfigWriter
{
    public void WriteConfig(string path, string content)
    {
        try
        {
            // FileMode.CreateNew fails atomically if file already exists
            using var stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write);
            using var writer = new StreamWriter(stream);
            writer.Write(content);
        }
        catch (IOException) when (File.Exists(path))
        {
            // File already exists -- safe to skip
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `if_statement`, `identifier`
- **Detection approach**: Find `if_statement` nodes whose condition contains an `invocation_expression` calling `File.Exists`, `Directory.Exists`, or `File.GetAttributes`, where the body contains calls to `File.WriteAllText`, `File.Create`, `File.Delete`, `File.Move`, or `new FileStream()` with the same path variable. The non-atomic check-then-act sequence is the vulnerability indicator.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (invocation_expression
    (member_access_expression
      expression: (identifier) @type
      name: (identifier) @method)
    (#eq? @type "File")
    (#eq? @method "Exists"))
  (block
    (expression_statement
      (invocation_expression
        (member_access_expression
          expression: (identifier) @action_type
          name: (identifier) @action_method)))))
```

### Pipeline Mapping
- **Pipeline name**: `toctou`
- **Pattern name**: `file_exists_then_write`
- **Severity**: warning
- **Confidence**: high
