# Type Safety Gaps -- Python

## Overview
Python's dynamic type system allows functions to be defined without type hints and permits the use of `Any` as a type annotation escape hatch. Both practices undermine the benefits of static type checking with tools like mypy, pyright, and IDE auto-completion.

## Why It's a Tech Debt Concern
Functions without type hints on parameters and return values force callers to inspect implementation details or rely on documentation that may be stale. As codebases grow, untyped function boundaries become the primary source of `TypeError` and `AttributeError` at runtime. Using `Any` silently disables type checking at that boundary, allowing type errors to propagate undetected through the call chain. Both patterns defeat the purpose of adopting a type checker.

## Applicability
- **Relevance**: high (type hints are standard practice in modern Python)
- **Languages covered**: `.py`, `.pyi`
- **Frameworks/libraries**: All Python codebases; mypy strict mode flags both patterns

---

## Pattern 1: Missing Type Hints on Function Parameters/Returns

### Description
Functions that lack type annotations on one or more parameters or on the return type. This includes both top-level functions and methods. Without type hints, static analysis tools cannot verify correctness at call sites.

### Bad Code (Anti-pattern)
```python
def calculate_discount(price, quantity, coupon_code):
    if coupon_code:
        discount = lookup_coupon(coupon_code)
        return price * quantity * (1 - discount)
    return price * quantity

class OrderProcessor:
    def process(self, order, notify=True):
        total = self.compute_total(order)
        if notify:
            self.send_confirmation(order, total)
        return total

    def compute_total(self, order):
        return sum(item.price * item.qty for item in order.items)
```

### Good Code (Fix)
```python
def calculate_discount(price: float, quantity: int, coupon_code: str | None) -> float:
    if coupon_code:
        discount = lookup_coupon(coupon_code)
        return price * quantity * (1 - discount)
    return price * quantity

class OrderProcessor:
    def process(self, order: Order, notify: bool = True) -> float:
        total = self.compute_total(order)
        if notify:
            self.send_confirmation(order, total)
        return total

    def compute_total(self, order: Order) -> float:
        return sum(item.price * item.qty for item in order.items)
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `parameters`, `typed_parameter`, `identifier`, `type`
- **Detection approach**: Find `function_definition` nodes and inspect their `parameters` children. For each parameter (excluding `self` and `cls`), check whether it has a `type` annotation child (i.e., is a `typed_parameter` or `typed_default_parameter`). Also check whether the function has a `return_type` annotation. Flag functions missing annotations on any parameter or the return type.
- **S-expression query sketch**:
```scheme
(function_definition
  name: (identifier) @func_name
  parameters: (parameters) @params
  return_type: (type)? @return_type)

(parameters
  (identifier) @untyped_param)
```

### Pipeline Mapping
- **Pipeline name**: `missing_type_hints`
- **Pattern name**: `untyped_function_signature`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using `Any` Type Annotation as Escape Hatch

### Description
Annotating parameters, return types, or variables with `typing.Any` effectively opts out of type checking at that boundary. While occasionally necessary for truly dynamic code, widespread use of `Any` defeats the purpose of type annotations and allows type errors to propagate silently.

### Bad Code (Anti-pattern)
```python
from typing import Any

def transform_data(data: Any) -> Any:
    result = process(data)
    return format_output(result)

def handle_event(event: Any, context: Any) -> Any:
    payload = event["body"]
    return {"statusCode": 200, "body": payload}

cache: dict[str, Any] = {}

def store(key: str, value: Any) -> None:
    cache[key] = value
```

### Good Code (Fix)
```python
from dataclasses import dataclass

@dataclass
class Event:
    body: str
    headers: dict[str, str]

@dataclass
class LambdaContext:
    function_name: str
    memory_limit_in_mb: int

def transform_data(data: list[dict[str, float]]) -> list[str]:
    result = process(data)
    return format_output(result)

def handle_event(event: Event, context: LambdaContext) -> dict[str, int | str]:
    payload = event.body
    return {"statusCode": 200, "body": payload}

cache: dict[str, str | int | float] = {}

def store(key: str, value: str | int | float) -> None:
    cache[key] = value
```

### Tree-sitter Detection Strategy
- **Target node types**: `type`, `identifier`, `attribute`
- **Detection approach**: Find `type` annotation nodes in `typed_parameter`, `typed_default_parameter`, `function_definition` return types, and `type` alias assignments where the type expression is `Any` (an `identifier` node with text `Any`) or `typing.Any` (an `attribute` node). Flag each occurrence. Optionally count `Any` usage per file or module to assess the severity of escape-hatch reliance.
- **S-expression query sketch**:
```scheme
(type
  (identifier) @type_name
  (#eq? @type_name "Any"))

(type
  (attribute
    object: (identifier) @module
    attribute: (identifier) @attr
    (#eq? @module "typing")
    (#eq? @attr "Any")))
```

### Pipeline Mapping
- **Pipeline name**: `missing_type_hints`
- **Pattern name**: `any_type_escape_hatch`
- **Severity**: warning
- **Confidence**: high
