# Dependency Graph Depth -- Python

## Overview
Dependency graph depth measures how many layers of module imports a Python file must traverse before reaching the actual implementation. In Python, deep dependency chains are especially common due to the `__init__.py` convention, which encourages re-exporting symbols from submodules to create a convenient top-level API. While this simplifies consumer imports, it creates hidden transitive chains that increase startup time, invite circular imports, and make the codebase harder to reason about.

## Why It's an Architecture Concern
Deep dependency chains in Python increase the blast radius of changes -- modifying a module buried several layers deep can trigger cascading import failures through `__init__.py` re-export chains. Python's eager module execution model means that every import in the chain runs at import time, so deep chains slow down application startup and test discovery. The `from .submodule import *` pattern in `__init__.py` files is particularly dangerous: it pulls in everything transitively, creates namespace pollution, and makes it nearly impossible to determine where a symbol actually originates. Keeping the module hierarchy shallow and imports explicit reduces circular dependency risk and improves import performance.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py, .pyi`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In Python, the barrel file pattern manifests as `__init__.py` files that import from submodules and re-export symbols using `from .submodule import *` or explicit `from .submodule import Name` statements. These files create a convenient public API for a package but add a layer of indirection that triggers eager loading of all submodules and invites circular import issues.

### Bad Code (Anti-pattern)
```python
# myapp/services/__init__.py -- barrel file re-exporting everything
from .auth import AuthService, TokenValidator
from .billing import BillingService, InvoiceGenerator
from .email import EmailService, TemplateRenderer
from .reporting import ReportService, ChartBuilder
from .storage import StorageService, FileManager
from .users import UserService, ProfileManager

__all__ = [
    "AuthService", "TokenValidator",
    "BillingService", "InvoiceGenerator",
    "EmailService", "TemplateRenderer",
    "ReportService", "ChartBuilder",
    "StorageService", "FileManager",
    "UserService", "ProfileManager",
]
```

### Good Code (Fix)
```python
# myapp/api/payment.py -- imports directly from source modules
from myapp.services.auth import AuthService
from myapp.services.billing import BillingService

class PaymentHandler:
    def __init__(self, auth: AuthService, billing: BillingService):
        self.auth = auth
        self.billing = billing

    def process(self, token: str, card_id: str) -> None:
        self.auth.validate(token)
        self.billing.charge(card_id)
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_from_statement`
- **Detection approach**: Count `from .submodule import ...` statements in a single file. Flag if count >= 5, especially if the file is named `__init__.py`. Also flag `from .submodule import *` wildcard re-exports. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Relative re-exports (common in __init__.py)
(import_from_statement
  module_name: (relative_import) @source_module
  name: [
    (dotted_name) @imported_name
    (wildcard_import) @wildcard
  ]) @reexport

;; Absolute re-exports
(import_from_statement
  module_name: (dotted_name) @source_module
  name: (dotted_name) @imported_name) @import_stmt
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In Python this appears as `from package.sub.deep.module import thing` with many dot-separated segments or deeply nested relative imports.

### Bad Code (Anti-pattern)
```python
from myapp.infrastructure.persistence.repositories.sqlalchemy import OrderRepository
from myapp.infrastructure.persistence.repositories.redis import CacheRepository
from myapp.domain.aggregates.orders.value_objects import OrderStatus
from myapp.application.services.orders.handlers import CreateOrderHandler
from myapp.presentation.api.serializers.v2 import OrderSerializer

class OrderController:
    def __init__(self):
        self.repo = OrderRepository()
        self.cache = CacheRepository()
        self.handler = CreateOrderHandler()

    def create_order(self, data: dict) -> dict:
        order = self.handler.execute(data)
        return OrderSerializer.dump(order)
```

### Good Code (Fix)
```python
from myapp.persistence import OrderRepository, CacheRepository
from myapp.domain.orders import OrderStatus
from myapp.services import CreateOrderHandler
from myapp.serializers import OrderSerializer

class OrderController:
    def __init__(self):
        self.repo = OrderRepository()
        self.cache = CacheRepository()
        self.handler = CreateOrderHandler()

    def create_order(self, data: dict) -> dict:
        order = self.handler.execute(data)
        return OrderSerializer.dump(order)
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_from_statement`, `dotted_name`
- **Detection approach**: Parse the module path in `from ... import` statements and count dot-separated segments. For relative imports, count the leading dots plus the module path segments. Flag if depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Absolute imports with module path
(import_from_statement
  module_name: (dotted_name) @module_path) @import_stmt

;; Relative imports with module path
(import_from_statement
  module_name: (relative_import
    (dotted_name) @rel_module_path)) @rel_import

;; Plain import statements
(import_statement
  name: (dotted_name) @module_path) @import_stmt
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
