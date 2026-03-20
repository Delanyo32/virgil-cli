# God Objects -- PHP

## Overview
God objects are classes that accumulate too many responsibilities — too many properties, too many methods, or too many lines — violating the Single Responsibility Principle. In PHP, this commonly appears as oversized Laravel controllers that handle all CRUD operations plus business logic, monolithic service classes, or "Manager" classes that become dumping grounds for unrelated functionality.

## Why It's a Tech Debt Concern
God classes in PHP become bottlenecks for team development since every feature modification requires touching the same file, causing frequent merge conflicts. Unit testing becomes impractical because the class has too many dependencies to mock, leading to either no tests or fragile integration tests. The cognitive load of reading a 1000+ line class discourages developers from understanding the full impact of changes, increasing the risk of breaking unrelated functionality.

## Applicability
- **Relevance**: high (PHP's class-based OOP and framework conventions make god classes common)
- **Languages covered**: `.php`
- **Frameworks/libraries**: Laravel (fat controllers, fat models), Symfony (oversized services/controllers), WordPress (monolithic plugin classes)

---

## Pattern 1: Oversized Class

### Description
A class with 30+ methods, 15+ properties, or exceeding 500 lines. The class handles multiple unrelated concerns such as user management, authentication, email, file storage, and reporting all within one type.

### Bad Code (Anti-pattern)
```php
class UserService
{
    private $db;
    private $mailer;
    private $cache;
    private $storage;
    private $logger;
    private $validator;
    private $queue;
    private $search;
    private $metrics;
    private $rateLimiter;
    private $encryption;
    private $sessionStore;
    private $auditLog;
    private $featureFlags;
    private $notifier;
    private $config;

    public function createUser(array $data): User { /* ... */ }
    public function updateUser(string $id, array $data): User { /* ... */ }
    public function deleteUser(string $id): void { /* ... */ }
    public function findUser(string $id): ?User { /* ... */ }
    public function listUsers(array $filters): array { /* ... */ }
    public function searchUsers(string $query): array { /* ... */ }
    public function validateEmail(string $email): bool { /* ... */ }
    public function validatePassword(string $password): bool { /* ... */ }
    public function validateUsername(string $username): bool { /* ... */ }
    public function hashPassword(string $password): string { /* ... */ }
    public function verifyPassword(string $plain, string $hashed): bool { /* ... */ }
    public function generateToken(User $user): string { /* ... */ }
    public function verifyToken(string $token): array { /* ... */ }
    public function refreshToken(string $token): string { /* ... */ }
    public function sendWelcomeEmail(User $user): void { /* ... */ }
    public function sendResetEmail(User $user): void { /* ... */ }
    public function sendVerificationEmail(User $user): void { /* ... */ }
    public function uploadAvatar(User $user, $file): string { /* ... */ }
    public function resizeAvatar($file, int $size): string { /* ... */ }
    public function cacheUser(User $user): void { /* ... */ }
    public function invalidateCache(string $userId): void { /* ... */ }
    public function logActivity(User $user, string $action): void { /* ... */ }
    public function checkPermission(User $user, string $resource): bool { /* ... */ }
    public function trackMetric(string $event, array $data): void { /* ... */ }
    public function rateLimitCheck(string $userId, string $action): bool { /* ... */ }
    public function exportUserData(User $user): string { /* ... */ }
    public function importUsers($csvFile): int { /* ... */ }
    public function generateReport(array $filters): array { /* ... */ }
    public function indexUser(User $user): void { /* ... */ }
    public function reindexAll(): void { /* ... */ }
    public function setupTwoFactor(User $user): array { /* ... */ }
    public function verifyTwoFactor(User $user, string $code): bool { /* ... */ }
}
```

### Good Code (Fix)
```php
class UserRepository
{
    private $db;

    public function create(array $data): User { /* ... */ }
    public function update(string $id, array $data): User { /* ... */ }
    public function delete(string $id): void { /* ... */ }
    public function findById(string $id): ?User { /* ... */ }
    public function list(array $filters): array { /* ... */ }
}

class UserValidator
{
    public function validateEmail(string $email): bool { /* ... */ }
    public function validatePassword(string $password): bool { /* ... */ }
    public function validateUsername(string $username): bool { /* ... */ }
}

class AuthService
{
    private $encryption;

    public function hashPassword(string $password): string { /* ... */ }
    public function verifyPassword(string $plain, string $hashed): bool { /* ... */ }
    public function generateToken(User $user): string { /* ... */ }
    public function verifyToken(string $token): array { /* ... */ }
}

class UserEmailService
{
    private $mailer;

    public function sendWelcome(User $user): void { /* ... */ }
    public function sendReset(User $user): void { /* ... */ }
    public function sendVerification(User $user): void { /* ... */ }
}

class UserSearchService
{
    private $search;

    public function search(string $query): array { /* ... */ }
    public function index(User $user): void { /* ... */ }
    public function reindexAll(): void { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration` (count `property_declaration` and `method_declaration` children in `declaration_list`)
- **Detection approach**: Count `property_declaration` nodes and `method_declaration` nodes within a class's `declaration_list`. Flag when methods exceed 20, properties exceed 15, or total lines exceed 500. Include `__construct` in the method count.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (name) @class_name
    body: (declaration_list
      (method_declaration
        name: (name) @method_name)))

  (class_declaration
    body: (declaration_list
      (property_declaration) @property))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_class`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single class handling HTTP request processing, input validation, business logic, database access, and notification — a clear SRP violation. In PHP, this frequently appears as a Laravel controller that does everything: validates input, queries the database, sends emails, and computes business rules without delegating to services.

### Bad Code (Anti-pattern)
```php
class OrderController extends Controller
{
    public function store(Request $request)
    {
        // Validation
        $validated = $request->validate([
            'items' => 'required|array|min:1',
            'items.*.id' => 'required|exists:products,id',
            'items.*.quantity' => 'required|integer|min:1',
            'email' => 'required|email',
            'coupon' => 'nullable|string',
        ]);

        // Business logic
        $subtotal = collect($validated['items'])->sum(function ($item) {
            $product = Product::find($item['id']);
            return $product->price * $item['quantity'];
        });
        $tax = $subtotal * 0.08;
        $discount = $this->calculateDiscount($validated['coupon'] ?? null);
        $total = $subtotal + $tax - $discount;

        // Database access
        $order = Order::create(['total' => $total, 'tax' => $tax, 'status' => 'pending']);
        foreach ($validated['items'] as $item) {
            OrderItem::create(['order_id' => $order->id, 'product_id' => $item['id'], 'qty' => $item['quantity']]);
            Product::where('id', $item['id'])->decrement('stock', $item['quantity']);
        }

        // Email
        Mail::to($validated['email'])->send(new OrderConfirmation($order));

        // Logging
        Log::info("Order {$order->id} created for {$validated['email']}");

        return response()->json($order);
    }

    private function calculateDiscount(?string $coupon): float { /* ... */ }
    public function show(string $id) { /* ... */ }
    public function update(Request $request, string $id) { /* ... */ }
    public function destroy(string $id) { /* ... */ }
    public function refund(string $id) { /* ... */ }
    public function index(Request $request) { /* ... */ }
    public function export(Request $request) { /* ... */ }
    public function invoice(string $id) { /* ... */ }
    private function validateCoupon(string $code): bool { /* ... */ }
    private function calculateShipping(array $items, array $address): float { /* ... */ }
    private function notifyWarehouse(Order $order): void { /* ... */ }
    private function updateInventory(array $items): void { /* ... */ }
    private function sendTrackingEmail(Order $order): void { /* ... */ }
}
```

### Good Code (Fix)
```php
class OrderController extends Controller
{
    public function __construct(private OrderService $orderService) {}

    public function store(StoreOrderRequest $request)
    {
        $order = $this->orderService->create($request->validated());
        return response()->json($order);
    }

    public function show(string $id)
    {
        return response()->json($this->orderService->findById($id));
    }
}

class OrderService
{
    public function __construct(
        private OrderRepository $orderRepo,
        private PricingService $pricingService,
        private NotificationService $notificationService,
        private InventoryService $inventoryService,
    ) {}

    public function create(array $data): Order
    {
        $total = $this->pricingService->calculate($data['items'], $data['coupon'] ?? null);
        $order = $this->orderRepo->save($data['items'], $total);
        $this->inventoryService->deduct($data['items']);
        $this->notificationService->orderConfirmed($data['email'], $order);
        return $order;
    }
}

class OrderRepository
{
    public function save(array $items, float $total): Order { /* ... */ }
    public function findById(string $id): Order { /* ... */ }
    public function list(array $filters): array { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `declaration_list`, `method_declaration` — heuristic based on method name prefixes
- **Detection approach**: Categorize methods by name prefix/pattern (`get`/`find`/`list`/`show`/`index` = accessor, `validate`/`check` = validation, `save`/`store`/`update`/`destroy`/`delete` = persistence, `send`/`notify`/`mail` = communication, `track`/`log` = observability, `calculate`/`compute` = business logic). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    body: (declaration_list
      (method_declaration
        name: (name) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_class`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
