# Concurrency Misuse -- Python

## Overview
Python's Global Interpreter Lock (GIL) means that threading does not provide true CPU parallelism, yet developers frequently use `threading.Thread` for CPU-bound work. Additionally, shared mutable state across threads without proper synchronization leads to data races that are difficult to reproduce and debug.

## Why It's a Tech Debt Concern
Threading CPU-bound work in Python gives the illusion of parallelism while actually running slower than serial execution due to GIL contention and context-switching overhead. This wastes developer time on a concurrency model that cannot deliver speedup. Unsynchronized shared mutable state produces intermittent bugs — corrupted data structures, lost updates, and crashes that only appear under load and are nearly impossible to reproduce in testing.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py`, `.pyi`
- **Frameworks/libraries**: threading, multiprocessing, concurrent.futures, asyncio

---

## Pattern 1: GIL-Bound CPU Work in Threads

### Description
Using `threading.Thread` or `ThreadPoolExecutor` for CPU-intensive operations (numeric computation, image processing, data transformation) instead of `multiprocessing.Process` or `ProcessPoolExecutor`. The GIL prevents threads from running Python bytecode in parallel, so CPU-bound threads run sequentially with added context-switching overhead.

### Bad Code (Anti-pattern)
```python
import threading

def compute_hash(data_chunk):
    result = 0
    for byte in data_chunk:
        result = (result * 31 + byte) % (2**64)
    return result

def process_all(chunks):
    threads = []
    results = [None] * len(chunks)
    for i, chunk in enumerate(chunks):
        t = threading.Thread(target=lambda idx=i, c=chunk: results.__setitem__(idx, compute_hash(c)))
        threads.append(t)
        t.start()
    for t in threads:
        t.join()
    return results
```

### Good Code (Fix)
```python
from multiprocessing import Pool

def compute_hash(data_chunk):
    result = 0
    for byte in data_chunk:
        result = (result * 31 + byte) % (2**64)
    return result

def process_all(chunks):
    with Pool() as pool:
        results = pool.map(compute_hash, chunks)
    return results
```

### Tree-sitter Detection Strategy
- **Target node types**: `call` (call expression), `attribute` (dotted access), `import_from_statement`
- **Detection approach**: Find `call` nodes that invoke `threading.Thread(target=...)` or `ThreadPoolExecutor().submit(...)`. Check if the target function contains CPU-intensive patterns: tight loops over data (`for ... in` with arithmetic), no I/O calls (`open`, `requests`, `socket`, `await`). Flag when the target function is purely computational.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @class_name)
  arguments: (argument_list
    (keyword_argument
      name: (identifier) @kwarg_name
      value: (_) @target_func)))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `gil_bound_cpu_threads`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Shared Mutable State Without Locks

### Description
Multiple threads reading and writing to the same mutable data structure (list, dict, object attribute) without using `threading.Lock`, `threading.RLock`, or other synchronization primitives. Even though some CPython operations are GIL-atomic, relying on this is an implementation detail that breaks under other interpreters and across GIL-release boundaries.

### Bad Code (Anti-pattern)
```python
import threading

shared_counter = 0
shared_results = []

def worker(items):
    global shared_counter
    for item in items:
        result = process(item)
        shared_results.append(result)
        shared_counter += 1  # Race condition: read-modify-write is not atomic

threads = [threading.Thread(target=worker, args=(chunk,)) for chunk in chunks]
for t in threads:
    t.start()
for t in threads:
    t.join()
```

### Good Code (Fix)
```python
import threading

lock = threading.Lock()
shared_counter = 0
shared_results = []

def worker(items):
    global shared_counter
    for item in items:
        result = process(item)
        with lock:
            shared_results.append(result)
            shared_counter += 1

threads = [threading.Thread(target=worker, args=(chunk,)) for chunk in chunks]
for t in threads:
    t.start()
for t in threads:
    t.join()
```

### Tree-sitter Detection Strategy
- **Target node types**: `global_statement`, `augmented_assignment`, `call` (for `threading.Thread`), `attribute`
- **Detection approach**: Find functions that are used as `threading.Thread` targets. Within those functions, look for `global` declarations or writes to module-level variables (via `augmented_assignment` like `+=`, or attribute assignment). Flag when no `with lock:` statement or `lock.acquire()`/`lock.release()` calls appear in the same function body.
- **S-expression query sketch**:
```scheme
(function_definition
  body: (block
    (global_statement
      (identifier) @global_var)
    (expression_statement
      (augmented_assignment
        left: (identifier) @modified_var))))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `shared_state_no_lock`
- **Severity**: warning
- **Confidence**: medium
