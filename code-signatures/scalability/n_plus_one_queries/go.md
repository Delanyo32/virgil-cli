# N+1 Queries -- Go

## Overview
N+1 query patterns in Go appear when `database/sql` methods like `db.Query()`, `db.QueryRow()`, or `db.Exec()` are called inside `for` loops, making individual database calls instead of batch queries.

## Why It's a Scalability Concern
Go services often handle high concurrency via goroutines. N+1 patterns multiply database load per request — with 100 concurrent requests each executing N+1 queries, the database sees 100*(N+1) queries instead of 200. This exhausts connection pools, increases tail latency, and can trigger database connection limits.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: database/sql, sqlx, GORM, pgx, net/http

---

## Pattern 1: db.Query/QueryRow Inside For Loop

### Description
Calling `db.Query()`, `db.QueryRow()`, or `db.Exec()` inside a `for` range loop, issuing one query per iteration.

### Bad Code (Anti-pattern)
```go
func GetOrderDetails(db *sql.DB, orderIDs []int) ([]OrderDetail, error) {
    var results []OrderDetail
    for _, id := range orderIDs {
        row := db.QueryRow("SELECT * FROM orders WHERE id = $1", id)
        var order Order
        if err := row.Scan(&order.ID, &order.Name, &order.Total); err != nil {
            return nil, err
        }
        results = append(results, OrderDetail{Order: order})
    }
    return results, nil
}
```

### Good Code (Fix)
```go
func GetOrderDetails(db *sql.DB, orderIDs []int) ([]OrderDetail, error) {
    query, args, err := sqlx.In("SELECT * FROM orders WHERE id IN (?)", orderIDs)
    if err != nil {
        return nil, err
    }
    query = db.Rebind(query)
    rows, err := db.Query(query, args...)
    if err != nil {
        return nil, err
    }
    defer rows.Close()
    var results []OrderDetail
    for rows.Next() {
        var order Order
        if err := rows.Scan(&order.ID, &order.Name, &order.Total); err != nil {
            return nil, err
        }
        results = append(results, OrderDetail{Order: order})
    }
    return results, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `call_expression`, `selector_expression`
- **Detection approach**: Find `call_expression` nodes where the function is a `selector_expression` with field name `Query`, `QueryRow`, `Exec`, or `QueryContext` nested inside a `for_statement` (range loop). Walk parent nodes to confirm loop containment.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (short_var_declaration
      right: (expression_list
        (call_expression
          function: (selector_expression
            field: (field_identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `db_query_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Transaction Query Inside Loop

### Description
Calling `tx.Query()`, `tx.QueryRow()`, or `tx.Exec()` inside a loop within a transaction, which still creates separate round-trips despite being in the same transaction.

### Bad Code (Anti-pattern)
```go
func UpdatePrices(db *sql.DB, updates []PriceUpdate) error {
    tx, err := db.Begin()
    if err != nil {
        return err
    }
    defer tx.Rollback()
    for _, update := range updates {
        _, err := tx.Exec("UPDATE products SET price = $1 WHERE id = $2", update.Price, update.ID)
        if err != nil {
            return err
        }
    }
    return tx.Commit()
}
```

### Good Code (Fix)
```go
func UpdatePrices(db *sql.DB, updates []PriceUpdate) error {
    tx, err := db.Begin()
    if err != nil {
        return err
    }
    defer tx.Rollback()
    stmt, err := tx.Prepare("UPDATE products SET price = $1 WHERE id = $2")
    if err != nil {
        return err
    }
    defer stmt.Close()
    for _, update := range updates {
        _, err := stmt.Exec(update.Price, update.ID)
        if err != nil {
            return err
        }
    }
    return tx.Commit()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `call_expression`, `selector_expression`
- **Detection approach**: Find `call_expression` where the function is a `selector_expression` with field `Exec`, `Query`, or `QueryRow` on a variable (typically `tx`), inside a `for_statement`. Distinguish from prepared statement `.Exec()` by checking that the object is not from `tx.Prepare()`.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (expression_statement
      (call_expression
        function: (selector_expression
          operand: (identifier) @obj
          field: (field_identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `tx_query_in_loop`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: HTTP Client Call Inside Loop

### Description
Making HTTP requests via `http.Get()`, `http.Post()`, or `client.Do()` inside a loop, causing sequential network round-trips to external services.

### Bad Code (Anti-pattern)
```go
func EnrichUsers(client *http.Client, users []User) error {
    for i, user := range users {
        resp, err := client.Get(fmt.Sprintf("https://api.example.com/profiles/%d", user.ID))
        if err != nil {
            return err
        }
        defer resp.Body.Close()
        var profile Profile
        json.NewDecoder(resp.Body).Decode(&profile)
        users[i].Profile = profile
    }
    return nil
}
```

### Good Code (Fix)
```go
func EnrichUsers(client *http.Client, users []User) error {
    ids := make([]string, len(users))
    for i, u := range users {
        ids[i] = strconv.Itoa(u.ID)
    }
    resp, err := client.Get("https://api.example.com/profiles?ids=" + strings.Join(ids, ","))
    if err != nil {
        return err
    }
    defer resp.Body.Close()
    var profiles []Profile
    json.NewDecoder(resp.Body).Decode(&profiles)
    profileMap := make(map[int]Profile)
    for _, p := range profiles {
        profileMap[p.UserID] = p
    }
    for i, u := range users {
        users[i].Profile = profileMap[u.ID]
    }
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `call_expression`, `selector_expression`
- **Detection approach**: Find `call_expression` where the function is `http.Get`, `http.Post`, or a `selector_expression` with field `Do`, `Get`, `Post` on an `http.Client`-like variable, nested inside a `for_statement`.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (short_var_declaration
      right: (expression_list
        (call_expression
          function: (selector_expression
            operand: (identifier) @client
            field: (field_identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `http_call_in_loop`
- **Severity**: warning
- **Confidence**: medium
