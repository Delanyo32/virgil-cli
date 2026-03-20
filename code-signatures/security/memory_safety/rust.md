# Memory Safety -- Rust

## Overview
Rust's ownership system and borrow checker prevent most memory safety issues at compile time. However, `unsafe` blocks allow raw pointer dereferences that bypass these guarantees, reintroducing the full spectrum of memory corruption bugs. Additionally, integer overflow in arithmetic operations (which panics in debug but wraps silently in release mode) can lead to undersized allocations and subsequent buffer overflows.

## Why It's a Security Concern
Rust is often chosen specifically for its memory safety guarantees. When `unsafe` blocks are used carelessly, they create the same vulnerability classes as C/C++ -- but in a codebase where reviewers may assume safety. Integer overflow in release builds silently wraps, meaning a size calculation like `count * element_size` can produce a small value, leading to an undersized allocation and heap buffer overflow when data is written.

## Applicability
- **Relevance**: medium
- **Languages covered**: .rs
- **Frameworks/libraries**: std (ptr, mem, alloc), libc, nix, winapi

---

## Pattern 1: Unsafe Block with Raw Pointer Dereference

### Description
Dereferencing raw pointers (`*const T` or `*mut T`) inside `unsafe` blocks without proper null checks, bounds validation, or lifetime guarantees. This can cause null pointer dereferences, use-after-free, buffer overflows, and data races -- all the vulnerabilities that safe Rust prevents.

### Bad Code (Anti-pattern)
```rust
fn read_from_buffer(ptr: *const u8, offset: usize) -> u8 {
    unsafe {
        *ptr.add(offset) // no null check, no bounds check
    }
}

fn process_data(data: &[u8], index: usize) {
    let ptr = data.as_ptr();
    drop(data);
    unsafe {
        // use-after-free: data may be deallocated
        let val = *ptr.add(index);
        println!("{}", val);
    }
}
```

### Good Code (Fix)
```rust
fn read_from_buffer(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied() // bounds-checked, no unsafe needed
}

fn process_data(data: &[u8], index: usize) {
    if let Some(&val) = data.get(index) {
        println!("{}", val);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `unsafe_block`, `unary_expression`, `call_expression`, `field_expression`
- **Detection approach**: Find `unsafe_block` nodes containing `unary_expression` with operator `*` (dereference) applied to a `call_expression` with method `add`, `offset`, or `as_ref` on a raw pointer, or direct dereference of a raw pointer variable. Flag if no preceding null check (`is_null()`) or bounds validation exists within the same block.
- **S-expression query sketch**:
```scheme
(unsafe_block
  (block
    (expression_statement
      (unary_expression
        operator: "*"
        operand: (call_expression
          function: (field_expression
            field: (field_identifier) @method)
          (#match? @method "^(add|offset|as_ref)$"))))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `unsafe_memory`
- **Severity**: error
- **Confidence**: medium

---

## Pattern 2: Integer Overflow in Arithmetic

### Description
Performing arithmetic operations (multiplication, addition) on user-influenced values without using `checked_mul()`, `checked_add()`, `saturating_*()`, or `overflowing_*()` methods. In release builds, Rust wraps on overflow instead of panicking, which can produce an unexpectedly small value used as a buffer size or array index.

### Bad Code (Anti-pattern)
```rust
fn allocate_grid(width: usize, height: usize) -> Vec<u8> {
    let size = width * height; // wraps on overflow in release mode
    vec![0u8; size] // undersized allocation if overflow occurred
}

fn compute_offset(base: u32, count: u32, stride: u32) -> u32 {
    base + count * stride // silent overflow in release
}
```

### Good Code (Fix)
```rust
fn allocate_grid(width: usize, height: usize) -> Option<Vec<u8>> {
    let size = width.checked_mul(height)?;
    Some(vec![0u8; size])
}

fn compute_offset(base: u32, count: u32, stride: u32) -> Option<u32> {
    let product = count.checked_mul(stride)?;
    base.checked_add(product)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `let_declaration`, `call_expression`
- **Detection approach**: Find `binary_expression` with operator `*` or `+` on integer variables where the result is used as a size argument to `vec![]`, `Vec::with_capacity()`, `alloc::alloc()`, or similar allocation functions. Flag if neither operand is a compile-time constant and no `checked_*` or `saturating_*` method is used.
- **S-expression query sketch**:
```scheme
(let_declaration
  pattern: (identifier) @size_var
  value: (binary_expression
    left: (identifier) @lhs
    operator: "*"
    right: (identifier) @rhs))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `integer_overflow`
- **Severity**: warning
- **Confidence**: medium
