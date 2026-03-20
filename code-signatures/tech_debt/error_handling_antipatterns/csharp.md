# Error Handling Anti-patterns -- C#

## Overview
Errors that are silently swallowed, broadly caught, or used for control flow make debugging impossible and hide real failures. In C#, empty catch blocks, catching the base `Exception` type, and using exceptions for expected conditions are the most common anti-patterns.

## Why It's a Tech Debt Concern
Empty catch blocks silently discard exceptions, allowing corrupted state and failed operations to go completely unnoticed. Catching the base `Exception` type inadvertently catches `NullReferenceException`, `InvalidOperationException`, `OutOfMemoryException`, and other critical failures that indicate bugs rather than expected error conditions. Using exceptions for control flow (e.g., `int.Parse` with catch instead of `int.TryParse`) adds unnecessary overhead and obscures the distinction between normal program flow and genuine error states.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cs`

---

## Pattern 1: Empty Catch

### Description
A `catch` block that contains no statements, or a bare `catch { }` without even specifying an exception type. The exception is caught and completely discarded, making failures invisible.

### Bad Code (Anti-pattern)
```csharp
public void SaveSettings(Settings settings)
{
    try
    {
        var json = JsonSerializer.Serialize(settings);
        File.WriteAllText(_settingsPath, json);
    }
    catch { }
}

public DbConnection GetConnection()
{
    try
    {
        return new SqlConnection(_connectionString);
    }
    catch (SqlException)
    {
        // Will fix later
    }
    return null;
}
```

### Good Code (Fix)
```csharp
public void SaveSettings(Settings settings)
{
    try
    {
        var json = JsonSerializer.Serialize(settings);
        File.WriteAllText(_settingsPath, json);
    }
    catch (IOException ex)
    {
        _logger.LogError(ex, "Failed to save settings to {Path}", _settingsPath);
        throw new SettingsException("Could not save settings", ex);
    }
    catch (JsonException ex)
    {
        _logger.LogError(ex, "Failed to serialize settings");
        throw new SettingsException("Could not serialize settings", ex);
    }
}

public DbConnection GetConnection()
{
    try
    {
        return new SqlConnection(_connectionString);
    }
    catch (SqlException ex)
    {
        _logger.LogError(ex, "Database connection failed");
        throw new DatabaseException("Could not establish connection", ex);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `block`
- **Detection approach**: Find `catch_clause` nodes whose body `block` has zero child statements. Also flag bare `catch { }` (catch clause with no `catch_declaration`). Check for blocks containing only comments as well.
- **S-expression query sketch**:
```scheme
(try_statement
  (catch_clause
    body: (block) @catch_body))

(try_statement
  (catch_clause
    (catch_declaration
      type: (identifier) @exception_type
      name: (identifier) @exception_var)
    body: (block) @catch_body))
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `empty_catch`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Catching Base Exception

### Description
Catching `Exception` or `System.Exception` instead of specific exception types. This catches all CLR exceptions including `NullReferenceException`, `StackOverflowException`, `AccessViolationException`, and other critical errors that indicate bugs, not expected failure modes.

### Bad Code (Anti-pattern)
```csharp
public async Task<UserProfile> GetProfileAsync(int userId)
{
    try
    {
        var response = await _httpClient.GetAsync($"/api/users/{userId}");
        var content = await response.Content.ReadAsStringAsync();
        return JsonSerializer.Deserialize<UserProfile>(content);
    }
    catch (Exception ex)
    {
        _logger.LogError(ex, "Failed to get profile");
        return null;
    }
}

public decimal CalculateTotal(Order order)
{
    try
    {
        return order.Items.Sum(i => i.Price * i.Quantity) + CalculateTax(order);
    }
    catch (Exception)
    {
        return 0m;
    }
}
```

### Good Code (Fix)
```csharp
public async Task<UserProfile> GetProfileAsync(int userId)
{
    try
    {
        var response = await _httpClient.GetAsync($"/api/users/{userId}");
        response.EnsureSuccessStatusCode();
        var content = await response.Content.ReadAsStringAsync();
        return JsonSerializer.Deserialize<UserProfile>(content);
    }
    catch (HttpRequestException ex)
    {
        _logger.LogError(ex, "HTTP request failed for user {UserId}", userId);
        throw new ProfileException($"Could not fetch profile for user {userId}", ex);
    }
    catch (JsonException ex)
    {
        _logger.LogError(ex, "Invalid JSON in profile response for user {UserId}", userId);
        throw new ProfileException($"Invalid profile data for user {userId}", ex);
    }
}

public decimal CalculateTotal(Order order)
{
    ArgumentNullException.ThrowIfNull(order);

    if (order.Items == null || order.Items.Count == 0)
        return 0m;

    return order.Items.Sum(i => i.Price * i.Quantity) + CalculateTax(order);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `catch_clause`, `catch_declaration`, `identifier`
- **Detection approach**: Find `catch_clause` nodes where the `catch_declaration` type is the identifier `Exception` or the qualified name `System.Exception`. Flag these as overly broad catches. Also flag bare `catch` without any declaration.
- **S-expression query sketch**:
```scheme
(catch_clause
  (catch_declaration
    type: (identifier) @exception_type))

(catch_clause
  (catch_declaration
    type: (qualified_name) @exception_type))
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `broad_exception_catch`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Exception for Control Flow

### Description
Using try/catch to handle expected conditions instead of checking preconditions or using `TryParse`/`TryGetValue` patterns. Common examples include `int.Parse` with catch instead of `int.TryParse`, dictionary access with `KeyNotFoundException` instead of `TryGetValue`, and type casting with `InvalidCastException` instead of `is`/`as`.

### Bad Code (Anti-pattern)
```csharp
public int ParseUserAge(string input)
{
    try
    {
        return int.Parse(input);
    }
    catch (FormatException)
    {
        return -1;
    }
}

public string GetSetting(string key)
{
    try
    {
        return _settings[key];
    }
    catch (KeyNotFoundException)
    {
        return string.Empty;
    }
}

public decimal GetAmount(object value)
{
    try
    {
        return (decimal)value;
    }
    catch (InvalidCastException)
    {
        return 0m;
    }
}
```

### Good Code (Fix)
```csharp
public int ParseUserAge(string input)
{
    if (int.TryParse(input, out int age) && age >= 0)
    {
        return age;
    }
    return -1;
}

public string GetSetting(string key)
{
    return _settings.TryGetValue(key, out var value) ? value : string.Empty;
}

public decimal GetAmount(object value)
{
    return value is decimal amount ? amount : 0m;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `catch_declaration`, `identifier`, `return_statement`
- **Detection approach**: Find `catch_clause` nodes where the caught type is `FormatException`, `KeyNotFoundException`, `InvalidCastException`, `ArgumentException`, or `OverflowException` and the catch body contains a `return_statement` returning a default/literal value. The short try body (1-2 statements) and specific exception types strongly indicate control-flow usage.
- **S-expression query sketch**:
```scheme
(try_statement
  body: (block) @try_body
  (catch_clause
    (catch_declaration
      type: (identifier) @exception_type)
    body: (block
      (return_statement
        (integer_literal) @default_value))))
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `exception_control_flow`
- **Severity**: info
- **Confidence**: medium
