# Circular Dependencies -- Rust

## Overview
Circular dependencies in Rust are prevented at the crate level by Cargo, which enforces an acyclic dependency graph between crates. However, within a single crate, modules can freely reference each other via `use crate::`, `use super::`, and `use self::` paths, creating intra-crate cycles. These intra-crate circular dependencies compile successfully but indicate tightly coupled modules that resist extraction into separate crates. They make the internal architecture harder to understand and are a barrier to future modularization.

## Why It's an Architecture Concern
While Rust's crate system prevents inter-crate cycles, intra-crate module cycles make it impossible to later split a growing crate into smaller, independently versioned crates without first untangling the mutual dependencies. Modules in a cycle cannot be reasoned about independently — understanding one requires understanding all modules in the cycle. Testing becomes harder because you cannot isolate a module's behavior from its cyclic partners. `mod.rs` files that re-export submodule items while those submodules `use` items from the parent create especially confusing dependency structures. Cycles indicate tangled responsibilities that violate the principle of each module having a clear, focused purpose.

## Applicability
- **Relevance**: high (intra-crate cycles are common and the compiler does not prevent them)
- **Languages covered**: `.rs`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```rust
// --- src/engine.rs ---
use crate::renderer::Renderer;  // engine.rs imports from renderer.rs
use crate::renderer::RenderConfig;

pub struct Engine {
    renderer: Renderer,
    tick_count: u64,
}

impl Engine {
    pub fn new(config: RenderConfig) -> Self {
        Self { renderer: Renderer::new(config), tick_count: 0 }
    }

    pub fn update(&mut self) {
        self.tick_count += 1;
        self.renderer.draw_frame(self.tick_count);
    }
}

// --- src/renderer.rs ---
use crate::engine::Engine;  // renderer.rs imports from engine.rs -- CIRCULAR

pub struct Renderer {
    frame_count: u64,
}

pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
}

impl Renderer {
    pub fn new(_config: RenderConfig) -> Self {
        Self { frame_count: 0 }
    }

    pub fn draw_frame(&mut self, tick: u64) {
        self.frame_count += 1;
    }

    pub fn needs_engine_state(&self, engine: &Engine) -> bool {
        engine.tick_count > 0  // accesses Engine internals
    }
}
```

### Good Code (Fix)
```rust
// --- src/types.rs --- (shared types extracted to break cycle)
pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
}

pub struct FrameContext {
    pub tick_count: u64,
}

// --- src/renderer.rs ---
use crate::types::{RenderConfig, FrameContext};  // depends only on types

pub struct Renderer {
    frame_count: u64,
}

impl Renderer {
    pub fn new(_config: RenderConfig) -> Self {
        Self { frame_count: 0 }
    }

    pub fn draw_frame(&mut self, ctx: &FrameContext) {
        self.frame_count += 1;
    }
}

// --- src/engine.rs ---
use crate::types::{RenderConfig, FrameContext};  // depends only on types
use crate::renderer::Renderer;  // unidirectional

pub struct Engine {
    renderer: Renderer,
    tick_count: u64,
}

impl Engine {
    pub fn update(&mut self) {
        self.tick_count += 1;
        let ctx = FrameContext { tick_count: self.tick_count };
        self.renderer.draw_frame(&ctx);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`
- **Detection approach**: Per-file: extract all `use` paths (especially `crate::`, `super::`, `self::` prefixed paths) from each Rust file. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each module to its imported modules, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files that both `use` from and are `use`d by the same sibling module.
- **S-expression query sketch**:
```scheme
(use_declaration
  argument: (scoped_identifier
    path: (identifier) @root_path
    name: (identifier) @imported_name))

(use_declaration
  argument: (use_as_clause
    path: (scoped_identifier) @import_source))
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `mutual_import`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Hub Module (Bidirectional)

### Description
A module with high fan-in (many dependents) AND high fan-out (many dependencies), acting as a nexus that participates in or enables dependency cycles.

### Bad Code (Anti-pattern)
```rust
// --- src/common.rs --- (hub module with high fan-in and fan-out)
use crate::auth::AuthManager;
use crate::billing::PaymentProcessor;
use crate::cache::CacheLayer;
use crate::config::AppConfig;
use crate::database::Pool;
use crate::logging::Logger;
use crate::messaging::EventBus;

// High fan-out (7 use statements) AND high fan-in
// (every module above does `use crate::common::*`)
pub struct AppContext {
    pub auth: AuthManager,
    pub billing: PaymentProcessor,
    pub cache: CacheLayer,
    pub config: AppConfig,
    pub db: Pool,
    pub logger: Logger,
    pub events: EventBus,
}

pub fn init() -> AppContext {
    // initializes everything — tightly couples all modules
    todo!()
}
```

### Good Code (Fix)
```rust
// --- src/main.rs --- (composition at entry point, no hub)
mod auth;
mod billing;
mod config;
mod database;
mod logging;

fn main() {
    let config = config::AppConfig::load();
    let logger = logging::Logger::new(&config);
    let db = database::Pool::connect(&config.database_url);
    let auth = auth::AuthManager::new(&config.auth, &logger);
    let billing = billing::PaymentProcessor::new(&config.stripe_key);

    let server = Server::new(db, auth, billing, logger);
    server.run();
}

// Each module depends only on what it needs via function parameters
// No shared hub module required
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`
- **Detection approach**: Per-file: count `use` declarations with `crate::` prefix to estimate intra-crate fan-out. Cross-file: query imports.parquet to count how many other files in the same crate import from this module (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(use_declaration
  argument: (scoped_identifier) @import_source)

(use_declaration
  argument: (use_as_clause
    path: (scoped_identifier) @import_source))
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
