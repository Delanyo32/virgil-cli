# Stringly Typed -- Rust

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs

---

## Pattern 1: String Comparisons for State/Status

### Description
Using string equality checks to determine state transitions, roles, or status instead of Rust's powerful `enum` types. This is especially wasteful in Rust, where enums with `match` provide exhaustive compile-time checking.

### Bad Code (Anti-pattern)
```rust
fn process_order(order: &Order) {
    if order.status == "active" {
        start_fulfillment(order);
    } else if order.status == "pending" {
        notify_customer(order);
    } else if order.status == "cancelled" {
        refund_payment(order);
    } else if order.status == "shipped" {
        track_delivery(order);
    } else if order.status == "delivered" {
        request_review(order);
    }
}

fn get_status_color(status: &str) -> &str {
    match status {
        "active" => "green",
        "pending" => "yellow",
        "cancelled" => "red",
        "shipped" => "blue",
        "delivered" => "gray",
        _ => "white",
    }
}
```

### Good Code (Fix)
```rust
enum Status {
    Active,
    Pending,
    Cancelled,
    Shipped,
    Delivered,
}

fn process_order(order: &Order) {
    match order.status {
        Status::Active => start_fulfillment(order),
        Status::Pending => notify_customer(order),
        Status::Cancelled => refund_payment(order),
        Status::Shipped => track_delivery(order),
        Status::Delivered => request_review(order),
    }
}

impl Status {
    fn color(&self) -> &str {
        match self {
            Status::Active => "green",
            Status::Pending => "yellow",
            Status::Cancelled => "red",
            Status::Shipped => "blue",
            Status::Delivered => "gray",
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `==`), `string_literal`, `if_expression`, `match_expression`, `match_arm`
- **Detection approach**: Find equality comparisons (`==`) where one operand is a string literal. Also detect `match` expressions where 3+ `match_arm` patterns are string literals (indicates an enum is appropriate). The presence of a wildcard `_` arm with string match arms is a strong signal.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (field_expression
    field: (field_identifier) @field)
  right: (string_literal) @string_val)

(match_arm
  pattern: (string_literal) @arm_string)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for configuration access via `HashMap<String, String>` or similar, instead of typed configuration structs. In Rust, this foregoes the compiler's ability to catch missing or mistyped fields.

### Bad Code (Anti-pattern)
```rust
use std::collections::HashMap;

fn setup_app(config: &HashMap<String, String>) {
    let db_host = config.get("database_host").unwrap();
    let db_port = config.get("database_port").unwrap();
    let db_name = config.get("database_name").unwrap();
    let redis_url = config.get("redis_url").unwrap();
    let api_key = config.get("api_key").unwrap();
    let log_level = config.get("log_level").unwrap();

    connect_db(db_host, db_port, db_name);
    connect_redis(redis_url);
    init_logger(log_level);
}

fn dispatch_event(event: &str, data: &EventData) {
    match event {
        "user_created" => handle_user_created(data),
        "user_deleted" => handle_user_deleted(data),
        "order_placed" => handle_order_placed(data),
        "order_shipped" => handle_order_shipped(data),
        _ => log::warn!("Unknown event: {}", event),
    }
}
```

### Good Code (Fix)
```rust
struct DatabaseConfig {
    host: String,
    port: u16,
    name: String,
}

struct AppConfig {
    database: DatabaseConfig,
    redis_url: String,
    api_key: String,
    log_level: LogLevel,
}

fn setup_app(config: &AppConfig) {
    connect_db(&config.database.host, config.database.port, &config.database.name);
    connect_redis(&config.redis_url);
    init_logger(&config.log_level);
}

enum Event {
    UserCreated(UserData),
    UserDeleted(UserId),
    OrderPlaced(OrderData),
    OrderShipped(OrderId),
}

fn dispatch_event(event: Event) {
    match event {
        Event::UserCreated(data) => handle_user_created(data),
        Event::UserDeleted(id) => handle_user_deleted(id),
        Event::OrderPlaced(data) => handle_order_placed(data),
        Event::OrderShipped(id) => handle_order_shipped(id),
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression` with `field_expression` (`.get()`), `string_literal` argument
- **Detection approach**: Find repeated `.get("key")` calls on the same `HashMap` or similar container variable where string literals are used as keys. Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    value: (identifier) @obj
    field: (field_identifier) @method)
  arguments: (arguments
    (string_literal) @key))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
