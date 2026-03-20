# N+1 Queries -- Python

## Overview
N+1 query patterns in Python typically arise when ORM methods or raw database cursors are called inside `for` loops, issuing one query per iteration instead of batching operations.

## Why It's a Scalability Concern
Each loop iteration creates a separate database round-trip, turning O(1) batch operations into O(N) sequential queries. With ORMs like Django and SQLAlchemy, lazy loading makes this pattern particularly insidious — it works correctly but degrades linearly with data growth.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: Django ORM, SQLAlchemy, psycopg2, sqlite3, asyncpg

---

## Pattern 1: Django ORM Query Inside Loop

### Description
Calling Django ORM methods like `Model.objects.get()`, `.filter()`, or `.first()` inside a `for` loop, often when iterating over a queryset and accessing related objects.

### Bad Code (Anti-pattern)
```python
def get_order_details(order_ids):
    results = []
    for order_id in order_ids:
        order = Order.objects.get(id=order_id)
        customer = Customer.objects.get(id=order.customer_id)
        results.append({"order": order, "customer": customer})
    return results
```

### Good Code (Fix)
```python
def get_order_details(order_ids):
    orders = Order.objects.filter(id__in=order_ids).select_related("customer")
    return [{"order": o, "customer": o.customer} for o in orders]
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `call`, `attribute`, `identifier`
- **Detection approach**: Find `call` nodes where the function is an `attribute` chain containing `.objects.get`, `.objects.filter`, or `.objects.first` that are nested inside a `for_statement` body. Walk ancestors to confirm loop containment.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (expression_statement
      (assignment
        right: (call
          function: (attribute
            attribute: (identifier) @method))))) @loop)
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `django_orm_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: SQLAlchemy Query Inside Loop

### Description
Calling `session.query()`, `session.execute()`, or `session.get()` inside a loop, issuing individual queries instead of using bulk operations or eager loading.

### Bad Code (Anti-pattern)
```python
def get_users_with_orders(session, user_ids):
    results = []
    for user_id in user_ids:
        user = session.query(User).filter_by(id=user_id).first()
        orders = session.query(Order).filter_by(user_id=user_id).all()
        results.append({"user": user, "orders": orders})
    return results
```

### Good Code (Fix)
```python
def get_users_with_orders(session, user_ids):
    users = (
        session.query(User)
        .options(joinedload(User.orders))
        .filter(User.id.in_(user_ids))
        .all()
    )
    return [{"user": u, "orders": u.orders} for u in users]
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `call`, `attribute`
- **Detection approach**: Find `call` nodes where the function chain includes `session.query`, `session.execute`, or `session.get`, nested inside a `for_statement`.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (expression_statement
      (assignment
        right: (call
          function: (attribute
            object: (call
              function: (attribute
                object: (identifier) @obj
                attribute: (identifier) @method))))))) @loop)
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `sqlalchemy_query_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Raw Cursor Execute Inside Loop

### Description
Calling `cursor.execute()` inside a loop instead of using `executemany()` or parameterized batch queries.

### Bad Code (Anti-pattern)
```python
def insert_records(cursor, records):
    for record in records:
        cursor.execute(
            "INSERT INTO users (name, email) VALUES (%s, %s)",
            (record["name"], record["email"]),
        )
```

### Good Code (Fix)
```python
def insert_records(cursor, records):
    cursor.executemany(
        "INSERT INTO users (name, email) VALUES (%s, %s)",
        [(r["name"], r["email"]) for r in records],
    )
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `call`, `attribute`
- **Detection approach**: Find `call` nodes where the callee is `attribute` with name `execute` on an identifier (cursor-like), nested inside a `for_statement`.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (expression_statement
      (call
        function: (attribute
          object: (identifier) @cursor
          attribute: (identifier) @method)))) @loop)
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `raw_cursor_in_loop`
- **Severity**: warning
- **Confidence**: high
