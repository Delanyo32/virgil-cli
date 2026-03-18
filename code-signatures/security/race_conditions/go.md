# Race Conditions -- Go

## Overview
Go's concurrency model based on goroutines and channels makes concurrent programming accessible, but also makes data races easy to introduce. When multiple goroutines access shared variables without synchronization (mutexes, channels, or atomic operations), the result is a data race -- undefined behavior in Go. Additionally, TOCTOU vulnerabilities in file and resource checks follow the same pattern as in other languages, where a check and a subsequent operation are not performed atomically.

## Why It's a Security Concern
Data races in Go lead to corrupted state, incorrect authorization decisions, double-processing of transactions, and memory safety violations (Go's race detector documentation explicitly states that data races can cause arbitrary memory corruption). TOCTOU races in file operations enable symlink attacks and privilege escalation. Go services often handle high-concurrency workloads, amplifying the window for exploitation.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: sync, sync/atomic, os, net/http, goroutines

---

## Pattern 1: Data Race on Shared Variable

### Description
Accessing a shared variable from multiple goroutines without synchronization. Common patterns include incrementing a counter, appending to a slice, or reading/writing a map concurrently. Go maps are not safe for concurrent use -- concurrent read and write causes a runtime panic, while other types silently produce corrupted values.

### Bad Code (Anti-pattern)
```go
package main

import "sync"

var count int

func main() {
    var wg sync.WaitGroup
    for i := 0; i < 1000; i++ {
        wg.Add(1)
        go func() {
            defer wg.Done()
            // DATA RACE: multiple goroutines read-modify-write without sync
            count++
        }()
    }
    wg.Wait()
}
```

### Good Code (Fix)
```go
package main

import (
    "sync"
    "sync/atomic"
)

var count int64

func main() {
    var wg sync.WaitGroup
    for i := 0; i < 1000; i++ {
        wg.Add(1)
        go func() {
            defer wg.Done()
            atomic.AddInt64(&count, 1)
        }()
    }
    wg.Wait()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `go_statement`, `inc_statement`, `assignment_statement`, `identifier`, `short_var_declaration`
- **Detection approach**: Find `go_statement` nodes (goroutine launches) whose body contains an `inc_statement` (e.g., `count++`) or `assignment_statement` referencing a variable declared outside the goroutine's closure scope (a package-level or enclosing-function variable). The absence of `sync.Mutex` Lock/Unlock calls or `atomic.*` calls wrapping the access indicates a data race.
- **S-expression query sketch**:
```scheme
(go_statement
  (call_expression
    function: (func_literal
      body: (block
        (inc_statement
          (identifier) @shared_var)))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `goroutine_data_race`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: TOCTOU in File/Resource Checks

### Description
Using `os.Stat()` or `os.IsNotExist()` to check whether a file exists before creating, opening, or deleting it. Between the stat check and the file operation, the file system state can change -- another process or goroutine can create, delete, or replace the file with a symlink.

### Bad Code (Anti-pattern)
```go
package main

import "os"

func writeIfNotExists(path string, data []byte) error {
    _, err := os.Stat(path)
    if os.IsNotExist(err) {
        // RACE: file can be created or symlinked between Stat and WriteFile
        return os.WriteFile(path, data, 0644)
    }
    return nil
}
```

### Good Code (Fix)
```go
package main

import "os"

func writeIfNotExists(path string, data []byte) error {
    // O_CREATE | O_EXCL: atomic create-or-fail
    f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0644)
    if err != nil {
        if os.IsExist(err) {
            return nil  // file already exists -- safe to skip
        }
        return err
    }
    defer f.Close()
    _, err = f.Write(data)
    return err
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `if_statement`, `identifier`
- **Detection approach**: Find `call_expression` nodes invoking `os.Stat` or `os.Lstat` where the result is checked with `os.IsNotExist()` in an `if_statement` condition, and the body of that `if_statement` contains calls to `os.WriteFile`, `os.Create`, `os.Remove`, or `os.OpenFile` operating on the same path variable. The two-step check-then-act pattern indicates a TOCTOU race.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (call_expression
    function: (selector_expression
      operand: (identifier) @pkg
      field: (field_identifier) @method)
    (#eq? @pkg "os")
    (#eq? @method "IsNotExist"))
  consequence: (block
    (call_expression
      function: (selector_expression
        field: (field_identifier) @action_method))))
```

### Pipeline Mapping
- **Pipeline name**: `toctou`
- **Pattern name**: `stat_then_create`
- **Severity**: warning
- **Confidence**: high
