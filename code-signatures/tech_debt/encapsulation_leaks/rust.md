# Encapsulation Leaks -- Rust

## Overview
Encapsulation leaks in Rust occur when public struct fields expose internal representation details to consumers, or when public APIs use concrete types instead of trait bounds, coupling callers to specific implementations. Both patterns make refactoring expensive because any change to the internal layout or chosen implementation type becomes a breaking API change.

## Why It's a Tech Debt Concern
Public fields allow any consumer to construct, destructure, or directly modify a struct's internals, preventing the owning crate from enforcing invariants or evolving its representation. Adding a field to a struct with all-public fields is a breaking change in Rust (pattern exhaustiveness). Missing trait abstractions force all downstream code to depend on concrete types, making it impossible to swap implementations for testing, optimization, or feature variation without modifying every call site.

## Applicability
- **Relevance**: high (pub fields and concrete-type APIs are common in early Rust codebases)
- **Languages covered**: `.rs`
- **Frameworks/libraries**: Actix-web/Axum (state structs), serde (DTO structs), any library crate exposing types

---

## Pattern 1: Public Field Leakage

### Description
A struct marks fields as `pub` when they represent internal state that should be accessed through methods. This allows consumers to bypass validation, break invariants, and depend on the exact field layout. Adding, removing, or renaming a public field is a semver-breaking change.

### Bad Code (Anti-pattern)
```rust
pub struct ConnectionPool {
    pub connections: Vec<Connection>,
    pub max_size: usize,
    pub timeout_ms: u64,
    pub retry_count: u32,
    pub active_count: usize,
    pub metrics: PoolMetrics,
}

impl ConnectionPool {
    pub fn new(max_size: usize) -> Self {
        Self {
            connections: Vec::with_capacity(max_size),
            max_size,
            timeout_ms: 5000,
            retry_count: 3,
            active_count: 0,
            metrics: PoolMetrics::default(),
        }
    }

    pub fn acquire(&mut self) -> Result<&Connection, PoolError> {
        if self.active_count >= self.max_size {
            return Err(PoolError::Exhausted);
        }
        self.active_count += 1;
        // ...
    }
}

// consumer can break invariants
let mut pool = ConnectionPool::new(10);
pool.active_count = 0;  // reset counter, bypassing release logic
pool.max_size = 1000;   // change capacity without resizing vec
pool.connections.clear(); // destroy connections without cleanup
```

### Good Code (Fix)
```rust
pub struct ConnectionPool {
    connections: Vec<Connection>,
    max_size: usize,
    timeout_ms: u64,
    retry_count: u32,
    active_count: usize,
    metrics: PoolMetrics,
}

impl ConnectionPool {
    pub fn new(max_size: usize) -> Self {
        Self {
            connections: Vec::with_capacity(max_size),
            max_size,
            timeout_ms: 5000,
            retry_count: 3,
            active_count: 0,
            metrics: PoolMetrics::default(),
        }
    }

    pub fn acquire(&mut self) -> Result<&Connection, PoolError> {
        if self.active_count >= self.max_size {
            return Err(PoolError::Exhausted);
        }
        self.active_count += 1;
        // ...
    }

    pub fn release(&mut self, conn: Connection) {
        self.active_count = self.active_count.saturating_sub(1);
        // cleanup logic
    }

    pub fn active_count(&self) -> usize {
        self.active_count
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    pub fn set_timeout(&mut self, ms: u64) {
        self.timeout_ms = ms;
    }

    pub fn metrics(&self) -> &PoolMetrics {
        &self.metrics
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_item` with `field_declaration_list`, `field_declaration` with `visibility_modifier`
- **Detection approach**: Find `struct_item` nodes that are `pub` (have a `visibility_modifier`). Within their `field_declaration_list`, count `field_declaration` children that also have a `visibility_modifier` (i.e., `pub` fields). Flag structs where more than half the fields are `pub` and the struct has 3+ fields, excluding obvious DTO/data-transfer structs (those deriving `Serialize`/`Deserialize` only).
- **S-expression query sketch**:
  ```scheme
  (struct_item
    (visibility_modifier) @struct_vis
    name: (type_identifier) @struct_name
    body: (field_declaration_list
      (field_declaration
        (visibility_modifier) @field_vis
        name: (field_identifier) @field_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `pub_field_leakage`
- **Pattern name**: `public_struct_fields`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Missing Trait Abstraction

### Description
Public function signatures accept or return concrete types (specific structs) when a trait bound would decouple the API from the implementation. This forces callers to construct or depend on a specific type, preventing substitution for testing, alternative implementations, or future refactoring.

### Bad Code (Anti-pattern)
```rust
pub struct PostgresUserRepo {
    pool: PgPool,
}

impl PostgresUserRepo {
    pub fn find_by_id(&self, id: Uuid) -> Result<User> { /* ... */ }
    pub fn save(&self, user: &User) -> Result<()> { /* ... */ }
    pub fn delete(&self, id: Uuid) -> Result<()> { /* ... */ }
}

pub struct UserService {
    repo: PostgresUserRepo,      // concrete type
    mailer: SmtpMailer,          // concrete type
    cache: RedisCache,           // concrete type
}

impl UserService {
    pub fn new(repo: PostgresUserRepo, mailer: SmtpMailer, cache: RedisCache) -> Self {
        Self { repo, mailer, cache }
    }

    pub fn create_user(&self, data: CreateUserRequest) -> Result<User> {
        let user = self.repo.save(&data.into())?;
        self.mailer.send_welcome(&user)?;
        self.cache.set(&user.id.to_string(), &user)?;
        Ok(user)
    }
}

// Tests must construct real PostgresUserRepo, SmtpMailer, RedisCache
// Cannot substitute mocks or in-memory implementations
```

### Good Code (Fix)
```rust
pub trait UserRepository {
    fn find_by_id(&self, id: Uuid) -> Result<User>;
    fn save(&self, user: &User) -> Result<()>;
    fn delete(&self, id: Uuid) -> Result<()>;
}

pub trait Mailer {
    fn send_welcome(&self, user: &User) -> Result<()>;
}

pub trait Cache {
    fn set(&self, key: &str, value: &impl Serialize) -> Result<()>;
    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>;
}

pub struct UserService<R: UserRepository, M: Mailer, C: Cache> {
    repo: R,
    mailer: M,
    cache: C,
}

impl<R: UserRepository, M: Mailer, C: Cache> UserService<R, M, C> {
    pub fn new(repo: R, mailer: M, cache: C) -> Self {
        Self { repo, mailer, cache }
    }

    pub fn create_user(&self, data: CreateUserRequest) -> Result<User> {
        let user = User::from(data);
        self.repo.save(&user)?;
        self.mailer.send_welcome(&user)?;
        self.cache.set(&user.id.to_string(), &user)?;
        Ok(user)
    }
}

// Tests can use mock implementations
struct MockRepo { users: Vec<User> }
impl UserRepository for MockRepo { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_item` field types, `function_item` parameter types and return types
- **Detection approach**: In `pub` function signatures and `pub` struct fields, look for type references that are concrete struct names (not generic parameters, not trait objects `dyn Trait`, not `impl Trait`). Heuristic: flag `pub` functions where 3+ parameters or return type reference concrete structs that are defined in the same crate and have corresponding trait-like method sets. Naming patterns like `Postgres*`, `Redis*`, `Smtp*`, `Http*` suggest concrete implementations.
- **S-expression query sketch**:
  ```scheme
  (function_item
    (visibility_modifier) @vis
    name: (identifier) @fn_name
    parameters: (parameters
      (parameter
        type: (type_identifier) @param_type)))

  (struct_item
    (visibility_modifier) @vis
    name: (type_identifier) @struct_name
    body: (field_declaration_list
      (field_declaration
        type: (type_identifier) @field_type)))
  ```

### Pipeline Mapping
- **Pipeline name**: `missing_trait_abstraction`
- **Pattern name**: `concrete_type_in_public_api`
- **Severity**: info
- **Confidence**: low
