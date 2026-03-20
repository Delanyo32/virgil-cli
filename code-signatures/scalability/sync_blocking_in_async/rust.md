# Sync Blocking in Async -- Rust

## Overview
Synchronous blocking in Rust async contexts occurs when standard library blocking APIs (`std::fs`, `std::thread::sleep`, `std::net`) are used inside `async fn` bodies, blocking the tokio/async-std runtime thread and preventing other tasks from being polled.

## Why It's a Scalability Concern
Tokio's default runtime uses a fixed-size thread pool (typically equal to CPU cores). A blocking call inside an async task holds one of these threads hostage — with 8 threads and 8 blocking calls, the entire runtime stalls. No other futures get polled, causing timeout cascades and apparent hangs.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: tokio, async-std, hyper, axum, actix-web, reqwest
- **Existing pipeline**: `async_blocking.rs` in `src/audit/pipelines/rust/` — extends with additional patterns

---

## Pattern 1: std::fs Operations in Async Fn

### Description
Using `std::fs::read()`, `std::fs::write()`, `std::fs::File::open()`, or other `std::fs` functions inside an `async fn`, which performs blocking I/O on the runtime thread.

### Bad Code (Anti-pattern)
```rust
async fn read_config() -> Result<Config> {
    let content = std::fs::read_to_string("/etc/app/config.toml")?;
    Ok(toml::from_str(&content)?)
}
```

### Good Code (Fix)
```rust
async fn read_config() -> Result<Config> {
    let content = tokio::fs::read_to_string("/etc/app/config.toml").await?;
    Ok(toml::from_str(&content)?)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item` (with `async` keyword), `call_expression`, `scoped_identifier`
- **Detection approach**: Find `call_expression` where the function is a `scoped_identifier` with path prefix `std::fs` or just `fs::` (when `use std::fs` is in scope), inside a function with `async` keyword.
- **S-expression query sketch**:
```scheme
(function_item
  "async"
  body: (block
    (let_declaration
      value: (call_expression
        function: (scoped_identifier) @func_path))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `std_fs_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: thread::sleep in Async Fn

### Description
Using `std::thread::sleep()` or `thread::sleep()` inside an `async fn`, which blocks the runtime thread for the entire duration instead of yielding with `tokio::time::sleep().await`.

### Bad Code (Anti-pattern)
```rust
async fn retry_with_backoff<F, T>(f: F) -> Result<T>
where
    F: Fn() -> Result<T>,
{
    for attempt in 0..3 {
        match f() {
            Ok(val) => return Ok(val),
            Err(_) => std::thread::sleep(Duration::from_secs(1 << attempt)),
        }
    }
    f()
}
```

### Good Code (Fix)
```rust
async fn retry_with_backoff<F, Fut, T>(f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    for attempt in 0..3 {
        match f().await {
            Ok(val) => return Ok(val),
            Err(_) => tokio::time::sleep(Duration::from_secs(1 << attempt)).await,
        }
    }
    f().await
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `call_expression`, `scoped_identifier`
- **Detection approach**: Find `call_expression` calling `std::thread::sleep` or `thread::sleep` inside an `async` function.
- **S-expression query sketch**:
```scheme
(function_item
  "async"
  body: (block
    (expression_statement
      (call_expression
        function: (scoped_identifier) @func_path))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `thread_sleep_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: .join() on Thread in Async Fn

### Description
Calling `.join()` on a `JoinHandle` inside an `async fn`, which blocks the runtime thread while waiting for the spawned thread to complete.

### Bad Code (Anti-pattern)
```rust
async fn process_data(data: Vec<u8>) -> Result<Vec<u8>> {
    let handle = std::thread::spawn(move || {
        heavy_computation(&data)
    });
    Ok(handle.join().unwrap())
}
```

### Good Code (Fix)
```rust
async fn process_data(data: Vec<u8>) -> Result<Vec<u8>> {
    let result = tokio::task::spawn_blocking(move || {
        heavy_computation(&data)
    }).await?;
    Ok(result)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `call_expression`, `field_expression`
- **Detection approach**: Find `call_expression` with `field_expression` where the field is `join` on a handle variable, inside an `async` function. The call has no `.await` suffix.
- **S-expression query sketch**:
```scheme
(function_item
  "async"
  body: (block
    (expression_statement
      (call_expression
        function: (field_expression
          field: (field_identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `thread_join_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: std::net::TcpStream in Async Fn

### Description
Using `std::net::TcpStream::connect()` or other `std::net` blocking I/O inside an `async fn` instead of `tokio::net::TcpStream`.

### Bad Code (Anti-pattern)
```rust
async fn check_health(addr: &str) -> Result<bool> {
    let stream = std::net::TcpStream::connect(addr)?;
    let mut buf = [0u8; 1024];
    stream.read(&mut buf)?;
    Ok(buf[0] == b'O' && buf[1] == b'K')
}
```

### Good Code (Fix)
```rust
async fn check_health(addr: &str) -> Result<bool> {
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let mut buf = [0u8; 1024];
    stream.read(&mut buf).await?;
    Ok(buf[0] == b'O' && buf[1] == b'K')
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `call_expression`, `scoped_identifier`
- **Detection approach**: Find `call_expression` calling `std::net::TcpStream::connect`, `std::net::TcpListener::bind`, `std::net::UdpSocket::bind` inside an `async` function.
- **S-expression query sketch**:
```scheme
(function_item
  "async"
  body: (block
    (let_declaration
      value: (call_expression
        function: (scoped_identifier) @func_path))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `std_net_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 5: Blocking stdin in Async Fn

### Description
Using `std::io::stdin().read_line()` or `BufRead::lines()` inside an `async fn`, which blocks the runtime thread waiting for input.

### Bad Code (Anti-pattern)
```rust
async fn interactive_prompt() -> Result<String> {
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
```

### Good Code (Fix)
```rust
async fn interactive_prompt() -> Result<String> {
    let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut input = String::new();
    reader.read_line(&mut input).await?;
    Ok(input.trim().to_string())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `call_expression`, `scoped_identifier`, `field_expression`
- **Detection approach**: Find `call_expression` calling `std::io::stdin` or method chain `.read_line()` on stdin result, inside an `async` function.
- **S-expression query sketch**:
```scheme
(function_item
  "async"
  body: (block
    (expression_statement
      (call_expression
        function: (field_expression
          value: (call_expression
            function: (scoped_identifier) @func_path)
          field: (field_identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_stdin_in_async`
- **Severity**: info
- **Confidence**: high
