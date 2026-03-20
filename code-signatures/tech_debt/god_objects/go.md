# God Objects -- Go

## Overview
God objects in Go manifest as structs with too many fields and methods spanning multiple unrelated concerns. Since Go does not have classes, the struct-plus-methods pattern serves the same role, and oversized structs violate the Single Responsibility Principle just as god classes do in OOP languages.

## Why It's a Tech Debt Concern
Large structs with many fields create complex initialization and make it unclear which methods depend on which fields. Testing becomes painful because mocking requires satisfying many dependencies, and changes to one concern risk breaking unrelated functionality. Merge conflicts increase as multiple developers add methods to the same struct, and the lack of clear boundaries makes it hard for newcomers to understand the code.

## Applicability
- **Relevance**: high (struct + methods is Go's primary abstraction mechanism)
- **Languages covered**: `.go`
- **Frameworks/libraries**: Gin/Echo/Chi (oversized handler structs), gRPC (monolithic service implementations)

---

## Pattern 1: Oversized Struct

### Description
A struct with 15+ fields or 30+ methods defined on it, handling multiple unrelated concerns such as database access, caching, email, logging, and business logic all within one type.

### Bad Code (Anti-pattern)
```go
type AppService struct {
    db          *sql.DB
    redis       *redis.Client
    mailer      *gomail.Dialer
    s3          *s3.Client
    cache       *bigcache.BigCache
    logger      *zap.Logger
    metrics     *prometheus.Registry
    queue       *amqp.Channel
    search      *elastic.Client
    config      *Config
    rateLimiter *limiter.Limiter
    encryption  *Encryption
    sessionStore *sessions.Store
    validator   *validator.Validate
    auditLog    *AuditLog
    featureFlags *FeatureFlags
}

func (s *AppService) CreateUser(ctx context.Context, data CreateUserReq) (*User, error) { /* ... */ }
func (s *AppService) UpdateUser(ctx context.Context, id string, data UpdateUserReq) (*User, error) { /* ... */ }
func (s *AppService) DeleteUser(ctx context.Context, id string) error { /* ... */ }
func (s *AppService) FindUser(ctx context.Context, id string) (*User, error) { /* ... */ }
func (s *AppService) ListUsers(ctx context.Context, filters UserFilters) ([]*User, error) { /* ... */ }
func (s *AppService) SearchUsers(ctx context.Context, query string) ([]*User, error) { /* ... */ }
func (s *AppService) ValidateEmail(email string) error { /* ... */ }
func (s *AppService) ValidatePassword(password string) error { /* ... */ }
func (s *AppService) HashPassword(password string) (string, error) { /* ... */ }
func (s *AppService) VerifyPassword(plain, hashed string) error { /* ... */ }
func (s *AppService) GenerateToken(user *User) (string, error) { /* ... */ }
func (s *AppService) VerifyToken(token string) (*Claims, error) { /* ... */ }
func (s *AppService) SendWelcomeEmail(user *User) error { /* ... */ }
func (s *AppService) SendResetEmail(user *User) error { /* ... */ }
func (s *AppService) SendVerificationEmail(user *User) error { /* ... */ }
func (s *AppService) UploadAvatar(user *User, data []byte) (string, error) { /* ... */ }
func (s *AppService) CacheUser(user *User) error { /* ... */ }
func (s *AppService) InvalidateCache(id string) error { /* ... */ }
func (s *AppService) LogActivity(user *User, action string) error { /* ... */ }
func (s *AppService) CheckPermission(user *User, resource string) (bool, error) { /* ... */ }
func (s *AppService) TrackMetric(event string, data map[string]interface{}) { /* ... */ }
func (s *AppService) RateLimitCheck(userID, action string) error { /* ... */ }
func (s *AppService) ExportData(user *User) ([]byte, error) { /* ... */ }
func (s *AppService) IndexUser(user *User) error { /* ... */ }
func (s *AppService) SetupTwoFactor(user *User) (*TwoFactorSetup, error) { /* ... */ }
func (s *AppService) VerifyTwoFactor(user *User, code string) (bool, error) { /* ... */ }
func (s *AppService) GenerateReport(filters ReportFilters) (*Report, error) { /* ... */ }
func (s *AppService) NotifyAdmin(event string, data interface{}) error { /* ... */ }
func (s *AppService) CleanupSessions() error { /* ... */ }
func (s *AppService) RotateSecrets() error { /* ... */ }
```

### Good Code (Fix)
```go
type UserRepository struct {
    db *sql.DB
}

func (r *UserRepository) Create(ctx context.Context, data CreateUserReq) (*User, error) { /* ... */ }
func (r *UserRepository) Update(ctx context.Context, id string, data UpdateUserReq) (*User, error) { /* ... */ }
func (r *UserRepository) Delete(ctx context.Context, id string) error { /* ... */ }
func (r *UserRepository) FindByID(ctx context.Context, id string) (*User, error) { /* ... */ }
func (r *UserRepository) List(ctx context.Context, filters UserFilters) ([]*User, error) { /* ... */ }

type AuthService struct {
    encryption *Encryption
}

func (a *AuthService) HashPassword(password string) (string, error) { /* ... */ }
func (a *AuthService) VerifyPassword(plain, hashed string) error { /* ... */ }
func (a *AuthService) GenerateToken(user *User) (string, error) { /* ... */ }
func (a *AuthService) VerifyToken(token string) (*Claims, error) { /* ... */ }

type EmailService struct {
    mailer *gomail.Dialer
}

func (e *EmailService) SendWelcome(user *User) error { /* ... */ }
func (e *EmailService) SendReset(user *User) error { /* ... */ }
func (e *EmailService) SendVerification(user *User) error { /* ... */ }

type UserSearchService struct {
    search *elastic.Client
}

func (s *UserSearchService) Search(ctx context.Context, query string) ([]*User, error) { /* ... */ }
func (s *UserSearchService) Index(user *User) error { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_declaration` containing `struct_type` (count `field_declaration` children), `method_declaration` (count methods per receiver type)
- **Detection approach**: Count `field_declaration` nodes in a `struct_type`'s `field_declaration_list`. Collect all `method_declaration` nodes sharing the same receiver type and count them. Flag when fields exceed 15, methods exceed 20, or combined lines exceed 300.
- **S-expression query sketch**:
  ```scheme
  (type_declaration
    (type_spec
      name: (type_identifier) @struct_name
      type: (struct_type
        (field_declaration_list
          (field_declaration) @field))))

  (method_declaration
    receiver: (parameter_list
      (parameter_declaration
        type: (_) @receiver_type))
    name: (field_identifier) @method_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `god_struct`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single struct with methods spanning HTTP handling, input validation, database access, business logic, and notification — a clear SRP violation. In Go this commonly appears as a handler struct that does everything instead of delegating to focused service types.

### Bad Code (Anti-pattern)
```go
type OrderHandler struct {
    db      *sql.DB
    mailer  *gomail.Dialer
    cache   *redis.Client
    metrics *prometheus.Registry
}

// HTTP handling
func (h *OrderHandler) HandleCreateOrder(w http.ResponseWriter, r *http.Request) { /* ... */ }
func (h *OrderHandler) HandleGetOrder(w http.ResponseWriter, r *http.Request) { /* ... */ }
func (h *OrderHandler) HandleListOrders(w http.ResponseWriter, r *http.Request) { /* ... */ }

// Validation
func (h *OrderHandler) validateItems(items []OrderItem) error { /* ... */ }
func (h *OrderHandler) validateCoupon(code string) (*Discount, error) { /* ... */ }

// Business logic
func (h *OrderHandler) calculateTotal(items []OrderItem, discount *Discount) float64 { /* ... */ }
func (h *OrderHandler) calculateTax(subtotal float64) float64 { /* ... */ }
func (h *OrderHandler) calculateShipping(items []OrderItem, addr *Address) float64 { /* ... */ }

// Database access
func (h *OrderHandler) saveOrder(ctx context.Context, order *Order) error { /* ... */ }
func (h *OrderHandler) saveOrderItems(ctx context.Context, orderID string, items []OrderItem) error { /* ... */ }
func (h *OrderHandler) updateInventory(ctx context.Context, items []OrderItem) error { /* ... */ }
func (h *OrderHandler) findOrderByID(ctx context.Context, id string) (*Order, error) { /* ... */ }

// Notifications
func (h *OrderHandler) sendConfirmation(email string, order *Order) error { /* ... */ }
func (h *OrderHandler) notifyWarehouse(order *Order) error { /* ... */ }

// Observability
func (h *OrderHandler) trackPurchase(order *Order) { /* ... */ }
func (h *OrderHandler) logOrderEvent(orderID, event string) { /* ... */ }
```

### Good Code (Fix)
```go
type OrderHandler struct {
    orderService OrderService
}

func (h *OrderHandler) HandleCreateOrder(w http.ResponseWriter, r *http.Request) {
    var req CreateOrderReq
    json.NewDecoder(r.Body).Decode(&req)
    order, err := h.orderService.Create(r.Context(), req)
    if err != nil {
        http.Error(w, err.Error(), http.StatusBadRequest)
        return
    }
    json.NewEncoder(w).Encode(order)
}

type OrderService struct {
    repo          OrderRepository
    pricing       PricingService
    notifications NotificationService
    inventory     InventoryService
}

func (s *OrderService) Create(ctx context.Context, req CreateOrderReq) (*Order, error) {
    total := s.pricing.Calculate(req.Items, req.Coupon)
    order, err := s.repo.Save(ctx, req.Items, total)
    if err != nil { return nil, err }
    s.inventory.Deduct(ctx, req.Items)
    s.notifications.OrderConfirmed(req.Email, order)
    return order, nil
}

type OrderRepository struct {
    db *sql.DB
}

func (r *OrderRepository) Save(ctx context.Context, items []OrderItem, total float64) (*Order, error) { /* ... */ }
func (r *OrderRepository) FindByID(ctx context.Context, id string) (*Order, error) { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `field_identifier` — heuristic based on method name patterns
- **Detection approach**: Group `method_declaration` nodes by receiver type. Categorize methods by name prefix/pattern (`Handle`/`Get`/`List` = HTTP/accessor, `validate`/`check` = validation, `save`/`update`/`delete`/`find` = persistence, `send`/`notify` = communication, `track`/`log` = observability, `calculate`/`compute` = business logic). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    receiver: (parameter_list
      (parameter_declaration
        type: (_) @receiver_type))
    name: (field_identifier) @method_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `god_struct`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
