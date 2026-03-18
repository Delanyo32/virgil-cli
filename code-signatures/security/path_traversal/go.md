# Path Traversal -- Go

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks, or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: os, path/filepath, io/ioutil, net/http, gin, echo, chi

---

## Pattern 1: User Input in File Path

### Description
Using `filepath.Join(base, userInput)` to build a file path from user-supplied input without resolving the absolute path via `filepath.Abs()` and verifying it starts with the intended base directory using `strings.HasPrefix()`.

### Bad Code (Anti-pattern)
```go
func serveFile(w http.ResponseWriter, r *http.Request) {
    filename := r.URL.Query().Get("name")
    filePath := filepath.Join("./uploads", filename)
    data, err := os.ReadFile(filePath)
    if err != nil {
        http.Error(w, "Not found", 404)
        return
    }
    w.Write(data)
}
```

### Good Code (Fix)
```go
func serveFile(w http.ResponseWriter, r *http.Request) {
    filename := r.URL.Query().Get("name")
    baseDir, _ := filepath.Abs("./uploads")
    filePath, _ := filepath.Abs(filepath.Join(baseDir, filename))
    if !strings.HasPrefix(filePath, baseDir+string(os.PathSeparator)) {
        http.Error(w, "Forbidden", 403)
        return
    }
    data, err := os.ReadFile(filePath)
    if err != nil {
        http.Error(w, "Not found", 404)
        return
    }
    w.Write(data)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes invoking `filepath.Join` where one argument originates from user input (e.g., `r.URL.Query().Get()`, `r.FormValue()`, `c.Param()`). Flag when the result is passed to `os.ReadFile()`, `os.Open()`, or similar without a preceding `filepath.Abs()` + `strings.HasPrefix()` check.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @method)
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
```go
func downloadHandler(w http.ResponseWriter, r *http.Request) {
    filename := r.URL.Query().Get("file")
    // No check for ".." — attacker sends ?file=../../../etc/passwd
    data, err := os.ReadFile("./public/" + filename)
    if err != nil {
        http.Error(w, "Not found", 404)
        return
    }
    w.Write(data)
}
```

### Good Code (Fix)
```go
func downloadHandler(w http.ResponseWriter, r *http.Request) {
    filename := r.URL.Query().Get("file")
    if strings.Contains(filename, "..") {
        http.Error(w, "Invalid filename", 400)
        return
    }
    baseDir, _ := filepath.Abs("./public")
    filePath, _ := filepath.Abs(filepath.Join(baseDir, filename))
    if !strings.HasPrefix(filePath, baseDir+string(os.PathSeparator)) {
        http.Error(w, "Forbidden", 403)
        return
    }
    data, err := os.ReadFile(filePath)
    if err != nil {
        http.Error(w, "Not found", 404)
        return
    }
    w.Write(data)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `binary_expression`, `selector_expression`, `interpreted_string_literal`
- **Detection approach**: Find `call_expression` nodes invoking `os.ReadFile`, `os.Open`, `os.Create`, or similar, where the path argument is a string concatenation that includes a variable from user input (request parameters). Flag when there is no preceding check for `".."` via `strings.Contains()` or `filepath.Abs()` + `strings.HasPrefix()` validation.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @method)
  arguments: (argument_list
    (binary_expression
      left: (interpreted_string_literal) @path_prefix
      right: (identifier) @user_var)))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
