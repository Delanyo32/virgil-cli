# Insecure Deserialization -- Rust

## Overview
Insecure deserialization in Rust is less severe than in most languages due to the type system enforcing structure at compile time. However, deserializing untrusted input with serde into permissive types (e.g., `serde_json::Value`) or skipping post-deserialization validation can still lead to logic bugs, denial of service, or data corruption.

## Why It's a Security Concern
While Rust's type system prevents arbitrary code execution during deserialization (unlike Python's pickle or Java's ObjectInputStream), deserializing into unvalidated types can allow oversized allocations (DoS via deeply nested or extremely large payloads), unexpected field values that bypass business logic, or type confusion when using `#[serde(untagged)]` enums.

## Applicability
- **Relevance**: low
- **Languages covered**: .rs
- **Frameworks/libraries**: serde, serde_json, serde_yaml, bincode, ciborium, rmp-serde, actix-web, axum

---

## Pattern 1: Deserializing Untrusted Input With serde Without Validation

### Description
Calling `serde_json::from_str()`, `serde_json::from_slice()`, `serde_json::from_reader()`, or equivalent serde deserialize functions on untrusted input and using the result directly without validating field ranges, string lengths, or collection sizes. Particularly risky when deserializing into `serde_json::Value` (accepts any shape) or structs without `#[serde(deny_unknown_fields)]`.

### Bad Code (Anti-pattern)
```rust
use serde_json::Value;

async fn handle_request(body: String) -> Result<(), Error> {
    let data: Value = serde_json::from_str(&body)?; // Accepts any shape
    let count = data["count"].as_u64().unwrap_or(0);
    allocate_items(count as usize); // Attacker sends count: 999999999 → OOM
    Ok(())
}
```

### Good Code (Fix)
```rust
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
struct RequestData {
    #[validate(range(min = 1, max = 1000))]
    count: u32,
    #[validate(length(max = 256))]
    name: String,
}

async fn handle_request(body: String) -> Result<(), Error> {
    let data: RequestData = serde_json::from_str(&body)?;
    data.validate()?; // Enforces business constraints
    allocate_items(data.count as usize);
    Ok(())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`, `type_identifier`, `generic_type`
- **Detection approach**: Find `call_expression` nodes invoking `serde_json::from_str`, `serde_json::from_slice`, `serde_json::from_reader`, or `serde_json::from_value` where the type parameter is `Value` (generic or turbofish). Also flag when the deserialized struct does not have a subsequent `.validate()` call in the same function body.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @module
    name: (identifier) @func)
  arguments: (arguments (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `serde_no_validation`
- **Severity**: warning
- **Confidence**: low
