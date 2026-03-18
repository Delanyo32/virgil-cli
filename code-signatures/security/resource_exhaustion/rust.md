# Resource Exhaustion -- Rust

## Overview
Resource exhaustion vulnerabilities in Rust occur when user-controlled input drives unbounded memory allocation or when panics in server request handlers cause denial of service. Despite Rust's memory safety guarantees, the language does not prevent logic errors that allocate arbitrary amounts of memory or crash threads via `panic!`, `unwrap()`, or array index out-of-bounds in hot paths.

## Why It's a Security Concern
Unbounded allocation from user input (e.g., `Vec::with_capacity(user_size)`) allows attackers to exhaust server memory with a single request containing a large numeric value. Panics in server handlers -- from `unwrap()` on `None`/`Err`, out-of-bounds indexing, or explicit `panic!` -- abort the handling thread or task. In frameworks without catch_unwind protection, repeated panics can crash the entire server process or degrade throughput.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: std (Vec, HashMap), actix-web, axum, warp, tokio, hyper

---

## Pattern 1: Unbounded Allocation from User Input -- Vec::with_capacity(user_size)

### Description
Using a user-controlled value as the capacity or length argument to `Vec::with_capacity()`, `Vec::resize()`, `vec![0; user_size]`, `HashMap::with_capacity()`, or `String::with_capacity()`. An attacker can supply an extremely large value (e.g., `usize::MAX / 2`) causing the allocator to request gigabytes of memory, triggering an OOM abort.

### Bad Code (Anti-pattern)
```rust
use actix_web::{web, HttpResponse};
use serde::Deserialize;

#[derive(Deserialize)]
struct BatchRequest {
    count: usize,
    data: Vec<String>,
}

async fn process_batch(req: web::Json<BatchRequest>) -> HttpResponse {
    // User controls 'count' -- could be billions
    let mut results = Vec::with_capacity(req.count);
    for item in &req.data {
        results.push(process_item(item));
    }
    HttpResponse::Ok().json(results)
}

async fn create_buffer(size: usize) -> Vec<u8> {
    // Directly using user-supplied size for allocation
    vec![0u8; size]
}
```

### Good Code (Fix)
```rust
use actix_web::{web, HttpResponse};
use serde::Deserialize;

const MAX_BATCH_SIZE: usize = 10_000;
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024; // 10 MB

#[derive(Deserialize)]
struct BatchRequest {
    count: usize,
    data: Vec<String>,
}

async fn process_batch(req: web::Json<BatchRequest>) -> HttpResponse {
    if req.count > MAX_BATCH_SIZE {
        return HttpResponse::BadRequest().body("Batch size too large");
    }
    // Use actual data length, not user-supplied count
    let mut results = Vec::with_capacity(req.data.len().min(MAX_BATCH_SIZE));
    for item in &req.data {
        results.push(process_item(item));
    }
    HttpResponse::Ok().json(results)
}

async fn create_buffer(size: usize) -> Result<Vec<u8>, &'static str> {
    if size > MAX_BUFFER_SIZE {
        return Err("Requested buffer too large");
    }
    Ok(vec![0u8; size])
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `macro_invocation`, `field_expression`, `integer_literal`, `identifier`
- **Detection approach**: Find `call_expression` nodes invoking `Vec::with_capacity()`, `HashMap::with_capacity()`, `String::with_capacity()`, or `Vec::resize()` where the argument is an `identifier` (not a constant or literal), indicating a potentially user-controlled value. Also find `macro_invocation` of `vec!` where the length expression is a variable. Flag when no preceding bounds check (comparison with a constant) is found in the enclosing function.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @type_name
    name: (identifier) @method)
  arguments: (arguments
    (identifier) @size_arg))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_allocation_user_input`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Panic in Server Handler Causing DoS

### Description
Using `.unwrap()`, `.expect()`, array indexing (`arr[i]`), or explicit `panic!` in HTTP request handler functions. When these panic, the handler thread/task aborts. Frameworks like actix-web catch panics per-request, but others (hyper, custom servers) may not. Repeated panics degrade server capacity or crash the process entirely.

### Bad Code (Anti-pattern)
```rust
use actix_web::{web, HttpResponse};
use serde_json::Value;

async fn get_item(path: web::Path<String>) -> HttpResponse {
    let id = path.into_inner();
    // unwrap() panics if parsing fails -- attacker sends non-numeric ID
    let numeric_id: u64 = id.parse().unwrap();
    let item = fetch_from_db(numeric_id).await.unwrap();
    HttpResponse::Ok().json(item)
}

async fn process_json(body: web::Json<Value>) -> HttpResponse {
    // Direct index panics if key missing
    let name = body["user"]["name"].as_str().unwrap();
    // Array index panics if out of bounds
    let first_tag = body["tags"][0].as_str().unwrap();
    HttpResponse::Ok().body(format!("{}: {}", name, first_tag))
}
```

### Good Code (Fix)
```rust
use actix_web::{web, HttpResponse};
use serde::Deserialize;

#[derive(Deserialize)]
struct ItemPath {
    id: u64,
}

#[derive(Deserialize)]
struct UserPayload {
    user: UserInfo,
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    name: String,
}

async fn get_item(path: web::Path<ItemPath>) -> HttpResponse {
    // Type-safe extraction -- returns 400 on parse failure, no panic
    let item = match fetch_from_db(path.id).await {
        Ok(item) => item,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    HttpResponse::Ok().json(item)
}

async fn process_json(body: web::Json<UserPayload>) -> HttpResponse {
    let first_tag = body.tags.first().unwrap_or(&String::new());
    HttpResponse::Ok().body(format!("{}: {}", body.user.name, first_tag))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `identifier`, `index_expression`
- **Detection approach**: Find `call_expression` nodes invoking `.unwrap()` or `.expect()` inside `async fn` handler functions (identified by their signature matching web framework handler patterns -- taking `web::Path`, `web::Json`, `HttpRequest`, etc.). Also find `index_expression` nodes (direct array/map indexing) on request-derived values. Flag when these appear in handler functions without surrounding `match` or `if let` guards.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    field: (field_identifier) @method)
  arguments: (arguments))
```

### Pipeline Mapping
- **Pipeline name**: `panic_dos`
- **Pattern name**: `panic_in_handler`
- **Severity**: error
- **Confidence**: high
