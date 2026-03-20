# API Surface Area -- Python

## Overview
API surface area in Python is governed by naming convention: identifiers starting with an underscore (`_`) are considered private by convention, while all others are public. The `__all__` variable can explicitly control what `from module import *` exposes, but in practice most Python code relies on the underscore convention. Tracking the ratio of non-underscore symbols to total symbols identifies modules that lack encapsulation, exposing internal helpers and data structures as part of the public interface.

## Why It's an Architecture Concern
Python's lack of enforced access control means that any public function or class can be imported and depended upon by consumers. When a module exports nearly all of its symbols, every internal helper, data class, and constant becomes a de facto public API that callers may rely on. Renaming or restructuring these symbols later requires updating all dependents. Large public surfaces also clutter IDE autocompletion and documentation, making it harder for users to find the intended entry points. Prefixing internal symbols with `_` and defining `__all__` communicates clear boundaries and preserves the freedom to refactor internals without breaking consumers.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.py, .pyi`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```python
def connect_to_database(dsn: str): ...
def disconnect(conn): ...
def execute_query(conn, sql: str): ...
def fetch_one(cursor): ...
def fetch_all(cursor): ...
def build_insert_sql(table: str, columns: list) -> str: ...
def build_update_sql(table: str, columns: list) -> str: ...
def build_delete_sql(table: str) -> str: ...
def sanitize_input(value: str) -> str: ...
def parse_dsn(dsn: str) -> dict: ...
def retry_connection(dsn: str, attempts: int): ...
def log_query(sql: str, duration: float): ...
```

### Good Code (Fix)
```python
__all__ = ["connect", "execute", "fetch_one", "fetch_all"]

def connect(dsn: str): ...
def execute(conn, sql: str): ...
def fetch_one(cursor): ...
def fetch_all(cursor): ...

def _disconnect(conn): ...
def _build_insert_sql(table: str, columns: list) -> str: ...
def _build_update_sql(table: str, columns: list) -> str: ...
def _build_delete_sql(table: str) -> str: ...
def _sanitize_input(value: str) -> str: ...
def _parse_dsn(dsn: str) -> dict: ...
def _retry_connection(dsn: str, attempts: int): ...
def _log_query(sql: str, duration: float): ...
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_definition` at module level
- **Detection approach**: Count all top-level function and class definitions. A symbol is exported if its name does not start with `_`. Flag modules where total >= 10 and exported/total > 0.8. Optionally check for `__all__` assignment and compare its length.
- **S-expression query sketch**:
```scheme
;; Match top-level function definitions
(module
  (function_definition
    name: (identifier) @func.name))

;; Match top-level class definitions
(module
  (class_definition
    name: (identifier) @class.name))

;; Match decorated definitions (unwrap to inner)
(module
  (decorated_definition
    definition: (function_definition
      name: (identifier) @decorated.func.name)))

;; Post-process: check if name starts with "_"
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```python
class ConnectionPool:
    def __init__(self, dsn: str, max_size: int = 10):
        self.connections: list[Connection] = []
        self.available: list[Connection] = []
        self.dsn: str = dsn
        self.max_size: int = max_size
        self.retry_delays: list[float] = [0.1, 0.5, 1.0, 5.0]
        self.error_log: list[str] = []
        self.stats: dict[str, int] = {"acquired": 0, "released": 0}

    def acquire(self) -> Connection: ...
    def release(self, conn: Connection) -> None: ...
```

### Good Code (Fix)
```python
class ConnectionPool:
    def __init__(self, dsn: str, max_size: int = 10):
        self._connections: list[Connection] = []
        self._available: list[Connection] = []
        self._dsn: str = dsn
        self._max_size: int = max_size
        self._retry_delays: list[float] = [0.1, 0.5, 1.0, 5.0]
        self._error_log: list[str] = []
        self._stats: dict[str, int] = {"acquired": 0, "released": 0}

    def acquire(self) -> Connection: ...
    def release(self, conn: Connection) -> None: ...

    @property
    def size(self) -> int:
        return len(self._connections)

    @property
    def available_count(self) -> int:
        return len(self._available)
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment` inside `__init__` method using `self.` attribute access
- **Detection approach**: Find classes whose names do not start with `_` (exported). Within their `__init__` method, check attribute assignments via `self.<name>`. If the attribute name does not start with `_`, it is publicly exposed. Flag classes with 3+ non-underscore instance attributes.
- **S-expression query sketch**:
```scheme
;; Match self.attribute assignments in __init__
(class_definition
  name: (identifier) @class.name
  body: (block
    (function_definition
      name: (identifier) @method.name
      (#eq? @method.name "__init__")
      body: (block
        (expression_statement
          (assignment
            left: (attribute
              object: (identifier) @self
              attribute: (identifier) @attr.name)
            (#eq? @self "self")))))))

;; Post-process: check @attr.name does not start with "_"
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
