# Error Handling Anti-patterns -- Go

## Overview
Errors that are silently swallowed, ignored, or propagated without context make debugging impossible and hide real failures. In Go, the explicit `error` return value convention means anti-patterns manifest as discarded error returns, empty error checks, and errors that are logged but not propagated.

## Why It's a Tech Debt Concern
Go's error handling model is explicit by design -- every function that can fail returns an `error`. When developers bypass this contract by assigning errors to `_`, returning bare errors without context, or logging errors without propagation, they defeat the entire purpose of Go's error model. Swallowed errors in production cause silent data corruption, incomplete transactions, and cascading failures that are nearly impossible to trace back to their origin.

## Applicability
- **Relevance**: high
- **Languages covered**: `.go`

---

## Pattern 1: Ignored Error Return

### Description
Assigning the error return value to `_` with `result, _ := someFunc()` or calling a function that returns an error without capturing the return value at all. The error is completely discarded and the program continues with potentially invalid data.

### Bad Code (Anti-pattern)
```go
func processFile(path string) []byte {
    data, _ := os.ReadFile(path)
    // data is nil if ReadFile failed, but we use it anyway
    return data
}

func updateDatabase(db *sql.DB, user User) {
    db.Exec("UPDATE users SET name = ? WHERE id = ?", user.Name, user.ID)
    // Return value with error completely ignored
}

func closeResources(f *os.File, conn net.Conn) {
    f.Close()
    conn.Close()
    // Close errors silently ignored
}
```

### Good Code (Fix)
```go
func processFile(path string) ([]byte, error) {
    data, err := os.ReadFile(path)
    if err != nil {
        return nil, fmt.Errorf("reading file %s: %w", path, err)
    }
    return data, nil
}

func updateDatabase(db *sql.DB, user User) error {
    _, err := db.Exec("UPDATE users SET name = ? WHERE id = ?", user.Name, user.ID)
    if err != nil {
        return fmt.Errorf("updating user %d: %w", user.ID, err)
    }
    return nil
}

func closeResources(f *os.File, conn net.Conn) error {
    if err := f.Close(); err != nil {
        return fmt.Errorf("closing file: %w", err)
    }
    if err := conn.Close(); err != nil {
        return fmt.Errorf("closing connection: %w", err)
    }
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `short_var_declaration`, `assignment_statement`, `call_expression`, `expression_statement`
- **Detection approach**: Find `short_var_declaration` or `assignment_statement` nodes where the left side is an `expression_list` containing a blank identifier `_` in the error position (typically the last element). Also flag `expression_statement` nodes containing a `call_expression` where the function is known to return an error (heuristic: most Go functions return error).
- **S-expression query sketch**:
```scheme
;; result, _ := someFunc()
(short_var_declaration
  left: (expression_list
    (identifier)
    (identifier) @blank_id)
  right: (expression_list
    (call_expression) @call))

;; someFunc() as a bare statement
(expression_statement
  (call_expression
    function: (selector_expression
      field: (field_identifier) @method)))
```

### Pipeline Mapping
- **Pipeline name**: `error_swallowing`
- **Pattern name**: `ignored_error_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Empty Error Check

### Description
Checking `if err != nil` but then returning or continuing without adding context via `fmt.Errorf` with `%w`. The error propagates up the call stack but loses all context about where and why it occurred, making production debugging a guessing game.

### Bad Code (Anti-pattern)
```go
func getUser(db *sql.DB, id int64) (*User, error) {
    row := db.QueryRow("SELECT * FROM users WHERE id = ?", id)
    var user User
    err := row.Scan(&user.ID, &user.Name, &user.Email)
    if err != nil {
        return nil, err  // No context -- caller gets raw sql error
    }
    return &user, nil
}

func processOrder(order Order) error {
    if err := validateOrder(order); err != nil {
        return err  // Which order? Which validation failed?
    }
    if err := chargePayment(order); err != nil {
        return err
    }
    return nil
}
```

### Good Code (Fix)
```go
func getUser(db *sql.DB, id int64) (*User, error) {
    row := db.QueryRow("SELECT * FROM users WHERE id = ?", id)
    var user User
    err := row.Scan(&user.ID, &user.Name, &user.Email)
    if err != nil {
        return nil, fmt.Errorf("scanning user %d: %w", id, err)
    }
    return &user, nil
}

func processOrder(order Order) error {
    if err := validateOrder(order); err != nil {
        return fmt.Errorf("validating order %s: %w", order.ID, err)
    }
    if err := chargePayment(order); err != nil {
        return fmt.Errorf("charging payment for order %s: %w", order.ID, err)
    }
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `binary_expression`, `return_statement`
- **Detection approach**: Find `if_statement` nodes where the condition is a `binary_expression` comparing an identifier (typically `err`) with `nil` using `!=`, and the body contains a `return_statement` that returns the error identifier directly without wrapping it in `fmt.Errorf`. Check that the return expression is just the bare identifier, not a `call_expression`.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (binary_expression
    left: (identifier) @err_var
    right: (nil))
  consequence: (block
    (return_statement
      (expression_list
        (identifier) @returned_err))))
```

### Pipeline Mapping
- **Pipeline name**: `error_swallowing`
- **Pattern name**: `unwrapped_error_return`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Error Swallowing

### Description
Logging an error (via `log.Println`, `fmt.Println`, etc.) but then continuing execution as if nothing happened. The error is acknowledged but not propagated, so the caller has no indication that the operation failed.

### Bad Code (Anti-pattern)
```go
func syncData(src, dst *sql.DB) {
    rows, err := src.Query("SELECT * FROM records")
    if err != nil {
        log.Println("query failed:", err)
        return  // Silently returns, caller thinks sync succeeded
    }
    defer rows.Close()

    for rows.Next() {
        var record Record
        if err := rows.Scan(&record.ID, &record.Data); err != nil {
            log.Printf("scan error: %v", err)
            continue  // Skips corrupted row silently
        }
        if _, err := dst.Exec("INSERT INTO records VALUES (?, ?)", record.ID, record.Data); err != nil {
            fmt.Println("insert failed:", err)
            // Continues to next record, data loss
        }
    }
}
```

### Good Code (Fix)
```go
func syncData(src, dst *sql.DB) error {
    rows, err := src.Query("SELECT * FROM records")
    if err != nil {
        return fmt.Errorf("querying source records: %w", err)
    }
    defer rows.Close()

    var errs []error
    for rows.Next() {
        var record Record
        if err := rows.Scan(&record.ID, &record.Data); err != nil {
            errs = append(errs, fmt.Errorf("scanning record: %w", err))
            continue
        }
        if _, err := dst.Exec("INSERT INTO records VALUES (?, ?)", record.ID, record.Data); err != nil {
            errs = append(errs, fmt.Errorf("inserting record %d: %w", record.ID, err))
        }
    }
    if len(errs) > 0 {
        return fmt.Errorf("sync completed with %d errors: %w", len(errs), errors.Join(errs...))
    }
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `binary_expression`, `call_expression`, `selector_expression`
- **Detection approach**: Find `if_statement` blocks checking `err != nil` where the body contains a `call_expression` to `log.Println`, `log.Printf`, `fmt.Println`, or similar logging functions, but does not contain a `return_statement` that returns the error. Also flag when the body ends with `continue` instead of error accumulation.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (binary_expression
    left: (identifier) @err_var
    right: (nil))
  consequence: (block
    (expression_statement
      (call_expression
        function: (selector_expression
          operand: (identifier) @log_pkg
          field: (field_identifier) @log_method)))))
```

### Pipeline Mapping
- **Pipeline name**: `error_swallowing`
- **Pattern name**: `logged_not_propagated`
- **Severity**: warning
- **Confidence**: medium
