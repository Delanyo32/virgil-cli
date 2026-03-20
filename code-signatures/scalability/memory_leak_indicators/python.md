# Memory Leak Indicators -- Python

## Overview
Memory leaks in Python occur despite garbage collection when objects are retained via global collections, unclosed resources, or unjoined threads. Python's reference-counting GC cannot reclaim cycles involving `__del__` methods, and global mutable state accumulates without bounds.

## Why It's a Scalability Concern
Long-running Python services (Django, FastAPI, Celery workers) process thousands of requests over their lifetime. Leaked objects accumulate in memory, increasing RSS over time. Python's GC pauses also worsen as the number of tracked objects grows, causing latency spikes.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: stdlib (collections, threading, signal), Django, Flask

---

## Pattern 1: Unbounded dict/list Growth in Loop

### Description
Appending to a `list` or setting keys on a `dict` inside a loop or repeatedly-called function without any size limit, pruning, or removal.

### Bad Code (Anti-pattern)
```python
request_log = {}

def handle_request(request):
    request_log[request.id] = {
        "timestamp": time.time(),
        "path": request.path,
        "body": request.body,
    }
    process(request)
```

### Good Code (Fix)
```python
from collections import OrderedDict

request_log = OrderedDict()
MAX_LOG_SIZE = 10000

def handle_request(request):
    request_log[request.id] = {
        "timestamp": time.time(),
        "path": request.path,
        "body": request.body,
    }
    while len(request_log) > MAX_LOG_SIZE:
        request_log.popitem(last=False)
    process(request)
```

### Tree-sitter Detection Strategy
- **Target node types**: `subscript`, `assignment`, `call`, `attribute`, `for_statement`
- **Detection approach**: Find `assignment` to a subscript (`dict[key] = value`) or `call` to `.append()` on a list inside a `for_statement` or a function body. Check if the collection is defined at module level (global). Flag if no `.pop()`, `.clear()`, `del`, or size check exists in the module.
- **S-expression query sketch**:
```scheme
(assignment
  left: (subscript
    value: (identifier) @collection)
  right: (_))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `unbounded_collection_growth`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: signal.signal() Without Cleanup

### Description
Registering signal handlers with `signal.signal()` that capture closures over large objects, without restoring the previous handler.

### Bad Code (Anti-pattern)
```python
def setup_handler(large_data):
    def handler(signum, frame):
        save_data(large_data)
        sys.exit(0)
    signal.signal(signal.SIGTERM, handler)
```

### Good Code (Fix)
```python
def setup_handler(large_data):
    previous_handler = signal.getsignal(signal.SIGTERM)

    def handler(signum, frame):
        save_data(large_data)
        signal.signal(signal.SIGTERM, previous_handler)
        sys.exit(0)

    signal.signal(signal.SIGTERM, handler)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`
- **Detection approach**: Find `call` to `signal.signal()` where the handler argument is a closure (nested function or lambda). Flag if no corresponding `signal.signal()` restoring the previous handler exists in the same scope.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  (#eq? @module "signal")
  (#eq? @method "signal"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `signal_handler_no_cleanup`
- **Severity**: info
- **Confidence**: low

---

## Pattern 3: open() Without with Statement

### Description
Using `open()` without a `with` statement (context manager), risking file descriptor leaks if an exception occurs before `.close()`.

### Bad Code (Anti-pattern)
```python
def read_data(path):
    f = open(path, 'r')
    data = f.read()
    f.close()
    return data
```

### Good Code (Fix)
```python
def read_data(path):
    with open(path, 'r') as f:
        return f.read()
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment`, `call`, `identifier`
- **Detection approach**: Find `call` to `open()` that is assigned to a variable (not inside a `with_statement`'s `with_clause`). Walk the assignment's parent to verify it is NOT a `with_item`.
- **S-expression query sketch**:
```scheme
(assignment
  left: (identifier) @var
  right: (call
    function: (identifier) @func
    (#eq? @func "open")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `file_open_without_with`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: Thread().start() Without .join()

### Description
Creating and starting threads without storing the handle for `.join()`, creating fire-and-forget threads that may leak resources or prevent clean shutdown.

### Bad Code (Anti-pattern)
```python
def process_batch(items):
    for item in items:
        threading.Thread(target=process_item, args=(item,)).start()
```

### Good Code (Fix)
```python
def process_batch(items):
    threads = []
    for item in items:
        t = threading.Thread(target=process_item, args=(item,))
        t.start()
        threads.append(t)
    for t in threads:
        t.join()
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `expression_statement`
- **Detection approach**: Find method chain `Thread(...).start()` as an `expression_statement` (not assigned to a variable). The `.start()` call on a `Thread()` construction that is not stored indicates a fire-and-forget thread.
- **S-expression query sketch**:
```scheme
(expression_statement
  (call
    function: (attribute
      object: (call
        function: (attribute
          attribute: (identifier) @constructor))
      attribute: (identifier) @method)
    (#eq? @method "start")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `thread_no_join`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 5: Global Mutable Collection Accumulation

### Description
A module-level mutable collection (list, dict, set) that only grows (via append, add, setitem) with no pruning, removal, or size check anywhere in the module.

### Bad Code (Anti-pattern)
```python
_metrics = []

def record_metric(name, value):
    _metrics.append({"name": name, "value": value, "ts": time.time()})
```

### Good Code (Fix)
```python
from collections import deque

_metrics = deque(maxlen=10000)

def record_metric(name, value):
    _metrics.append({"name": name, "value": value, "ts": time.time()})
```

### Tree-sitter Detection Strategy
- **Target node types**: `expression_statement`, `call`, `attribute`, `identifier`
- **Detection approach**: Find module-level variable assigned to `[]`, `{}`, or `set()`. Then search for `.append()`, `.add()`, or subscript assignment on that variable. Flag if no `.pop()`, `.remove()`, `.clear()`, `del`, `.popleft()`, or `maxlen` exists in the module.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @collection
    attribute: (identifier) @method)
  (#eq? @method "append"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `global_collection_accumulation`
- **Severity**: warning
- **Confidence**: medium
