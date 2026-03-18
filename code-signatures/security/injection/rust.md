# Injection -- Rust

## Overview
While Rust's type system and ownership model provide strong memory safety guarantees, injection vulnerabilities can still occur when user-supplied input is interpolated into SQL queries or shell commands via string formatting. The `format!` macro and string concatenation bypass the safety provided by parameterized query APIs and structured command builders.

## Why It's a Security Concern
SQL injection allows attackers to read, modify, or delete arbitrary database records, potentially compromising the entire data layer. Command injection via `std::process::Command` with unsanitized arguments can lead to arbitrary code execution on the host system. Rust's safety guarantees do not extend to logical injection flaws in string handling.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: sqlx, diesel, rusqlite, tokio-postgres, std::process::Command

---

## Pattern 1: SQL Injection via format! in sqlx::query

### Description
Using `format!` or `format_args!` to build SQL query strings with user-supplied values, then passing the formatted string to `sqlx::query()` or `sqlx::raw_sql()`. This bypasses sqlx's built-in parameterized query support (`$1`, `$2` bind parameters).

### Bad Code (Anti-pattern)
```rust
use sqlx::PgPool;

async fn get_user(pool: &PgPool, user_id: &str) -> Result<Option<User>, sqlx::Error> {
    let query = format!("SELECT * FROM users WHERE id = '{}'", user_id);
    let user = sqlx::query_as::<_, User>(&query)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}
```

### Good Code (Fix)
```rust
use sqlx::PgPool;

async fn get_user(pool: &PgPool, user_id: &str) -> Result<Option<User>, sqlx::Error> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `macro_invocation`, `token_tree`, `string_literal`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the function path contains `sqlx::query`, `sqlx::query_as`, or `sqlx::raw_sql` and the argument is an `identifier` or `call_expression` referencing `format!`. Alternatively, find `macro_invocation` of `format!` whose result is stored in a variable that is subsequently passed to a sqlx query function. The presence of SQL keywords in the format string confirms the pattern.
- **S-expression query sketch**:
```scheme
(let_declaration
  pattern: (identifier) @var_name
  value: (macro_invocation
    macro: (identifier) @macro_name
    (token_tree
      (string_literal) @sql_fragment)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_format_macro`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via Command::new with Unsanitized Args

### Description
Constructing shell commands using `std::process::Command::new()` where user-controlled input is passed as arguments without validation. While `Command` does not invoke a shell by default (unlike C's `system()`), passing user input to `Command::new("sh").arg("-c").arg(user_string)` or using user input as the program name reintroduces shell injection risks.

### Bad Code (Anti-pattern)
```rust
use std::process::Command;

fn run_tool(user_input: &str) -> std::io::Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("tool --file {}", user_input))
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_program(program_name: &str) -> std::io::Result<String> {
    let output = Command::new(program_name)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

### Good Code (Fix)
```rust
use std::process::Command;

fn run_tool(user_input: &str) -> std::io::Result<String> {
    // Pass arguments directly, not through a shell
    let output = Command::new("tool")
        .arg("--file")
        .arg(user_input)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_program(program_name: &str) -> std::io::Result<String> {
    // Validate against an allowlist
    let allowed = ["tool1", "tool2", "tool3"];
    if !allowed.contains(&program_name) {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "program not allowed"));
    }
    let output = Command::new(program_name)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `macro_invocation`, `string_literal`
- **Detection approach**: Find call chains starting with `Command::new("sh")` or `Command::new("bash")` followed by `.arg("-c")` and a `.arg()` containing a `format!` macro or variable reference. Also detect `Command::new()` where the argument is a function parameter or variable (not a string literal), indicating the program name is user-controlled.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    value: (call_expression
      function: (field_expression
        field: (field_identifier) @method))
    field: (field_identifier) @chained_method)
  arguments: (arguments
    (macro_invocation
      macro: (identifier) @macro_name)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_process`
- **Severity**: error
- **Confidence**: high
