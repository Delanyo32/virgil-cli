# Dead Code -- Go

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. Go's compiler enforces no unused variables or imports at compile time, so this analysis focuses on unused functions and unreachable code which the compiler does not catch.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Function/Method

### Description
A function or method defined but never called from anywhere in the codebase. Go's compiler catches unused variables and imports but does not flag unused functions.

### Bad Code (Anti-pattern)
```go
// formatBytes was used by the old logging system removed in v3
func formatBytes(n int64) string {
    units := []string{"B", "KB", "MB", "GB", "TB"}
    idx := 0
    size := float64(n)
    for size >= 1024 && idx < len(units)-1 {
        size /= 1024
        idx++
    }
    return fmt.Sprintf("%.1f %s", size, units[idx])
}

func FormatSize(n int64) string {
    return humanize.Bytes(uint64(n))
}
```

### Good Code (Fix)
```go
func FormatSize(n int64) string {
    return humanize.Bytes(uint64(n))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`
- **Detection approach**: Collect all function/method definitions and their names. Cross-reference with all `call_expression` and `selector_expression` nodes across the project. Functions with zero references are candidates. Exclude exported functions (uppercase first letter) in library packages, `init()` functions, `main()`, functions matching interface signatures, and functions registered as HTTP handlers or goroutines.
- **S-expression query sketch**:
  ```scheme
  (function_declaration name: (identifier) @fn_name)
  (method_declaration name: (field_identifier) @method_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Panic

### Description
Code statements that appear after an unconditional return, `log.Fatal()`, `os.Exit()`, or `panic()` — they can never execute.

### Bad Code (Anti-pattern)
```go
func loadConfig(path string) (*Config, error) {
    data, err := os.ReadFile(path)
    if err != nil {
        log.Fatalf("cannot read config: %v", err)
        return nil, err // unreachable — log.Fatalf calls os.Exit
    }

    var cfg Config
    if err := json.Unmarshal(data, &cfg); err != nil {
        panic(fmt.Sprintf("invalid config JSON: %v", err))
        return nil, err // unreachable — panic diverges
    }
    return &cfg, nil
}
```

### Good Code (Fix)
```go
func loadConfig(path string) (*Config, error) {
    data, err := os.ReadFile(path)
    if err != nil {
        log.Fatalf("cannot read config: %v", err)
    }

    var cfg Config
    if err := json.Unmarshal(data, &cfg); err != nil {
        panic(fmt.Sprintf("invalid config JSON: %v", err))
    }
    return &cfg, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `call_expression` (for `panic`, `log.Fatal`, `log.Fatalf`, `os.Exit`)
- **Detection approach**: For each diverging statement, check if there are sibling statements after it in the same block. In Go, `panic()`, `os.Exit()`, `log.Fatal()`, and `log.Fatalf()` are diverging. Also check for statements after unconditional `return`. Exclude deferred function calls which execute on different control flow.
- **S-expression query sketch**:
  ```scheme
  (block
    (return_statement) @exit
    .
    (_) @unreachable)
  (block
    (expression_statement
      (call_expression function: (identifier) @fn_name
        (#eq? @fn_name "panic"))) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```go
func handleRequest(w http.ResponseWriter, r *http.Request) {
    // func authenticate(r *http.Request) (*User, error) {
    //     token := r.Header.Get("Authorization")
    //     if token == "" {
    //         return nil, fmt.Errorf("missing auth header")
    //     }
    //     claims, err := jwt.Parse(token, keyFunc)
    //     if err != nil {
    //         return nil, fmt.Errorf("invalid token: %w", err)
    //     }
    //     return lookupUser(claims.Subject)
    // }

    serveContent(w, r)
}
```

### Good Code (Fix)
```go
func handleRequest(w http.ResponseWriter, r *http.Request) {
    serveContent(w, r)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment`
- **Detection approach**: Find comment nodes whose content matches Go code patterns (contains `func `, `if `, `for `, `return `, `:=`, `err != nil`, `fmt.`, `package `). Flag blocks of 5+ consecutive comment lines that look like code. Distinguish from build tags (`//go:build`), generate directives (`//go:generate`), and godoc comments.
- **S-expression query sketch**:
  ```scheme
  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
