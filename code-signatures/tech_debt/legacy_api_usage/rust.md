# Legacy API Usage -- Rust

## Overview
Legacy API usage in Rust refers to patterns that work but are suboptimal, wasteful, or unsafe for the context in which they appear. Common examples include unnecessary `.clone()` calls that bypass the borrow checker instead of restructuring ownership, and using `panic!`/`unwrap()`/`expect()` in library code where errors should be propagated to the caller.

## Why It's a Tech Debt Concern
Unnecessary cloning masks ownership design problems and adds heap allocations that degrade performance -- especially in hot paths or when cloning large data structures like `Vec`, `String`, or `HashMap`. Using `panic!` or `unwrap()` in library code forces the entire program to abort on errors that the caller could handle gracefully, making the library unusable in contexts where resilience is required (servers, embedded systems). Both patterns indicate that the developer worked around Rust's type system rather than working with it.

## Applicability
- **Relevance**: high (both patterns are extremely common in Rust codebases written by developers still learning ownership and error handling)
- **Languages covered**: `.rs`
- **Frameworks/libraries**: N/A (language-level patterns)

---

## Pattern 1: Unnecessary clone()

### Description
Calling `.clone()` on values to satisfy the borrow checker when restructuring the code to use references, lifetimes, or `Cow<T>` would avoid the allocation entirely. This is especially costly when cloning `String`, `Vec<T>`, `HashMap<K, V>`, or other heap-allocated types inside loops or hot paths.

### Bad Code (Anti-pattern)
```rust
fn process_users(users: &[User]) -> Vec<String> {
    let mut results = Vec::new();
    for user in users {
        // Cloning the entire String just to pass it to a function
        let name = user.name.clone();
        let email = user.email.clone();
        let result = format_user_summary(name, email);
        results.push(result);
    }
    results
}

fn format_user_summary(name: String, email: String) -> String {
    format!("{} <{}>", name, email)
}

fn find_matching_orders(orders: &[Order], customer_id: &str) -> Vec<Order> {
    let mut matching = Vec::new();
    for order in orders {
        if order.customer_id == customer_id {
            // Cloning entire Order struct with all its fields
            matching.push(order.clone());
        }
    }
    matching
}

fn build_index(items: &[Item]) -> HashMap<String, Item> {
    let mut index = HashMap::new();
    for item in items {
        // Double clone: key and value
        index.insert(item.id.clone(), item.clone());
    }
    index
}
```

### Good Code (Fix)
```rust
fn process_users(users: &[User]) -> Vec<String> {
    let mut results = Vec::new();
    for user in users {
        // Pass references instead of cloning
        let result = format_user_summary(&user.name, &user.email);
        results.push(result);
    }
    results
}

fn format_user_summary(name: &str, email: &str) -> String {
    format!("{} <{}>", name, email)
}

fn find_matching_orders<'a>(orders: &'a [Order], customer_id: &str) -> Vec<&'a Order> {
    // Return references instead of cloning
    orders.iter()
        .filter(|order| order.customer_id == customer_id)
        .collect()
}

fn build_index(items: &[Item]) -> HashMap<&str, &Item> {
    // Use references for both key and value
    let mut index = HashMap::new();
    for item in items {
        index.insert(item.id.as_str(), item);
    }
    index
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression` with `field_expression` (`.clone()`)
- **Detection approach**: Find `call_expression` nodes where the function is a `field_expression` with field name `clone` and no arguments. Flag when the cloned value is immediately passed to a function that could accept a reference, or when `.clone()` appears inside a `for_expression` body (loop hot path). Higher confidence when the cloned type is `String`, `Vec`, or `HashMap`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    value: (_) @cloned_value
    field: (field_identifier) @method)
  arguments: (arguments)
  (#eq? @method "clone"))
```

### Pipeline Mapping
- **Pipeline name**: `clone_detection`
- **Pattern name**: `unnecessary_clone`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: panic!/unwrap()/expect() in Library Code

### Description
Using `panic!()`, `.unwrap()`, or `.expect()` in library code (non-`main`, non-test modules) instead of propagating errors with `Result<T, E>` and the `?` operator. These calls terminate the entire program on failure, making the library hostile to callers who need graceful error handling.

### Bad Code (Anti-pattern)
```rust
pub fn parse_config(path: &str) -> Config {
    let content = std::fs::read_to_string(path).unwrap();
    let config: Config = serde_json::from_str(&content).unwrap();

    if config.port == 0 {
        panic!("Port must be non-zero");
    }

    let db_url = config.database_url.as_ref()
        .expect("database_url is required");

    let timeout = config.timeout_ms.unwrap();

    config
}

pub fn connect_database(url: &str) -> Connection {
    let conn = Database::connect(url).unwrap();
    conn.execute("SELECT 1").expect("Database health check failed");
    conn
}

pub fn process_record(data: &[u8]) -> Record {
    let parsed = Record::from_bytes(data)
        .expect("Failed to parse record");
    let validated = parsed.validate()
        .unwrap();
    validated
}
```

### Good Code (Fix)
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    IoError(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("invalid config: {0}")]
    ValidationError(String),
}

pub fn parse_config(path: &str) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&content)?;

    if config.port == 0 {
        return Err(ConfigError::ValidationError("port must be non-zero".into()));
    }

    let _db_url = config.database_url.as_ref()
        .ok_or_else(|| ConfigError::ValidationError("database_url is required".into()))?;

    config.timeout_ms
        .ok_or_else(|| ConfigError::ValidationError("timeout_ms is required".into()))?;

    Ok(config)
}

pub fn connect_database(url: &str) -> Result<Connection, DatabaseError> {
    let conn = Database::connect(url)?;
    conn.execute("SELECT 1")?;
    Ok(conn)
}

pub fn process_record(data: &[u8]) -> Result<Record, RecordError> {
    let parsed = Record::from_bytes(data)?;
    let validated = parsed.validate()?;
    Ok(validated)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression` (for `panic!`), `call_expression` with `field_expression` (for `.unwrap()` and `.expect()`)
- **Detection approach**: Find `macro_invocation` nodes with macro name `panic` and `call_expression` nodes where the field is `unwrap` or `expect`. Exclude occurrences inside `#[test]` or `#[cfg(test)]` modules and inside `fn main()`. Flag all others as library code that should use `Result` propagation. Higher confidence when the function signature returns a concrete type (not `Result`).
- **S-expression query sketch**:
```scheme
(macro_invocation
  macro: (identifier) @macro_name
  (#eq? @macro_name "panic"))

(call_expression
  function: (field_expression
    field: (field_identifier) @method)
  (#match? @method "^(unwrap|expect)$"))
```

### Pipeline Mapping
- **Pipeline name**: `panic_detection`
- **Pattern name**: `panic_unwrap_in_library`
- **Severity**: warning
- **Confidence**: high
