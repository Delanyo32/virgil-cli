# Type Confusion -- C#

## Overview
C# provides both safe and unsafe casting mechanisms. The direct cast `(Type)obj` throws an `InvalidCastException` at runtime if the object is not of the expected type. C# offers safer alternatives -- `as` (returns `null` on failure) and `is` (boolean type check) -- but developers frequently use direct casts for brevity, especially with `object` parameters, deserialized data, or loosely typed collections.

## Why It's a Security Concern
An unguarded direct cast in a server application can crash a request handler, potentially causing denial of service if the exception is unhandled. When casting objects from deserialized payloads, session state, or dependency injection containers, an attacker who controls the object's runtime type can trigger `InvalidCastException` at security-critical points. The exception's stack trace and message may reveal internal type names, assembly versions, and code structure. In some cases, a failed cast in a finally block or disposal path can mask the original security exception and leave resources in an inconsistent state.

## Applicability
- **Relevance**: medium
- **Languages covered**: .cs
- **Frameworks/libraries**: ASP.NET Core, Entity Framework, System.Text.Json, Newtonsoft.Json, Unity (game engine), WCF

---

## Pattern 1: Invalid Cast Without as/is Check

### Description
Using a direct cast `(Type)obj` on an `object`, `dynamic`, or base-class reference without first verifying the runtime type using `is` or the null-safe `as` operator. This is dangerous when the source object originates from deserialization, HTTP context items, session state, cache entries, or any API that returns `object`.

### Bad Code (Anti-pattern)
```csharp
public class RequestProcessor
{
    public void ProcessRequest(HttpContext context)
    {
        // Direct cast -- throws InvalidCastException if wrong type
        var session = (UserSession)context.Items["session"];
        var userId = session.UserId;

        // Unsafe cast from deserialized object
        object payload = JsonSerializer.Deserialize<object>(requestBody);
        var config = (Dictionary<string, object>)payload;
        var apiKey = (string)config["key"];

        ExecuteAction(userId, apiKey);
    }

    public void HandleMessage(object message)
    {
        // Direct cast without type verification
        var command = (CommandMessage)message;
        command.Execute();
    }
}
```

### Good Code (Fix)
```csharp
public class RequestProcessor
{
    public void ProcessRequest(HttpContext context)
    {
        // Pattern matching with is -- safe and concise
        if (context.Items["session"] is not UserSession session)
        {
            context.Response.StatusCode = 401;
            return;
        }
        var userId = session.UserId;

        // Deserialize to a strongly typed object
        var config = JsonSerializer.Deserialize<RequestConfig>(requestBody);
        if (config is null || string.IsNullOrEmpty(config.Key))
        {
            context.Response.StatusCode = 400;
            return;
        }

        ExecuteAction(userId, config.Key);
    }

    public void HandleMessage(object message)
    {
        // Use pattern matching for safe type checking
        if (message is CommandMessage command)
        {
            command.Execute();
        }
        else
        {
            _logger.LogWarning("Unexpected message type: {Type}", message.GetType().Name);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `cast_expression`, `type_identifier`, `identifier`, `invocation_expression`
- **Detection approach**: Find `cast_expression` nodes (the `(Type)expr` syntax) where the target type is a reference type (not a value type like `int`, `float`, `bool`). Check whether the expression being cast is an `object`-typed variable, a method returning `object`, or a dictionary/collection indexer. Flag casts that lack a preceding `is` check or enclosing `try-catch` block within the same method. Exclude explicit numeric conversions and boxing/unboxing of known value types.
- **S-expression query sketch**:
```scheme
(cast_expression
  type: (_) @cast_type
  value: (_) @cast_value)
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `invalid_cast`
- **Severity**: warning
- **Confidence**: medium
