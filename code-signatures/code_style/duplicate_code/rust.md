# Duplicate Code -- Rust

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more functions with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared function with parameters or a generic function.

### Bad Code (Anti-pattern)
```rust
fn process_user_request(payload: &UserPayload) -> Result<Response, AppError> {
    let validated = validate_fields(&payload.name, &payload.email)?;
    if validated.name.is_empty() {
        return Err(AppError::Validation("Name is required".into()));
    }
    let normalized_name = validated.name.trim().to_lowercase();
    let record = db::users::insert(UserRecord {
        name: normalized_name,
        email: validated.email.clone(),
        created_at: Utc::now(),
    })?;
    log::info!("Created user {}", record.id);
    Ok(Response::created(record))
}

fn process_vendor_request(payload: &VendorPayload) -> Result<Response, AppError> {
    let validated = validate_fields(&payload.name, &payload.email)?;
    if validated.name.is_empty() {
        return Err(AppError::Validation("Name is required".into()));
    }
    let normalized_name = validated.name.trim().to_lowercase();
    let record = db::vendors::insert(VendorRecord {
        name: normalized_name,
        email: validated.email.clone(),
        created_at: Utc::now(),
    })?;
    log::info!("Created vendor {}", record.id);
    Ok(Response::created(record))
}
```

### Good Code (Fix)
```rust
trait Insertable {
    type Record;
    fn name(&self) -> &str;
    fn email(&self) -> &str;
    fn insert(name: String, email: String) -> Result<Self::Record, AppError>;
    fn entity_type() -> &'static str;
}

fn process_request<T: Insertable>(payload: &T) -> Result<Response, AppError>
where
    T::Record: serde::Serialize,
{
    let validated = validate_fields(payload.name(), payload.email())?;
    if validated.name.is_empty() {
        return Err(AppError::Validation("Name is required".into()));
    }
    let normalized_name = validated.name.trim().to_lowercase();
    let record = T::insert(normalized_name, validated.email.clone())?;
    log::info!("Created {} record", T::entity_type());
    Ok(Response::created(record))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `block`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(function_item
  name: (identifier) @func_name
  body: (block) @func_body)
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `cloned_function_bodies`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Repeated Logic Blocks Within a Function

### Description
The same sequence of 5+ statements repeated within a function or across methods in the same impl block, often due to copy-paste during development. Common in duplicated match arm bodies and repeated Result handling chains.

### Bad Code (Anti-pattern)
```rust
fn handle_event(event: Event) -> Result<(), AppError> {
    match event {
        Event::UserCreated(data) => {
            let conn = pool.get()?;
            let normalized = data.name.trim().to_lowercase();
            let exists = conn.query_row("SELECT COUNT(*) FROM users WHERE email = ?", [&data.email], |r| r.get::<_, i64>(0))?;
            if exists > 0 {
                log::warn!("Duplicate user: {}", data.email);
                return Ok(());
            }
            conn.execute("INSERT INTO users (name, email) VALUES (?, ?)", [&normalized, &data.email])?;
            log::info!("Inserted user: {}", data.email);
        }
        Event::VendorCreated(data) => {
            let conn = pool.get()?;
            let normalized = data.name.trim().to_lowercase();
            let exists = conn.query_row("SELECT COUNT(*) FROM vendors WHERE email = ?", [&data.email], |r| r.get::<_, i64>(0))?;
            if exists > 0 {
                log::warn!("Duplicate vendor: {}", data.email);
                return Ok(());
            }
            conn.execute("INSERT INTO vendors (name, email) VALUES (?, ?)", [&normalized, &data.email])?;
            log::info!("Inserted vendor: {}", data.email);
        }
    }
    Ok(())
}
```

### Good Code (Fix)
```rust
fn upsert_entity(pool: &Pool, table: &str, name: &str, email: &str) -> Result<(), AppError> {
    let conn = pool.get()?;
    let normalized = name.trim().to_lowercase();
    let exists = conn.query_row(
        &format!("SELECT COUNT(*) FROM {} WHERE email = ?", table),
        [email],
        |r| r.get::<_, i64>(0),
    )?;
    if exists > 0 {
        log::warn!("Duplicate {}: {}", table, email);
        return Ok(());
    }
    conn.execute(
        &format!("INSERT INTO {} (name, email) VALUES (?, ?)", table),
        [&normalized, email],
    )?;
    log::info!("Inserted into {}: {}", table, email);
    Ok(())
}

fn handle_event(event: Event) -> Result<(), AppError> {
    match event {
        Event::UserCreated(data) => upsert_entity(&pool, "users", &data.name, &data.email)?,
        Event::VendorCreated(data) => upsert_entity(&pool, "vendors", &data.name, &data.email)?,
    }
    Ok(())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `block`, `expression_statement`, `let_declaration`, `if_expression`, `match_arm`
- **Detection approach**: Sliding window comparison of statement sequences within and across function bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences. Match arm bodies are particularly prone to duplication.
- **S-expression query sketch**:
```scheme
(block
  (_) @stmt)

(match_arm
  body: (block
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
