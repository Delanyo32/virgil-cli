# Injection -- Python

## Overview
Injection vulnerabilities in Python occur when untrusted input is embedded into SQL queries, shell commands, or dynamically evaluated code without proper sanitization. Python's flexible string formatting (f-strings, `.format()`, `%`) and powerful built-in functions (`eval`, `exec`, `os.system`) make it particularly susceptible to these patterns when used carelessly.

## Why It's a Security Concern
SQL injection can expose or corrupt entire databases. Command injection via `os.system()` or `subprocess.run(shell=True)` grants attackers arbitrary command execution on the server. Code injection through `eval()`/`exec()` allows running arbitrary Python code with the application's full privileges. These vulnerabilities routinely lead to complete system compromise.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: sqlite3, psycopg2, SQLAlchemy, Django ORM, subprocess, os, Flask, FastAPI

---

## Pattern 1: SQL Injection via f-string/format in cursor.execute()

### Description
Constructing SQL queries using f-strings, `.format()`, or `%` string formatting and passing the result to `cursor.execute()`. User-supplied values are interpolated directly into the query string instead of being passed as parameterized arguments.

### Bad Code (Anti-pattern)
```python
def get_user(cursor, user_id: str):
    query = f"SELECT * FROM users WHERE id = '{user_id}'"
    cursor.execute(query)
    return cursor.fetchone()

def search_products(cursor, name: str):
    cursor.execute("SELECT * FROM products WHERE name LIKE '%%{}%%'".format(name))
    return cursor.fetchall()
```

### Good Code (Fix)
```python
def get_user(cursor, user_id: str):
    cursor.execute("SELECT * FROM users WHERE id = %s", (user_id,))
    return cursor.fetchone()

def search_products(cursor, name: str):
    cursor.execute("SELECT * FROM products WHERE name LIKE %s", (f"%{name}%",))
    return cursor.fetchall()
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `string`, `formatted_string`, `binary_operator`
- **Detection approach**: Find `call` nodes where the function is an `attribute` ending in `execute` and the first argument is a `formatted_string` (f-string), a `call` to `.format()` on a string, or a `binary_operator` using `%` with a string left operand. Confirm SQL keywords in the string content.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    attribute: (identifier) @method)
  arguments: (argument_list
    (formatted_string) @sql_query))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_fstring_execute`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via os.system / subprocess with shell=True

### Description
Passing user-controlled strings to `os.system()`, `os.popen()`, or `subprocess.run()`/`subprocess.call()` with `shell=True`. The shell interprets metacharacters in the input, allowing command chaining and arbitrary execution.

### Bad Code (Anti-pattern)
```python
import os
import subprocess

def ping_host(hostname: str):
    os.system(f"ping -c 4 {hostname}")

def list_files(directory: str):
    subprocess.run(f"ls -la {directory}", shell=True)
```

### Good Code (Fix)
```python
import subprocess
import shlex

def ping_host(hostname: str):
    subprocess.run(["ping", "-c", "4", hostname], check=True)

def list_files(directory: str):
    subprocess.run(["ls", "-la", directory], check=True)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `argument_list`, `formatted_string`, `keyword_argument`
- **Detection approach**: Find `call` nodes where the function is `os.system`, `os.popen`, or a `subprocess` method (`run`, `call`, `Popen`, `check_output`) with a `keyword_argument` of `shell=True`. Check that the first positional argument is a `formatted_string`, a `.format()` call, or string concatenation containing variables.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  arguments: (argument_list
    (formatted_string) @cmd))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_os_system`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: Code Injection via eval/exec with User Input

### Description
Passing user-controlled strings to `eval()` or `exec()`, which execute arbitrary Python code. This is extremely dangerous when the input originates from HTTP requests, form fields, or any external source.

### Bad Code (Anti-pattern)
```python
from flask import request

@app.route('/calculate')
def calculate():
    expression = request.args.get('expr')
    result = eval(expression)
    return str(result)

def run_user_script(code: str):
    exec(code)
```

### Good Code (Fix)
```python
import ast
from flask import request

@app.route('/calculate')
def calculate():
    expression = request.args.get('expr')
    result = ast.literal_eval(expression)
    return str(result)

def run_user_script(code: str):
    # Use a sandboxed execution environment or whitelist allowed operations
    allowed_ops = {'+', '-', '*', '/'}
    tree = ast.parse(code, mode='eval')
    # Validate AST nodes before evaluation
    validate_expression(tree)
    result = eval(compile(tree, '<string>', 'eval'), {"__builtins__": {}})
    return result
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `identifier`, `argument_list`, `attribute`
- **Detection approach**: Find `call` nodes where the function is `eval` or `exec` and the first argument is not a string literal -- i.e., it is an `identifier`, `attribute` access (e.g., `request.args.get(...)`), `subscript`, or any other expression that could carry user input. Calls with only constant string arguments are lower risk.
- **S-expression query sketch**:
```scheme
(call
  function: (identifier) @func_name
  arguments: (argument_list
    (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `code_injection_eval`
- **Severity**: error
- **Confidence**: high
