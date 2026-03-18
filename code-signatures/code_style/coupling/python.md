# Coupling -- Python

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file/module importing from many different modules (high fan-in), indicating it depends on too many parts of the system. Typically a "god module" that orchestrates everything. Python's dynamic nature makes this especially problematic — large import sets slow startup and make dependency tracking difficult.

### Bad Code (Anti-pattern)
```python
# app/controllers/order_controller.py
from auth.auth_service import authenticate, authorize
from auth.token_validator import validate_token
from users.user_service import get_user_by_id
from users.preferences_service import get_user_preferences
from orders.order_service import create_order, update_order
from orders.order_validator import validate_order
from billing.tax_service import calculate_tax
from billing.payment_gateway import process_payment
from billing.discount_engine import apply_discount
from notifications.email_service import send_confirmation_email
from notifications.push_service import send_push_notification
from logging_module.event_logger import log_event
from analytics.tracker import track_analytics
from cache.cache_manager import cache_result, invalidate_cache
from queue_module.job_queue import enqueue_job
from utils.formatters import format_currency
from database.connection import get_db_session
```

### Good Code (Fix)
```python
# app/controllers/order_controller.py
from auth.auth_service import authenticate
from orders.order_service import OrderService
from logging_module.event_logger import log_event


class OrderController:
    def __init__(self, order_service: OrderService):
        self.order_service = order_service

    def create_order(self, request):
        user = authenticate(request)
        order = self.order_service.create(user, request.data)
        log_event("order_created", order_id=order.id)
        return order


# app/orders/order_service.py — encapsulates billing, notifications, caching
from billing.billing_service import BillingService
from notifications.notification_service import NotificationService
from orders.order_validator import validate_order


class OrderService:
    def __init__(self, billing: BillingService, notifications: NotificationService):
        self.billing = billing
        self.notifications = notifications

    def create(self, user, data):
        validate_order(data)
        total = self.billing.process_order(data)
        self.notifications.send_order_confirmation(user, total)
        return {"data": data, "total": total}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement`, `import_from_statement`
- **Detection approach**: Count unique module sources per file. For `import x` statements, extract the module name. For `from x import y` statements, extract the module name from the `module_name` child. Flag files exceeding threshold (e.g., 15+ unique module imports). Distinguish between standard library, third-party, and project-internal imports using relative import dots.
- **S-expression query sketch**:
```scheme
;; import x, import x.y
(import_statement
  name: (dotted_name) @import_module)

;; from x import y
(import_from_statement
  module_name: (dotted_name) @from_module)

;; from . import y (relative)
(import_from_statement
  module_name: (relative_import) @relative_module)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more modules that import each other (directly or transitively), creating a dependency cycle. In Python, circular imports cause `ImportError` at runtime or result in partially initialized modules where attributes are `None` or missing. Developers often work around this with deferred imports inside functions, which is itself a code smell.

### Bad Code (Anti-pattern)
```python
# models/user.py
from models.order import Order  # Circular: user -> order -> user

class User:
    def __init__(self, name: str):
        self.name = name
        self.orders: list[Order] = []

    def add_order(self, order: Order):
        self.orders.append(order)


# models/order.py
from models.user import User  # ImportError — circular dependency

class Order:
    def __init__(self, user: User, amount: float):
        self.user = user
        self.amount = amount
        user.add_order(self)
```

### Good Code (Fix)
```python
# models/types.py — shared protocol, no circular dependency
from typing import Protocol


class HasOrders(Protocol):
    def add_order(self, order: "OrderLike") -> None: ...

class OrderLike(Protocol):
    owner_id: str
    amount: float


# models/user.py
from models.types import OrderLike

class User:
    def __init__(self, name: str):
        self.name = name
        self.orders: list[OrderLike] = []

    def add_order(self, order: OrderLike):
        self.orders.append(order)


# models/order.py
from models.types import OrderLike

class Order:
    def __init__(self, owner_id: str, amount: float):
        self.owner_id = owner_id
        self.amount = amount
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement`, `import_from_statement`
- **Detection approach**: Build a directed graph of file-to-file imports by resolving dotted module names and relative imports to file paths. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Flag deferred imports (imports inside function bodies) as potential indicators of existing circular dependency workarounds.
- **S-expression query sketch**:
```scheme
;; Build import graph from all import forms
(import_statement
  name: (dotted_name) @import_path)

(import_from_statement
  module_name: (dotted_name) @from_path)

(import_from_statement
  module_name: (relative_import) @relative_path)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
