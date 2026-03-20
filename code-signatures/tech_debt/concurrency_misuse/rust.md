# Concurrency Misuse -- Rust

## Overview
Rust's ownership system prevents data races at compile time, but concurrency misuse still occurs through overuse of `Mutex` when lock-free alternatives exist, and through blocking calls inside async contexts that stall the runtime. These patterns degrade performance and scalability.

## Why It's a Tech Debt Concern
Wrapping everything in `Mutex<T>` when `AtomicU64`, `RwLock`, or channel-based designs would suffice introduces unnecessary contention — every access serializes through the lock, creating bottlenecks under concurrent load. Blocking in async contexts (covered in scalability/sync_blocking_in_async) ties up runtime threads, but the concurrency-specific concern is that it breaks the cooperative scheduling contract and can cascade into deadlock-like stalls.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`
- **Frameworks/libraries**: std::sync, tokio, async-std, crossbeam, parking_lot
- **Existing pipeline**: `mutex_overuse` in `src/audit/pipelines/rust/` — extends with detection patterns
- **Existing pipeline**: `async_blocking` in `src/audit/pipelines/rust/` — cross-ref `scalability/sync_blocking_in_async/rust.md`

---

## Pattern 1: Mutex Overuse

### Description
Using `Mutex<T>` for simple counters, flags, or read-heavy data where `AtomicU64`/`AtomicBool`, `RwLock`, or lock-free data structures would provide better performance. Mutex serializes all access (including reads), creating unnecessary contention in read-heavy workloads.

### Bad Code (Anti-pattern)
```rust
use std::sync::Mutex;

struct Metrics {
    request_count: Mutex<u64>,
    error_count: Mutex<u64>,
    is_healthy: Mutex<bool>,
    config: Mutex<Config>,  // Read 1000x per write
}

impl Metrics {
    fn increment_requests(&self) {
        let mut count = self.request_count.lock().unwrap();
        *count += 1;
    }

    fn is_healthy(&self) -> bool {
        *self.is_healthy.lock().unwrap()
    }

    fn get_config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }
}
```

### Good Code (Fix)
```rust
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::RwLock;

struct Metrics {
    request_count: AtomicU64,
    error_count: AtomicU64,
    is_healthy: AtomicBool,
    config: RwLock<Config>,  // Many readers, rare writers
}

impl Metrics {
    fn increment_requests(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
    }

    fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::Relaxed)
    }

    fn get_config(&self) -> Config {
        self.config.read().unwrap().clone()
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_identifier`, `generic_type`, `field_declaration`, `struct_item`
- **Detection approach**: Find `field_declaration` nodes inside `struct_item` where the type is `Mutex<T>` and `T` is a primitive type (`u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`, `usize`, `isize`, `bool`, `f32`, `f64`). These are candidates for atomic types. Also count `Mutex` fields in a struct — more than 4 suggests architectural review.
- **S-expression query sketch**:
```scheme
(struct_item
  body: (field_declaration_list
    (field_declaration
      type: (generic_type
        type: (type_identifier) @wrapper_type
        type_arguments: (type_arguments
          (type_identifier) @inner_type)))))
```

### Pipeline Mapping
- **Pipeline name**: `mutex_overuse`
- **Pattern name**: `mutex_on_primitive`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Blocking in Async Context

### Description
Calling synchronous blocking operations (`std::thread::sleep`, `std::fs::read`, `Mutex::lock`) inside an `async fn` or async block. This blocks the runtime thread, preventing other tasks from being polled, and can stall the entire runtime when all threads are blocked. This pattern is the concurrency-specific facet of the broader sync-blocking-in-async scalability concern.

### Bad Code (Anti-pattern)
```rust
async fn handle_request(db: Arc<Mutex<Database>>) -> Response {
    // Blocks the async runtime thread while waiting for the lock
    let db = db.lock().unwrap();
    let result = db.query("SELECT * FROM users")?;

    // Blocks the runtime thread for 1 second
    std::thread::sleep(Duration::from_secs(1));

    Response::ok(result)
}
```

### Good Code (Fix)
```rust
async fn handle_request(db: Arc<tokio::sync::Mutex<Database>>) -> Response {
    // Async-aware mutex that yields while waiting
    let db = db.lock().await;
    let result = db.query("SELECT * FROM users")?;

    // Yields to the runtime, allowing other tasks to progress
    tokio::time::sleep(Duration::from_secs(1)).await;

    Response::ok(result)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item` (with `async` keyword), `call_expression`, `scoped_identifier`, `field_expression`
- **Detection approach**: Find `call_expression` nodes inside `async` functions where the callee resolves to `std::thread::sleep`, `std::fs::*`, or `.lock().unwrap()` on a `std::sync::Mutex`. The presence of `async` on the enclosing function is the key discriminator. Cross-reference with `scalability/sync_blocking_in_async/rust.md` for the full set of blocking APIs.
- **S-expression query sketch**:
```scheme
(function_item
  "async"
  body: (block
    (let_declaration
      value: (call_expression
        function: (field_expression
          field: (field_identifier) @method_name)))))
```

### Pipeline Mapping
- **Pipeline name**: `async_blocking`
- **Pattern name**: `sync_lock_in_async`
- **Severity**: warning
- **Confidence**: high
