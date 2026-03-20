# Coupling -- PHP

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file importing from many different namespaces or requiring many files (high fan-in), indicating it depends on too many parts of the system. In PHP, coupling manifests through `use` statements (namespace imports), `require`/`include` directives, and Composer autoloading. Laravel's facades and service container can mask coupling by hiding concrete dependencies behind static-looking calls.

### Bad Code (Anti-pattern)
```php
<?php
// Controllers/OrderController.php
namespace App\Controllers;

use App\Auth\AuthService;
use App\Auth\TokenValidator;
use App\Users\UserService;
use App\Users\PreferencesService;
use App\Orders\OrderService;
use App\Orders\OrderValidator;
use App\Billing\TaxService;
use App\Billing\PaymentGateway;
use App\Billing\DiscountEngine;
use App\Notifications\EmailService;
use App\Notifications\PushService;
use App\Logging\EventLogger;
use App\Analytics\Tracker;
use App\Cache\CacheManager;
use App\Queue\JobQueue;
use App\Utils\CurrencyFormatter;
use App\Database\Connection;

class OrderController
{
    public function createOrder(Request $request): Response
    {
        $user = (new AuthService())->authenticate($request);
        TokenValidator::validate($request->bearerToken());
        $prefs = (new PreferencesService())->get($user->id);
        OrderValidator::validate($request->all());
        $tax = (new TaxService())->calculate($request->get('amount'));
        $discount = (new DiscountEngine())->apply($request->get('coupon'));
        $payment = (new PaymentGateway())->charge($request->get('amount') + $tax - $discount);
        $order = (new OrderService())->create($user, $request->all(), $payment);
        (new EmailService())->sendConfirmation($user, $order);
        (new PushService())->notify($user, 'Order created');
        (new EventLogger())->log('order_created', ['order_id' => $order->id]);
        (new Tracker())->track('purchase', ['amount' => $order->total]);
        (new CacheManager())->invalidate('user_orders_' . $user->id);
        (new JobQueue())->enqueue('process_order', $order->id);
        return new Response($order);
    }
}
```

### Good Code (Fix)
```php
<?php
// Controllers/OrderController.php
namespace App\Controllers;

use App\Auth\AuthService;
use App\Orders\OrderService;
use App\Logging\EventLogger;

class OrderController
{
    public function __construct(
        private OrderService $orderService,
        private AuthService $authService,
        private EventLogger $logger,
    ) {}

    public function createOrder(Request $request): Response
    {
        $user = $this->authService->authenticate($request);
        $order = $this->orderService->create($user, $request->all());
        $this->logger->log('order_created', ['order_id' => $order->id]);
        return new Response($order);
    }
}

// Orders/OrderService.php — encapsulates billing, notifications, caching
namespace App\Orders;

use App\Billing\BillingService;
use App\Notifications\NotificationService;

class OrderService
{
    public function __construct(
        private BillingService $billing,
        private NotificationService $notifications,
    ) {}

    public function create(User $user, array $data): Order
    {
        OrderValidator::validate($data);
        $total = $this->billing->processOrder($data);
        $this->notifications->sendOrderConfirmation($user, $total);
        return new Order($data, $total);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `namespace_use_declaration`, `include_expression`, `require_expression`
- **Detection approach**: Count unique namespace/file sources per file. For `use` statements, extract the qualified name. For grouped `use` statements (`use App\Models\{User, Post}`), expand each member into a full path. For `require`/`include`, extract the file path string. Flag files exceeding threshold (e.g., 15+ unique imports). Distinguish between framework namespaces and project-internal namespaces.
- **S-expression query sketch**:
```scheme
;; use App\Services\OrderService
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @use_path))

;; use App\Models\{User, Post} (grouped)
(namespace_use_declaration
  (namespace_use_group
    (namespace_use_clause
      (qualified_name) @grouped_use_path)))

;; require/include
(expression_statement
  (include_expression
    (string) @include_path))

(expression_statement
  (require_expression
    (string) @require_path))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more classes/namespaces that import each other (directly or transitively), creating a dependency cycle. PHP's autoloading makes circular dependencies less immediately visible than in compiled languages, but they still cause issues: circular constructor injection fails at runtime, classes may reference partially loaded dependencies, and the codebase becomes impossible to decompose into independent packages.

### Bad Code (Anti-pattern)
```php
<?php
// Models/User.php
namespace App\Models;

use App\Services\OrderService;  // Models depends on Services

class User
{
    public string $id;
    public string $name;

    public function getActiveOrders(): array
    {
        return OrderService::getInstance()->getOrdersForUser($this->id);  // Tight coupling
    }
}

// Services/OrderService.php
namespace App\Services;

use App\Models\User;  // Services depends on Models — circular

class OrderService
{
    private static ?self $instance = null;

    public static function getInstance(): self
    {
        return self::$instance ??= new self();
    }

    public function processOrder(User $user, float $amount): Order
    {
        return new Order($user, $amount);
    }

    public function getOrdersForUser(string $userId): array
    {
        return [];
    }
}
```

### Good Code (Fix)
```php
<?php
// Contracts/OrderRepositoryInterface.php — shared interface, no cycle
namespace App\Contracts;

interface OrderRepositoryInterface
{
    public function getOrdersForUser(string $userId): array;
}

// Models/User.php — no dependency on Services
namespace App\Models;

class User
{
    public function __construct(
        public string $id,
        public string $name,
    ) {}
}

// Services/OrderService.php — implements interface, depends on Models (one direction)
namespace App\Services;

use App\Contracts\OrderRepositoryInterface;
use App\Models\User;

class OrderService implements OrderRepositoryInterface
{
    public function getOrdersForUser(string $userId): array
    {
        return [];
    }

    public function processOrder(User $user, float $amount): Order
    {
        return new Order(userId: $user->id, amount: $amount);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `namespace_use_declaration`, `include_expression`, `require_expression`
- **Detection approach**: Build a directed graph of namespace-to-namespace imports by extracting the namespace prefix from each `use` statement and the current namespace from `namespace_definition`. Include `require`/`include` paths for file-level coupling. Detect cycles using DFS with back-edge detection. Report the shortest cycle found.
- **S-expression query sketch**:
```scheme
;; Collect all use paths to build dependency graph
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @use_path))

;; Current namespace to identify source node
(namespace_definition
  name: (namespace_name) @current_namespace)

;; require/include create file-level coupling
(expression_statement
  (include_expression
    (string) @include_path))

(expression_statement
  (require_expression
    (string) @require_path))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
