# Stringly Typed -- Python

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi

---

## Pattern 1: String Comparisons for State/Status

### Description
Using string equality checks to determine state transitions, roles, or status instead of `enum.Enum` or constants.

### Bad Code (Anti-pattern)
```python
def process_order(order):
    if order.status == "active":
        start_fulfillment(order)
    elif order.status == "pending":
        notify_customer(order)
    elif order.status == "cancelled":
        refund_payment(order)
    elif order.status == "shipped":
        track_delivery(order)
    elif order.status == "delivered":
        request_review(order)

def get_status_color(status: str) -> str:
    if status == "active":
        return "green"
    elif status == "pending":
        return "yellow"
    elif status == "cancelled":
        return "red"
    return "gray"
```

### Good Code (Fix)
```python
from enum import Enum
from typing import Literal

class Status(Enum):
    ACTIVE = "active"
    PENDING = "pending"
    CANCELLED = "cancelled"
    SHIPPED = "shipped"
    DELIVERED = "delivered"

def process_order(order):
    match order.status:
        case Status.ACTIVE:
            start_fulfillment(order)
        case Status.PENDING:
            notify_customer(order)
        case Status.CANCELLED:
            refund_payment(order)
        case Status.SHIPPED:
            track_delivery(order)
        case Status.DELIVERED:
            request_review(order)

# Or with Literal type hint for simpler cases:
StatusType = Literal["active", "pending", "cancelled", "shipped", "delivered"]
```

### Tree-sitter Detection Strategy
- **Target node types**: `comparison_operator` (with `==`), `string`, `if_statement`, `elif_clause`
- **Detection approach**: Find equality comparisons (`==`) where one operand is a string literal. Flag when the same variable is compared against 3+ different string literals across `if`/`elif` chains (indicates an enum is appropriate).
- **S-expression query sketch**:
```scheme
(comparison_operator
  (attribute
    object: (identifier) @obj
    attribute: (identifier) @attr)
  (string) @string_val)

(if_statement
  condition: (comparison_operator
    (string) @cond_string))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for dictionary-based configuration access or event dispatch instead of typed dataclasses, `TypedDict`, or named constants.

### Bad Code (Anti-pattern)
```python
def setup_app(config: dict):
    db_host = config["database_host"]
    db_port = config["database_port"]
    db_name = config["database_name"]
    redis_url = config["redis_url"]
    api_key = config["api_key"]
    log_level = config["log_level"]

    connect_db(db_host, db_port, db_name)
    connect_redis(redis_url)
    init_logger(log_level)

def handle_event(event_name: str, data: dict):
    if event_name == "user_created":
        create_user(data)
    elif event_name == "user_deleted":
        delete_user(data)
    elif event_name == "order_placed":
        place_order(data)
    elif event_name == "order_shipped":
        ship_order(data)
    elif event_name == "payment_received":
        process_payment(data)
```

### Good Code (Fix)
```python
from dataclasses import dataclass
from enum import Enum

@dataclass
class DatabaseConfig:
    host: str
    port: int
    name: str

@dataclass
class AppConfig:
    database: DatabaseConfig
    redis_url: str
    api_key: str
    log_level: str

def setup_app(config: AppConfig):
    connect_db(config.database.host, config.database.port, config.database.name)
    connect_redis(config.redis_url)
    init_logger(config.log_level)

class EventType(Enum):
    USER_CREATED = "user_created"
    USER_DELETED = "user_deleted"
    ORDER_PLACED = "order_placed"
    ORDER_SHIPPED = "order_shipped"
    PAYMENT_RECEIVED = "payment_received"

def handle_event(event: EventType, data: dict):
    match event:
        case EventType.USER_CREATED:
            create_user(data)
        case EventType.USER_DELETED:
            delete_user(data)
```

### Tree-sitter Detection Strategy
- **Target node types**: `subscript` with `string` literal, `identifier`
- **Detection approach**: Find repeated dictionary access patterns where string literals are used as keys via bracket notation (`config["key"]`). Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(subscript
  value: (identifier) @obj
  subscript: (string) @key)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
