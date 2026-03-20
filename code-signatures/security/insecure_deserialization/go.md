# Insecure Deserialization -- Go

## Overview
Insecure deserialization in Go occurs when `encoding/json`, `encoding/gob`, `encoding/xml`, or similar packages unmarshal untrusted input without validating the resulting struct. While Go's static typing provides some protection, deserializing into `interface{}` or `map[string]interface{}` bypasses type safety.

## Why It's a Security Concern
Unmarshaling untrusted data without input validation can lead to unexpected field values, type confusion, oversized allocations (denial of service), or logic bypass. The `encoding/gob` package is particularly risky because it can decode into interface types and is designed for trusted inter-process communication, not adversarial input.

## Applicability
- **Relevance**: medium
- **Languages covered**: .go
- **Frameworks/libraries**: encoding/json, encoding/gob, encoding/xml, github.com/gin-gonic/gin, net/http

---

## Pattern 1: Unmarshal Without Input Validation

### Description
Calling `json.Unmarshal()`, `json.NewDecoder().Decode()`, `gob.NewDecoder().Decode()`, or `xml.Unmarshal()` on untrusted input and using the result directly without validating field values, sizes, or ranges. Especially dangerous when the target type is `interface{}` or `map[string]interface{}`.

### Bad Code (Anti-pattern)
```go
func handleRequest(w http.ResponseWriter, r *http.Request) {
    var data map[string]interface{}
    body, _ := io.ReadAll(r.Body)
    json.Unmarshal(body, &data) // No size limit on body, no validation on data
    count := int(data["count"].(float64))
    items := make([]Item, count) // Attacker sends count: 999999999
    processItems(items)
}
```

### Good Code (Fix)
```go
type RequestData struct {
    Count int    `json:"count" validate:"required,min=1,max=1000"`
    Name  string `json:"name" validate:"required,max=256"`
}

func handleRequest(w http.ResponseWriter, r *http.Request) {
    r.Body = http.MaxBytesReader(w, r.Body, 1<<20) // Limit body size
    var data RequestData
    if err := json.NewDecoder(r.Body).Decode(&data); err != nil {
        http.Error(w, "invalid input", http.StatusBadRequest)
        return
    }
    if err := validator.New().Struct(data); err != nil {
        http.Error(w, "validation failed", http.StatusBadRequest)
        return
    }
    items := make([]Item, data.Count)
    processItems(items)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `identifier`, `qualified_type`
- **Detection approach**: Find `call_expression` nodes invoking `json.Unmarshal`, `xml.Unmarshal`, or method calls `.Decode()` on decoder objects from `json.NewDecoder`, `gob.NewDecoder`, `xml.NewDecoder`. Flag when the target variable is `interface{}` or `map[string]interface{}`, or when no validation call follows in the same function body.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @package
    field: (field_identifier) @method)
  arguments: (argument_list (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `unmarshal_no_validation`
- **Severity**: warning
- **Confidence**: medium
