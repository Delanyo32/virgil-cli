# Concurrency Misuse -- Go

## Overview
Go's goroutines and channels make concurrency accessible, but misuse leads to goroutine leaks, lost context propagation, and deadlocks from improper mutex handling. These are among the most common production issues in Go services.

## Why It's a Tech Debt Concern
Leaked goroutines accumulate over the lifetime of a process, consuming memory and CPU indefinitely — a slow resource leak that eventually causes OOM kills or performance degradation. Missing context propagation breaks cancellation chains, causing downstream operations to continue after the caller has timed out or cancelled, wasting resources and potentially corrupting state. Unlocked mutexes on error paths cause deadlocks that manifest only under specific failure conditions.

## Applicability
- **Relevance**: high
- **Languages covered**: `.go`
- **Frameworks/libraries**: standard library (sync, context), net/http, gRPC, database/sql
- **Existing pipeline**: `goroutine_leak` in `src/audit/pipelines/go/` — extends with detection patterns
- **Existing pipeline**: `context_not_propagated` in `src/audit/pipelines/go/` — extends with detection patterns
- **Existing pipeline**: `mutex_misuse` in `src/audit/pipelines/go/` — extends with detection patterns

---

## Pattern 1: Goroutine Leak

### Description
A goroutine is started with `go func()` but has no exit condition — it blocks forever on a channel read, a select without a done/context case, or an infinite loop without a termination signal. These leaked goroutines accumulate and are never garbage collected.

### Bad Code (Anti-pattern)
```go
func startWorker(jobs <-chan Job) {
    go func() {
        for {
            job := <-jobs  // Blocks forever if channel is never closed
            process(job)
        }
    }()
}

func watchChanges(ch <-chan Event) {
    go func() {
        for {
            select {
            case event := <-ch:
                handleEvent(event)
            // No context.Done() or quit channel — no way to stop
            }
        }
    }()
}
```

### Good Code (Fix)
```go
func startWorker(ctx context.Context, jobs <-chan Job) {
    go func() {
        for {
            select {
            case <-ctx.Done():
                return
            case job, ok := <-jobs:
                if !ok {
                    return  // Channel closed
                }
                process(job)
            }
        }
    }()
}

func watchChanges(ctx context.Context, ch <-chan Event) {
    go func() {
        for {
            select {
            case <-ctx.Done():
                return
            case event, ok := <-ch:
                if !ok {
                    return
                }
                handleEvent(event)
            }
        }
    }()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `go_statement`, `func_literal`, `for_statement`, `select_statement`, `communication_case`
- **Detection approach**: Find `go_statement` nodes containing a `func_literal` with a `for_statement` (infinite loop). Inside the loop, look for `select_statement` blocks. Flag when no `communication_case` reads from a `context.Done()` channel or a quit/done channel. Also flag bare channel receives without an `ok` check for closed channels.
- **S-expression query sketch**:
```scheme
(go_statement
  (call_expression
    function: (func_literal
      body: (block
        (for_statement
          body: (block
            (select_statement
              (communication_case) @case)))))))
```

### Pipeline Mapping
- **Pipeline name**: `goroutine_leak`
- **Pattern name**: `goroutine_no_exit`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Context Not Propagated

### Description
A function receives a `context.Context` parameter but calls downstream functions (HTTP requests, database queries, gRPC calls) without passing the context, or creates a new `context.Background()` / `context.TODO()` instead of propagating the caller's context. This breaks cancellation, timeout, and deadline propagation.

### Bad Code (Anti-pattern)
```go
func GetUserOrders(ctx context.Context, userID string) ([]Order, error) {
    // Ignores the incoming context entirely
    user, err := db.QueryRow(context.Background(), "SELECT * FROM users WHERE id = $1", userID)
    if err != nil {
        return nil, err
    }

    // Creates a new context instead of propagating
    orders, err := orderClient.ListOrders(context.TODO(), &pb.ListRequest{UserID: userID})
    if err != nil {
        return nil, err
    }

    // HTTP call without context
    resp, err := http.Get(fmt.Sprintf("https://api.example.com/enrichment/%s", userID))
    if err != nil {
        return nil, err
    }
    defer resp.Body.Close()

    return orders, nil
}
```

### Good Code (Fix)
```go
func GetUserOrders(ctx context.Context, userID string) ([]Order, error) {
    user, err := db.QueryRowContext(ctx, "SELECT * FROM users WHERE id = $1", userID)
    if err != nil {
        return nil, err
    }

    orders, err := orderClient.ListOrders(ctx, &pb.ListRequest{UserID: userID})
    if err != nil {
        return nil, err
    }

    req, err := http.NewRequestWithContext(ctx, http.MethodGet,
        fmt.Sprintf("https://api.example.com/enrichment/%s", userID), nil)
    if err != nil {
        return nil, err
    }
    resp, err := http.DefaultClient.Do(req)
    if err != nil {
        return nil, err
    }
    defer resp.Body.Close()

    return orders, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `parameter_list`, `call_expression`, `selector_expression`
- **Detection approach**: Find functions where the first parameter is `ctx context.Context`. Within the function body, find `call_expression` nodes that pass `context.Background()` or `context.TODO()` as an argument, or calls to known context-aware APIs (e.g., `http.Get` instead of `http.NewRequestWithContext`) that omit the context parameter entirely.
- **S-expression query sketch**:
```scheme
(function_declaration
  parameters: (parameter_list
    (parameter_declaration
      name: (identifier) @ctx_name
      type: (qualified_type) @ctx_type))
  body: (block
    (expression_statement
      (call_expression
        arguments: (argument_list
          (call_expression
            function: (selector_expression
              field: (field_identifier) @bg_method)))))))
```

### Pipeline Mapping
- **Pipeline name**: `context_not_propagated`
- **Pattern name**: `context_background_in_handler`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Mutex Misuse -- Unlock Not Deferred

### Description
Calling `mu.Lock()` without an immediately following `defer mu.Unlock()`. Manual unlock calls are easily skipped on early return paths, panic paths, or error branches, leading to deadlocks that only manifest under specific failure conditions.

### Bad Code (Anti-pattern)
```go
func (s *Store) Update(key string, value interface{}) error {
    s.mu.Lock()

    existing, ok := s.data[key]
    if !ok {
        s.mu.Unlock()
        return fmt.Errorf("key %s not found", key)  // OK here, but fragile
    }

    if err := validate(value); err != nil {
        // Forgot to unlock before returning!
        return fmt.Errorf("validation failed: %w", err)
    }

    s.data[key] = value
    s.mu.Unlock()
    return nil
}
```

### Good Code (Fix)
```go
func (s *Store) Update(key string, value interface{}) error {
    s.mu.Lock()
    defer s.mu.Unlock()

    existing, ok := s.data[key]
    if !ok {
        return fmt.Errorf("key %s not found", key)
    }

    if err := validate(value); err != nil {
        return fmt.Errorf("validation failed: %w", err)
    }

    s.data[key] = value
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `defer_statement`, `expression_statement`
- **Detection approach**: Find `call_expression` nodes that invoke `.Lock()` on a receiver (via `selector_expression` with field `Lock`). Then check whether the immediately following statement is a `defer_statement` containing `.Unlock()` on the same receiver. Flag when `Lock()` is not followed by `defer Unlock()`.
- **S-expression query sketch**:
```scheme
(expression_statement
  (call_expression
    function: (selector_expression
      operand: (_) @receiver
      field: (field_identifier) @lock_method)))
```

### Pipeline Mapping
- **Pipeline name**: `mutex_misuse`
- **Pattern name**: `unlock_not_deferred`
- **Severity**: warning
- **Confidence**: high
