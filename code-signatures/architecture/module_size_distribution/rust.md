# Module Size Distribution -- Rust

## Overview
Module size distribution measures how symbol definitions are spread across source files in a Rust codebase. Rust's module system encourages splitting code into focused files, and balanced module sizes make it easier to understand each file's responsibility, keep compile times manageable, and reduce merge conflicts. Files that are excessively large or contain only a trivial definition signal structural problems.

## Why It's an Architecture Concern
Oversized Rust modules concentrate too many functions, structs, enums, and trait implementations into a single file, making the file difficult to navigate and reason about. Because Rust's borrow checker errors can cascade, a large file with many interdependent types becomes especially hard to refactor. Oversized modules also slow down incremental compilation since the entire file must be re-checked for any change. Anemic modules that wrap a single constant or trivial function add module tree complexity and `use` statement overhead without providing meaningful organizational benefit.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```rust
// lib.rs -- monolithic module mixing unrelated concerns
pub fn parse_config(path: &str) -> Config { /* ... */ }
pub fn validate_input(input: &str) -> bool { /* ... */ }
pub fn format_output(code: i32) -> String { /* ... */ }
pub fn send_request(url: &str) -> Result<Response, Error> { /* ... */ }
pub fn hash_password(pw: &str) -> String { /* ... */ }

pub struct Config { /* ... */ }
pub struct Response { /* ... */ }
pub struct User { /* ... */ }
pub struct Session { /* ... */ }

pub enum Error { /* ... */ }
pub enum Status { /* ... */ }

pub trait Serializable { fn serialize(&self) -> Vec<u8>; }
pub trait Cacheable { fn cache_key(&self) -> String; }

pub type Result<T> = std::result::Result<T, Error>;
pub const MAX_RETRIES: u32 = 5;
pub const DEFAULT_TIMEOUT: u64 = 30;
pub static GLOBAL_COUNTER: AtomicU64 = AtomicU64::new(0);
// ... 15 more functions, structs, enums, and constants
```

### Good Code (Fix)
```rust
// config.rs -- focused on configuration
pub struct Config {
    pub timeout: u64,
    pub retries: u32,
}

pub fn parse_config(path: &str) -> Config { /* ... */ }
pub fn validate_config(config: &Config) -> bool { /* ... */ }
```

```rust
// network.rs -- focused on network operations
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

pub fn send_request(url: &str) -> Result<Response, Error> { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `struct_item`, `enum_item`, `trait_item`, `type_item`, `const_item`, `static_item`, `impl_item`, `mod_item`, `macro_definition`
- **Detection approach**: Count all top-level symbol definitions (direct children of `source_file`). Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(source_file
  [
    (function_item name: (identifier) @name) @def
    (struct_item name: (type_identifier) @name) @def
    (enum_item name: (type_identifier) @name) @def
    (trait_item name: (type_identifier) @name) @def
    (type_item name: (type_identifier) @name) @def
    (const_item name: (identifier) @name) @def
    (static_item name: (identifier) @name) @def
    (impl_item) @def
    (mod_item name: (identifier) @name) @def
    (macro_definition name: (identifier) @name) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```rust
// lib.rs -- too many public symbols
pub mod config;
pub mod network;
pub mod cache;
pub mod auth;

pub use config::{Config, ConfigError, parse_config, validate_config};
pub use network::{HttpClient, Response, send_request, download_file};
pub use cache::{Cache, CacheEntry, CachePolicy, evict, warm_up};
pub use auth::{authenticate, authorize, Token, Credentials, Role};
pub use self::utils::{hash, encode, decode, slugify, truncate};
// 25+ public symbols re-exported at the crate root
```

### Good Code (Fix)
```rust
// lib.rs -- curated public API with focused re-exports
pub mod config;
pub mod network;
pub mod cache;
pub mod auth;

pub use config::Config;
pub use network::HttpClient;
pub use cache::Cache;
pub use auth::Token;
```

```rust
// config/mod.rs -- sub-module owns its detailed exports
pub struct Config { /* ... */ }
pub fn parse_config(path: &str) -> Config { /* ... */ }
pub fn validate_config(config: &Config) -> bool { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `struct_item`, `enum_item`, `trait_item`, `type_item`, `const_item`, `static_item`, `macro_definition`, `use_declaration`
- **Detection approach**: Count symbols with a `visibility_modifier` (any `pub` variant). Also count `pub use` re-exports. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(function_item
  (visibility_modifier) @vis
  name: (identifier) @name) @def

(struct_item
  (visibility_modifier) @vis
  name: (type_identifier) @name) @def

(enum_item
  (visibility_modifier) @vis
  name: (type_identifier) @name) @def

(use_declaration
  (visibility_modifier) @vis) @def
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```rust
// version.rs
pub const VERSION: &str = "1.2.3";
```

### Good Code (Fix)
```rust
// app.rs -- merge the trivial constant into a related module
pub const VERSION: &str = "1.2.3";
pub const BUILD_DATE: &str = env!("BUILD_DATE");

pub fn print_banner() {
    println!("{} v{}", env!("CARGO_PKG_NAME"), VERSION);
}

pub fn default_config() -> Config {
    Config { retries: 3, timeout: 30 }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`, `struct_item`, `enum_item`, `trait_item`, `type_item`, `const_item`, `static_item`, `impl_item`, `mod_item`, `macro_definition`
- **Detection approach**: Count top-level symbol definitions (direct children of `source_file`). Flag if count == 1, excluding `main.rs`, `lib.rs`, `mod.rs`, `build.rs`, and test files.
- **S-expression query sketch**:
```scheme
(source_file
  [
    (function_item name: (identifier) @name) @def
    (struct_item name: (type_identifier) @name) @def
    (enum_item name: (type_identifier) @name) @def
    (trait_item name: (type_identifier) @name) @def
    (type_item name: (type_identifier) @name) @def
    (const_item name: (identifier) @name) @def
    (static_item name: (identifier) @name) @def
    (impl_item) @def
    (mod_item name: (identifier) @name) @def
    (macro_definition name: (identifier) @name) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
