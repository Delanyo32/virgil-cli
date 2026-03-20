# Injection -- C#

## Overview
Injection vulnerabilities in C# occur when untrusted input is embedded into SQL commands, shell process arguments, or LDAP queries through string interpolation or concatenation. C#'s string interpolation (`$"..."`) and convenient `Process.Start` API make it easy to inadvertently introduce injection vectors, especially when developers bypass parameterized APIs like `SqlParameter` or structured query builders.

## Why It's a Security Concern
SQL injection through `SqlCommand` with interpolated strings can lead to complete database compromise. Command injection via `Process.Start()` allows attackers to execute arbitrary system commands. LDAP injection can bypass authentication or extract directory information. These vulnerabilities are critical in ASP.NET web applications and Windows services that process external input.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Data.SqlClient, Microsoft.Data.SqlClient, Entity Framework, System.Diagnostics.Process, System.DirectoryServices

---

## Pattern 1: SQL Injection via String Interpolation in SqlCommand

### Description
Using C# string interpolation (`$"..."`) or `String.Format` to embed user input directly into SQL command text passed to `SqlCommand`. This bypasses the parameterized query mechanism (`@param` placeholders with `SqlParameter`).

### Bad Code (Anti-pattern)
```csharp
public User GetUser(SqlConnection conn, string userId)
{
    var query = $"SELECT * FROM Users WHERE Id = '{userId}'";
    using var cmd = new SqlCommand(query, conn);
    using var reader = cmd.ExecuteReader();
    if (reader.Read())
    {
        return new User { Id = reader.GetString(0), Name = reader.GetString(1) };
    }
    return null;
}
```

### Good Code (Fix)
```csharp
public User GetUser(SqlConnection conn, string userId)
{
    var query = "SELECT * FROM Users WHERE Id = @userId";
    using var cmd = new SqlCommand(query, conn);
    cmd.Parameters.AddWithValue("@userId", userId);
    using var reader = cmd.ExecuteReader();
    if (reader.Read())
    {
        return new User { Id = reader.GetString(0), Name = reader.GetString(1) };
    }
    return null;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `interpolated_string_expression`, `argument_list`, `identifier`
- **Detection approach**: Find `object_creation_expression` nodes creating `SqlCommand` where the first argument is an `interpolated_string_expression` or a variable assigned from string concatenation/interpolation containing SQL keywords. Also detect `assignment_expression` nodes setting `.CommandText` to an interpolated string.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (identifier) @class_name
  (argument_list
    (interpolated_string_expression) @sql_query))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_interpolation_sqlcommand`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via Process.Start with User Input

### Description
Passing user-controlled strings as arguments to `Process.Start()` or `ProcessStartInfo`, especially when `UseShellExecute` is true or when user input is interpolated into the `Arguments` property. Attackers can inject additional commands or arguments.

### Bad Code (Anti-pattern)
```csharp
public string ConvertFile(string userFilename)
{
    var process = new Process
    {
        StartInfo = new ProcessStartInfo
        {
            FileName = "convert",
            Arguments = $"{userFilename} output.png",
            RedirectStandardOutput = true,
            UseShellExecute = false
        }
    };
    process.Start();
    return process.StandardOutput.ReadToEnd();
}

public void OpenUrl(string url)
{
    Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
}
```

### Good Code (Fix)
```csharp
public string ConvertFile(string userFilename)
{
    // Validate filename against allowed patterns
    if (!Regex.IsMatch(userFilename, @"^[\w\-. ]+$"))
        throw new ArgumentException("Invalid filename");

    var process = new Process
    {
        StartInfo = new ProcessStartInfo
        {
            FileName = "convert",
            RedirectStandardOutput = true,
            UseShellExecute = false
        }
    };
    process.StartInfo.ArgumentList.Add(userFilename);
    process.StartInfo.ArgumentList.Add("output.png");
    process.Start();
    return process.StandardOutput.ReadToEnd();
}

public void OpenUrl(string url)
{
    var uri = new Uri(url);
    if (uri.Scheme != "https")
        throw new ArgumentException("Only HTTPS URLs are allowed");
    Process.Start(new ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `assignment_expression`, `interpolated_string_expression`, `member_access_expression`
- **Detection approach**: Find `object_creation_expression` nodes creating `ProcessStartInfo` where the `Arguments` property is set to an `interpolated_string_expression` or string concatenation containing variables. Also detect `Process.Start()` calls where the argument is a variable (not a validated literal). Check for `UseShellExecute = true` as an aggravating factor.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (member_access_expression
    name: (identifier) @prop)
  right: (interpolated_string_expression) @args)
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_process_start`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: LDAP Injection

### Description
Constructing LDAP filter strings by concatenating or interpolating user input without escaping. Attackers can modify the LDAP query logic to bypass authentication, enumerate users, or extract directory information by injecting LDAP filter metacharacters like `*`, `(`, `)`, `\`, and `|`.

### Bad Code (Anti-pattern)
```csharp
public SearchResult FindUser(string username)
{
    var searcher = new DirectorySearcher
    {
        Filter = $"(&(objectClass=user)(sAMAccountName={username}))"
    };
    return searcher.FindOne();
}
```

### Good Code (Fix)
```csharp
public SearchResult FindUser(string username)
{
    // Escape LDAP special characters
    string escapedUsername = EscapeLdapFilterValue(username);
    var searcher = new DirectorySearcher
    {
        Filter = $"(&(objectClass=user)(sAMAccountName={escapedUsername}))"
    };
    return searcher.FindOne();
}

private static string EscapeLdapFilterValue(string value)
{
    var sb = new StringBuilder();
    foreach (char c in value)
    {
        switch (c)
        {
            case '\\': sb.Append("\\5c"); break;
            case '*': sb.Append("\\2a"); break;
            case '(': sb.Append("\\28"); break;
            case ')': sb.Append("\\29"); break;
            case '\0': sb.Append("\\00"); break;
            default: sb.Append(c); break;
        }
    }
    return sb.ToString();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression`, `member_access_expression`, `interpolated_string_expression`, `object_creation_expression`
- **Detection approach**: Find assignments to a `Filter` property on `DirectorySearcher` or `DirectoryEntry` where the value is an `interpolated_string_expression` containing LDAP filter syntax (`objectClass`, `sAMAccountName`, `cn=`, etc.) with embedded interpolation expressions. The presence of LDAP filter operators (`&`, `|`, `!`) in the string confirms the pattern.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (member_access_expression
    name: (identifier) @prop)
  right: (interpolated_string_expression
    (interpolation) @user_input))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `ldap_injection_filter`
- **Severity**: error
- **Confidence**: high
