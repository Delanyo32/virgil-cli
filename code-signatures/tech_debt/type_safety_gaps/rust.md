# Type Safety Gaps -- Rust

## Overview
Rust's type system provides strong safety guarantees, but developers can bypass them with `as` casts that silently truncate or overflow values and with `unsafe` blocks that perform type transmutation. These patterns undermine the compiler's ability to catch type-related bugs at compile time.

## Why It's a Tech Debt Concern
The `as` keyword performs unchecked casts that can silently truncate values (e.g., `u64` to `u32`), change sign (e.g., `i32` to `u32` on negative values), or lose precision (e.g., `f64` to `f32`). These bugs are invisible at the cast site and manifest as incorrect data downstream. Unsafe transmutation (`std::mem::transmute`, `transmute_copy`) reinterprets memory without any type checking, risking undefined behavior if the source and target types have different layouts, validity invariants, or sizes.

## Applicability
- **Relevance**: high (these patterns bypass Rust's core safety guarantees)
- **Languages covered**: `.rs`
- **Frameworks/libraries**: All Rust codebases; Clippy lints `as_conversions`, `cast_possible_truncation`, `transmute_ptr_to_ref`

---

## Pattern 1: Unnecessary `as` Type Casting

### Description
Using the `as` keyword for numeric casts that could silently truncate, overflow, or change sign. The `as` cast performs no runtime checks -- a `u64` value of 300 cast to `u8` silently becomes 44. Safe alternatives like `try_into()`, `try_from()`, and explicit range checks should be used instead.

### Bad Code (Anti-pattern)
```rust
fn process_packet(data: &[u8]) -> u8 {
    let length = data.len() as u8;  // Truncates if data.len() > 255
    let offset = calculate_offset() as u32;  // i64 -> u32 may lose sign/magnitude
    let ratio = compute_ratio() as f32;  // f64 -> f32 loses precision
    let index = user_input as usize;  // i32 -> usize: negative becomes huge
    length
}

fn write_header(total: u64, buf: &mut [u8]) {
    buf[0] = total as u8;
    buf[1] = (total >> 8) as u8;
    let count = items.len() as u16;  // Truncates if more than 65535 items
}
```

### Good Code (Fix)
```rust
use std::convert::TryFrom;

fn process_packet(data: &[u8]) -> Result<u8, PacketError> {
    let length = u8::try_from(data.len())
        .map_err(|_| PacketError::TooLarge(data.len()))?;
    let offset = u32::try_from(calculate_offset())
        .map_err(|_| PacketError::InvalidOffset)?;
    let ratio = compute_ratio();  // Keep as f64 unless f32 is required
    let index = usize::try_from(user_input)
        .map_err(|_| PacketError::InvalidIndex(user_input))?;
    Ok(length)
}

fn write_header(total: u64, buf: &mut [u8]) -> Result<(), HeaderError> {
    buf[0] = u8::try_from(total & 0xFF).unwrap();  // Masked, safe
    buf[1] = u8::try_from((total >> 8) & 0xFF).unwrap();  // Masked, safe
    let count = u16::try_from(items.len())
        .map_err(|_| HeaderError::TooManyItems(items.len()))?;
    Ok(())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_cast_expression`, `primitive_type`
- **Detection approach**: Find `type_cast_expression` nodes (which represent `expr as Type`). Extract the target type from the `type` child. Flag casts between numeric types where truncation or sign change is possible: any cast from a larger integer to a smaller one, signed to unsigned, or `f64` to `f32`. Exclude identity casts and casts where the value is masked beforehand.
- **S-expression query sketch**:
```scheme
(type_cast_expression
  value: (_) @value
  type: (primitive_type) @target_type)
```

### Pipeline Mapping
- **Pipeline name**: `type_safety_gaps`
- **Pattern name**: `unsafe_as_cast`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Using `unsafe` Blocks for Type Transmutation

### Description
Using `std::mem::transmute` or `std::mem::transmute_copy` inside `unsafe` blocks to reinterpret memory from one type to another. Transmutation bypasses all type checking and can cause undefined behavior if the types differ in size, alignment, or validity invariants.

### Bad Code (Anti-pattern)
```rust
use std::mem;

fn to_bytes(value: &f64) -> [u8; 8] {
    unsafe { mem::transmute(*value) }
}

fn parse_flags(raw: u32) -> MyFlags {
    unsafe { mem::transmute(raw) }  // UB if raw contains invalid flag combinations
}

fn cast_ref<'a>(data: &'a [u8]) -> &'a Header {
    unsafe { mem::transmute(&data[0]) }  // Alignment and size not verified
}

fn int_to_enum(val: i32) -> Status {
    unsafe { mem::transmute(val) }  // UB if val is not a valid Status discriminant
}
```

### Good Code (Fix)
```rust
fn to_bytes(value: &f64) -> [u8; 8] {
    value.to_ne_bytes()
}

fn parse_flags(raw: u32) -> Result<MyFlags, FlagError> {
    MyFlags::from_bits(raw).ok_or(FlagError::Invalid(raw))
}

fn cast_ref(data: &[u8]) -> Result<&Header, ParseError> {
    if data.len() < std::mem::size_of::<Header>() {
        return Err(ParseError::TooShort);
    }
    // Use a safe parsing library like zerocopy or bytemuck
    bytemuck::try_from_bytes(&data[..std::mem::size_of::<Header>()])
        .map_err(|e| ParseError::Alignment(e))
}

fn int_to_enum(val: i32) -> Result<Status, ConvertError> {
    match val {
        0 => Ok(Status::Active),
        1 => Ok(Status::Inactive),
        2 => Ok(Status::Pending),
        _ => Err(ConvertError::InvalidDiscriminant(val)),
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `unsafe_block`, `call_expression`, `scoped_identifier`
- **Detection approach**: Find `call_expression` nodes inside `unsafe_block` where the function path is `mem::transmute`, `std::mem::transmute`, `mem::transmute_copy`, or `std::mem::transmute_copy`. Also match unqualified `transmute` if it is imported from `std::mem`. Flag every transmute call.
- **S-expression query sketch**:
```scheme
(unsafe_block
  (block
    (expression_statement
      (call_expression
        function: (scoped_identifier) @func_path))))
```

### Pipeline Mapping
- **Pipeline name**: `type_safety_gaps`
- **Pattern name**: `unsafe_transmute`
- **Severity**: error
- **Confidence**: high
