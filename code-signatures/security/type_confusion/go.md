# Type Confusion -- Go

## Overview
Go is a statically typed language, but its interface system introduces runtime type assertions that can fail if not handled correctly. The expression `x.(ConcreteType)` performs an unchecked type assertion that panics at runtime if `x` does not hold a value of `ConcreteType`. When this occurs in a server handling concurrent requests, a single malformed input can crash the entire process via an unrecovered panic.

## Why It's a Security Concern
In Go, an unrecovered panic in a goroutine terminates the entire program. If a type assertion panic occurs in an HTTP handler, gRPC service method, or message processor, an attacker can trigger a denial of service by sending input that causes an interface value to hold an unexpected concrete type. The "comma-ok" pattern (`val, ok := x.(ConcreteType)`) provides a safe alternative that returns a zero value and `false` instead of panicking, but developers frequently omit it for brevity.

## Applicability
- **Relevance**: medium
- **Languages covered**: .go
- **Frameworks/libraries**: net/http, gRPC, encoding/json, any code using interface{}/any parameters

---

## Pattern 1: Unchecked Type Assertion Panic

### Description
Performing a type assertion `x.(ConcreteType)` without using the comma-ok form `val, ok := x.(ConcreteType)`. If the interface value `x` does not contain a `ConcreteType`, the assertion panics. This is particularly dangerous when `x` originates from user input, decoded JSON, deserialized data, or any external source where the concrete type is not guaranteed.

### Bad Code (Anti-pattern)
```go
func handleRequest(w http.ResponseWriter, r *http.Request) {
    var payload interface{}
    json.NewDecoder(r.Body).Decode(&payload)

    // Panics if payload is not map[string]interface{} (e.g., JSON array or scalar)
    data := payload.(map[string]interface{})

    // Panics if "count" is not float64 (e.g., null, string, or missing key)
    count := data["count"].(float64)

    // Panics if "name" is not a string
    name := data["name"].(string)

    processOrder(name, int(count))
}

func getConfig(store map[string]interface{}, key string) string {
    // Panics if value is nil or wrong type
    return store[key].(string)
}
```

### Good Code (Fix)
```go
func handleRequest(w http.ResponseWriter, r *http.Request) {
    var payload interface{}
    if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
        http.Error(w, "invalid JSON", http.StatusBadRequest)
        return
    }

    data, ok := payload.(map[string]interface{})
    if !ok {
        http.Error(w, "expected JSON object", http.StatusBadRequest)
        return
    }

    countVal, ok := data["count"].(float64)
    if !ok {
        http.Error(w, "count must be a number", http.StatusBadRequest)
        return
    }

    name, ok := data["name"].(string)
    if !ok {
        http.Error(w, "name must be a string", http.StatusBadRequest)
        return
    }

    processOrder(name, int(countVal))
}

func getConfig(store map[string]interface{}, key string) (string, error) {
    val, ok := store[key].(string)
    if !ok {
        return "", fmt.Errorf("key %q is not a string", key)
    }
    return val, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_assertion_expression`, `short_var_declaration`, `assignment_statement`, `expression_statement`
- **Detection approach**: Find `type_assertion_expression` nodes (the `x.(Type)` syntax) that are NOT the right-hand side of a two-variable `short_var_declaration` or `assignment_statement` (the comma-ok pattern produces two left-hand values). If the type assertion appears in a single-value context -- as an `expression_statement`, a single-variable assignment, a function argument, or a return value -- it is an unchecked assertion that will panic on type mismatch.
- **S-expression query sketch**:
```scheme
(type_assertion_expression
  operand: (_) @value
  type: (_) @asserted_type)
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `unchecked_type_assertion`
- **Severity**: warning
- **Confidence**: high
