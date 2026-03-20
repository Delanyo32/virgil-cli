# Path Traversal -- C#

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks (database credentials, API keys), or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.IO, ASP.NET Core, ASP.NET MVC, System.IO.Path

---

## Pattern 1: User Input in File Path

### Description
Using `Path.Combine(base, userInput)` to build a file path from user-supplied input without resolving the full path via `Path.GetFullPath()` and verifying it starts with the intended base directory.

### Bad Code (Anti-pattern)
```csharp
[HttpGet("files/{name}")]
public IActionResult GetFile(string name)
{
    var filePath = Path.Combine(_uploadsDir, name);
    var content = System.IO.File.ReadAllBytes(filePath);
    return File(content, "application/octet-stream");
}
```

### Good Code (Fix)
```csharp
[HttpGet("files/{name}")]
public IActionResult GetFile(string name)
{
    var baseDir = Path.GetFullPath(_uploadsDir);
    var filePath = Path.GetFullPath(Path.Combine(baseDir, name));
    if (!filePath.StartsWith(baseDir + Path.DirectorySeparatorChar))
    {
        return Forbid();
    }
    var content = System.IO.File.ReadAllBytes(filePath);
    return File(content, "application/octet-stream");
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `invocation_expression` nodes invoking `Path.Combine` where one argument originates from user input (e.g., controller method parameter, `Request.Query`). Flag when the result is passed to `File.ReadAllBytes()`, `File.ReadAllText()`, `new FileStream()`, or similar without a preceding `Path.GetFullPath()` + `.StartsWith()` check.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (member_access_expression
    expression: (identifier) @type
    name: (identifier) @method)
  arguments: (argument_list
    (_)
    (identifier) @user_input))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `user_input_in_file_path`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Directory Traversal via ../

### Description
Accepting file paths that contain `../` or `..\\` sequences without rejection or sanitization, allowing attackers to escape the intended directory.

### Bad Code (Anti-pattern)
```csharp
[HttpGet("download")]
public IActionResult Download([FromQuery] string file)
{
    // No check for ".." — attacker sends ?file=..\..\..\windows\system.ini
    var content = System.IO.File.ReadAllText($"./public/{file}");
    return Content(content);
}
```

### Good Code (Fix)
```csharp
[HttpGet("download")]
public IActionResult Download([FromQuery] string file)
{
    if (file.Contains(".."))
    {
        return BadRequest("Invalid filename");
    }
    var baseDir = Path.GetFullPath("./public");
    var filePath = Path.GetFullPath(Path.Combine(baseDir, file));
    if (!filePath.StartsWith(baseDir + Path.DirectorySeparatorChar))
    {
        return Forbid();
    }
    var content = System.IO.File.ReadAllText(filePath);
    return Content(content);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `interpolated_string_expression`, `member_access_expression`, `identifier`
- **Detection approach**: Find `invocation_expression` nodes calling `File.ReadAllText`, `File.ReadAllBytes`, `File.OpenRead`, or similar, where the path argument is an interpolated string that includes a variable from user input. Flag when there is no preceding check for `".."` via `.Contains("..")` or `Path.GetFullPath()` + `.StartsWith()` validation.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (member_access_expression
    expression: (identifier) @type
    name: (identifier) @method)
  arguments: (argument_list
    (interpolated_string_expression
      (interpolation
        (identifier) @user_var))))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
