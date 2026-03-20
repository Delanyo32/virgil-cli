# API Surface Area -- Rust

## Overview
API surface area in Rust is precisely controlled through visibility modifiers: `pub`, `pub(crate)`, `pub(super)`, and the default private. Rust's module system enforces these boundaries at compile time, making visibility a first-class architectural tool. Tracking the ratio of `pub` items to total items per module identifies modules that over-expose their internals, weakening the encapsulation guarantees that Rust's type system is designed to uphold.

## Why It's an Architecture Concern
Every `pub` item in a Rust crate is part of its public API and subject to semver compatibility rules. A module where nearly everything is `pub` provides no encapsulation: consumers can depend on internal types, helper functions, and data structures that were never intended as stable API. This creates coupling that makes major version bumps necessary for routine refactoring. In library crates especially, a wide `pub` surface means more documentation burden, more potential for misuse, and more breaking changes per release. Using `pub(crate)` for crate-internal sharing and keeping module-level items private by default preserves the freedom to restructure modules without affecting downstream users.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```rust
pub fn parse_header(input: &[u8]) -> Result<Header, Error> { todo!() }
pub fn parse_body(input: &[u8]) -> Result<Body, Error> { todo!() }
pub fn parse_footer(input: &[u8]) -> Result<Footer, Error> { todo!() }
pub fn validate_checksum(data: &[u8]) -> bool { todo!() }
pub fn compute_offset(base: usize, delta: isize) -> usize { todo!() }
pub fn encode_payload(data: &[u8]) -> Vec<u8> { todo!() }
pub fn decode_payload(data: &[u8]) -> Vec<u8> { todo!() }
pub fn compress(data: &[u8]) -> Vec<u8> { todo!() }
pub fn decompress(data: &[u8]) -> Vec<u8> { todo!() }
pub fn format_output(data: &[u8]) -> String { todo!() }
pub struct Header { pub fields: Vec<(String, String)> }
pub struct Body { pub content: Vec<u8> }
```

### Good Code (Fix)
```rust
// Public API — stable contract
pub fn parse_message(input: &[u8]) -> Result<Message, Error> { todo!() }
pub fn encode_payload(data: &[u8]) -> Vec<u8> { todo!() }
pub fn decode_payload(data: &[u8]) -> Vec<u8> { todo!() }
pub struct Message { /* fields private, accessed via methods */ }

// Internal helpers — not exported
fn parse_header(input: &[u8]) -> Result<Header, Error> { todo!() }
fn parse_body(input: &[u8]) -> Result<Body, Error> { todo!() }
fn parse_footer(input: &[u8]) -> Result<Footer, Error> { todo!() }
fn validate_checksum(data: &[u8]) -> bool { todo!() }
fn compute_offset(base: usize, delta: isize) -> usize { todo!() }
fn compress(data: &[u8]) -> Vec<u8> { todo!() }
fn decompress(data: &[u8]) -> Vec<u8> { todo!() }
fn format_output(data: &[u8]) -> String { todo!() }

struct Header { fields: Vec<(String, String)> }
struct Body { content: Vec<u8> }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `struct_item`, `enum_item`, `trait_item`, `type_item`, `const_item`, `static_item`
- **Detection approach**: Count all top-level items in the module. An item is exported if it has a `visibility_modifier` child (any `pub` variant). Flag modules where total >= 10 and exported/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match pub function items
(function_item
  (visibility_modifier) @vis
  name: (identifier) @pub.func.name)

;; Match all function items (pub or private)
(function_item
  name: (identifier) @all.func.name)

;; Match pub struct items
(struct_item
  (visibility_modifier) @vis
  name: (type_identifier) @pub.struct.name)

;; Match pub enum items
(enum_item
  (visibility_modifier) @vis
  name: (type_identifier) @pub.enum.name)

;; Match pub trait items
(trait_item
  (visibility_modifier) @vis
  name: (type_identifier) @pub.trait.name)
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```rust
pub struct ConnectionPool {
    pub connections: Vec<TcpStream>,
    pub available: VecDeque<usize>,
    pub max_size: usize,
    pub timeout: Duration,
    pub retry_delays: Vec<Duration>,
    pub metrics: HashMap<String, u64>,
}

impl ConnectionPool {
    pub fn new(max_size: usize) -> Self { todo!() }
    pub fn acquire(&mut self) -> Result<&TcpStream, Error> { todo!() }
    pub fn release(&mut self, idx: usize) { todo!() }
}
```

### Good Code (Fix)
```rust
pub struct ConnectionPool {
    connections: Vec<TcpStream>,
    available: VecDeque<usize>,
    max_size: usize,
    timeout: Duration,
    retry_delays: Vec<Duration>,
    metrics: HashMap<String, u64>,
}

impl ConnectionPool {
    pub fn new(max_size: usize, timeout: Duration) -> Self { todo!() }
    pub fn acquire(&mut self) -> Result<PooledConnection<'_>, Error> { todo!() }
    pub fn release(&mut self, conn: PooledConnection<'_>) { todo!() }
    pub fn active_count(&self) -> usize { self.connections.len() }
    pub fn metrics(&self) -> &HashMap<String, u64> { &self.metrics }
}

/// RAII guard that releases on drop
pub struct PooledConnection<'a> { /* private fields */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` with `visibility_modifier` inside `struct_item`
- **Detection approach**: Find `pub` struct items and inspect their field declarations. A field is leaked if it has its own `visibility_modifier`. Flag structs where `pub` fields expose concrete collection types (`Vec`, `HashMap`, `VecDeque`) or implementation handles (`TcpStream`, `File`).
- **S-expression query sketch**:
```scheme
;; Match pub structs with pub fields
(struct_item
  (visibility_modifier) @struct.vis
  name: (type_identifier) @struct.name
  body: (field_declaration_list
    (field_declaration
      (visibility_modifier) @field.vis
      name: (field_identifier) @field.name
      type: (_) @field.type)))

;; Post-process: struct.vis is "pub", field.vis is "pub",
;; check field.type for concrete collection types
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
