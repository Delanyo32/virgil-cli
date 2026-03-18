# Magic Values -- Rust

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```rust
fn process_request(data: &[u8]) -> Result<(), Error> {
    if data.len() > 1024 {
        return Err(Error::PayloadTooLarge);
    }
    for _ in 0..3 {
        std::thread::sleep(Duration::from_secs(86400));
    }
    if response.status() == 200 {
        cache.insert(key, data, 3600);
    } else if response.status() == 404 {
        return Ok(());
    }
    Ok(())
}
```

### Good Code (Fix)
```rust
const MAX_PAYLOAD_SIZE: usize = 1024;
const MAX_RETRIES: u32 = 3;
const SECONDS_PER_DAY: u64 = 86400;
const HTTP_OK: u16 = 200;
const HTTP_NOT_FOUND: u16 = 404;
const CACHE_TTL_SECONDS: u64 = 3600;

fn process_request(data: &[u8]) -> Result<(), Error> {
    if data.len() > MAX_PAYLOAD_SIZE {
        return Err(Error::PayloadTooLarge);
    }
    for _ in 0..MAX_RETRIES {
        std::thread::sleep(Duration::from_secs(SECONDS_PER_DAY));
    }
    if response.status() == HTTP_OK {
        cache.insert(key, data, CACHE_TTL_SECONDS);
    } else if response.status() == HTTP_NOT_FOUND {
        return Ok(());
    }
    Ok(())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `integer_literal`, `float_literal` (excludes 0, 1, -1)
- **Detection approach**: Find `integer_literal` and `float_literal` nodes in expressions. Exclude literals inside `const_item`, `static_item`, `enum_variant`, or `attribute_item` ancestor nodes. Also exclude `index_expression` index positions. Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
[(integer_literal) @number (float_literal) @number]
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```rust
fn handle_user(user: &User) {
    if user.role == "admin" {
        grant_access("dashboard");
    }
    match user.status.as_str() {
        "active" => notify(user),
        "pending" => queue(user),
        _ => {}
    }
    let db_url = config.get("database_url").unwrap();
}
```

### Good Code (Fix)
```rust
const ROLE_ADMIN: &str = "admin";
const STATUS_ACTIVE: &str = "active";
const STATUS_PENDING: &str = "pending";
const CONFIG_DATABASE_URL: &str = "database_url";

fn handle_user(user: &User) {
    if user.role == ROLE_ADMIN {
        grant_access("dashboard");
    }
    match user.status.as_str() {
        STATUS_ACTIVE => notify(user),
        STATUS_PENDING => queue(user),
        _ => {}
    }
    let db_url = config.get(CONFIG_DATABASE_URL).unwrap();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string_literal` in `binary_expression` (equality checks), `match_pattern` (match arms), or `arguments` (function call arguments)
- **Detection approach**: Find `string_literal` nodes used in equality comparisons (`==`, `!=`), match arm patterns, or as arguments to map/config lookup methods (`.get()`, `.insert()`). Exclude logging strings, error messages, and format macro strings. Flag repeated identical strings across a function or file.
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: ["==" "!="]
  [left: (string_literal) @string_lit
   right: (string_literal) @string_lit])

(match_arm
  pattern: (string_literal) @string_lit)
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
