# God Objects -- JavaScript

## Overview
God objects are classes or modules that accumulate too many responsibilities — too many methods, too many exports, or too many lines — violating the Single Responsibility Principle. In JavaScript, this manifests as oversized classes, massive Express routers, or module files that export dozens of unrelated functions.

## Why It's a Tech Debt Concern
God objects become merge-conflict magnets because every feature touches the same file, slowing down team velocity. They are extremely difficult to unit test in isolation since responsibilities are entangled, leading to brittle integration-style tests. The cognitive load of understanding a 1000+ line class or module discourages refactoring and increases the risk of introducing regressions.

## Applicability
- **Relevance**: high (classes and module-as-object patterns are widespread)
- **Languages covered**: `.js`, `.jsx`
- **Frameworks/libraries**: Express (oversized routers), React (god components), NestJS (oversized controllers/services)

---

## Pattern 1: Oversized Class/Module

### Description
A class with 20+ fields or 30+ methods, or a module file exporting 20+ functions, handling multiple unrelated concerns such as user management, email, validation, and logging all in one place.

### Bad Code (Anti-pattern)
```javascript
class UserManager {
  constructor() {
    this.db = new Database();
    this.mailer = new Mailer();
    this.logger = new Logger();
    this.cache = new Cache();
    this.validator = new Validator();
    this.rateLimiter = new RateLimiter();
    this.metrics = new Metrics();
    this.queue = new Queue();
    this.storage = new Storage();
    this.notifier = new Notifier();
    this.encryption = new Encryption();
    this.audit = new AuditLog();
    this.session = new SessionManager();
    this.permissions = new Permissions();
    this.twoFactor = new TwoFactorAuth();
    this.oauth = new OAuthHandler();
  }

  createUser(data) { /* ... */ }
  updateUser(id, data) { /* ... */ }
  deleteUser(id) { /* ... */ }
  findUser(id) { /* ... */ }
  listUsers(filters) { /* ... */ }
  validateEmail(email) { /* ... */ }
  validatePassword(password) { /* ... */ }
  validateUsername(username) { /* ... */ }
  sendWelcomeEmail(user) { /* ... */ }
  sendPasswordResetEmail(user) { /* ... */ }
  sendVerificationEmail(user) { /* ... */ }
  hashPassword(password) { /* ... */ }
  comparePassword(plain, hashed) { /* ... */ }
  generateToken(user) { /* ... */ }
  verifyToken(token) { /* ... */ }
  logActivity(user, action) { /* ... */ }
  checkPermission(user, resource) { /* ... */ }
  uploadAvatar(user, file) { /* ... */ }
  resizeImage(file, dimensions) { /* ... */ }
  exportUserData(user) { /* ... */ }
  importUsers(csvFile) { /* ... */ }
  generateReport(filters) { /* ... */ }
  cacheUser(user) { /* ... */ }
  invalidateCache(userId) { /* ... */ }
  trackMetric(event, data) { /* ... */ }
  rateLimit(userId, action) { /* ... */ }
  setupTwoFactor(user) { /* ... */ }
  verifyTwoFactor(user, code) { /* ... */ }
  handleOAuthCallback(provider, code) { /* ... */ }
  syncExternalProfile(user, provider) { /* ... */ }
}
```

### Good Code (Fix)
```javascript
class UserRepository {
  constructor(db) { this.db = db; }
  create(data) { /* ... */ }
  update(id, data) { /* ... */ }
  delete(id) { /* ... */ }
  findById(id) { /* ... */ }
  list(filters) { /* ... */ }
}

class UserValidator {
  validateEmail(email) { /* ... */ }
  validatePassword(password) { /* ... */ }
  validateUsername(username) { /* ... */ }
}

class EmailService {
  constructor(mailer) { this.mailer = mailer; }
  sendWelcome(user) { /* ... */ }
  sendPasswordReset(user) { /* ... */ }
  sendVerification(user) { /* ... */ }
}

class AuthService {
  constructor(encryption) { this.encryption = encryption; }
  hashPassword(password) { /* ... */ }
  comparePassword(plain, hashed) { /* ... */ }
  generateToken(user) { /* ... */ }
  verifyToken(token) { /* ... */ }
}

class AvatarService {
  constructor(storage) { this.storage = storage; }
  upload(user, file) { /* ... */ }
  resize(file, dimensions) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `class` (class expression), `export_statement`
- **Detection approach**: Count `method_definition` nodes and `field_definition` / constructor assignments within a `class_body`. Flag when methods exceed 20 or fields exceed 15 or total lines exceed 300. For module-level detection, count exported declarations in a file.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (identifier) @class_name
    body: (class_body) @class_body)

  (class_body
    (method_definition
      name: (property_identifier) @method_name))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single class or Express router that handles HTTP request processing, input validation, database queries, email sending, and business logic all in one place — a clear Single Responsibility Principle violation.

### Bad Code (Anti-pattern)
```javascript
class OrderController {
  async createOrder(req, res) {
    // Validation
    if (!req.body.items || req.body.items.length === 0) {
      return res.status(400).json({ error: 'No items' });
    }
    // Business logic
    const total = req.body.items.reduce((sum, i) => sum + i.price * i.qty, 0);
    const tax = total * 0.08;
    const discount = this.calculateDiscount(req.body.coupon);
    // Database access
    const order = await db.query('INSERT INTO orders ...', [total, tax]);
    for (const item of req.body.items) {
      await db.query('INSERT INTO order_items ...', [order.id, item.id]);
      await db.query('UPDATE inventory SET stock = stock - $1 ...', [item.qty]);
    }
    // Email notification
    await this.sendConfirmationEmail(req.body.email, order);
    // Analytics
    this.trackPurchase(order, req.body.items);
    // Logging
    this.logger.info(`Order ${order.id} created`);
    res.json(order);
  }

  calculateDiscount(coupon) { /* ... */ }
  sendConfirmationEmail(email, order) { /* ... */ }
  trackPurchase(order, items) { /* ... */ }
  getOrder(req, res) { /* ... */ }
  updateOrder(req, res) { /* ... */ }
  cancelOrder(req, res) { /* ... */ }
  refundOrder(req, res) { /* ... */ }
  listOrders(req, res) { /* ... */ }
  exportOrders(req, res) { /* ... */ }
  generateInvoice(req, res) { /* ... */ }
  validateCoupon(code) { /* ... */ }
  calculateShipping(items, address) { /* ... */ }
  notifyWarehouse(order) { /* ... */ }
  updateInventory(items) { /* ... */ }
}
```

### Good Code (Fix)
```javascript
class OrderController {
  constructor(orderService) { this.orderService = orderService; }
  async createOrder(req, res) {
    const result = await this.orderService.create(req.body);
    res.json(result);
  }
  async getOrder(req, res) {
    const order = await this.orderService.findById(req.params.id);
    res.json(order);
  }
}

class OrderService {
  constructor(orderRepo, inventoryService, notificationService, pricingService) {
    this.orderRepo = orderRepo;
    this.inventoryService = inventoryService;
    this.notificationService = notificationService;
    this.pricingService = pricingService;
  }
  async create(data) {
    const total = this.pricingService.calculate(data.items, data.coupon);
    const order = await this.orderRepo.save({ ...data, total });
    await this.inventoryService.deduct(data.items);
    await this.notificationService.orderConfirmation(data.email, order);
    return order;
  }
}

class OrderRepository {
  async save(order) { /* ... */ }
  async findById(id) { /* ... */ }
  async list(filters) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_body`, `method_definition` — heuristic based on method name prefixes
- **Detection approach**: Categorize methods by name prefix/pattern (`get`/`set`/`find`/`list` = accessor, `validate`/`check` = validation, `save`/`update`/`delete`/`insert` = persistence, `send`/`notify`/`email` = communication, `track`/`log` = observability, `calculate`/`compute` = business logic). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (class_body
    (method_definition
      name: (property_identifier) @method_name))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
