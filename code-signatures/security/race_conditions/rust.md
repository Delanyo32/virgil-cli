# Race Conditions -- Rust

## Overview
Rust's ownership and borrow checker prevent most data races at compile time, but two categories of race conditions remain possible: TOCTOU vulnerabilities in file system operations (which are an OS-level concern independent of language safety) and data races introduced through `unsafe` code that bypasses the borrow checker's guarantees. Both can lead to exploitable conditions in security-sensitive Rust applications.

## Why It's a Security Concern
TOCTOU races in Rust file operations are exploitable the same way as in any language -- an attacker can swap a checked path with a symlink between the check and the use. Data races via `unsafe` code can corrupt memory, break invariants relied upon by safe code, and introduce undefined behavior that the compiler is free to exploit in surprising ways. Since Rust programs are often chosen for security-critical systems precisely because of their safety guarantees, unsafe data races are especially dangerous as they undermine the core safety promise.

## Applicability
- **Relevance**: medium
- **Languages covered**: .rs
- **Frameworks/libraries**: std::fs, std::path, std::ptr, std::sync, tokio::fs

---

## Pattern 1: TOCTOU in File Operations

### Description
Checking a file's existence or metadata with `Path::exists()`, `fs::metadata()`, or similar methods and then performing a file operation based on the result. Between the check and the operation, the file system state can change, allowing symlink attacks or unexpected behavior.

### Bad Code (Anti-pattern)
```rust
use std::fs;
use std::path::Path;

fn safe_create(path: &Path, content: &str) -> std::io::Result<()> {
    if !path.exists() {
        // RACE: file can be created (or symlinked) between check and write
        fs::write(path, content)?;
    }
    Ok(())
}
```

### Good Code (Fix)
```rust
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

fn safe_create(path: &Path, content: &str) -> std::io::Result<()> {
    // create_new(true) uses O_CREAT | O_EXCL -- atomic create-or-fail
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes())?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `if_expression`, `macro_invocation`
- **Detection approach**: Find `if_expression` nodes whose condition contains a `call_expression` or `field_expression` invoking `.exists()`, `.is_file()`, `.is_dir()`, or `metadata()` on a path variable, where the body contains `fs::write()`, `fs::remove_file()`, `File::create()`, or similar file-mutating calls on the same path. The non-atomic check-then-act pattern indicates a TOCTOU vulnerability.
- **S-expression query sketch**:
```scheme
(if_expression
  condition: (call_expression
    function: (field_expression
      field: (field_identifier) @method)
    (#match? @method "^(exists|is_file|is_dir)$"))
  consequence: (block
    (expression_statement
      (call_expression
        function: (_) @write_func))))
```

### Pipeline Mapping
- **Pipeline name**: `toctou`
- **Pattern name**: `path_exists_then_write`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Data Race via Unsafe Code

### Description
Using `unsafe` blocks to create mutable references to shared state (via raw pointers, `static mut`, or transmuting references) without proper synchronization. This bypasses the borrow checker's guarantee that mutable references are exclusive, allowing concurrent threads to read and write the same memory simultaneously -- a textbook data race that is undefined behavior in Rust.

### Bad Code (Anti-pattern)
```rust
static mut COUNTER: u64 = 0;

fn increment() {
    unsafe {
        // DATA RACE: multiple threads can read-modify-write simultaneously
        COUNTER += 1;
    }
}

fn spawn_workers() {
    let handles: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| increment()))
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}
```

### Good Code (Fix)
```rust
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn increment() {
    COUNTER.fetch_add(1, Ordering::Relaxed);
}

fn spawn_workers() {
    let handles: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| increment()))
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `unsafe_block`, `assignment_expression`, `compound_assignment_expr`, `identifier`
- **Detection approach**: Find `unsafe_block` nodes containing an `assignment_expression` or `compound_assignment_expr` (e.g., `+=`) where the target is a `static mut` variable. Also detect raw pointer dereferences (`*ptr = value`) inside unsafe blocks when the pointer originates from shared state. The combination of `unsafe` + mutation of globally accessible state without `Mutex`/`RwLock`/`Atomic` is the indicator.
- **S-expression query sketch**:
```scheme
(unsafe_block
  (block
    (expression_statement
      (compound_assignment_expr
        left: (identifier) @target))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `unsafe_data_race`
- **Severity**: error
- **Confidence**: high
