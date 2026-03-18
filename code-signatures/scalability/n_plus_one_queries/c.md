# N+1 Queries -- C

## Overview
N+1 query patterns in C appear when database API functions like `mysql_query()`, `sqlite3_exec()`, or `PQexec()` are called inside `for` or `while` loops, executing individual SQL statements instead of constructing batch queries.

## Why It's a Scalability Concern
C applications often serve as high-performance backends or embedded systems. Each database call inside a loop incurs network round-trip overhead, query parsing cost, and context switching. Since C lacks ORM abstractions, developers manually construct queries — making it easy to default to per-item queries when batch alternatives exist.

## Applicability
- **Relevance**: medium
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libmysqlclient, libsqlite3, libpq (PostgreSQL)

---

## Pattern 1: Database Query Function in Loop

### Description
Calling `mysql_query()`, `sqlite3_exec()`, `PQexec()`, or similar database functions inside a `for` or `while` loop, executing one query per iteration.

### Bad Code (Anti-pattern)
```c
int get_users(MYSQL *conn, int *user_ids, int count) {
    char query[256];
    for (int i = 0; i < count; i++) {
        snprintf(query, sizeof(query), "SELECT * FROM users WHERE id = %d", user_ids[i]);
        if (mysql_query(conn, query) != 0) {
            return -1;
        }
        MYSQL_RES *result = mysql_store_result(conn);
        process_result(result);
        mysql_free_result(result);
    }
    return 0;
}
```

### Good Code (Fix)
```c
int get_users(MYSQL *conn, int *user_ids, int count) {
    char query[4096];
    int offset = snprintf(query, sizeof(query), "SELECT * FROM users WHERE id IN (");
    for (int i = 0; i < count; i++) {
        offset += snprintf(query + offset, sizeof(query) - offset, "%s%d", i > 0 ? "," : "", user_ids[i]);
    }
    snprintf(query + offset, sizeof(query) - offset, ")");
    if (mysql_query(conn, query) != 0) {
        return -1;
    }
    MYSQL_RES *result = mysql_store_result(conn);
    process_all_results(result);
    mysql_free_result(result);
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `while_statement`, `call_expression`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the function is an `identifier` matching `mysql_query`, `sqlite3_exec`, `sqlite3_step`, `PQexec`, `PQexecParams` nested inside a `for_statement` or `while_statement` body.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @func_name))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `db_query_in_loop`
- **Severity**: warning
- **Confidence**: high
