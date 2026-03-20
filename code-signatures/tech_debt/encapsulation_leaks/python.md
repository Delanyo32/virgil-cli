# Encapsulation Leaks -- Python

## Overview
Encapsulation leaks in Python arise when module-level mutable state is shared and modified across functions or when mutable objects are used as default argument values. Module-level mutable globals create hidden coupling between functions, while mutable defaults persist across calls, producing bugs that are notoriously difficult to diagnose.

## Why It's a Tech Debt Concern
Module-level mutable globals turn functions into stateful procedures — their behavior depends on when they are called relative to other functions that modify the same global, making unit testing unreliable and parallelization unsafe. Mutable default arguments are a classic Python gotcha where a list or dict created once at function definition time is shared across all calls, leading to data leaking between invocations. Both patterns undermine the ability to reason about function behavior in isolation.

## Applicability
- **Relevance**: high (mutable globals and mutable defaults are pervasive Python pitfalls)
- **Languages covered**: `.py`, `.pyi`
- **Frameworks/libraries**: Django (global settings mutation), Flask (app-level globals), Celery (task-shared state)

---

## Pattern 1: Module-Level Mutable Globals

### Description
A module defines mutable variables (lists, dicts, sets, or reassigned scalars) at the top level that are read and modified by multiple functions using the `global` keyword or direct attribute mutation. This creates invisible coupling where calling one function affects the behavior of another.

### Bad Code (Anti-pattern)
```python
# registry.py
_handlers = {}
_plugins = []
_config = {"debug": False, "verbose": False}
_request_count = 0

def register_handler(name, fn):
    global _request_count
    _handlers[name] = fn
    _request_count += 1

def get_handler(name):
    return _handlers.get(name)

def load_plugins(plugin_list):
    _plugins.clear()
    _plugins.extend(plugin_list)

def set_debug(enabled):
    _config["debug"] = enabled

def process(name, data):
    global _request_count
    _request_count += 1
    handler = _handlers.get(name)
    if _config["debug"]:
        print(f"Processing {name} (call #{_request_count})")
    if handler:
        return handler(data)

# test_registry.py
def test_process():
    register_handler("greet", lambda d: f"Hello {d}")
    result = process("greet", "World")
    assert result == "Hello World"
    # FAILS if another test already registered "greet" with different fn
    # _request_count leaks between tests
```

### Good Code (Fix)
```python
# registry.py
class HandlerRegistry:
    def __init__(self, debug=False):
        self._handlers = {}
        self._plugins = []
        self._config = {"debug": debug}
        self._request_count = 0

    def register_handler(self, name, fn):
        self._handlers[name] = fn

    def get_handler(self, name):
        return self._handlers.get(name)

    def load_plugins(self, plugin_list):
        self._plugins = list(plugin_list)

    def set_debug(self, enabled):
        self._config["debug"] = enabled

    def process(self, name, data):
        self._request_count += 1
        handler = self._handlers.get(name)
        if self._config["debug"]:
            print(f"Processing {name} (call #{self._request_count})")
        if handler:
            return handler(data)

    @property
    def request_count(self):
        return self._request_count

# test_registry.py
def test_process():
    registry = HandlerRegistry()  # fresh instance per test
    registry.register_handler("greet", lambda d: f"Hello {d}")
    result = registry.process("greet", "World")
    assert result == "Hello World"
```

### Tree-sitter Detection Strategy
- **Target node types**: `global_statement`, `module` (top-level `expression_statement` with `assignment`)
- **Detection approach**: Find `global_statement` nodes inside `function_definition` bodies — these declare intent to mutate a module-level variable. Cross-reference with module-level assignments to mutable types (list, dict, set literals or constructor calls). Flag functions that use `global` to modify module-level mutable state.
- **S-expression query sketch**:
  ```scheme
  (module
    (expression_statement
      (assignment
        left: (identifier) @global_var)))

  (function_definition
    body: (block
      (global_statement
        (identifier) @mutated_global)))
  ```

### Pipeline Mapping
- **Pipeline name**: `encapsulation_leaks`
- **Pattern name**: `mutable_module_globals`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mutable Default Arguments

### Description
A function uses a mutable object (list, dict, set, or custom object) as a default parameter value. Because Python evaluates default arguments once at function definition time, all calls that use the default share the same object instance, causing mutations to persist across calls.

### Bad Code (Anti-pattern)
```python
def add_item(item, items=[]):
    items.append(item)
    return items

# First call works as expected
result1 = add_item("a")  # ["a"]
# Second call sees the SAME list
result2 = add_item("b")  # ["a", "b"] — unexpected!

def create_user(name, roles=[], metadata={}):
    roles.append("viewer")  # mutates the shared default
    metadata["created"] = True
    return {"name": name, "roles": roles, "metadata": metadata}

user1 = create_user("Alice")
# user1 = {"name": "Alice", "roles": ["viewer"], "metadata": {"created": True}}
user2 = create_user("Bob")
# user2 = {"name": "Bob", "roles": ["viewer", "viewer"], "metadata": {"created": True}}
# user1's roles is ALSO ["viewer", "viewer"] now

def process_events(events, seen=set()):
    for event in events:
        if event.id not in seen:
            seen.add(event.id)
            handle(event)
```

### Good Code (Fix)
```python
def add_item(item, items=None):
    if items is None:
        items = []
    items.append(item)
    return items

result1 = add_item("a")  # ["a"]
result2 = add_item("b")  # ["b"] — correct

def create_user(name, roles=None, metadata=None):
    if roles is None:
        roles = []
    if metadata is None:
        metadata = {}
    roles.append("viewer")
    metadata["created"] = True
    return {"name": name, "roles": roles, "metadata": metadata}

user1 = create_user("Alice")
# {"name": "Alice", "roles": ["viewer"], "metadata": {"created": True}}
user2 = create_user("Bob")
# {"name": "Bob", "roles": ["viewer"], "metadata": {"created": True}}
# user1 is unaffected

def process_events(events, seen=None):
    if seen is None:
        seen = set()
    for event in events:
        if event.id not in seen:
            seen.add(event.id)
            handle(event)
```

### Tree-sitter Detection Strategy
- **Target node types**: `default_parameter` inside `parameters` of `function_definition`
- **Detection approach**: Find `default_parameter` nodes where the `value` child is a `list` (`[]`), `dictionary` (`{}`), `set` (`set()`), or a `call` expression constructing a known mutable type (e.g., `list()`, `dict()`, `set()`, `collections.defaultdict()`). Flag each occurrence.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    parameters: (parameters
      (default_parameter
        name: (identifier) @param_name
        value: (list) @mutable_default)))

  (function_definition
    parameters: (parameters
      (default_parameter
        name: (identifier) @param_name
        value: (dictionary) @mutable_default)))

  (function_definition
    parameters: (parameters
      (default_parameter
        name: (identifier) @param_name
        value: (set) @mutable_default)))
  ```

### Pipeline Mapping
- **Pipeline name**: `mutable_default_args`
- **Pattern name**: `mutable_default_parameter`
- **Severity**: warning
- **Confidence**: high
