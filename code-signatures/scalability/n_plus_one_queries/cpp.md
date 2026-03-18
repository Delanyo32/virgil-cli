# N+1 Queries -- C++

## Overview
N+1 query patterns in C++ occur when database driver methods like `stmt->execute()`, `query.exec()`, or ODBC calls are invoked inside loops instead of using prepared statements with batch parameters or constructing single queries with `IN` clauses.

## Why It's a Scalability Concern
C++ services are often chosen for performance-critical paths. N+1 patterns negate this advantage by introducing per-item round-trip latency. Each loop iteration incurs connection overhead, query parsing, and result marshaling. In high-throughput systems, this can bottleneck on database connections rather than compute.

## Applicability
- **Relevance**: medium
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: MySQL Connector/C++, Qt SQL, SOCI, ODBC, libpqxx

---

## Pattern 1: MySQL Connector Statement Execute in Loop

### Description
Calling `stmt->execute()`, `stmt->executeQuery()`, or `stmt->executeUpdate()` inside a `for` or `while` loop with per-iteration parameter binding.

### Bad Code (Anti-pattern)
```cpp
std::vector<User> getUsers(sql::Connection* conn, const std::vector<int>& userIds) {
    std::vector<User> users;
    for (int id : userIds) {
        auto stmt = conn->prepareStatement("SELECT * FROM users WHERE id = ?");
        stmt->setInt(1, id);
        auto rs = stmt->executeQuery();
        if (rs->next()) {
            users.push_back(mapUser(rs));
        }
        delete rs;
        delete stmt;
    }
    return users;
}
```

### Good Code (Fix)
```cpp
std::vector<User> getUsers(sql::Connection* conn, const std::vector<int>& userIds) {
    std::string placeholders;
    for (size_t i = 0; i < userIds.size(); ++i) {
        if (i > 0) placeholders += ",";
        placeholders += "?";
    }
    auto stmt = conn->prepareStatement("SELECT * FROM users WHERE id IN (" + placeholders + ")");
    for (size_t i = 0; i < userIds.size(); ++i) {
        stmt->setInt(i + 1, userIds[i]);
    }
    auto rs = stmt->executeQuery();
    std::vector<User> users;
    while (rs->next()) {
        users.push_back(mapUser(rs));
    }
    delete rs;
    delete stmt;
    return users;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_range_loop`, `for_statement`, `while_statement`, `call_expression`, `field_expression`
- **Detection approach**: Find `call_expression` where the function is a `field_expression` with field name `executeQuery`, `executeUpdate`, or `execute` on a pointer (`->`) expression, nested inside a `for_range_loop` or `for_statement`.
- **S-expression query sketch**:
```scheme
(for_range_loop
  body: (compound_statement
    (declaration
      declarator: (init_declarator
        value: (call_expression
          function: (field_expression
            field: (field_identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `mysql_connector_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Connection Object Query in Loop

### Description
Calling query execution methods on a database connection object (`conn->query()`, `db.exec()`, `conn.execute()`) inside a loop, where each iteration builds and sends a separate SQL statement.

### Bad Code (Anti-pattern)
```cpp
void processOrders(pqxx::connection& conn, const std::vector<int>& orderIds) {
    for (int id : orderIds) {
        pqxx::work txn(conn);
        auto result = txn.exec("SELECT * FROM orders WHERE id = " + std::to_string(id));
        for (auto row : result) {
            processOrder(row);
        }
        txn.commit();
    }
}
```

### Good Code (Fix)
```cpp
void processOrders(pqxx::connection& conn, const std::vector<int>& orderIds) {
    pqxx::work txn(conn);
    std::string ids;
    for (size_t i = 0; i < orderIds.size(); ++i) {
        if (i > 0) ids += ",";
        ids += std::to_string(orderIds[i]);
    }
    auto result = txn.exec("SELECT * FROM orders WHERE id IN (" + ids + ")");
    for (auto row : result) {
        processOrder(row);
    }
    txn.commit();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_range_loop`, `for_statement`, `call_expression`, `field_expression`
- **Detection approach**: Find `call_expression` calling `.exec()`, `.execute()`, `.query()` on an object inside a loop. The method is invoked via `field_expression` (`.`) or through pointer `->` syntax.
- **S-expression query sketch**:
```scheme
(for_range_loop
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (field_expression
          field: (field_identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `connection_query_in_loop`
- **Severity**: warning
- **Confidence**: medium
