# N+1 Queries -- Java

## Overview
N+1 query patterns in Java occur when JPA/Hibernate entity lookups, JDBC statement executions, or Spring Data repository calls are made inside loops instead of using batch fetching, joins, or `IN` clauses.

## Why It's a Scalability Concern
Each loop iteration triggers a separate database round-trip through the connection pool. With JPA's lazy loading defaults, traversing entity relationships in loops silently generates queries. Under load, this exhausts connection pools, increases GC pressure from short-lived result objects, and causes timeouts.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: JPA/Hibernate, Spring Data JPA, JDBC, MyBatis

---

## Pattern 1: JPA EntityManager Find in Loop

### Description
Calling `entityManager.find()` or `entityManager.createQuery().getSingleResult()` inside a `for` or `while` loop instead of using a single query with an `IN` clause or batch fetch.

### Bad Code (Anti-pattern)
```java
public List<OrderDTO> getOrderDetails(List<Long> orderIds) {
    List<OrderDTO> results = new ArrayList<>();
    for (Long id : orderIds) {
        Order order = entityManager.find(Order.class, id);
        Customer customer = entityManager.find(Customer.class, order.getCustomerId());
        results.add(new OrderDTO(order, customer));
    }
    return results;
}
```

### Good Code (Fix)
```java
public List<OrderDTO> getOrderDetails(List<Long> orderIds) {
    List<Order> orders = entityManager
        .createQuery("SELECT o FROM Order o JOIN FETCH o.customer WHERE o.id IN :ids", Order.class)
        .setParameter("ids", orderIds)
        .getResultList();
    return orders.stream()
        .map(o -> new OrderDTO(o, o.getCustomer()))
        .collect(Collectors.toList());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `enhanced_for_statement`, `for_statement`, `while_statement`, `method_invocation`
- **Detection approach**: Find `method_invocation` nodes calling `find`, `createQuery`, `getSingleResult` on an object, nested inside a loop construct. Walk ancestor nodes to check for `enhanced_for_statement`, `for_statement`, or `while_statement`.
- **S-expression query sketch**:
```scheme
(enhanced_for_statement
  body: (block
    (local_variable_declaration
      declarator: (variable_declarator
        value: (method_invocation
          name: (identifier) @method
          object: (identifier) @target)))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `jpa_find_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: JDBC Statement Execute in Loop

### Description
Calling `statement.executeQuery()` or `preparedStatement.executeQuery()` inside a loop, executing individual SQL statements instead of batch operations.

### Bad Code (Anti-pattern)
```java
public List<User> getUsersByIds(Connection conn, List<Integer> userIds) throws SQLException {
    List<User> users = new ArrayList<>();
    for (int id : userIds) {
        PreparedStatement stmt = conn.prepareStatement("SELECT * FROM users WHERE id = ?");
        stmt.setInt(1, id);
        ResultSet rs = stmt.executeQuery();
        if (rs.next()) {
            users.add(mapUser(rs));
        }
    }
    return users;
}
```

### Good Code (Fix)
```java
public List<User> getUsersByIds(Connection conn, List<Integer> userIds) throws SQLException {
    String placeholders = userIds.stream().map(id -> "?").collect(Collectors.joining(","));
    PreparedStatement stmt = conn.prepareStatement("SELECT * FROM users WHERE id IN (" + placeholders + ")");
    for (int i = 0; i < userIds.size(); i++) {
        stmt.setInt(i + 1, userIds.get(i));
    }
    ResultSet rs = stmt.executeQuery();
    List<User> users = new ArrayList<>();
    while (rs.next()) {
        users.add(mapUser(rs));
    }
    return users;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `enhanced_for_statement`, `for_statement`, `method_invocation`
- **Detection approach**: Find `method_invocation` with name `executeQuery` or `executeUpdate` nested inside a loop body. The method is typically called on a `PreparedStatement` or `Statement` variable.
- **S-expression query sketch**:
```scheme
(enhanced_for_statement
  body: (block
    (expression_statement
      (method_invocation
        name: (identifier) @method
        object: (identifier) @stmt))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `jdbc_execute_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Spring Data Repository Call in Loop

### Description
Calling Spring Data repository methods like `findById()`, `findByName()`, or custom query methods inside a loop instead of using `findAllById()` or `@Query` with `IN`.

### Bad Code (Anti-pattern)
```java
public List<ProductDTO> getProductsWithCategories(List<Long> productIds) {
    List<ProductDTO> results = new ArrayList<>();
    for (Long id : productIds) {
        Product product = productRepository.findById(id).orElseThrow();
        Category category = categoryRepository.findById(product.getCategoryId()).orElseThrow();
        results.add(new ProductDTO(product, category));
    }
    return results;
}
```

### Good Code (Fix)
```java
public List<ProductDTO> getProductsWithCategories(List<Long> productIds) {
    List<Product> products = productRepository.findAllById(productIds);
    Set<Long> categoryIds = products.stream().map(Product::getCategoryId).collect(Collectors.toSet());
    Map<Long, Category> categoryMap = categoryRepository.findAllById(categoryIds)
        .stream().collect(Collectors.toMap(Category::getId, Function.identity()));
    return products.stream()
        .map(p -> new ProductDTO(p, categoryMap.get(p.getCategoryId())))
        .collect(Collectors.toList());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `enhanced_for_statement`, `method_invocation`
- **Detection approach**: Find `method_invocation` whose name starts with `find` (e.g., `findById`, `findByName`) called on a variable ending in `Repository` or `Repo`, nested inside a loop.
- **S-expression query sketch**:
```scheme
(enhanced_for_statement
  body: (block
    (local_variable_declaration
      declarator: (variable_declarator
        value: (method_invocation
          name: (identifier) @method
          object: (identifier) @repo)))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `spring_repo_in_loop`
- **Severity**: warning
- **Confidence**: high
