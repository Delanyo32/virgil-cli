# Resource Lifecycle -- Go

## Overview
Resources that are acquired but never properly released cause goroutine leaks, file descriptor exhaustion, and connection pool starvation. In Go, the most common manifestations are `defer` calls inside loops that do not execute until function return, and HTTP response bodies that are never closed.

## Why It's a Tech Debt Concern
`defer` in a loop accumulates deferred calls on the stack that do not execute until the enclosing function returns. For long-running loops processing thousands of items, this means thousands of file handles, connections, or locks remain open simultaneously, exhausting OS resources. Failing to close `resp.Body` after an HTTP request leaks the underlying TCP connection, preventing it from being returned to the connection pool. Under load, the application opens new connections for every request until the system runs out of file descriptors or ephemeral ports.

## Applicability
- **Relevance**: high (HTTP clients, file processing, database operations)
- **Languages covered**: `.go`
- **Frameworks/libraries**: `net/http`, `os`, `database/sql`, any resource with a `Close()` method

---

## Pattern 1: defer in Loop

### Description
Using `defer` inside a `for` loop to close a resource. The deferred call does not execute at the end of each loop iteration -- it executes when the enclosing function returns. This means all resources opened during the loop remain open simultaneously, leading to resource exhaustion for loops with many iterations.

### Bad Code (Anti-pattern)
```go
func processFiles(paths []string) error {
    for _, path := range paths {
        f, err := os.Open(path)
        if err != nil {
            return fmt.Errorf("opening %s: %w", path, err)
        }
        defer f.Close() // Deferred until function returns, not loop iteration
        data, err := io.ReadAll(f)
        if err != nil {
            return fmt.Errorf("reading %s: %w", path, err)
        }
        process(data)
    }
    return nil
}

func queryAll(db *sql.DB, queries []string) ([][]Row, error) {
    var results [][]Row
    for _, q := range queries {
        rows, err := db.Query(q)
        if err != nil {
            return nil, err
        }
        defer rows.Close() // All row sets open until function returns
        var batch []Row
        for rows.Next() {
            var r Row
            rows.Scan(&r.ID, &r.Name)
            batch = append(batch, r)
        }
        results = append(results, batch)
    }
    return results, nil
}
```

### Good Code (Fix)
```go
func processFiles(paths []string) error {
    for _, path := range paths {
        if err := processFile(path); err != nil {
            return err
        }
    }
    return nil
}

func processFile(path string) error {
    f, err := os.Open(path)
    if err != nil {
        return fmt.Errorf("opening %s: %w", path, err)
    }
    defer f.Close() // Now deferred to this function's return -- one file at a time
    data, err := io.ReadAll(f)
    if err != nil {
        return fmt.Errorf("reading %s: %w", path, err)
    }
    process(data)
    return nil
}

func queryAll(db *sql.DB, queries []string) ([][]Row, error) {
    var results [][]Row
    for _, q := range queries {
        batch, err := executeQuery(db, q)
        if err != nil {
            return nil, err
        }
        results = append(results, batch)
    }
    return results, nil
}

func executeQuery(db *sql.DB, q string) ([]Row, error) {
    rows, err := db.Query(q)
    if err != nil {
        return nil, err
    }
    defer rows.Close()
    var batch []Row
    for rows.Next() {
        var r Row
        rows.Scan(&r.ID, &r.Name)
        batch = append(batch, r)
    }
    return batch, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `defer_statement`, `for_statement`, `call_expression`
- **Detection approach**: Find `defer_statement` nodes that are descendants of a `for_statement` body block. Any `defer` inside a `for` loop is a potential resource leak. Check that the deferred call involves a `.Close()` or similar resource-releasing method to reduce false positives.
- **S-expression query sketch**:
  ```scheme
  (for_statement
    body: (block
      (defer_statement
        (call_expression
          function: (selector_expression
            field: (field_identifier) @deferred_method)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `defer_in_loop`
- **Pattern name**: `deferred_close_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Response Body Not Closed

### Description
Making an HTTP request with `http.Get`, `http.Post`, or `client.Do` and failing to close `resp.Body`. The HTTP transport keeps the underlying TCP connection open until the body is fully read and closed. Leaked bodies prevent connection reuse, exhaust the client's connection pool, and eventually cause "dial tcp: too many open files" errors.

### Bad Code (Anti-pattern)
```go
func fetchData(url string) ([]byte, error) {
    resp, err := http.Get(url)
    if err != nil {
        return nil, err
    }
    // resp.Body never closed
    return io.ReadAll(resp.Body)
}

func checkHealth(urls []string) map[string]bool {
    results := make(map[string]bool)
    for _, url := range urls {
        resp, err := http.Get(url)
        if err != nil {
            results[url] = false
            continue
        }
        results[url] = resp.StatusCode == 200
        // Body not closed -- connection leaked for every URL
    }
    return results
}

func postJSON(url string, payload interface{}) error {
    body, _ := json.Marshal(payload)
    resp, err := http.Post(url, "application/json", bytes.NewReader(body))
    if err != nil {
        return err
    }
    if resp.StatusCode != 200 {
        return fmt.Errorf("unexpected status: %d", resp.StatusCode)
        // Body not closed on error path
    }
    return nil
}
```

### Good Code (Fix)
```go
func fetchData(url string) ([]byte, error) {
    resp, err := http.Get(url)
    if err != nil {
        return nil, err
    }
    defer resp.Body.Close()
    return io.ReadAll(resp.Body)
}

func checkHealth(urls []string) map[string]bool {
    results := make(map[string]bool)
    for _, url := range urls {
        resp, err := http.Get(url)
        if err != nil {
            results[url] = false
            continue
        }
        results[url] = resp.StatusCode == 200
        // Drain and close body to allow connection reuse
        io.Copy(io.Discard, resp.Body)
        resp.Body.Close()
    }
    return results
}

func postJSON(url string, payload interface{}) error {
    body, _ := json.Marshal(payload)
    resp, err := http.Post(url, "application/json", bytes.NewReader(body))
    if err != nil {
        return err
    }
    defer resp.Body.Close()
    if resp.StatusCode != 200 {
        return fmt.Errorf("unexpected status: %d", resp.StatusCode)
    }
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `short_var_declaration`, `call_expression`, `selector_expression`, `defer_statement`
- **Detection approach**: Find `short_var_declaration` nodes where the right side is a `call_expression` to `http.Get`, `http.Post`, `http.Do`, or `client.Do` (via `selector_expression`). Then check if the enclosing function body contains a `defer_statement` or direct call closing `resp.Body.Close()`. Flag functions where the response variable is assigned but no corresponding `.Body.Close()` call exists.
- **S-expression query sketch**:
  ```scheme
  ;; HTTP request assignment
  (short_var_declaration
    left: (expression_list
      (identifier) @resp_var
      (identifier) @err_var)
    right: (expression_list
      (call_expression
        function: (selector_expression
          operand: (identifier) @http_pkg
          field: (field_identifier) @http_method))))

  ;; defer resp.Body.Close()
  (defer_statement
    (call_expression
      function: (selector_expression
        operand: (selector_expression
          operand: (identifier) @resp_ref
          field: (field_identifier) @body_field)
        field: (field_identifier) @close_method)))
  ```

### Pipeline Mapping
- **Pipeline name**: `defer_in_loop`
- **Pattern name**: `response_body_not_closed`
- **Severity**: warning
- **Confidence**: high
