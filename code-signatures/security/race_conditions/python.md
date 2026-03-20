# Race Conditions -- Python

## Overview
Python applications are vulnerable to race conditions through two primary vectors: TOCTOU (time-of-check-to-time-of-use) in file operations where `os.path.exists()` is followed by `open()`, and shared mutable state in multi-threaded code where variables are read and written without proper locking. While the GIL prevents true parallel execution of Python bytecode, it does not prevent thread interleaving between bytecode instructions, making compound operations on shared state unsafe.

## Why It's a Security Concern
TOCTOU races in file operations can be exploited via symlink attacks to read or overwrite arbitrary files, bypass access controls, or escalate privileges. Shared mutable state races in threaded Python can corrupt data structures, produce incorrect authorization decisions, cause double-processing of financial transactions, or leak sensitive data across request contexts in web frameworks using thread pools.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: os, os.path, pathlib, threading, concurrent.futures, Django, Flask

---

## Pattern 1: TOCTOU in File Operations

### Description
Using `os.path.exists()`, `os.path.isfile()`, or `os.access()` to check a file's state, then performing an operation on that file (open, read, write, delete) based on the result. Between the check and the operation, an attacker can replace the file with a symlink or another process can modify it, invalidating the assumption.

### Bad Code (Anti-pattern)
```python
import os

def write_config(path: str, data: str) -> None:
    if not os.path.exists(path):
        # RACE: attacker can create a symlink here pointing to /etc/passwd
        with open(path, 'w') as f:
            f.write(data)
```

### Good Code (Fix)
```python
import os
import tempfile

def write_config(path: str, data: str) -> None:
    # Use O_CREAT | O_EXCL for atomic create-or-fail
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
        with os.fdopen(fd, 'w') as f:
            f.write(data)
    except FileExistsError:
        pass  # file already exists -- safe to skip
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `if_statement`, `attribute`, `identifier`
- **Detection approach**: Find `call` nodes invoking `os.path.exists`, `os.path.isfile`, `os.path.isdir`, or `os.access` inside an `if_statement` condition, where the body of the `if_statement` (or its `else` clause) contains a `call` to `open()`, `os.remove()`, `os.rename()`, or similar file-mutating functions operating on the same path variable.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (call
    function: (attribute
      object: (attribute
        object: (identifier) @os
        attribute: (identifier) @path_mod)
      attribute: (identifier) @check_method))
  consequence: (block
    (with_statement
      (call
        function: (identifier) @open_func))))
```

### Pipeline Mapping
- **Pipeline name**: `toctou`
- **Pattern name**: `file_exists_then_open`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Shared Mutable State Without Locks

### Description
Accessing shared mutable variables (counters, dictionaries, lists) from multiple threads without using `threading.Lock`, `threading.RLock`, or other synchronization primitives. The GIL does not protect compound operations (check-then-modify, read-modify-write) from interleaving, so operations like `counter += 1` or `if key not in dict: dict[key] = value` are not thread-safe.

### Bad Code (Anti-pattern)
```python
import threading

counter = 0

def increment():
    global counter
    for _ in range(100000):
        # NOT atomic: read counter, add 1, write counter
        # threads interleave between these bytecode instructions
        counter += 1

threads = [threading.Thread(target=increment) for _ in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()
# counter is likely less than 400000
```

### Good Code (Fix)
```python
import threading

counter = 0
lock = threading.Lock()

def increment():
    global counter
    for _ in range(100000):
        with lock:
            counter += 1

threads = [threading.Thread(target=increment) for _ in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()
# counter is exactly 400000
```

### Tree-sitter Detection Strategy
- **Target node types**: `global_statement`, `augmented_assignment`, `function_definition`, `call`
- **Detection approach**: Find `function_definition` nodes that contain a `global_statement` declaring a variable, followed by an `augmented_assignment` (`+=`, `-=`, etc.) on that variable. Check that the function is passed as a `target` argument to `threading.Thread()`. Flag if no `with lock:` or `lock.acquire()` statement wraps the augmented assignment.
- **S-expression query sketch**:
```scheme
(function_definition
  body: (block
    (global_statement
      (identifier) @shared_var)
    (expression_statement
      (augmented_assignment
        left: (identifier) @modified_var))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `shared_state_no_lock`
- **Severity**: error
- **Confidence**: medium
