# Memory Leak Indicators -- Go

## Overview
Memory leaks in Go occur when goroutines run indefinitely, maps grow without bounds, timers accumulate, or channels are created without being closed. Go's garbage collector cannot reclaim goroutines that are blocked on channel operations or objects referenced by leaked goroutines.

## Why It's a Scalability Concern
Go servers are designed for high concurrency via goroutines. A goroutine leak adds ~8KB minimum per goroutine, and leaked goroutines often hold references to request-scoped data, connections, or buffers. Under sustained traffic, goroutine leaks cause memory growth proportional to request count, eventually causing OOM kills.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: goroutines, channels, sync, time, net/http
- **Existing pipeline**: `goroutine_leak.rs` in `src/audit/pipelines/go/` — extends with additional memory patterns

---

## Pattern 1: Map Growth Without Delete

### Description
Inserting into a `map` inside a loop or repeatedly-called function without any `delete()` call or size check on the same map, causing unbounded memory growth.

### Bad Code (Anti-pattern)
```go
var cache = make(map[string][]byte)

func HandleRequest(w http.ResponseWriter, r *http.Request) {
    key := r.URL.Path + r.Header.Get("Authorization")
    if _, ok := cache[key]; !ok {
        cache[key] = computeResult(r)
    }
    w.Write(cache[key])
}
```

### Good Code (Fix)
```go
var cache = make(map[string]cacheEntry)
var mu sync.RWMutex

type cacheEntry struct {
    data      []byte
    timestamp time.Time
}

func HandleRequest(w http.ResponseWriter, r *http.Request) {
    key := r.URL.Path + r.Header.Get("Authorization")
    mu.RLock()
    entry, ok := cache[key]
    mu.RUnlock()
    if !ok || time.Since(entry.timestamp) > 5*time.Minute {
        data := computeResult(r)
        mu.Lock()
        cache[key] = cacheEntry{data: data, timestamp: time.Now()}
        if len(cache) > 10000 {
            evictOldest(cache)
        }
        mu.Unlock()
        w.Write(data)
        return
    }
    w.Write(entry.data)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `index_expression`, `assignment_statement`, `for_statement`, `call_expression`
- **Detection approach**: Find `assignment_statement` where the left side is an `index_expression` (map assignment `m[k] = v`). Check the module/function scope for `delete(m, ...)` calls on the same map variable. Flag if the map is package-level and has no corresponding delete.
- **S-expression query sketch**:
```scheme
(assignment_statement
  left: (expression_list
    (index_expression
      operand: (identifier) @map_name)))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `map_growth_no_delete`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Goroutine Without Completion

### Description
Launching a goroutine with `go func()` that has no exit path — it blocks on a channel, waits forever, or runs an infinite loop without a `context.Done()` check or shutdown signal.

### Bad Code (Anti-pattern)
```go
func startWorker(ch <-chan Job) {
    go func() {
        for {
            job := <-ch
            process(job)
        }
    }()
}
```

### Good Code (Fix)
```go
func startWorker(ctx context.Context, ch <-chan Job) {
    go func() {
        for {
            select {
            case job := <-ch:
                process(job)
            case <-ctx.Done():
                return
            }
        }
    }()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `go_statement`, `func_literal`, `for_statement`, `select_statement`
- **Detection approach**: Find `go_statement` containing a `func_literal` with a `for_statement` whose body does NOT contain a `select_statement` with a `context.Done()` or cancel channel check. An infinite `for` loop in a goroutine without context cancellation is a leak risk.
- **S-expression query sketch**:
```scheme
(go_statement
  (call_expression
    function: (func_literal
      body: (block
        (for_statement
          body: (block) @loop_body)))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `goroutine_no_exit`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: time.After() in Loop (Timer Leak)

### Description
Using `time.After()` inside a `for`/`select` loop creates a new timer on each iteration. Timers are not garbage collected until they fire, so in a tight loop, unreferenced timers accumulate.

### Bad Code (Anti-pattern)
```go
func processEvents(ch <-chan Event) {
    for {
        select {
        case event := <-ch:
            handle(event)
        case <-time.After(30 * time.Second):
            log.Println("timeout")
            return
        }
    }
}
```

### Good Code (Fix)
```go
func processEvents(ch <-chan Event) {
    timer := time.NewTimer(30 * time.Second)
    defer timer.Stop()
    for {
        timer.Reset(30 * time.Second)
        select {
        case event := <-ch:
            handle(event)
        case <-timer.C:
            log.Println("timeout")
            return
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `for_statement`, `communication_case`
- **Detection approach**: Find `call_expression` calling `time.After` inside a `communication_case` within a `select_statement` that is inside a `for_statement`. Each loop iteration creates a new timer.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (expression_switch_statement
      (communication_case
        (receive_statement
          right: (call_expression
            function: (selector_expression
              operand: (identifier) @pkg
              field: (field_identifier) @method)))))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `time_after_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: Channel Created in Loop Without Close

### Description
Creating channels with `make(chan ...)` inside a loop without ever closing them. Buffered channels hold memory for their buffer, and goroutines blocked on unclosed channels are never released.

### Bad Code (Anti-pattern)
```go
func fanOut(jobs []Job) []Result {
    var results []Result
    for _, job := range jobs {
        ch := make(chan Result, 1)
        go func(j Job) {
            ch <- compute(j)
        }(job)
        results = append(results, <-ch)
    }
    return results
}
```

### Good Code (Fix)
```go
func fanOut(jobs []Job) []Result {
    results := make([]Result, len(jobs))
    var wg sync.WaitGroup
    for i, job := range jobs {
        wg.Add(1)
        go func(idx int, j Job) {
            defer wg.Done()
            results[idx] = compute(j)
        }(i, job)
    }
    wg.Wait()
    return results
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `for_statement`, `short_var_declaration`
- **Detection approach**: Find `call_expression` calling `make` with a `chan` type argument inside a `for_statement`. Check if `close()` is called on the channel within the loop body or deferred.
- **S-expression query sketch**:
```scheme
(for_statement
  body: (block
    (short_var_declaration
      right: (expression_list
        (call_expression
          function: (identifier) @func
          (#eq? @func "make"))))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `channel_in_loop_no_close`
- **Severity**: info
- **Confidence**: low

---

## Pattern 5: sync.Pool Pointer Escape

### Description
Getting an object from `sync.Pool` and storing a reference to it beyond the current scope (e.g., in a struct field or global), preventing the pool from reclaiming it.

### Bad Code (Anti-pattern)
```go
var bufPool = sync.Pool{
    New: func() interface{} { return new(bytes.Buffer) },
}

type Connection struct {
    buf *bytes.Buffer
}

func NewConnection() *Connection {
    buf := bufPool.Get().(*bytes.Buffer)
    return &Connection{buf: buf} // pool object escapes
}
```

### Good Code (Fix)
```go
var bufPool = sync.Pool{
    New: func() interface{} { return new(bytes.Buffer) },
}

func ProcessConnection(conn *Connection) {
    buf := bufPool.Get().(*bytes.Buffer)
    defer func() {
        buf.Reset()
        bufPool.Put(buf)
    }()
    // use buf locally only
    buf.WriteString(conn.Data)
    sendData(buf.Bytes())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `composite_literal`, `return_statement`
- **Detection approach**: Find `call_expression` calling `.Get()` on a `sync.Pool` variable. Track the variable it's assigned to. Flag if that variable is used in a `composite_literal` (struct initialization), assigned to a struct field, or appears in a `return_statement` without a corresponding `.Put()` call.
- **S-expression query sketch**:
```scheme
(short_var_declaration
  left: (expression_list (identifier) @pool_obj)
  right: (expression_list
    (type_assertion_expression
      (call_expression
        function: (selector_expression
          field: (field_identifier) @method)
        (#eq? @method "Get")))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `sync_pool_escape`
- **Severity**: info
- **Confidence**: low
