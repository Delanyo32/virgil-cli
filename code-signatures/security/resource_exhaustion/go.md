# Resource Exhaustion -- Go

## Overview
Resource exhaustion vulnerabilities in Go arise from unbounded creation of goroutines or reading entire request bodies into memory without size limits. Go's lightweight goroutines make it tempting to spawn one per request or task, but without bounds this can exhaust memory and scheduler resources. Similarly, reading HTTP request bodies without `io.LimitReader` or `http.MaxBytesReader` allows attackers to consume all available memory.

## Why It's a Security Concern
Unbounded goroutine creation allows attackers to overwhelm the Go runtime by flooding the server with requests, each spawning new goroutines that accumulate without limit. Each goroutine consumes at least 2-8KB of stack space, and millions of goroutines can exhaust memory. Reading entire request bodies without limits allows a single request with a multi-gigabyte payload to crash the server via OOM. Both attacks are cheap to execute and cause complete service unavailability.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: net/http, io, io/ioutil, gorilla/mux, gin, echo

---

## Pattern 1: Unbounded Goroutine Creation from Requests

### Description
Spawning a new goroutine (`go func()`) for each incoming request or user-controlled iteration without any concurrency limiting mechanism (semaphore, worker pool, or rate limiter). Under load or attack, this creates millions of goroutines that exhaust memory and CPU scheduler resources.

### Bad Code (Anti-pattern)
```go
package main

import (
    "fmt"
    "net/http"
)

func handleRequest(w http.ResponseWriter, r *http.Request) {
    items := r.URL.Query()["item"]
    // Spawns unbounded goroutines based on user input length
    for _, item := range items {
        go func(i string) {
            result := processItem(i) // Long-running operation
            fmt.Println(result)
        }(item)
    }
    w.WriteHeader(http.StatusAccepted)
}

func startWorkers(count int) {
    // User-controlled count with no upper bound
    for i := 0; i < count; i++ {
        go worker(i)
    }
}
```

### Good Code (Fix)
```go
package main

import (
    "context"
    "fmt"
    "net/http"
    "golang.org/x/sync/semaphore"
)

var sem = semaphore.NewWeighted(100) // Max 100 concurrent goroutines

func handleRequest(w http.ResponseWriter, r *http.Request) {
    items := r.URL.Query()["item"]
    const maxItems = 1000
    if len(items) > maxItems {
        http.Error(w, "Too many items", http.StatusBadRequest)
        return
    }

    for _, item := range items {
        if err := sem.Acquire(r.Context(), 1); err != nil {
            http.Error(w, "Server busy", http.StatusServiceUnavailable)
            return
        }
        go func(i string) {
            defer sem.Release(1)
            result := processItem(i)
            fmt.Println(result)
        }(item)
    }
    w.WriteHeader(http.StatusAccepted)
}

func startWorkers(count int) {
    const maxWorkers = 100
    if count > maxWorkers {
        count = maxWorkers
    }
    for i := 0; i < count; i++ {
        go worker(i)
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `go_statement`, `for_statement`, `func_literal`, `call_expression`, `identifier`
- **Detection approach**: Find `go_statement` nodes inside `for_statement` or `range_clause` loops where the loop bound is derived from user input (function parameters, HTTP request fields). Check for the absence of a semaphore `Acquire()` call or worker pool pattern in the enclosing scope. Also detect `go_statement` where the goroutine count is controlled by a variable with no upper-bound check.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (go_statement
      (call_expression
        function: (func_literal) @goroutine))))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_goroutine_creation`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Reading Entire Request Body Without io.LimitReader

### Description
Using `io.ReadAll()`, `ioutil.ReadAll()`, or `body.Read()` on `http.Request.Body` without wrapping it in `io.LimitReader` or `http.MaxBytesReader` first. Since HTTP request bodies have no inherent size limit (unless the server enforces one), an attacker can send an arbitrarily large payload that is read entirely into memory.

### Bad Code (Anti-pattern)
```go
package main

import (
    "encoding/json"
    "io"
    "net/http"
)

func handleUpload(w http.ResponseWriter, r *http.Request) {
    // Reads entire body with no size limit
    body, err := io.ReadAll(r.Body)
    if err != nil {
        http.Error(w, "read error", http.StatusInternalServerError)
        return
    }
    processData(body)
    w.WriteHeader(http.StatusOK)
}

func handleJSON(w http.ResponseWriter, r *http.Request) {
    // json.NewDecoder without body size limit
    var payload map[string]interface{}
    err := json.NewDecoder(r.Body).Decode(&payload)
    if err != nil {
        http.Error(w, "bad json", http.StatusBadRequest)
        return
    }
    w.WriteHeader(http.StatusOK)
}
```

### Good Code (Fix)
```go
package main

import (
    "encoding/json"
    "io"
    "net/http"
)

const maxBodySize = 10 * 1024 * 1024 // 10 MB

func handleUpload(w http.ResponseWriter, r *http.Request) {
    // Limit request body to maxBodySize
    r.Body = http.MaxBytesReader(w, r.Body, maxBodySize)
    body, err := io.ReadAll(r.Body)
    if err != nil {
        http.Error(w, "payload too large", http.StatusRequestEntityTooLarge)
        return
    }
    processData(body)
    w.WriteHeader(http.StatusOK)
}

func handleJSON(w http.ResponseWriter, r *http.Request) {
    // Limit body before decoding
    r.Body = http.MaxBytesReader(w, r.Body, maxBodySize)
    var payload map[string]interface{}
    decoder := json.NewDecoder(r.Body)
    decoder.DisallowUnknownFields()
    if err := decoder.Decode(&payload); err != nil {
        http.Error(w, "bad json", http.StatusBadRequest)
        return
    }
    w.WriteHeader(http.StatusOK)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `identifier`, `qualified_type`
- **Detection approach**: Find `call_expression` nodes invoking `io.ReadAll()` or `ioutil.ReadAll()` where the argument is `r.Body` or a variable assigned from `http.Request.Body`. Check the enclosing function for a preceding call to `http.MaxBytesReader()` or `io.LimitReader()` wrapping the body. Flag when no such limiter is found. Also detect `json.NewDecoder(r.Body)` without a prior `MaxBytesReader` wrapper.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @func)
  arguments: (argument_list
    (selector_expression
      operand: (identifier) @req
      field: (field_identifier) @field)))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_body_read`
- **Severity**: warning
- **Confidence**: high
