# Dead Code -- Rust

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. It also increases compilation time and complicates refactoring. Rust's compiler already warns on unused code, but `#[allow(dead_code)]` suppressions can mask legitimate dead code that should be removed.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Suppressed Dead Code Warnings

### Description
Functions or types annotated with `#[allow(dead_code)]` to silence the compiler instead of removing the unused code. While sometimes intentional for public API stubs, this is often used to defer cleanup indefinitely.

### Bad Code (Anti-pattern)
```rust
#[allow(dead_code)]
fn legacy_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 5381;
    for &byte in data {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    hash
}

fn hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}
```

### Good Code (Fix)
```rust
fn hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `attribute_item` containing `allow(dead_code)`
- **Detection approach**: Find `attribute_item` nodes whose content includes `allow(dead_code)`. The annotated item (function, struct, enum, impl block) is a candidate for removal. Exclude items in library crates that are part of the public API (`pub` visibility), and items with doc comments indicating intentional future use.
- **S-expression query sketch**:
  ```scheme
  (attribute_item
    (attribute
      (identifier) @attr_name
      arguments: (token_tree) @args)
    (#eq? @attr_name "allow"))
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `suppressed_dead_code`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Panic/Unreachable

### Description
Code statements that appear after an unconditional return, `panic!()`, `unreachable!()`, `std::process::exit()`, or diverging expression — they can never execute.

### Bad Code (Anti-pattern)
```rust
fn parse_mode(input: &str) -> Mode {
    match input {
        "fast" => Mode::Fast,
        "safe" => Mode::Safe,
        _ => {
            panic!("Unknown mode: {input}");
            Mode::Safe // unreachable — panic diverges
        }
    }
}

fn shutdown(code: i32) {
    cleanup_resources();
    std::process::exit(code);
    log::info!("Shutdown complete"); // unreachable
}
```

### Good Code (Fix)
```rust
fn parse_mode(input: &str) -> Mode {
    match input {
        "fast" => Mode::Fast,
        "safe" => Mode::Safe,
        _ => panic!("Unknown mode: {input}"),
    }
}

fn shutdown(code: i32) {
    cleanup_resources();
    std::process::exit(code);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_expression`, `macro_invocation` (for `panic!`, `unreachable!`, `todo!`), `call_expression` (for `std::process::exit`)
- **Detection approach**: For each diverging expression, check if there are sibling statements after it in the same block. In Rust, `panic!`, `unreachable!`, `todo!`, and `std::process::exit` are diverging (return `!`). Also check for statements after `return` expressions. Exclude statements in `unsafe` blocks that may have platform-specific control flow.
- **S-expression query sketch**:
  ```scheme
  (block
    (expression_statement
      (macro_invocation macro: (identifier) @macro_name
        (#any-of? @macro_name "panic" "unreachable" "todo"))) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```rust
fn process_request(req: &Request) -> Response {
    // fn validate_headers(req: &Request) -> Result<(), HeaderError> {
    //     let auth = req.headers().get("Authorization")
    //         .ok_or(HeaderError::Missing("Authorization"))?;
    //     let content_type = req.headers().get("Content-Type")
    //         .ok_or(HeaderError::Missing("Content-Type"))?;
    //     if !content_type.to_str()?.starts_with("application/json") {
    //         return Err(HeaderError::InvalidContentType);
    //     }
    //     Ok(())
    // }

    Response::ok(handle(req))
}
```

### Good Code (Fix)
```rust
fn process_request(req: &Request) -> Response {
    Response::ok(handle(req))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `line_comment`, `block_comment`
- **Detection approach**: Find comment nodes whose content matches Rust code patterns (contains `fn `, `let `, `struct `, `impl `, `match `, `::`, `->`, `.unwrap()`, `.expect()`). Flag blocks of 5+ consecutive comment lines that look like code. Distinguish from doc comments (`///`, `//!`) and descriptive comments.
- **S-expression query sketch**:
  ```scheme
  (line_comment) @comment
  (block_comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
