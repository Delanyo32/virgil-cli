# Module Size Distribution -- Python

## Overview
Module size distribution measures how symbol definitions are spread across source files in a Python codebase. Well-sized modules align with the principle of having each file serve a clear, singular purpose. Balanced module sizes improve readability, reduce merge conflicts, and make it easier for developers to find and understand code without wading through unrelated definitions.

## Why It's an Architecture Concern
Oversized Python modules concentrate too many functions, classes, and constants into a single file, making the module hard to navigate, slow to import (Python executes all top-level code on import), and prone to circular import issues. They often evolve into "utils.py" dumping grounds that everything depends on, creating a tightly coupled hub in the dependency graph. Anemic modules that define only a single function or class create unnecessary indirection -- Python's module system is lightweight, but opening many single-symbol files to trace a simple workflow adds cognitive overhead without improving encapsulation.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py, .pyi`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```python
# utils.py -- a grab-bag of unrelated utilities
import os
import json
import hashlib
from datetime import datetime

def format_date(d: datetime) -> str: ...
def parse_date(s: str) -> datetime: ...
def format_currency(amount: float) -> str: ...
def slugify(s: str) -> str: ...
def truncate(s: str, length: int) -> str: ...
def deep_merge(a: dict, b: dict) -> dict: ...
def flatten_list(lst: list) -> list: ...
def hash_string(s: str) -> str: ...
def validate_email(email: str) -> bool: ...
def validate_url(url: str) -> bool: ...

class HttpClient:
    def get(self, url: str): ...
    def post(self, url: str, data: dict): ...

class CacheManager:
    def get(self, key: str): ...
    def set(self, key: str, value): ...

MAX_RETRIES = 5
DEFAULT_TIMEOUT = 30
API_VERSION = "v2"
# ... 15 more functions, classes, and constants
```

### Good Code (Fix)
```python
# formatting.py -- focused on formatting utilities
from datetime import datetime

def format_date(d: datetime) -> str: ...
def parse_date(s: str) -> datetime: ...
def format_currency(amount: float) -> str: ...
```

```python
# validation.py -- focused on input validation
def validate_email(email: str) -> bool: ...
def validate_url(url: str) -> bool: ...
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_definition`, `assignment`, `decorated_definition`
- **Detection approach**: Count all top-level symbol definitions (direct children of `module`). For `decorated_definition`, unwrap to the inner function or class. Skip `if __name__` guard blocks. Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(module
  [
    (function_definition name: (identifier) @name) @def
    (class_definition name: (identifier) @name) @def
    (expression_statement
      (assignment left: (identifier) @name)) @def
    (decorated_definition
      definition: [
        (function_definition name: (identifier) @name)
        (class_definition name: (identifier) @name)
      ]) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```python
# __init__.py -- barrel module re-exporting everything
from .formatting import format_date, parse_date, format_currency
from .strings import slugify, truncate, capitalize, snake_case
from .collections import deep_merge, flatten_list, chunk_list
from .http import HttpClient, Response, RequestError
from .cache import CacheManager, CacheEntry
from .validation import validate_email, validate_url, validate_phone
from .hashing import hash_string, hash_file, verify_hash
from .config import MAX_RETRIES, DEFAULT_TIMEOUT, API_VERSION
# 25+ total symbols re-exported
```

### Good Code (Fix)
```python
# __init__.py -- curated public API
from .formatting import format_date, format_currency
from .http import HttpClient
from .cache import CacheManager
```

```python
# formatting/__init__.py -- sub-package with focused exports
from .dates import format_date, parse_date
from .currency import format_currency
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_definition`, `assignment`, `decorated_definition`
- **Detection approach**: Count top-level symbols whose name does not start with an underscore (Python convention for public symbols). Also count symbols explicitly listed in `__all__` if present. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(module
  (function_definition
    name: (identifier) @name
    (#not-match? @name "^_")) @def)

(module
  (class_definition
    name: (identifier) @name
    (#not-match? @name "^_")) @def)

(module
  (expression_statement
    (assignment
      left: (identifier) @name
      (#not-match? @name "^_"))) @def)
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```python
# max_retries.py
MAX_RETRIES = 5
```

### Good Code (Fix)
```python
# config.py -- merge the trivial constant into a related module
MAX_RETRIES = 5
DEFAULT_TIMEOUT = 30
API_VERSION = "v2"

def load_config(path: str) -> dict:
    """Load configuration from a YAML file."""
    ...
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_definition`, `assignment`, `decorated_definition`
- **Detection approach**: Count top-level symbol definitions (direct children of `module`). Flag if count == 1, excluding test files (`test_*.py`, `*_test.py`), `__init__.py`, `__main__.py`, and `conftest.py`.
- **S-expression query sketch**:
```scheme
(module
  [
    (function_definition name: (identifier) @name) @def
    (class_definition name: (identifier) @name) @def
    (expression_statement
      (assignment left: (identifier) @name)) @def
    (decorated_definition
      definition: [
        (function_definition name: (identifier) @name)
        (class_definition name: (identifier) @name)
      ]) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
