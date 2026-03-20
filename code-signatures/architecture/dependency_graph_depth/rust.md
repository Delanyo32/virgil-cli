# Dependency Graph Depth -- Rust

## Overview
Dependency graph depth measures how many layers of module imports a Rust source file must traverse before reaching the actual implementation. In Rust, deep dependency chains manifest as deeply nested `pub use` re-export chains in `mod.rs` or `lib.rs` files and long `use crate::a::b::c::d` paths, indicating excessive module layering that makes the codebase harder to navigate and more fragile to restructuring.

## Why It's an Architecture Concern
Deep dependency chains in Rust increase the blast radius of changes -- restructuring a module buried several layers deep requires updating re-exports in every intermediate `mod.rs` and every consumer that references the item. `pub use` re-exports in `mod.rs` files create transitive chains where a symbol passes through multiple modules before reaching its consumer, making it difficult to determine where the implementation actually lives. Deeply qualified `use` paths signal an over-nested module hierarchy that adds cognitive overhead without corresponding architectural benefit. Keeping the module tree shallow and re-exports minimal reduces coupling, simplifies `cargo doc` output, and makes refactoring less risky.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In Rust, the barrel file pattern manifests as `mod.rs` or `lib.rs` files that declare submodules and re-export their contents via `pub use`, providing a flattened public API for a module. While this is idiomatic for crate root APIs, internal modules that extensively re-export create unnecessary indirection layers and mask the true module structure from both developers and tooling.

### Bad Code (Anti-pattern)
```rust
// src/services/mod.rs -- barrel module re-exporting everything
mod auth;
mod billing;
mod email;
mod reporting;
mod storage;
mod users;

pub use auth::{AuthService, TokenValidator};
pub use billing::{BillingService, InvoiceGenerator};
pub use email::{EmailService, TemplateRenderer};
pub use reporting::{ReportService, ChartBuilder};
pub use storage::{StorageService, FileManager};
pub use users::{UserService, ProfileManager};
```

### Good Code (Fix)
```rust
// src/api/payment.rs -- imports directly from source modules
use crate::services::auth::AuthService;
use crate::services::billing::BillingService;

pub struct PaymentHandler {
    auth: AuthService,
    billing: BillingService,
}

impl PaymentHandler {
    pub fn process(&self, token: &str, card_id: &str) -> Result<(), Error> {
        self.auth.validate(token)?;
        self.billing.charge(card_id)
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration` with `visibility_modifier`
- **Detection approach**: Count `pub use` re-export statements in a single file. Flag if count >= 5, especially if the file is `mod.rs` or `lib.rs`. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Public re-exports (pub use)
(use_declaration
  (visibility_modifier) @vis
  argument: (scoped_identifier) @reexport_path) @pub_use

;; Public re-exports with use list (pub use mod::{A, B})
(use_declaration
  (visibility_modifier) @vis
  argument: (use_as_clause) @reexport_alias) @pub_use

;; Public re-exports with glob (pub use mod::*)
(use_declaration
  (visibility_modifier) @vis
  argument: (use_wildcard) @reexport_glob) @pub_use
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In Rust this appears as `use` declarations with many `::` separated segments, such as `use crate::a::b::c::d::Type`.

### Bad Code (Anti-pattern)
```rust
use crate::infrastructure::persistence::repositories::postgres::OrderRepository;
use crate::infrastructure::persistence::repositories::redis::CacheRepository;
use crate::domain::aggregates::orders::value_objects::OrderStatus;
use crate::application::services::orders::handlers::CreateOrderHandler;

pub struct OrderController {
    repo: OrderRepository,
    cache: CacheRepository,
    handler: CreateOrderHandler,
}

impl OrderController {
    pub fn create_order(&self, data: OrderData) -> Result<Order, Error> {
        self.handler.execute(data)
    }
}
```

### Good Code (Fix)
```rust
use crate::persistence::OrderRepository;
use crate::cache::CacheRepository;
use crate::domain::orders::OrderStatus;
use crate::services::CreateOrderHandler;

pub struct OrderController {
    repo: OrderRepository,
    cache: CacheRepository,
    handler: CreateOrderHandler,
}

impl OrderController {
    pub fn create_order(&self, data: OrderData) -> Result<Order, Error> {
        self.handler.execute(data)
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`, `scoped_identifier`
- **Detection approach**: Parse the use path and count `::` separated segments. For paths starting with `crate::`, `self::`, or `super::`, count segments after the prefix. Flag if depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture use declarations for path depth analysis
(use_declaration
  argument: (scoped_identifier) @use_path) @use_decl

;; Capture use declarations with use lists
(use_declaration
  argument: (scoped_use_list
    path: (scoped_identifier) @use_list_path)) @use_list_decl
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
