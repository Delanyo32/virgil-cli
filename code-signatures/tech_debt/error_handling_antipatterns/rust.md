# Error Handling Anti-patterns -- Rust

## Overview
Errors that are silently swallowed or handled by panicking instead of propagation make debugging impossible and hide real failures. In Rust, calling `.unwrap()` on `Result`/`Option` types and discarding `Result` values with `let _ =` are the most common anti-patterns.

## Why It's a Tech Debt Concern
`.unwrap()` calls convert recoverable errors into panics, crashing the entire thread or program at runtime in scenarios that should be handled gracefully. Discarding `Result` with `let _ =` silently ignores errors, allowing failed operations (file writes, network calls, lock acquisitions) to go unnoticed. Both patterns are easy to introduce during prototyping but become time bombs in production, especially in long-running services and concurrent code.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`

---

## Pattern 1: `.unwrap()` on Result/Option

### Description
Calling `.unwrap()` on a `Result<T, E>` or `Option<T>` causes a panic if the value is `Err` or `None`. While convenient during prototyping, it provides no recovery path and produces poor error messages. `.expect()` is marginally better for context but still panics.

### Bad Code (Anti-pattern)
```rust
fn load_config() -> Config {
    let contents = fs::read_to_string("config.toml").unwrap();
    let config: Config = toml::from_str(&contents).unwrap();
    config
}

fn get_user(db: &Database, id: i64) -> User {
    let user = db.find_user(id).unwrap();
    let profile = user.profile.unwrap();
    User { profile, ..user }
}

fn process_request(req: &Request) -> Response {
    let body: RequestBody = serde_json::from_slice(req.body()).unwrap();
    let result = handle(body).unwrap();
    Response::ok(result)
}
```

### Good Code (Fix)
```rust
fn load_config() -> Result<Config, ConfigError> {
    let contents = fs::read_to_string("config.toml")
        .map_err(|e| ConfigError::ReadFailed(e))?;
    let config: Config = toml::from_str(&contents)
        .map_err(|e| ConfigError::ParseFailed(e))?;
    Ok(config)
}

fn get_user(db: &Database, id: i64) -> Result<User, AppError> {
    let user = db.find_user(id)
        .ok_or_else(|| AppError::NotFound(format!("User {id} not found")))?;
    let profile = user.profile
        .ok_or_else(|| AppError::NotFound("User has no profile".into()))?;
    Ok(User { profile, ..user })
}

fn process_request(req: &Request) -> Result<Response, AppError> {
    let body: RequestBody = serde_json::from_slice(req.body())
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = handle(body)?;
    Ok(Response::ok(result))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `field_identifier`
- **Detection approach**: Find `call_expression` nodes where the function is a `field_expression` with `field_identifier` equal to `unwrap` or `expect`. These are method calls on any type, but in Rust they almost always indicate `Result::unwrap()` or `Option::unwrap()`. Optionally exclude test modules by checking if the enclosing `mod` or function has a `#[test]` or `#[cfg(test)]` attribute.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    field: (field_identifier) @method_name))
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `unwrap_call`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Discarding Result with `let _ =`

### Description
Using `let _ = expr` where `expr` returns a `Result` silently discards the error. The compiler will not warn about unused `Result` values when they are explicitly assigned to `_`. This is commonly seen with I/O operations, lock acquisitions, and channel sends.

### Bad Code (Anti-pattern)
```rust
fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

fn broadcast(tx: &Sender<Message>, msg: Message) {
    let _ = tx.send(msg);
    // If the receiver is dropped, we silently lose the message
}

fn update_cache(cache: &Mutex<HashMap<String, Value>>, key: String, val: Value) {
    let _ = cache.lock().map(|mut c| c.insert(key, val));
    // If the mutex is poisoned, we silently skip the update
}
```

### Good Code (Fix)
```rust
fn cleanup(path: &Path) -> io::Result<()> {
    fs::remove_file(path)?;
    if let Some(parent) = path.parent() {
        fs::remove_dir_all(parent)?;
    }
    Ok(())
}

fn broadcast(tx: &Sender<Message>, msg: Message) -> Result<(), SendError<Message>> {
    tx.send(msg).map_err(|e| {
        tracing::error!("Failed to broadcast message: receiver dropped");
        e
    })
}

fn update_cache(cache: &Mutex<HashMap<String, Value>>, key: String, val: Value) {
    match cache.lock() {
        Ok(mut c) => { c.insert(key, val); }
        Err(e) => {
            tracing::error!("Cache mutex poisoned: {e}");
            // Decide: clear poison, propagate, or skip
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `let_declaration`, `identifier`, `call_expression`
- **Detection approach**: Find `let_declaration` nodes where the pattern is `_` (a single wildcard) and the value expression is a `call_expression` or method chain that could return a `Result`. Since tree-sitter does not provide type information, flag all `let _ = <call_expression>` patterns and rely on post-processing or allow-listing to reduce false positives.
- **S-expression query sketch**:
```scheme
(let_declaration
  pattern: (_) @binding
  value: (call_expression) @discarded_call)
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `discarded_result`
- **Severity**: info
- **Confidence**: medium
