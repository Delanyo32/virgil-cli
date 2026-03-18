# Magic Values -- Go

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```go
func processRequest(data []byte) error {
    if len(data) > 1024 {
        return ErrPayloadTooLarge
    }
    for i := 0; i < 3; i++ {
        time.Sleep(86400 * time.Second)
    }
    if resp.StatusCode == 200 {
        cache.Set(key, data, 3600)
    } else if resp.StatusCode == 404 {
        return nil
    }
    return nil
}
```

### Good Code (Fix)
```go
const (
    MaxPayloadSize   = 1024
    MaxRetries       = 3
    SecondsPerDay    = 86400
    HTTPOk           = 200
    HTTPNotFound     = 404
    CacheTTLSeconds  = 3600
)

func processRequest(data []byte) error {
    if len(data) > MaxPayloadSize {
        return ErrPayloadTooLarge
    }
    for i := 0; i < MaxRetries; i++ {
        time.Sleep(SecondsPerDay * time.Second)
    }
    if resp.StatusCode == HTTPOk {
        cache.Set(key, data, CacheTTLSeconds)
    } else if resp.StatusCode == HTTPNotFound {
        return nil
    }
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `int_literal`, `float_literal` (excludes 0, 1, -1)
- **Detection approach**: Find `int_literal` and `float_literal` nodes in expressions. Exclude literals inside `const_declaration` or `const_spec` ancestor nodes. Also exclude `index_expression` index positions. Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
[(int_literal) @number (float_literal) @number]
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```go
func handleUser(user *User) {
    if user.Role == "admin" {
        grantAccess("dashboard")
    }
    switch user.Status {
    case "active":
        notify(user)
    case "pending":
        queue(user)
    }
    dbURL := config["database_url"]
}
```

### Good Code (Fix)
```go
const (
    RoleAdmin       = "admin"
    StatusActive    = "active"
    StatusPending   = "pending"
    ConfigDBURL     = "database_url"
)

func handleUser(user *User) {
    if user.Role == RoleAdmin {
        grantAccess("dashboard")
    }
    switch user.Status {
    case StatusActive:
        notify(user)
    case StatusPending:
        queue(user)
    }
    dbURL := config[ConfigDBURL]
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `interpreted_string_literal` or `raw_string_literal` in `binary_expression` (equality checks), `expression_case` (switch cases), or `index_expression` (map access)
- **Detection approach**: Find `interpreted_string_literal` and `raw_string_literal` nodes used in equality comparisons (`==`, `!=`), switch/case expressions, or as map keys in `index_expression`. Exclude logging strings, error messages, and format strings. Flag repeated identical strings across a function or file.
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: ["==" "!="]
  [left: (interpreted_string_literal) @string_lit
   right: (interpreted_string_literal) @string_lit])

(expression_case
  value: (expression_list
    (interpreted_string_literal) @string_lit))

(index_expression
  index: (interpreted_string_literal) @string_lit)
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
