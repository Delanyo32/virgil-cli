# Coupling -- Rust

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file/module importing from many different crates or modules (high fan-in), indicating it depends on too many parts of the system. In Rust, excessive `use` declarations increase compile times and widen the blast radius of breaking changes. Glob imports (`use module::*`) are particularly problematic as they hide the true scope of coupling.

### Bad Code (Anti-pattern)
```rust
// src/controllers/order_controller.rs
use crate::auth::auth_service::{authenticate, authorize};
use crate::auth::token_validator::validate_token;
use crate::users::user_service::get_user_by_id;
use crate::users::preferences_service::get_user_preferences;
use crate::orders::order_service::{create_order, update_order};
use crate::orders::order_validator::validate_order;
use crate::billing::tax_service::calculate_tax;
use crate::billing::payment_gateway::process_payment;
use crate::billing::discount_engine::apply_discount;
use crate::notifications::email_service::send_confirmation_email;
use crate::notifications::push_service::send_push_notification;
use crate::logging::event_logger::log_event;
use crate::analytics::tracker::track_analytics;
use crate::cache::cache_manager::{cache_result, invalidate_cache};
use crate::queue::job_queue::enqueue_job;
use crate::utils::formatters::format_currency;
use crate::database::connection::DbPool;
```

### Good Code (Fix)
```rust
// src/controllers/order_controller.rs
use crate::auth::auth_service::authenticate;
use crate::orders::order_service::OrderService;
use crate::logging::event_logger::log_event;

pub struct OrderController {
    order_service: OrderService,
}

impl OrderController {
    pub fn new(order_service: OrderService) -> Self {
        Self { order_service }
    }

    pub fn create_order(&self, request: &Request) -> Result<Order, Error> {
        let user = authenticate(request)?;
        let order = self.order_service.create(&user, &request.data)?;
        log_event("order_created", &[("order_id", &order.id.to_string())]);
        Ok(order)
    }
}

// src/orders/order_service.rs — encapsulates billing, notifications
use crate::billing::billing_service::BillingService;
use crate::notifications::notification_service::NotificationService;
use crate::orders::order_validator::validate_order;

pub struct OrderService {
    billing: BillingService,
    notifications: NotificationService,
}

impl OrderService {
    pub fn create(&self, user: &User, data: &OrderData) -> Result<Order, Error> {
        validate_order(data)?;
        let total = self.billing.process_order(data)?;
        self.notifications.send_order_confirmation(user, total)?;
        Ok(Order { data: data.clone(), total })
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`
- **Detection approach**: Count unique crate/module sources per file. Extract the path from each `use_declaration` and take the first path segment (crate name or `crate`/`self`/`super`). For `crate::` paths, take the second segment as the module. Flag files exceeding threshold (e.g., 15+ unique module imports). Distinguish between `crate::` (internal) and external crate imports.
- **S-expression query sketch**:
```scheme
;; use crate::module::item
(use_declaration
  argument: (scoped_identifier) @use_path)

;; use crate::module::{item1, item2}
(use_declaration
  argument: (use_as_clause) @use_alias)

;; use crate::module::*
(use_declaration
  argument: (use_wildcard) @use_glob)

;; use declarations with use_list (grouped imports)
(use_declaration
  argument: (scoped_use_list) @use_group)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more modules that import each other (directly or transitively), creating a dependency cycle. Rust's module system prevents circular crate dependencies at compile time, but intra-crate circular `mod`/`use` dependencies are possible and create tightly coupled modules that are difficult to refactor or extract into separate crates.

### Bad Code (Anti-pattern)
```rust
// src/models/user.rs
use crate::models::order::Order;  // user depends on order

pub struct User {
    pub name: String,
    pub orders: Vec<Order>,
}

impl User {
    pub fn add_order(&mut self, order: Order) {
        self.orders.push(order);
    }

    pub fn total_spent(&self) -> f64 {
        self.orders.iter().map(|o| o.amount).sum()
    }
}

// src/models/order.rs
use crate::models::user::User;  // order depends on user — circular

pub struct Order {
    pub amount: f64,
    pub owner: User,
}

impl Order {
    pub fn new(user: User, amount: f64) -> Self {
        Self { amount, owner: user }
    }
}
```

### Good Code (Fix)
```rust
// src/models/types.rs — shared trait, breaks the cycle
pub trait HasId {
    fn id(&self) -> &str;
}

pub trait Purchasable {
    fn amount(&self) -> f64;
}

// src/models/user.rs
use crate::models::types::Purchasable;

pub struct User {
    pub id: String,
    pub name: String,
}

impl User {
    pub fn total_spent(&self, orders: &[impl Purchasable]) -> f64 {
        orders.iter().map(|o| o.amount()).sum()
    }
}

// src/models/order.rs — no dependency on user
use crate::models::types::Purchasable;

pub struct Order {
    pub amount: f64,
    pub owner_id: String,
}

impl Purchasable for Order {
    fn amount(&self) -> f64 {
        self.amount
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`, `mod_item`
- **Detection approach**: Build a directed graph of file-to-file imports by resolving `crate::`, `self::`, and `super::` paths to file paths. Include `mod` declarations to understand module hierarchy. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Inter-crate cycles are caught by `cargo` itself, so focus on intra-crate module cycles.
- **S-expression query sketch**:
```scheme
;; Collect all use paths to build dependency graph
(use_declaration
  argument: (scoped_identifier
    path: (scoped_identifier) @use_base
    name: (identifier) @use_name))

;; Module declarations establish module tree
(mod_item
  name: (identifier) @mod_name)

;; use with crate/self/super prefix
(use_declaration
  argument: (scoped_identifier
    path: (identifier) @root_path
    (#any-of? @root_path "crate" "self" "super")))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
