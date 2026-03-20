# Circular Dependencies -- PHP

## Overview
Circular dependencies in PHP occur when two or more classes or namespaces mutually reference each other through `use` statements, `require`/`include` directives, or constructor injection. PHP's autoloader resolves classes on demand, which masks many circular dependencies at runtime. However, cycles still cause problems during static analysis, complicate dependency injection container configuration, and indicate tightly coupled design that is difficult to test and maintain.

## Why It's an Architecture Concern
Circular references between namespaces or classes make modules inseparable — extracting one into a separate Composer package requires breaking the cycle first. They complicate dependency injection: containers like Symfony's DI or Laravel's service container may throw circular reference exceptions when two services depend on each other through constructor injection. Testing becomes difficult because mocking one class requires instantiating (or mocking) the other. Cycles indicate tangled responsibilities: if service A needs repository B and repository B needs service A, the separation between business logic and data access is violated, making the codebase harder to evolve.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.php`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```php
// --- src/Service/OrderService.php ---
<?php
namespace App\Service;

use App\Repository\OrderRepository;  // Service uses Repository

class OrderService
{
    public function __construct(
        private OrderRepository $repository
    ) {}

    public function processOrder(int $orderId): void
    {
        $order = $this->repository->findById($orderId);
        $order->setStatus('processed');
        $this->repository->save($order);
    }
}

// --- src/Repository/OrderRepository.php ---
<?php
namespace App\Repository;

use App\Service\OrderService;  // Repository uses Service -- CIRCULAR

class OrderRepository
{
    public function __construct(
        private OrderService $orderService
    ) {}

    public function findById(int $id): Order
    {
        $order = $this->db->find($id);
        // Eager validation via service — creates the cycle
        $this->orderService->validate($order);
        return $order;
    }
}
```

### Good Code (Fix)
```php
// --- src/Contract/OrderValidatorInterface.php --- (interface breaks cycle)
<?php
namespace App\Contract;

interface OrderValidatorInterface
{
    public function validate(Order $order): bool;
}

// --- src/Service/OrderService.php ---
<?php
namespace App\Service;

use App\Contract\OrderValidatorInterface;
use App\Repository\OrderRepository;

class OrderService implements OrderValidatorInterface
{
    public function __construct(private OrderRepository $repository) {}
    public function validate(Order $order): bool { /* ... */ }
}

// --- src/Repository/OrderRepository.php ---
<?php
namespace App\Repository;

use App\Contract\OrderValidatorInterface;  // depends on interface, not service

class OrderRepository
{
    public function __construct(
        private OrderValidatorInterface $validator
    ) {}

    public function findById(int $id): Order
    {
        $order = $this->db->find($id);
        $this->validator->validate($order);
        return $order;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `namespace_use_declaration`, `require_expression`, `include_expression`
- **Detection approach**: Per-file: extract all `use` namespace paths and `require`/`include` file paths from each PHP file. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each file (or namespace) to its imported namespaces, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files whose namespace is imported by a file they also import.
- **S-expression query sketch**:
```scheme
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @import_source))

(expression_statement
  (require_expression
    (string) @import_source))

(expression_statement
  (include_expression
    (string) @import_source))
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
```php
// --- src/Common/AppHelper.php ---
<?php
namespace App\Common;

use App\Auth\AuthManager;
use App\Billing\PaymentGateway;
use App\Cache\CacheStore;
use App\Config\ConfigLoader;
use App\Database\ConnectionPool;
use App\Logging\Logger;
use App\Queue\JobDispatcher;

// High fan-out (7 use statements) AND high fan-in (every namespace above uses AppHelper)
class AppHelper
{
    public static function getAuth(): AuthManager { return new AuthManager(); }
    public static function getPayment(): PaymentGateway { return new PaymentGateway(); }
    public static function getCache(): CacheStore { return CacheStore::instance(); }
    public static function getDb(): ConnectionPool { return ConnectionPool::instance(); }
    public static function getLogger(): Logger { return Logger::default(); }
    public static function dispatch(string $job): void { JobDispatcher::push($job); }
}
```

### Good Code (Fix)
```php
// --- src/Auth/AuthServiceInterface.php --- (focused contract)
<?php
namespace App\Auth;

interface AuthServiceInterface
{
    public function validateToken(string $token): bool;
}

// --- src/Billing/PaymentServiceInterface.php --- (focused contract)
<?php
namespace App\Billing;

interface PaymentServiceInterface
{
    public function charge(int $customerId, float $amount): void;
}

// Wire via DI container (Symfony, Laravel)
// Each service declares only the interfaces it needs
// No static helper hub required
```

### Tree-sitter Detection Strategy
- **Target node types**: `namespace_use_declaration`
- **Detection approach**: Per-file: count `use` declarations to estimate fan-out. Cross-file: query imports.parquet to count how many other files import from this file's namespace (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @import_source))
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
