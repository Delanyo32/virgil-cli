# Path Traversal -- Rust

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks, or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: std::path, std::fs, actix-web, axum, warp, rocket, tokio::fs

---

## Pattern 1: User Input in File Path

### Description
Using `Path::new(base).join(user_input)` to build a file path from user-supplied input without calling `.canonicalize()` on the result and verifying it starts with the intended base directory.

### Bad Code (Anti-pattern)
```rust
use std::path::Path;
use std::fs;

fn serve_file(base: &str, user_input: &str) -> Result<String, std::io::Error> {
    let path = Path::new(base).join(user_input);
    fs::read_to_string(path)
}
```

### Good Code (Fix)
```rust
use std::path::Path;
use std::fs;
use std::io;

fn serve_file(base: &str, user_input: &str) -> Result<String, io::Error> {
    let base_dir = Path::new(base).canonicalize()?;
    let file_path = base_dir.join(user_input).canonicalize()?;
    if !file_path.starts_with(&base_dir) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "path escapes base directory"));
    }
    fs::read_to_string(file_path)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `identifier`, `string_literal`
- **Detection approach**: Find `call_expression` nodes invoking `.join()` on a `Path` or `PathBuf` where an argument comes from user input (function parameter, request extractor). Flag when the result is passed to `fs::read_to_string()`, `fs::read()`, `File::open()`, or similar without a preceding `.canonicalize()` + `.starts_with()` check.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    field: (field_identifier) @method)
  arguments: (arguments
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
Accepting file paths that contain `..` components without rejection or sanitization, allowing attackers to escape the intended directory.

### Bad Code (Anti-pattern)
```rust
use std::fs;

fn read_upload(filename: &str) -> Result<String, std::io::Error> {
    // No check for ".." — attacker sends "../../etc/passwd"
    let path = format!("./uploads/{}", filename);
    fs::read_to_string(path)
}
```

### Good Code (Fix)
```rust
use std::path::{Path, Component};
use std::fs;
use std::io;

fn read_upload(filename: &str) -> Result<String, io::Error> {
    let path = Path::new(filename);
    // Reject any path containing ".." components
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid filename"));
    }
    let base = Path::new("./uploads").canonicalize()?;
    let full_path = base.join(filename).canonicalize()?;
    if !full_path.starts_with(&base) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "path escapes base directory"));
    }
    fs::read_to_string(full_path)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `macro_invocation`, `call_expression`, `string_literal`, `identifier`
- **Detection approach**: Find `macro_invocation` nodes for `format!` where the format string contains a path pattern like `"./uploads/{}"` and the interpolated variable comes from user input. Flag when there is no preceding check for `..` via `.contains("..")`, `Component::ParentDir`, or `.canonicalize()` + `.starts_with()` validation.
- **S-expression query sketch**:
```scheme
(macro_invocation
  macro: (identifier) @macro_name
  (token_tree
    (string_literal) @format_str
    (identifier) @user_var))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
