# Path Traversal -- Python

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks (database credentials, API keys), or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: os, os.path, pathlib, Flask, Django, FastAPI

---

## Pattern 1: User Input in File Path

### Description
Using `os.path.join()` to combine a base directory with user-supplied input without resolving the real path via `os.path.realpath()` and verifying the result starts with the intended base directory.

### Bad Code (Anti-pattern)
```python
import os

def serve_file(base_dir, user_input):
    file_path = os.path.join(base_dir, user_input)
    with open(file_path, 'r') as f:
        return f.read()
```

### Good Code (Fix)
```python
import os

def serve_file(base_dir, user_input):
    base_dir = os.path.realpath(base_dir)
    file_path = os.path.realpath(os.path.join(base_dir, user_input))
    if not file_path.startswith(base_dir + os.sep):
        raise ValueError("Access denied: path escapes base directory")
    with open(file_path, 'r') as f:
        return f.read()
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`, `argument_list`
- **Detection approach**: Find `call` nodes invoking `os.path.join` where one argument is a variable from user input (e.g., function parameter, request attribute). Flag when the return value is passed to `open()`, `os.read()`, or similar without a preceding `os.path.realpath()` + `.startswith()` guard.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (attribute
      object: (identifier) @os
      attribute: (identifier) @path_mod)
    attribute: (identifier) @join_method)
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
```python
from flask import request, send_file

@app.route('/download')
def download():
    filename = request.args.get('file')
    # No check for ".." — attacker sends ?file=../../../etc/passwd
    return send_file(f'./uploads/{filename}')
```

### Good Code (Fix)
```python
import os
from flask import request, send_file, abort

@app.route('/download')
def download():
    filename = request.args.get('file')
    if '..' in filename:
        abort(400, 'Invalid filename')
    base_dir = os.path.realpath('./uploads')
    file_path = os.path.realpath(os.path.join(base_dir, filename))
    if not file_path.startswith(base_dir + os.sep):
        abort(403, 'Forbidden')
    return send_file(file_path)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `string`, `formatted_string`, `identifier`
- **Detection approach**: Find `call` nodes invoking `open()`, `send_file()`, `os.path.join()`, or similar, where the path argument is an f-string or concatenation containing a variable from user input, and there is no preceding check for `'..'` via `in` operator or `os.path.realpath()` validation.
- **S-expression query sketch**:
```scheme
(call
  function: (identifier) @func_name
  arguments: (argument_list
    (formatted_string
      (interpolation
        (identifier) @user_var))))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
