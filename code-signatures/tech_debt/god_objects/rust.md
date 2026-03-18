# God Objects -- Rust

## Overview
God objects in Rust manifest as structs with too many fields, oversized `impl` blocks with too many methods, or multiple `impl` blocks for the same type that collectively span hundreds of lines. These types violate the Single Responsibility Principle by accumulating unrelated functionality behind a single struct.

## Why It's a Tech Debt Concern
Large structs with many fields create complex initialization and increase the surface area for bugs when modifying any one concern. Oversized `impl` blocks are difficult to navigate and test, and they make it harder to reason about which methods interact with which fields. Merge conflicts become frequent when multiple developers add methods to the same `impl` block, and refactoring becomes risky because responsibilities are entangled.

## Applicability
- **Relevance**: high (structs with `impl` blocks serve the same role as classes in OOP)
- **Languages covered**: `.rs`
- **Frameworks/libraries**: Actix-web (oversized handler structs), Axum (state structs), Bevy (god components/systems)

---

## Pattern 1: Oversized Struct/Impl

### Description
A struct with 15+ fields or an `impl` block with 20+ methods, or multiple `impl` blocks for the same type that collectively contain 30+ methods. The struct holds dependencies for many unrelated concerns.

### Bad Code (Anti-pattern)
```rust
pub struct AppService {
    db_pool: PgPool,
    redis: RedisClient,
    mailer: SmtpTransport,
    storage: S3Client,
    cache: Cache,
    logger: Logger,
    metrics: MetricsClient,
    queue: QueueClient,
    search: SearchClient,
    config: AppConfig,
    rate_limiter: RateLimiter,
    encryption: EncryptionService,
    session_store: SessionStore,
    validator: Validator,
    audit_log: AuditLog,
    feature_flags: FeatureFlags,
}

impl AppService {
    pub fn create_user(&self, data: CreateUserRequest) -> Result<User> { /* ... */ }
    pub fn update_user(&self, id: Uuid, data: UpdateUserRequest) -> Result<User> { /* ... */ }
    pub fn delete_user(&self, id: Uuid) -> Result<()> { /* ... */ }
    pub fn find_user(&self, id: Uuid) -> Result<User> { /* ... */ }
    pub fn list_users(&self, filters: UserFilters) -> Result<Vec<User>> { /* ... */ }
    pub fn validate_email(&self, email: &str) -> Result<bool> { /* ... */ }
    pub fn validate_password(&self, password: &str) -> Result<bool> { /* ... */ }
    pub fn hash_password(&self, password: &str) -> Result<String> { /* ... */ }
    pub fn verify_password(&self, plain: &str, hashed: &str) -> Result<bool> { /* ... */ }
    pub fn generate_token(&self, user: &User) -> Result<String> { /* ... */ }
    pub fn verify_token(&self, token: &str) -> Result<Claims> { /* ... */ }
    pub fn send_welcome_email(&self, user: &User) -> Result<()> { /* ... */ }
    pub fn send_reset_email(&self, user: &User) -> Result<()> { /* ... */ }
    pub fn upload_avatar(&self, user: &User, data: &[u8]) -> Result<String> { /* ... */ }
    pub fn cache_user(&self, user: &User) -> Result<()> { /* ... */ }
    pub fn invalidate_cache(&self, id: Uuid) -> Result<()> { /* ... */ }
    pub fn log_activity(&self, user: &User, action: &str) -> Result<()> { /* ... */ }
    pub fn check_permission(&self, user: &User, resource: &str) -> Result<bool> { /* ... */ }
    pub fn track_metric(&self, event: &str, data: serde_json::Value) -> Result<()> { /* ... */ }
    pub fn rate_limit_check(&self, user_id: Uuid, action: &str) -> Result<bool> { /* ... */ }
    pub fn export_data(&self, user: &User) -> Result<Vec<u8>> { /* ... */ }
    pub fn search_users(&self, query: &str) -> Result<Vec<User>> { /* ... */ }
    pub fn index_user(&self, user: &User) -> Result<()> { /* ... */ }
    pub fn setup_two_factor(&self, user: &User) -> Result<TwoFactorSetup> { /* ... */ }
    pub fn verify_two_factor(&self, user: &User, code: &str) -> Result<bool> { /* ... */ }
}
```

### Good Code (Fix)
```rust
pub struct UserRepository {
    db_pool: PgPool,
}

impl UserRepository {
    pub fn create(&self, data: CreateUserRequest) -> Result<User> { /* ... */ }
    pub fn update(&self, id: Uuid, data: UpdateUserRequest) -> Result<User> { /* ... */ }
    pub fn delete(&self, id: Uuid) -> Result<()> { /* ... */ }
    pub fn find_by_id(&self, id: Uuid) -> Result<User> { /* ... */ }
    pub fn list(&self, filters: UserFilters) -> Result<Vec<User>> { /* ... */ }
}

pub struct AuthService {
    encryption: EncryptionService,
}

impl AuthService {
    pub fn hash_password(&self, password: &str) -> Result<String> { /* ... */ }
    pub fn verify_password(&self, plain: &str, hashed: &str) -> Result<bool> { /* ... */ }
    pub fn generate_token(&self, user: &User) -> Result<String> { /* ... */ }
    pub fn verify_token(&self, token: &str) -> Result<Claims> { /* ... */ }
}

pub struct EmailService {
    mailer: SmtpTransport,
}

impl EmailService {
    pub fn send_welcome(&self, user: &User) -> Result<()> { /* ... */ }
    pub fn send_reset(&self, user: &User) -> Result<()> { /* ... */ }
}

pub struct UserSearchService {
    search: SearchClient,
}

impl UserSearchService {
    pub fn search(&self, query: &str) -> Result<Vec<User>> { /* ... */ }
    pub fn index(&self, user: &User) -> Result<()> { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_item` (count `field_declaration` children), `impl_item` (count `function_item` children in `declaration_list`)
- **Detection approach**: Count `field_declaration` nodes in a struct's `field_declaration_list`. Count `function_item` nodes inside `impl_item` `declaration_list`. Flag when fields exceed 15, methods exceed 20, or total `impl` lines exceed 300. Also detect multiple `impl` blocks for the same type and sum their methods.
- **S-expression query sketch**:
  ```scheme
  (struct_item
    name: (type_identifier) @struct_name
    body: (field_declaration_list
      (field_declaration) @field))

  (impl_item
    type: (type_identifier) @impl_type
    body: (declaration_list
      (function_item
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single struct and its `impl` block handling both data access and business logic, or both HTTP handling and database queries. In Rust, this often appears as an Actix/Axum handler struct that does request parsing, validation, database operations, and notification in one `impl`.

### Bad Code (Anti-pattern)
```rust
pub struct OrderHandler {
    db: PgPool,
    mailer: SmtpTransport,
    cache: RedisClient,
    metrics: MetricsClient,
}

impl OrderHandler {
    // HTTP handling
    pub async fn create_order(&self, req: HttpRequest, body: Json<CreateOrderReq>) -> HttpResponse { /* ... */ }
    pub async fn get_order(&self, req: HttpRequest) -> HttpResponse { /* ... */ }
    pub async fn list_orders(&self, req: HttpRequest) -> HttpResponse { /* ... */ }

    // Validation
    fn validate_items(&self, items: &[OrderItem]) -> Result<()> { /* ... */ }
    fn validate_coupon(&self, code: &str) -> Result<Discount> { /* ... */ }

    // Business logic
    fn calculate_total(&self, items: &[OrderItem], discount: &Discount) -> Decimal { /* ... */ }
    fn calculate_tax(&self, subtotal: Decimal) -> Decimal { /* ... */ }
    fn calculate_shipping(&self, items: &[OrderItem], address: &Address) -> Decimal { /* ... */ }

    // Database access
    fn save_order(&self, order: &Order) -> Result<Uuid> { /* ... */ }
    fn save_order_items(&self, order_id: Uuid, items: &[OrderItem]) -> Result<()> { /* ... */ }
    fn update_inventory(&self, items: &[OrderItem]) -> Result<()> { /* ... */ }
    fn find_order_by_id(&self, id: Uuid) -> Result<Order> { /* ... */ }

    // Notifications
    fn send_confirmation(&self, email: &str, order: &Order) -> Result<()> { /* ... */ }
    fn notify_warehouse(&self, order: &Order) -> Result<()> { /* ... */ }

    // Observability
    fn track_purchase(&self, order: &Order) { /* ... */ }
    fn log_order_event(&self, order_id: Uuid, event: &str) { /* ... */ }
}
```

### Good Code (Fix)
```rust
pub struct OrderHandler {
    order_service: Arc<OrderService>,
}

impl OrderHandler {
    pub async fn create_order(&self, body: Json<CreateOrderReq>) -> HttpResponse {
        match self.order_service.create(body.into_inner()).await {
            Ok(order) => HttpResponse::Ok().json(order),
            Err(e) => HttpResponse::BadRequest().json(e),
        }
    }
}

pub struct OrderService {
    repo: OrderRepository,
    pricing: PricingService,
    notifications: NotificationService,
    inventory: InventoryService,
}

impl OrderService {
    pub async fn create(&self, data: CreateOrderReq) -> Result<Order> {
        let total = self.pricing.calculate(&data.items, data.coupon.as_deref())?;
        let order = self.repo.save(&data.items, total).await?;
        self.inventory.deduct(&data.items).await?;
        self.notifications.order_confirmed(&data.email, &order).await?;
        Ok(order)
    }
}

pub struct OrderRepository {
    db: PgPool,
}

impl OrderRepository {
    pub async fn save(&self, items: &[OrderItem], total: Decimal) -> Result<Order> { /* ... */ }
    pub async fn find_by_id(&self, id: Uuid) -> Result<Order> { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `impl_item`, `function_item` — heuristic based on method name prefixes
- **Detection approach**: Categorize methods by name prefix/pattern (`get`/`find`/`list` = accessor, `validate`/`check` = validation, `save`/`update`/`delete` = persistence, `send`/`notify` = communication, `track`/`log` = observability, `calculate`/`compute` = business logic). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (impl_item
    type: (type_identifier) @impl_type
    body: (declaration_list
      (function_item
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
