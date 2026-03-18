# Path Traversal -- PHP

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks (database credentials, API keys), or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: PHP built-in filesystem functions, Laravel, Symfony, WordPress

---

## Pattern 1: User Input in File Path

### Description
Concatenating a base directory with user-supplied input using string concatenation (`$base . '/' . $userInput`) or variable interpolation without resolving the real path via `realpath()` and verifying it starts with the intended base directory using `str_starts_with()`.

### Bad Code (Anti-pattern)
```php
function serveFile(string $userInput): string {
    $filePath = '/var/uploads/' . $userInput;
    return file_get_contents($filePath);
}
```

### Good Code (Fix)
```php
function serveFile(string $userInput): string {
    $baseDir = realpath('/var/uploads');
    $filePath = realpath($baseDir . '/' . $userInput);
    if ($filePath === false || !str_starts_with($filePath, $baseDir . DIRECTORY_SEPARATOR)) {
        throw new RuntimeException('Access denied: path escapes base directory');
    }
    return file_get_contents($filePath);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `binary_expression`, `encapsed_string`, `variable_name`
- **Detection approach**: Find `function_call_expression` nodes invoking `file_get_contents`, `fopen`, `include`, `require`, or similar, where the path argument is a binary expression (concatenation) or encapsed string that includes a variable from user input (e.g., `$_GET`, `$_POST`, `$_REQUEST`, function parameter). Flag when there is no preceding `realpath()` + `str_starts_with()` check.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (binary_expression
      left: (string) @path_prefix
      right: (variable_name) @user_var)))
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
```php
$filename = $_GET['file'];
// No check for ".." — attacker sends ?file=../../../etc/passwd
$content = file_get_contents("./public/$filename");
echo $content;
```

### Good Code (Fix)
```php
$filename = $_GET['file'];
if (str_contains($filename, '..')) {
    http_response_code(400);
    die('Invalid filename');
}
$baseDir = realpath('./public');
$filePath = realpath($baseDir . '/' . $filename);
if ($filePath === false || !str_starts_with($filePath, $baseDir . DIRECTORY_SEPARATOR)) {
    http_response_code(403);
    die('Forbidden');
}
$content = file_get_contents($filePath);
echo $content;
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `encapsed_string`, `variable_name`, `subscript_expression`
- **Detection approach**: Find `function_call_expression` nodes invoking `file_get_contents`, `fopen`, `readfile`, `include`, or `require`, where the path argument is an encapsed string containing a variable from user input (e.g., `$_GET['file']`). Flag when there is no preceding check for `'..'` via `str_contains()`, `strpos()`, or `realpath()` + `str_starts_with()` validation.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (encapsed_string
      (variable_name) @user_var)))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
