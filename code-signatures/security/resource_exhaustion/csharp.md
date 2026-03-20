# Resource Exhaustion -- C#

## Overview
Resource exhaustion vulnerabilities in C# arise from Regular Expression Denial of Service (ReDoS) when using `System.Text.RegularExpressions.Regex` without a timeout, and from unbounded memory allocation where user-controlled values determine the size of arrays, lists, or buffers. The .NET regex engine uses backtracking and is susceptible to catastrophic backtracking unless a `matchTimeout` is specified.

## Why It's a Security Concern
ReDoS in .NET can lock up ASP.NET request threads indefinitely because the default `Regex` constructor has no timeout (it runs until completion or process termination). This exhausts the thread pool, causing all subsequent requests to queue and time out. Unbounded allocation from user-controlled sizes allows attackers to exhaust server memory with requests specifying enormous array or buffer sizes, leading to `OutOfMemoryException` and application pool crashes.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Text.RegularExpressions, ASP.NET Core, ASP.NET MVC, System.Buffers

---

## Pattern 1: ReDoS -- Regex Without Timeout

### Description
Constructing a `Regex` object or calling `Regex.IsMatch()`, `Regex.Match()`, or `Regex.Replace()` without specifying a `matchTimeout` parameter. Even patterns that appear safe may have edge cases causing exponential backtracking. Without a timeout, a single malicious input can block a thread forever.

### Bad Code (Anti-pattern)
```csharp
using System.Text.RegularExpressions;
using Microsoft.AspNetCore.Mvc;

[ApiController]
public class ValidationController : ControllerBase
{
    // No timeout -- vulnerable to ReDoS
    private static readonly Regex EmailRegex =
        new Regex(@"^([a-zA-Z0-9_\.\-]+)+@([a-zA-Z0-9\-]+\.)+[a-zA-Z]{2,}$");

    [HttpPost("validate")]
    public IActionResult Validate([FromBody] string email)
    {
        if (EmailRegex.IsMatch(email))
            return Ok("Valid");
        return BadRequest("Invalid");
    }

    [HttpPost("search")]
    public IActionResult Search([FromBody] string query)
    {
        // Static method with no timeout
        bool hasMatch = Regex.IsMatch(query, @"(\w+\s*)+\d+");
        return Ok(hasMatch);
    }
}
```

### Good Code (Fix)
```csharp
using System;
using System.Text.RegularExpressions;
using Microsoft.AspNetCore.Mvc;

[ApiController]
public class ValidationController : ControllerBase
{
    // Timeout after 1 second, non-backtracking pattern
    private static readonly Regex EmailRegex =
        new Regex(
            @"^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$",
            RegexOptions.Compiled,
            TimeSpan.FromSeconds(1));

    [HttpPost("validate")]
    public IActionResult Validate([FromBody] string email)
    {
        if (string.IsNullOrEmpty(email) || email.Length > 254)
            return BadRequest("Invalid");
        try
        {
            if (EmailRegex.IsMatch(email))
                return Ok("Valid");
        }
        catch (RegexMatchTimeoutException)
        {
            return BadRequest("Validation timeout");
        }
        return BadRequest("Invalid");
    }

    [HttpPost("search")]
    public IActionResult Search([FromBody] string query)
    {
        bool hasMatch = Regex.IsMatch(
            query,
            @"(\w+\s*)+\d+",
            RegexOptions.None,
            TimeSpan.FromSeconds(1));
        return Ok(hasMatch);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `invocation_expression`, `argument_list`, `string_literal`, `member_access_expression`
- **Detection approach**: Find `object_creation_expression` nodes creating `new Regex(...)` where the constructor arguments do not include a `TimeSpan` parameter (the timeout overload takes 3 arguments: pattern, options, timeout). Also find static `invocation_expression` nodes calling `Regex.IsMatch()`, `Regex.Match()`, or `Regex.Replace()` where the argument list does not include a `TimeSpan` or `RegexOptions` with timeout. A two-argument `new Regex(pattern, options)` without timeout is also vulnerable.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (identifier) @type_name
  (argument_list
    (string_literal) @pattern))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `regex_no_timeout`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Unbounded Memory Allocation from User-Controlled Size

### Description
Using a value from user input (query parameters, request body fields, headers) as the size argument when allocating arrays (`new byte[size]`), lists (`new List<T>(capacity)`), or buffers (`ArrayPool.Rent(size)`) without validating against an upper bound. Attackers can supply extremely large values to exhaust server memory.

### Bad Code (Anti-pattern)
```csharp
using Microsoft.AspNetCore.Mvc;

[ApiController]
public class DataController : ControllerBase
{
    [HttpPost("allocate")]
    public IActionResult AllocateBuffer([FromQuery] int size)
    {
        // User controls allocation size -- could be int.MaxValue
        byte[] buffer = new byte[size];
        FillBuffer(buffer);
        return Ok(buffer.Length);
    }

    [HttpPost("batch")]
    public IActionResult ProcessBatch([FromBody] BatchRequest request)
    {
        // User controls list capacity
        var results = new List<string>(request.ExpectedCount);
        foreach (var item in request.Items)
        {
            results.Add(Process(item));
        }
        return Ok(results);
    }
}
```

### Good Code (Fix)
```csharp
using Microsoft.AspNetCore.Mvc;
using System.Buffers;

[ApiController]
public class DataController : ControllerBase
{
    private const int MaxBufferSize = 10 * 1024 * 1024; // 10 MB
    private const int MaxBatchSize = 10_000;

    [HttpPost("allocate")]
    public IActionResult AllocateBuffer([FromQuery] int size)
    {
        if (size <= 0 || size > MaxBufferSize)
            return BadRequest($"Size must be between 1 and {MaxBufferSize}");

        // Use ArrayPool for large allocations
        byte[] buffer = ArrayPool<byte>.Shared.Rent(size);
        try
        {
            FillBuffer(buffer.AsSpan(0, size));
            return Ok(size);
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer);
        }
    }

    [HttpPost("batch")]
    public IActionResult ProcessBatch([FromBody] BatchRequest request)
    {
        if (request.ExpectedCount > MaxBatchSize)
            return BadRequest($"Batch size exceeds maximum of {MaxBatchSize}");

        // Use actual item count, capped
        var results = new List<string>(Math.Min(request.Items.Count, MaxBatchSize));
        foreach (var item in request.Items)
        {
            results.Add(Process(item));
        }
        return Ok(results);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `array_creation_expression`, `invocation_expression`, `identifier`, `parameter`
- **Detection approach**: Find `array_creation_expression` nodes (`new byte[size]`, `new int[size]`) or `object_creation_expression` nodes (`new List<T>(capacity)`) where the size/capacity argument is an `identifier` matching a method parameter or a value extracted from request objects. Check for the absence of a preceding range validation (`if (size > MAX)` or `Math.Min()`) in the enclosing method body.
- **S-expression query sketch**:
```scheme
(array_creation_expression
  type: (predefined_type) @type
  (array_rank_specifier
    (identifier) @size))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_allocation_user_size`
- **Severity**: warning
- **Confidence**: medium
