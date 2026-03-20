# God Objects -- Python

## Overview
God objects are classes or modules that accumulate too many responsibilities — too many methods, too many attributes, or too many lines — violating the Single Responsibility Principle. In Python, this also includes god functions: single functions doing too much with excessive branches, and module-level files that grow beyond 500 lines with dozens of unrelated functions.

## Why It's a Tech Debt Concern
Oversized classes and modules become merge-conflict magnets as multiple developers need to modify the same file for unrelated features. Testing becomes prohibitively difficult because tightly coupled responsibilities require complex setup and mocking. The cognitive load of reading a 1000+ line class discourages developers from fully understanding the code before making changes, leading to subtle bugs.

## Applicability
- **Relevance**: high (Python's class system and module-as-namespace pattern make god objects common)
- **Languages covered**: `.py`, `.pyi`
- **Frameworks/libraries**: Django (fat models, fat views), Flask (oversized route modules), FastAPI (oversized routers)

---

## Pattern 1: Oversized Class/Module

### Description
A class with 30+ methods or 15+ attributes, or a module-level file with 500+ lines and 20+ top-level functions, handling multiple unrelated concerns such as data access, business logic, serialization, and notification in one place.

### Bad Code (Anti-pattern)
```python
class UserService:
    def __init__(self):
        self.db = Database()
        self.mailer = Mailer()
        self.cache = Cache()
        self.logger = Logger()
        self.validator = Validator()
        self.storage = Storage()
        self.queue = Queue()
        self.metrics = Metrics()
        self.encryption = Encryption()
        self.session_store = SessionStore()
        self.rate_limiter = RateLimiter()
        self.audit_log = AuditLog()
        self.search_index = SearchIndex()
        self.feature_flags = FeatureFlags()
        self.notification_hub = NotificationHub()
        self.export_engine = ExportEngine()

    def create_user(self, data): ...
    def update_user(self, user_id, data): ...
    def delete_user(self, user_id): ...
    def find_user(self, user_id): ...
    def list_users(self, filters): ...
    def search_users(self, query): ...
    def validate_email(self, email): ...
    def validate_password(self, password): ...
    def validate_username(self, username): ...
    def hash_password(self, password): ...
    def verify_password(self, plain, hashed): ...
    def generate_token(self, user): ...
    def verify_token(self, token): ...
    def refresh_token(self, token): ...
    def send_welcome_email(self, user): ...
    def send_reset_email(self, user): ...
    def send_verification_email(self, user): ...
    def upload_avatar(self, user, file): ...
    def resize_avatar(self, file, size): ...
    def cache_user(self, user): ...
    def invalidate_cache(self, user_id): ...
    def log_activity(self, user, action): ...
    def check_permission(self, user, resource): ...
    def export_user_data(self, user): ...
    def import_users(self, csv_file): ...
    def generate_report(self, filters): ...
    def track_metric(self, event, data): ...
    def rate_limit_check(self, user_id, action): ...
    def index_user(self, user): ...
    def reindex_all(self): ...
    def setup_two_factor(self, user): ...
    def verify_two_factor(self, user, code): ...
```

### Good Code (Fix)
```python
class UserRepository:
    def __init__(self, db):
        self.db = db

    def create(self, data): ...
    def update(self, user_id, data): ...
    def delete(self, user_id): ...
    def find_by_id(self, user_id): ...
    def list(self, filters): ...


class UserValidator:
    def validate_email(self, email): ...
    def validate_password(self, password): ...
    def validate_username(self, username): ...


class AuthService:
    def __init__(self, encryption):
        self.encryption = encryption

    def hash_password(self, password): ...
    def verify_password(self, plain, hashed): ...
    def generate_token(self, user): ...
    def verify_token(self, token): ...


class EmailService:
    def __init__(self, mailer):
        self.mailer = mailer

    def send_welcome(self, user): ...
    def send_reset(self, user): ...
    def send_verification(self, user): ...


class UserSearchService:
    def __init__(self, search_index):
        self.search_index = search_index

    def search(self, query): ...
    def index_user(self, user): ...
    def reindex_all(self): ...
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_definition`, `module` (top-level file)
- **Detection approach**: Count `function_definition` nodes within a `class_definition` body (methods) and `self.xxx` assignments in `__init__` (fields/attributes). Flag when methods exceed 20 or attributes exceed 15 or total lines exceed 300. For module-level, count top-level `function_definition` nodes — flag when exceeding 20 in a single file.
- **S-expression query sketch**:
  ```scheme
  (class_definition
    name: (identifier) @class_name
    body: (block
      (function_definition
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_functions`, `god_class`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single class handling both data access and business logic, or both HTTP handling and database queries — a clear SRP violation. In Python this commonly appears as Django fat models or Flask view functions that do everything.

### Bad Code (Anti-pattern)
```python
class OrderManager:
    def create_order(self, request_data):
        # Input validation
        if not request_data.get("items"):
            raise ValidationError("No items provided")
        for item in request_data["items"]:
            if item["quantity"] <= 0:
                raise ValidationError("Invalid quantity")

        # Business logic - pricing
        subtotal = sum(i["price"] * i["quantity"] for i in request_data["items"])
        tax = subtotal * Decimal("0.08")
        discount = self._calculate_discount(request_data.get("coupon"))
        total = subtotal + tax - discount

        # Database access
        order = Order.objects.create(total=total, tax=tax, status="pending")
        for item in request_data["items"]:
            OrderItem.objects.create(order=order, product_id=item["id"], qty=item["quantity"])
            Product.objects.filter(id=item["id"]).update(stock=F("stock") - item["quantity"])

        # Email notification
        self._send_confirmation_email(request_data["email"], order)

        # Analytics tracking
        self._track_purchase(order)

        # Logging
        logger.info(f"Order {order.id} created for {request_data['email']}")

        return order

    def _calculate_discount(self, coupon_code): ...
    def _send_confirmation_email(self, email, order): ...
    def _track_purchase(self, order): ...
    def get_order(self, order_id): ...
    def cancel_order(self, order_id): ...
    def refund_order(self, order_id): ...
    def list_orders(self, filters): ...
    def export_orders(self, format): ...
    def generate_invoice(self, order_id): ...
    def validate_coupon(self, code): ...
    def calculate_shipping(self, items, address): ...
    def notify_warehouse(self, order): ...
    def update_inventory(self, items): ...
    def serialize_order(self, order): ...
```

### Good Code (Fix)
```python
class OrderController:
    def __init__(self, order_service: OrderService):
        self.order_service = order_service

    def create_order(self, request_data):
        return self.order_service.create(request_data)

    def get_order(self, order_id):
        return self.order_service.find_by_id(order_id)


class OrderService:
    def __init__(self, repo: OrderRepository, pricing: PricingService,
                 notifications: NotificationService, inventory: InventoryService):
        self.repo = repo
        self.pricing = pricing
        self.notifications = notifications
        self.inventory = inventory

    def create(self, data):
        total = self.pricing.calculate(data["items"], data.get("coupon"))
        order = self.repo.save(data["items"], total)
        self.inventory.deduct(data["items"])
        self.notifications.order_confirmed(data["email"], order)
        return order


class OrderRepository:
    def save(self, items, total): ...
    def find_by_id(self, order_id): ...
    def list(self, filters): ...
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_definition`, `function_definition` — heuristic based on method name prefixes
- **Detection approach**: Categorize methods by name prefix/pattern (`get`/`find`/`list` = accessor, `validate`/`check` = validation, `save`/`create`/`update`/`delete` = persistence, `send`/`notify`/`email` = communication, `track`/`log` = observability, `calculate`/`compute` = business logic). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (class_definition
    body: (block
      (function_definition
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_functions`, `god_class`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
