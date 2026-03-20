# Dead Code -- C#

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. It also increases compilation time and complicates refactoring. C#'s compiler warns on some unreachable code (CS0162) and unused variables (CS0219), but unused private methods and commented-out code are not caught.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Private Method

### Description
A private method defined but never called from anywhere in the class. The C# compiler does not warn on unused private methods.

### Bad Code (Anti-pattern)
```csharp
public class ReportGenerator
{
    private string FormatCurrency(decimal amount)
    {
        return amount.ToString("C2", CultureInfo.GetCultureInfo("en-US"));
    }

    private string FormatLegacyCurrency(decimal amount)
    {
        return "$" + Math.Round(amount, 2).ToString("F2");
    }

    public string GenerateReport(IEnumerable<Transaction> transactions)
    {
        var total = transactions.Sum(t => t.Amount);
        return $"Total: {FormatCurrency(total)}";
    }
}
```

### Good Code (Fix)
```csharp
public class ReportGenerator
{
    private string FormatCurrency(decimal amount)
    {
        return amount.ToString("C2", CultureInfo.GetCultureInfo("en-US"));
    }

    public string GenerateReport(IEnumerable<Transaction> transactions)
    {
        var total = transactions.Sum(t => t.Amount);
        return $"Total: {FormatCurrency(total)}";
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`
- **Detection approach**: Collect all private method definitions (methods whose `modifier` children include `private`). Cross-reference with all `invocation_expression` nodes within the same class. Private methods with zero references are candidates. Exclude methods with `[TestMethod]`, `[Fact]`, `[UsedImplicitly]`, `[OnDeserialized]` attributes, event handlers matching `void OnX(object sender, EventArgs e)` signatures, and methods called via reflection or delegates.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    (modifier) @mod
    name: (identifier) @method_name
    (#eq? @mod "private"))
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Throw

### Description
Code statements that appear after an unconditional return, throw, break, or continue — they can never execute. The C# compiler emits warning CS0162 for some cases, but patterns involving `Environment.Exit()` or `Application.Exit()` are not caught.

### Bad Code (Anti-pattern)
```csharp
public string GetConnectionString(string env)
{
    switch (env)
    {
        case "production":
            return _config["ProdDb"];
        case "staging":
            return _config["StagingDb"];
        default:
            throw new ArgumentException($"Unknown environment: {env}");
            return _config["LocalDb"]; // unreachable
    }
}

public void FatalError(string message)
{
    _logger.LogCritical(message);
    Environment.Exit(1);
    _logger.LogInformation("Application terminated gracefully"); // unreachable
}
```

### Good Code (Fix)
```csharp
public string GetConnectionString(string env)
{
    switch (env)
    {
        case "production":
            return _config["ProdDb"];
        case "staging":
            return _config["StagingDb"];
        default:
            throw new ArgumentException($"Unknown environment: {env}");
    }
}

public void FatalError(string message)
{
    _logger.LogCritical(message);
    Environment.Exit(1);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `throw_statement`, `break_statement`, `continue_statement`, `expression_statement` (for `Environment.Exit()`)
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `block`. C#'s compiler catches most trivial cases, but `Environment.Exit()`, `Application.Exit()`, and methods attributed with `[DoesNotReturn]` are not treated as diverging by the parser. Flag those patterns specifically.
- **S-expression query sketch**:
  ```scheme
  (block
    (return_statement) @exit
    .
    (_) @unreachable)
  (block
    (throw_statement) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```csharp
public class UserRepository
{
    public async Task<User> GetByIdAsync(int id)
    {
        // private async Task<User> GetByIdFromCache(int id)
        // {
        //     var cacheKey = $"user:{id}";
        //     var cached = await _cache.GetStringAsync(cacheKey);
        //     if (cached != null)
        //     {
        //         return JsonSerializer.Deserialize<User>(cached);
        //     }
        //     return null;
        // }
        //
        // var user = await GetByIdFromCache(id);
        // if (user != null) return user;

        return await _context.Users.FindAsync(id);
    }
}
```

### Good Code (Fix)
```csharp
public class UserRepository
{
    public async Task<User> GetByIdAsync(int id)
    {
        return await _context.Users.FindAsync(id);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment`
- **Detection approach**: Find comment nodes whose content matches C# code patterns (contains `public `, `private `, `var `, `await `, `return `, `new `, `if (`, `foreach (`, `async `, semicolons at end of lines). Flag blocks of 5+ consecutive comment lines that look like code. Distinguish from XML doc comments (`///`), region markers (`#region`/`#endregion`), and pragma directives.
- **S-expression query sketch**:
  ```scheme
  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
