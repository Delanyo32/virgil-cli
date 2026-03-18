# Type Safety Gaps -- Go

## Overview
Go's type system is statically typed but includes the empty interface (`interface{}` / `any`) and unchecked type assertions that bypass compile-time type safety. Using empty interfaces as parameter types loses all type information, and unchecked type assertions panic at runtime on type mismatches.

## Why It's a Tech Debt Concern
Accepting `interface{}` or `any` as a parameter type tells the compiler nothing about what the function expects, shifting type verification entirely to runtime and eliminating IDE autocompletion and refactoring support. Unchecked type assertions (`val := x.(Type)`) panic with a runtime error if `x` does not hold a value of `Type`, crashing the goroutine or entire program. Both patterns erode the benefits of Go's static type system and introduce failure modes that the compiler could otherwise prevent.

## Applicability
- **Relevance**: high (empty interfaces and type assertions are common in Go codebases)
- **Languages covered**: `.go`
- **Frameworks/libraries**: All Go codebases; common in middleware, event systems, and generic utility libraries (pre-generics code)

---

## Pattern 1: Empty Interface `interface{}` / `any` Used as Parameter Type

### Description
Functions that accept `interface{}` or `any` (its alias since Go 1.18) as parameter types instead of concrete types or constrained interfaces. This forces callers and implementations to rely on runtime type switches or assertions, negating compile-time type safety. With Go generics available since 1.18, most uses of empty interfaces for generic-like behavior can be replaced with type parameters.

### Bad Code (Anti-pattern)
```go
func Process(data interface{}) error {
    switch v := data.(type) {
    case string:
        return processString(v)
    case int:
        return processInt(v)
    case []byte:
        return processBytes(v)
    default:
        return fmt.Errorf("unsupported type: %T", data)
    }
}

func Store(key string, value any) {
    cache[key] = value
}

func Transform(input any, output any) error {
    bytes, err := json.Marshal(input)
    if err != nil {
        return err
    }
    return json.Unmarshal(bytes, output)
}
```

### Good Code (Fix)
```go
type Processable interface {
    string | int | []byte
}

func Process[T Processable](data T) error {
    return processTyped(data)
}

func Store[V any](key string, value V) {  // Generic with constraint if needed
    cache[key] = value
}

// Or use a concrete interface:
type Transformer interface {
    Marshal() ([]byte, error)
}

func Transform[T any](input T, output *T) error {
    bytes, err := json.Marshal(input)
    if err != nil {
        return err
    }
    return json.Unmarshal(bytes, output)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `parameter_declaration`, `interface_type`, `type_identifier`
- **Detection approach**: Find `parameter_declaration` nodes in `parameter_list` where the type is either an empty `interface_type` (with no methods -- `interface{}`) or the `type_identifier` `any`. Flag each occurrence. Exclude variadic parameters used for logging/formatting functions (e.g., `Printf`-style) via function name heuristics.
- **S-expression query sketch**:
```scheme
(parameter_declaration
  name: (identifier) @param_name
  type: (interface_type) @empty_iface)

(parameter_declaration
  name: (identifier) @param_name
  type: (type_identifier) @type_name
  (#eq? @type_name "any"))
```

### Pipeline Mapping
- **Pipeline name**: `naked_interface`
- **Pattern name**: `empty_interface_param`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Unchecked Type Assertion Without Comma-Ok

### Description
Using a type assertion `val := x.(Type)` without the comma-ok form `val, ok := x.(Type)`. The single-value form panics at runtime if `x` does not hold a value of `Type`, while the comma-ok form returns a zero value and `false`, allowing graceful handling.

### Bad Code (Anti-pattern)
```go
func handleMessage(msg interface{}) {
    payload := msg.(map[string]interface{})
    name := payload["name"].(string)
    age := payload["age"].(int)
    scores := payload["scores"].([]float64)
    process(name, age, scores)
}

func getConfig(settings map[string]interface{}, key string) string {
    return settings[key].(string)  // Panics if key missing or wrong type
}

func castAndUse(val interface{}) {
    conn := val.(*sql.DB)  // Panics if val is not *sql.DB
    conn.Ping()
}
```

### Good Code (Fix)
```go
func handleMessage(msg interface{}) error {
    payload, ok := msg.(map[string]interface{})
    if !ok {
        return fmt.Errorf("expected map, got %T", msg)
    }
    name, ok := payload["name"].(string)
    if !ok {
        return fmt.Errorf("expected string for name, got %T", payload["name"])
    }
    age, ok := payload["age"].(int)
    if !ok {
        return fmt.Errorf("expected int for age, got %T", payload["age"])
    }
    scores, ok := payload["scores"].([]float64)
    if !ok {
        return fmt.Errorf("expected []float64 for scores, got %T", payload["scores"])
    }
    process(name, age, scores)
    return nil
}

func getConfig(settings map[string]interface{}, key string) (string, error) {
    val, exists := settings[key]
    if !exists {
        return "", fmt.Errorf("key %q not found", key)
    }
    str, ok := val.(string)
    if !ok {
        return "", fmt.Errorf("key %q: expected string, got %T", key, val)
    }
    return str, nil
}

func castAndUse(val interface{}) error {
    conn, ok := val.(*sql.DB)
    if !ok {
        return fmt.Errorf("expected *sql.DB, got %T", val)
    }
    return conn.Ping()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_assertion_expression`, `short_var_declaration`, `assignment_statement`
- **Detection approach**: Find `type_assertion_expression` nodes (representing `x.(Type)`). Check the parent context: if the parent is a `short_var_declaration` or `assignment_statement` with only one left-hand-side identifier (not two), it is an unchecked assertion. If the parent has two left-hand identifiers (the comma-ok form), it is safe. Also flag bare `type_assertion_expression` used directly in function arguments or return statements.
- **S-expression query sketch**:
```scheme
(short_var_declaration
  left: (expression_list
    (identifier) @val)
  right: (expression_list
    (type_assertion_expression
      operand: (_) @source
      type: (_) @assert_type)))
```

### Pipeline Mapping
- **Pipeline name**: `naked_interface`
- **Pattern name**: `unchecked_type_assertion`
- **Severity**: warning
- **Confidence**: high
