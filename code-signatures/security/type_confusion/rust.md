# Type Confusion -- Rust

## Overview
Rust's type system provides strong guarantees at compile time, making type confusion vulnerabilities rare in safe code. However, `unsafe` blocks allow bypassing these guarantees, and `std::mem::transmute` can reinterpret the bits of one type as another without any validation. When used between incompatible types, `transmute` causes undefined behavior -- corrupted data, memory safety violations, and exploitable crashes.

## Why It's a Security Concern
`transmute` performs a bitwise reinterpretation with zero runtime checks. Transmuting between types of different sizes is a compile error, but transmuting between same-sized incompatible types (e.g., `u64` to `&T`, `i32` to `bool`, or between structs with different layouts) compiles and runs but produces undefined behavior. This can corrupt vtable pointers (enabling code execution), violate type invariants (e.g., creating invalid `bool` or `enum` values), or bypass lifetime and borrow checking guarantees. In security-critical code, transmute misuse can turn Rust's safety guarantees into a false sense of security.

## Applicability
- **Relevance**: low
- **Languages covered**: .rs
- **Frameworks/libraries**: std::mem, any crate using unsafe FFI or raw pointer manipulation

---

## Pattern 1: Transmute Between Incompatible Types

### Description
Using `std::mem::transmute` to convert between types that do not share a compatible memory layout or invariants. Common dangerous patterns include transmuting integers to references or pointers, transmuting between structs with different field layouts, creating invalid `enum` or `bool` values via transmute, and transmuting raw pointers to references without validating alignment or lifetime.

### Bad Code (Anti-pattern)
```rust
use std::mem;

// Transmuting an arbitrary integer to a reference -- undefined behavior
unsafe fn get_item(addr: usize) -> &'static Item {
    mem::transmute::<usize, &'static Item>(addr)
}

// Transmuting between unrelated struct types
#[repr(C)]
struct NetworkPacket {
    header: u32,
    payload: [u8; 256],
}

unsafe fn parse_as_config(packet: &NetworkPacket) -> &Config {
    // Config may have different size, alignment, or invariants
    mem::transmute(packet)
}

// Creating an invalid bool (only 0 and 1 are valid)
unsafe fn int_to_bool(val: u8) -> bool {
    mem::transmute(val) // UB if val is not 0 or 1
}
```

### Good Code (Fix)
```rust
use std::mem;

// Use proper pointer operations with validation
fn get_item(addr: usize) -> Option<&'static Item> {
    let ptr = addr as *const Item;
    if ptr.is_null() || (addr % mem::align_of::<Item>()) != 0 {
        return None;
    }
    // Only dereference after validation, and only if lifetime is guaranteed
    unsafe { ptr.as_ref() }
}

// Use proper deserialization
fn parse_as_config(data: &[u8]) -> Result<Config, ParseError> {
    Config::from_bytes(data) // structured parsing with validation
}

// Use explicit conversion with validation
fn int_to_bool(val: u8) -> Option<bool> {
    match val {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`, `unsafe_block`, `type_arguments`
- **Detection approach**: Find `call_expression` nodes where the function path is `mem::transmute`, `std::mem::transmute`, or a use-aliased `transmute`. Examine the `type_arguments` (turbofish syntax) or inferred types to determine whether the source and target types are structurally compatible. Flag all `transmute` calls as requiring manual review, with higher confidence when transmuting to/from references, pointers, `bool`, or `enum` types.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @module
    name: (identifier) @func)
  (#eq? @module "mem")
  (#eq? @func "transmute"))
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `unsafe_transmute`
- **Severity**: warning
- **Confidence**: high
