# Magic Values -- C#

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```csharp
public class RequestHandler
{
    public Response ProcessRequest(byte[] data)
    {
        if (data.Length > 1024)
        {
            return new Response(413);
        }
        for (int i = 0; i < 3; i++)
        {
            Thread.Sleep(86400 * 1000);
        }
        if (response.StatusCode == 200)
        {
            cache.Set(key, data, TimeSpan.FromSeconds(3600));
        }
        else if (response.StatusCode == 404)
        {
            return Response.NotFound();
        }
        return Response.Ok();
    }
}
```

### Good Code (Fix)
```csharp
public class RequestHandler
{
    private const int MaxPayloadSize = 1024;
    private const int MaxRetries = 3;
    private const int SecondsPerDay = 86400;
    private const int MsPerSecond = 1000;
    private const int HttpOk = 200;
    private const int HttpNotFound = 404;
    private const int CacheTtlSeconds = 3600;

    public Response ProcessRequest(byte[] data)
    {
        if (data.Length > MaxPayloadSize)
        {
            return new Response(HttpPayloadTooLarge);
        }
        for (int i = 0; i < MaxRetries; i++)
        {
            Thread.Sleep(SecondsPerDay * MsPerSecond);
        }
        if (response.StatusCode == HttpOk)
        {
            cache.Set(key, data, TimeSpan.FromSeconds(CacheTtlSeconds));
        }
        else if (response.StatusCode == HttpNotFound)
        {
            return Response.NotFound();
        }
        return Response.Ok();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `integer_literal`, `real_literal` (excludes 0, 1, -1)
- **Detection approach**: Find `integer_literal` and `real_literal` nodes in expressions. Exclude literals inside `field_declaration` ancestors that have a `const` modifier, `enum_member_declaration` ancestors, and `attribute` ancestors. Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
[(integer_literal) @number (real_literal) @number]
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```csharp
public class UserService
{
    public void HandleUser(User user)
    {
        if (user.Role == "admin")
        {
            GrantAccess("dashboard");
        }
        if (user.Status == "active" || user.Status == "pending")
        {
            Notify(user);
        }
        var dbUrl = config["database_url"];
        var mode = settings["production"];
    }
}
```

### Good Code (Fix)
```csharp
public class UserService
{
    private const string RoleAdmin = "admin";
    private const string StatusActive = "active";
    private const string StatusPending = "pending";
    private const string ConfigDatabaseUrl = "database_url";
    private const string ConfigMode = "production";

    public void HandleUser(User user)
    {
        if (user.Role == RoleAdmin)
        {
            GrantAccess("dashboard");
        }
        if (user.Status == StatusActive || user.Status == StatusPending)
        {
            Notify(user);
        }
        var dbUrl = config[ConfigDatabaseUrl];
        var mode = settings[ConfigMode];
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string_literal` in `binary_expression` (equality checks) or `element_access_expression` (indexer access)
- **Detection approach**: Find `string_literal` nodes used in equality comparisons (`==`, `!=`) or as indexer arguments in `element_access_expression`. Exclude logging strings, `nameof()` arguments, attribute parameters, and interpolated strings. Flag repeated identical strings across a method or class.
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: ["==" "!="]
  [left: (string_literal) @string_lit
   right: (string_literal) @string_lit])

(element_access_expression
  (bracketed_argument_list
    (argument
      (string_literal) @string_lit)))
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
