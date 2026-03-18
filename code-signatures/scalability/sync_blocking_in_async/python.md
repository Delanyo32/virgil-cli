# Sync Blocking in Async -- Python

## Overview
Synchronous blocking in Python async contexts occurs when blocking I/O operations, `time.sleep()`, or synchronous HTTP libraries are used inside `async def` functions, blocking the asyncio event loop thread and starving all other coroutines.

## Why It's a Scalability Concern
Python's asyncio event loop runs in a single thread. A blocking call inside a coroutine halts the entire loop — no other coroutines, callbacks, or I/O completions can proceed until the blocking call returns. This turns concurrent async code into sequential execution, negating the scalability benefits of asyncio.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: asyncio, aiohttp, FastAPI, Starlette, httpx, aiofiles

---

## Pattern 1: Blocking File I/O in Async Def

### Description
Using synchronous `open()`, `os.read()`, `os.write()`, or `pathlib.Path.read_text()` inside an `async def` function instead of async alternatives like `aiofiles`.

### Bad Code (Anti-pattern)
```python
async def read_config():
    with open("/etc/app/config.json") as f:
        return json.load(f)
```

### Good Code (Fix)
```python
async def read_config():
    async with aiofiles.open("/etc/app/config.json") as f:
        content = await f.read()
        return json.loads(content)
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` (with `async` keyword), `call`, `identifier`, `with_statement`
- **Detection approach**: Find `call` nodes invoking `open`, `os.read`, `os.write`, `os.path.exists`, `Path.read_text` inside a function marked as `async`. Check the function definition for the `async` keyword.
- **S-expression query sketch**:
```scheme
(function_definition
  "async"
  body: (block
    (with_statement
      (with_clause
        (with_item
          value: (call
            function: (identifier) @func_name))))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_file_io_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: time.sleep() in Async Def

### Description
Using `time.sleep()` inside an `async def` function instead of `await asyncio.sleep()`, which blocks the entire event loop for the sleep duration.

### Bad Code (Anti-pattern)
```python
async def retry_with_backoff(func, retries=3):
    for attempt in range(retries):
        try:
            return await func()
        except Exception:
            time.sleep(2 ** attempt)  # blocks entire event loop
```

### Good Code (Fix)
```python
async def retry_with_backoff(func, retries=3):
    for attempt in range(retries):
        try:
            return await func()
        except Exception:
            await asyncio.sleep(2 ** attempt)
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `call`, `attribute`
- **Detection approach**: Find `call` where the function is `attribute` with `time.sleep` inside a function with `async` keyword.
- **S-expression query sketch**:
```scheme
(function_definition
  "async"
  body: (block
    (expression_statement
      (call
        function: (attribute
          object: (identifier) @module
          attribute: (identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `time_sleep_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Synchronous HTTP Library in Async Def

### Description
Using `requests.get()`, `requests.post()`, or `urllib.request.urlopen()` inside an `async def` function instead of async HTTP libraries like `httpx` or `aiohttp`.

### Bad Code (Anti-pattern)
```python
async def fetch_user_profile(user_id: int):
    response = requests.get(f"https://api.example.com/users/{user_id}")
    return response.json()
```

### Good Code (Fix)
```python
async def fetch_user_profile(user_id: int):
    async with httpx.AsyncClient() as client:
        response = await client.get(f"https://api.example.com/users/{user_id}")
        return response.json()
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `call`, `attribute`
- **Detection approach**: Find `call` where the function is `attribute` like `requests.get`, `requests.post`, `requests.put`, `requests.delete`, `urllib.request.urlopen` inside an `async def` function.
- **S-expression query sketch**:
```scheme
(function_definition
  "async"
  body: (block
    (expression_statement
      (assignment
        right: (call
          function: (attribute
            object: (identifier) @module
            attribute: (identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `sync_http_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: subprocess.run() in Async Def

### Description
Using `subprocess.run()`, `subprocess.call()`, or `os.system()` inside an `async def` function, which blocks until the subprocess completes.

### Bad Code (Anti-pattern)
```python
async def compile_and_run(code: str):
    with open("script.py", "w") as f:
        f.write(code)
    result = subprocess.run(["python", "script.py"], capture_output=True, text=True)
    return result.stdout
```

### Good Code (Fix)
```python
async def compile_and_run(code: str):
    async with aiofiles.open("script.py", "w") as f:
        await f.write(code)
    proc = await asyncio.create_subprocess_exec(
        "python", "script.py",
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
    )
    stdout, _ = await proc.communicate()
    return stdout.decode()
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `call`, `attribute`
- **Detection approach**: Find `call` where the function is `subprocess.run`, `subprocess.call`, `subprocess.check_output`, `os.system`, or `os.popen` inside an `async def`.
- **S-expression query sketch**:
```scheme
(function_definition
  "async"
  body: (block
    (expression_statement
      (assignment
        right: (call
          function: (attribute
            object: (identifier) @module
            attribute: (identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `subprocess_in_async`
- **Severity**: warning
- **Confidence**: high
