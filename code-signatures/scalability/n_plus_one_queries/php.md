# N+1 Queries -- PHP

## Overview
N+1 query patterns in PHP are prevalent in Laravel/Eloquent applications where model lookups or raw PDO queries are called inside `foreach` or `for` loops instead of using eager loading or batch queries.

## Why It's a Scalability Concern
PHP processes each request in a single thread — N+1 queries serialize all database round-trips, directly inflating response time. With Eloquent's lazy loading, accessing `$post->author` in a Blade template loop triggers a hidden query per iteration, scaling linearly with collection size.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: Laravel/Eloquent, PDO, mysqli, Doctrine

---

## Pattern 1: Eloquent Find/Where in Loop

### Description
Calling `Model::find()`, `Model::where()->first()`, or accessing lazy-loaded relationships inside a `foreach` or `for` loop.

### Bad Code (Anti-pattern)
```php
function getOrderDetails(array $orderIds): array
{
    $results = [];
    foreach ($orderIds as $id) {
        $order = Order::find($id);
        $customer = Customer::find($order->customer_id);
        $results[] = ['order' => $order, 'customer' => $customer];
    }
    return $results;
}
```

### Good Code (Fix)
```php
function getOrderDetails(array $orderIds): array
{
    $orders = Order::with('customer')->whereIn('id', $orderIds)->get();
    return $orders->map(fn($order) => [
        'order' => $order,
        'customer' => $order->customer,
    ])->toArray();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `foreach_statement`, `for_statement`, `member_call_expression`, `scoped_call_expression`, `static_method`
- **Detection approach**: Find `scoped_call_expression` (static call like `Model::find()`) or `member_call_expression` (like `->where()->first()`) inside a `foreach_statement` body. Look for method names `find`, `findOrFail`, `first`, `firstOrFail`.
- **S-expression query sketch**:
```scheme
(foreach_statement
  body: (compound_statement
    (expression_statement
      (assignment_expression
        right: (scoped_call_expression
          name: (name) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `eloquent_find_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: PDO Execute in Loop

### Description
Calling `$pdo->query()` or `$stmt->execute()` inside a loop, issuing individual SQL statements instead of using batch inserts or parameterized `IN` clauses.

### Bad Code (Anti-pattern)
```php
function insertUsers(PDO $pdo, array $users): void
{
    foreach ($users as $user) {
        $stmt = $pdo->prepare("INSERT INTO users (name, email) VALUES (?, ?)");
        $stmt->execute([$user['name'], $user['email']]);
    }
}
```

### Good Code (Fix)
```php
function insertUsers(PDO $pdo, array $users): void
{
    $placeholders = implode(',', array_fill(0, count($users), '(?, ?)'));
    $stmt = $pdo->prepare("INSERT INTO users (name, email) VALUES $placeholders");
    $values = [];
    foreach ($users as $user) {
        $values[] = $user['name'];
        $values[] = $user['email'];
    }
    $stmt->execute($values);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `foreach_statement`, `for_statement`, `member_call_expression`, `method_name`
- **Detection approach**: Find `member_call_expression` calling `execute`, `query`, or `exec` on a PDO-like variable (`$pdo`, `$stmt`, `$db`), nested inside a loop.
- **S-expression query sketch**:
```scheme
(foreach_statement
  body: (compound_statement
    (expression_statement
      (member_call_expression
        name: (name) @method
        object: (variable_name) @var))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `pdo_execute_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: mysqli_query in Loop

### Description
Calling `mysqli_query()` procedural function inside a loop, executing individual queries instead of batch operations.

### Bad Code (Anti-pattern)
```php
function getUsers(mysqli $conn, array $userIds): array
{
    $users = [];
    foreach ($userIds as $id) {
        $result = mysqli_query($conn, "SELECT * FROM users WHERE id = $id");
        $users[] = mysqli_fetch_assoc($result);
    }
    return $users;
}
```

### Good Code (Fix)
```php
function getUsers(mysqli $conn, array $userIds): array
{
    $ids = implode(',', array_map('intval', $userIds));
    $result = mysqli_query($conn, "SELECT * FROM users WHERE id IN ($ids)");
    $users = [];
    while ($row = mysqli_fetch_assoc($result)) {
        $users[] = $row;
    }
    return $users;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `foreach_statement`, `for_statement`, `function_call_expression`, `name`
- **Detection approach**: Find `function_call_expression` calling `mysqli_query` nested inside a `foreach_statement` or `for_statement`.
- **S-expression query sketch**:
```scheme
(foreach_statement
  body: (compound_statement
    (expression_statement
      (assignment_expression
        right: (function_call_expression
          function: (name) @func_name)))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `mysqli_query_in_loop`
- **Severity**: warning
- **Confidence**: high
