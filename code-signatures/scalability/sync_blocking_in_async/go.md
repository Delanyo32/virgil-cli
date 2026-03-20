# Sync Blocking in Async -- Go

## Overview
While Go doesn't have traditional async/await, blocking patterns in concurrent Go code — such as `time.Sleep()` in request handlers, blocking channel operations without `select`, and I/O without context deadlines — degrade goroutine-based concurrency and cause resource exhaustion.

## Why It's a Scalability Concern
Go's goroutine scheduler is efficient but not immune to blocking. Blocking a goroutine that handles an HTTP request ties up resources (memory, open connections, file descriptors) for the duration. `time.Sleep()` in handlers, channel deadlocks, and I/O without timeouts can cause goroutine leaks that accumulate under load, eventually exhausting memory or file descriptors.

## Applicability
- **Relevance**: medium
- **Languages covered**: .go
- **Frameworks/libraries**: net/http, goroutines, channels, context, os

---

## Pattern 1: time.Sleep() in HTTP Handler

### Description
Using `time.Sleep()` inside an HTTP handler or request-processing goroutine for retry delays, which holds the goroutine and its resources for the sleep duration.

### Bad Code (Anti-pattern)
```go
func handleRetry(w http.ResponseWriter, r *http.Request) {
    for i := 0; i < 3; i++ {
        resp, err := callExternalAPI(r.Context())
        if err == nil {
            json.NewEncoder(w).Encode(resp)
            return
        }
        time.Sleep(time.Duration(i+1) * time.Second)
    }
    http.Error(w, "service unavailable", http.StatusServiceUnavailable)
}
```

### Good Code (Fix)
```go
func handleRetry(w http.ResponseWriter, r *http.Request) {
    for i := 0; i < 3; i++ {
        resp, err := callExternalAPI(r.Context())
        if err == nil {
            json.NewEncoder(w).Encode(resp)
            return
        }
        select {
        case <-time.After(time.Duration(i+1) * time.Second):
        case <-r.Context().Done():
            http.Error(w, "request cancelled", http.StatusRequestTimeout)
            return
        }
    }
    http.Error(w, "service unavailable", http.StatusServiceUnavailable)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `function_declaration`
- **Detection approach**: Find `call_expression` calling `time.Sleep` inside a function that is an HTTP handler (has `http.ResponseWriter` and `*http.Request` parameters) or inside a goroutine.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @method)
  (#eq? @pkg "time")
  (#eq? @method "Sleep"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `time_sleep_in_handler`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Blocking Channel Receive Without Select

### Description
Using a bare `<-ch` channel receive without a `select` statement and timeout/context, which can block indefinitely if the sender never sends.

### Bad Code (Anti-pattern)
```go
func processJob(ch <-chan Job) {
    job := <-ch  // blocks forever if channel is never written to
    result := compute(job)
    fmt.Println(result)
}
```

### Good Code (Fix)
```go
func processJob(ctx context.Context, ch <-chan Job) error {
    select {
    case job := <-ch:
        result := compute(job)
        fmt.Println(result)
        return nil
    case <-ctx.Done():
        return ctx.Err()
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `receive_statement`, `short_var_declaration`, `unary_expression`
- **Detection approach**: Find `unary_expression` with operator `<-` on a channel that is NOT inside a `select_statement` or `communication_case`. Check that the channel receive is a standalone statement or assignment, not inside a `for range` over a channel.
- **S-expression query sketch**:
```scheme
(short_var_declaration
  right: (expression_list
    (unary_expression
      operator: "<-"
      operand: (identifier) @channel)))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_channel_recv`
- **Severity**: info
- **Confidence**: low

---

## Pattern 3: File I/O Without Context/Timeout

### Description
Using `os.Open()`, `os.ReadFile()`, or `io.ReadAll()` inside a request handler without associating a context deadline, risking unbounded blocking on slow filesystems (NFS, FUSE mounts).

### Bad Code (Anti-pattern)
```go
func handleUpload(w http.ResponseWriter, r *http.Request) {
    data, err := os.ReadFile("/shared/nfs/config.json")
    if err != nil {
        http.Error(w, "failed to read config", 500)
        return
    }
    w.Write(data)
}
```

### Good Code (Fix)
```go
func handleUpload(w http.ResponseWriter, r *http.Request) {
    ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
    defer cancel()

    done := make(chan []byte, 1)
    errCh := make(chan error, 1)
    go func() {
        data, err := os.ReadFile("/shared/nfs/config.json")
        if err != nil {
            errCh <- err
            return
        }
        done <- data
    }()

    select {
    case data := <-done:
        w.Write(data)
    case err := <-errCh:
        http.Error(w, err.Error(), 500)
    case <-ctx.Done():
        http.Error(w, "timeout reading file", http.StatusGatewayTimeout)
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `function_declaration`
- **Detection approach**: Find `call_expression` calling `os.ReadFile`, `os.Open`, `io.ReadAll`, `ioutil.ReadAll` inside an HTTP handler function (identified by parameter types) where no `context.WithTimeout` or `context.WithDeadline` is present in the same function scope.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @method)
  (#eq? @pkg "os")
  (#match? @method "^(ReadFile|Open|OpenFile)$"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `file_io_without_timeout`
- **Severity**: info
- **Confidence**: low
