# Resource Exhaustion -- Python

## Overview
Resource exhaustion vulnerabilities in Python arise when applications allow uncontrolled consumption of CPU or memory based on user input. The primary vectors are Regular Expression Denial of Service (ReDoS) -- where the `re` module's backtracking engine chokes on crafted input -- and reading entire request bodies or files into memory without size constraints, enabling memory exhaustion attacks.

## Why It's a Security Concern
Python's `re` module uses a backtracking NFA engine susceptible to catastrophic backtracking on regex patterns with nested quantifiers. A single crafted request can pin a worker process for minutes or hours. Unbounded memory reads allow attackers to exhaust server RAM with oversized uploads or request bodies, causing OOM kills that crash the entire application. Both attacks are trivial to execute and devastate service availability.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: re, Flask, Django, FastAPI, aiohttp, requests

---

## Pattern 1: ReDoS -- Regex with Nested Quantifiers on User Input

### Description
Using `re.match()`, `re.search()`, `re.fullmatch()`, or `re.sub()` with a pattern containing nested quantifiers (e.g., `(a+)+`, `(\w+\s*)+`, `([\w.]+)+`) against user-supplied input. The Python `re` module is vulnerable to exponential backtracking on these patterns, causing the process to hang.

### Bad Code (Anti-pattern)
```python
import re
from flask import request

@app.route('/validate')
def validate_input():
    user_input = request.args.get('data', '')
    # Nested quantifiers: ([\w.]+)+ causes catastrophic backtracking
    pattern = r'^([\w.]+)+@([\w.]+)+\.\w+$'
    if re.match(pattern, user_input):
        return 'Valid', 200
    return 'Invalid', 400

def parse_log_line(line: str) -> dict:
    # Overlapping repetition in group
    match = re.search(r'(\s*\w+\s*=\s*"[^"]*"\s*)+', line)
    if match:
        return {'attrs': match.group(0)}
    return {}
```

### Good Code (Fix)
```python
import re
from flask import request

@app.route('/validate')
def validate_input():
    user_input = request.args.get('data', '')
    # Enforce input length limit before regex
    if len(user_input) > 254:
        return 'Invalid', 400
    # Non-backtracking pattern without nested quantifiers
    pattern = r'^[\w.]+@[\w.]+\.\w+$'
    if re.match(pattern, user_input):
        return 'Valid', 200
    return 'Invalid', 400

def parse_log_line(line: str) -> dict:
    # Use findall for individual key-value pairs instead of nested groups
    pairs = re.findall(r'\w+=("[^"]*")', line)
    return {'attrs': pairs}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `argument_list`, `string`, `identifier`
- **Detection approach**: Find `call` nodes where the function is `re.match`, `re.search`, `re.fullmatch`, `re.sub`, or `re.compile`. Extract the first argument (the pattern string). Analyze the pattern for nested quantifiers -- groups containing `+` or `*` that are themselves followed by `+` or `*`. Flag when the input argument is not a constant.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  arguments: (argument_list
    (string) @pattern
    (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `regex_nested_quantifiers`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Reading Entire Request/File into Memory Without Size Limit

### Description
Reading an entire HTTP request body, uploaded file, or file stream into memory without checking or limiting its size first. Attackers can send multi-gigabyte payloads that exhaust the server's available memory, triggering OOM kills and crashing all co-hosted processes.

### Bad Code (Anti-pattern)
```python
from flask import request

@app.route('/upload', methods=['POST'])
def upload():
    # Reads entire request body into memory regardless of size
    data = request.get_data()
    process(data)
    return 'OK', 200

def read_user_file(filepath: str) -> bytes:
    # No size check -- user-controlled path could point to huge file
    with open(filepath, 'rb') as f:
        return f.read()

@app.route('/stream', methods=['POST'])
def stream_upload():
    # Accumulates all chunks with no upper bound
    chunks = []
    while True:
        chunk = request.stream.read(8192)
        if not chunk:
            break
        chunks.append(chunk)
    body = b''.join(chunks)
    return 'OK', 200
```

### Good Code (Fix)
```python
from flask import request
from werkzeug.exceptions import RequestEntityTooLarge
import os

MAX_UPLOAD_SIZE = 10 * 1024 * 1024  # 10 MB

@app.route('/upload', methods=['POST'])
def upload():
    content_length = request.content_length
    if content_length is not None and content_length > MAX_UPLOAD_SIZE:
        raise RequestEntityTooLarge()
    data = request.get_data(cache=False)
    if len(data) > MAX_UPLOAD_SIZE:
        raise RequestEntityTooLarge()
    process(data)
    return 'OK', 200

def read_user_file(filepath: str, max_size: int = MAX_UPLOAD_SIZE) -> bytes:
    file_size = os.path.getsize(filepath)
    if file_size > max_size:
        raise ValueError(f"File too large: {file_size} bytes")
    with open(filepath, 'rb') as f:
        return f.read(max_size)

@app.route('/stream', methods=['POST'])
def stream_upload():
    chunks = []
    total_size = 0
    while True:
        chunk = request.stream.read(8192)
        if not chunk:
            break
        total_size += len(chunk)
        if total_size > MAX_UPLOAD_SIZE:
            raise RequestEntityTooLarge()
        chunks.append(chunk)
    body = b''.join(chunks)
    return 'OK', 200
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `argument_list`, `identifier`
- **Detection approach**: Find `call` nodes invoking `request.get_data()`, `request.stream.read()`, `f.read()`, or `open(...).read()` without a preceding size check. Look for the absence of `content_length` comparison or `os.path.getsize()` guard in the enclosing function body. Also detect accumulation loops (`while`/`for` appending chunks) that lack a total-size counter check.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @obj
    attribute: (identifier) @method)
  arguments: (argument_list))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_memory_read`
- **Severity**: warning
- **Confidence**: medium
