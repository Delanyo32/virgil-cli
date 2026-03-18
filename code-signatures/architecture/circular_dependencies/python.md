# Circular Dependencies -- Python

## Overview
Circular dependencies in Python occur when two or more modules mutually import each other, forming a cycle in the import graph. This is one of the most common causes of `ImportError` and `AttributeError` in Python projects. Because Python executes module code top-level during import, a circular import can result in partially-initialized modules where names are not yet defined at the point of use. The problem is especially prevalent with `from module import name` style imports, which require the name to exist at import time.

## Why It's an Architecture Concern
Circular imports make modules inseparable — you cannot import one without triggering the import of the other, and the order in which they are first imported determines whether the code works or crashes. They prevent independent testing because importing a test subject pulls in the entire cycle. `__init__.py` files that import from submodules which import back through the package are a particularly common source of cycles. Workarounds like deferred imports (importing inside functions) mask the problem without fixing the underlying coupling. Cycles indicate tangled responsibilities: if module A needs classes from B and B needs functions from A, neither module has a well-defined, self-contained purpose.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py, .pyi`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```python
# --- models/user.py ---
from models.order import Order  # user.py imports order.py

class User:
    def __init__(self, name: str):
        self.name = name
        self.orders: list[Order] = []

    def total_spent(self) -> float:
        return sum(o.total for o in self.orders)


# --- models/order.py ---
from models.user import User  # order.py imports user.py -- CIRCULAR

class Order:
    def __init__(self, total: float, customer: User):
        self.total = total
        self.customer = customer

    def summary(self) -> str:
        return f"Order for {self.customer.name}: ${self.total}"
```

### Good Code (Fix)
```python
# --- models/types.py --- (shared types extracted to break cycle)
from __future__ import annotations
from dataclasses import dataclass, field


@dataclass
class Order:
    total: float
    customer_name: str

    def summary(self) -> str:
        return f"Order for {self.customer_name}: ${self.total}"


@dataclass
class User:
    name: str
    orders: list[Order] = field(default_factory=list)

    def total_spent(self) -> float:
        return sum(o.total for o in self.orders)


# --- models/user.py ---
from models.types import User  # unidirectional

# --- models/order.py ---
from models.types import Order  # unidirectional
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_from_statement`, `import_statement`
- **Detection approach**: Per-file: extract all import module paths from each Python file. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each module to its imported modules, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files that both import from and are imported by the same module (requires cross-referencing imports.parquet). Pay special attention to `__init__.py` files that re-import from submodules.
- **S-expression query sketch**:
```scheme
(import_from_statement
  module_name: (dotted_name) @import_source)

(import_from_statement
  module_name: (relative_import) @import_source)

(import_statement
  name: (dotted_name) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `mutual_import`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Hub Module (Bidirectional)

### Description
A module with high fan-in (many dependents) AND high fan-out (many dependencies), acting as a nexus that participates in or enables dependency cycles.

### Bad Code (Anti-pattern)
```python
# --- utils/__init__.py --- (hub module re-exporting everything)
from app.auth.manager import AuthManager
from app.billing.processor import BillingProcessor
from app.cache.store import CacheStore
from app.config.loader import ConfigLoader
from app.database.pool import ConnectionPool
from app.logging.factory import LogFactory
from app.messaging.broker import MessageBroker

# High fan-out (7 imports) AND high fan-in (every module above
# does "from utils import ..." — creating implicit cycles through __init__.py)

__all__ = [
    "AuthManager", "BillingProcessor", "CacheStore",
    "ConfigLoader", "ConnectionPool", "LogFactory", "MessageBroker",
]
```

### Good Code (Fix)
```python
# --- app/auth/manager.py --- (import directly from source)
class AuthManager:
    def validate_token(self, token: str) -> bool:
        ...

# --- app/billing/processor.py ---
class BillingProcessor:
    def charge(self, customer_id: int, amount: float) -> None:
        ...

# Consumers import directly from the source module:
# from app.auth.manager import AuthManager
# from app.billing.processor import BillingProcessor
# No hub __init__.py re-export needed
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_from_statement`, `import_statement`
- **Detection approach**: Per-file: count import statements to estimate fan-out. Pay special attention to `__init__.py` files which often act as re-export hubs. Cross-file: query imports.parquet to count how many other files import from this module (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(import_from_statement
  module_name: (dotted_name) @import_source)

(import_statement
  name: (dotted_name) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
