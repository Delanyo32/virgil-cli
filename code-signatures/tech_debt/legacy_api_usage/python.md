# Legacy API Usage -- Python

## Overview
Legacy API usage in Python refers to relying on older patterns and APIs when modern, more readable, and more maintainable alternatives exist. Common examples include using `%`-formatting or `.format()` instead of f-strings, and using `print()` for debugging instead of the `logging` module.

## Why It's a Tech Debt Concern
Old-style string formatting (`%` operator and `.format()`) is more verbose, harder to read, and more error-prone than f-strings (available since Python 3.6). `print()` debugging offers no log levels, no structured output, no runtime configurability, and no way to disable debug output in production without removing the statements entirely. Both patterns signal code that has not been modernized and accumulate friction during code review.

## Applicability
- **Relevance**: high (these patterns are pervasive in codebases that predate Python 3.6 or lack logging infrastructure)
- **Languages covered**: `.py`, `.pyi`
- **Frameworks/libraries**: N/A (language-level patterns)

---

## Pattern 1: Old-Style String Formatting

### Description
Using the `%` operator or `.format()` method for string interpolation instead of f-strings. The `%` operator is inherited from C's `printf` family and is error-prone (wrong format specifier, argument count mismatch). `.format()` is safer but more verbose than f-strings. F-strings are faster, more readable, and embed expressions directly.

### Bad Code (Anti-pattern)
```python
def generate_report(user, orders, total):
    # %-formatting: error-prone, no IDE support for argument matching
    header = "Report for %s (ID: %d)" % (user.name, user.id)
    summary = "Total orders: %d, Revenue: $%.2f" % (len(orders), total)
    timestamp = "Generated at %s" % datetime.now().isoformat()

    # .format(): verbose, positional args hard to track
    detail = "User {} placed {} orders worth ${:.2f}".format(
        user.name, len(orders), total
    )
    url = "https://api.example.com/users/{}/orders?page={}&limit={}".format(
        user.id, page, limit
    )

    # Mixed styles in one function
    log_line = "[%s] %s - %s" % (
        "INFO",
        "Processing user {}".format(user.id),
        detail
    )
    return header + "\n" + summary + "\n" + detail
```

### Good Code (Fix)
```python
def generate_report(user, orders, total):
    header = f"Report for {user.name} (ID: {user.id})"
    summary = f"Total orders: {len(orders)}, Revenue: ${total:.2f}"
    timestamp = f"Generated at {datetime.now().isoformat()}"

    detail = f"User {user.name} placed {len(orders)} orders worth ${total:.2f}"
    url = f"https://api.example.com/users/{user.id}/orders?page={page}&limit={limit}"

    log_line = f"[INFO] Processing user {user.id} - {detail}"
    return f"{header}\n{summary}\n{detail}"
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `%` operator and `string` left operand), `call` (with `attribute` `.format`)
- **Detection approach**: For `%`-formatting, find `binary_expression` nodes where the operator is `%` and the left operand is a `string` node. For `.format()`, find `call` nodes whose function is an `attribute` with attribute name `format` and whose object is a `string` node. Exclude cases where the string is used in logging calls (e.g., `logger.info("msg %s", var)`) since `%`-style is idiomatic there.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (string) @fmt_string
  operator: "%"
  right: (_) @args)

(call
  function: (attribute
    object: (string) @fmt_string
    attribute: (identifier) @method)
  (#eq? @method "format"))
```

### Pipeline Mapping
- **Pipeline name**: `old_style_formatting`
- **Pattern name**: `percent_or_format_string`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: print() for Debugging Instead of Logging

### Description
Using `print()` statements for debugging, status messages, or error reporting instead of Python's built-in `logging` module. `print()` offers no log levels, no timestamps, no structured metadata, and no way to configure output destinations or disable debug messages in production.

### Bad Code (Anti-pattern)
```python
def process_payment(order):
    print(f"Processing payment for order {order.id}")
    print(f"Amount: {order.total}")
    print(f"Customer: {order.customer.email}")

    try:
        result = payment_gateway.charge(order.total, order.customer.card)
        print(f"Payment successful: {result.transaction_id}")
    except PaymentError as e:
        print(f"ERROR: Payment failed: {e}")
        print(f"Order {order.id} payment retry needed")
        raise

    print(f"Updating order status to paid")
    order.status = "paid"
    order.save()
    print(f"Order {order.id} marked as paid")

def sync_inventory(products):
    print("Starting inventory sync...")
    for product in products:
        print(f"  Syncing {product.sku}...")
        external_stock = api.get_stock(product.sku)
        if external_stock != product.stock:
            print(f"  Stock mismatch: local={product.stock}, remote={external_stock}")
            product.stock = external_stock
            product.save()
    print(f"Inventory sync complete. {len(products)} products checked.")
```

### Good Code (Fix)
```python
import logging

logger = logging.getLogger(__name__)

def process_payment(order):
    logger.info("Processing payment", extra={
        "order_id": order.id,
        "amount": order.total,
        "customer": order.customer.email,
    })

    try:
        result = payment_gateway.charge(order.total, order.customer.card)
        logger.info("Payment successful", extra={
            "order_id": order.id,
            "transaction_id": result.transaction_id,
        })
    except PaymentError:
        logger.exception("Payment failed for order %s", order.id)
        raise

    order.status = "paid"
    order.save()
    logger.info("Order marked as paid", extra={"order_id": order.id})

def sync_inventory(products):
    logger.info("Starting inventory sync", extra={"product_count": len(products)})
    for product in products:
        logger.debug("Syncing product", extra={"sku": product.sku})
        external_stock = api.get_stock(product.sku)
        if external_stock != product.stock:
            logger.warning("Stock mismatch", extra={
                "sku": product.sku,
                "local_stock": product.stock,
                "remote_stock": external_stock,
            })
            product.stock = external_stock
            product.save()
    logger.info("Inventory sync complete", extra={"product_count": len(products)})
```

### Tree-sitter Detection Strategy
- **Target node types**: `call` with `identifier` function name `print`
- **Detection approach**: Find `call` nodes whose function is an `identifier` with value `print`. Exclude test files (`test_*.py`, `*_test.py`) and `__main__` blocks. Flag all occurrences in library/application code. Higher confidence when `print` appears inside `try`/`except` blocks (error handling) or inside class methods (application logic rather than scripts).
- **S-expression query sketch**:
```scheme
(call
  function: (identifier) @func_name
  (#eq? @func_name "print"))
```

### Pipeline Mapping
- **Pipeline name**: `print_debugging`
- **Pattern name**: `print_instead_of_logging`
- **Severity**: warning
- **Confidence**: medium
