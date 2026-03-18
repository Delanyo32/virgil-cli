# Dependency Graph Depth -- PHP

## Overview
Dependency graph depth measures how many layers of namespace imports and file inclusions a PHP file must traverse before all dependencies are resolved. In PHP, deep dependency chains manifest as deeply nested namespace hierarchies and facade files that aggregate `use` declarations from many sub-namespaces, increasing coupling and making the codebase harder to refactor.

## Why It's an Architecture Concern
Deep dependency chains in PHP increase the blast radius of changes -- restructuring a namespace buried several layers deep requires updating `use` declarations across every file that references it, either directly or through facade classes. PHP's autoloading convention (PSR-4) ties namespace depth directly to directory depth, so excessive nesting creates deep directory trees that are cumbersome to navigate. Facade files that aggregate many sub-namespace imports add a maintenance layer that must be kept in sync with the underlying implementations. Keeping namespace hierarchies reasonably flat and imports direct simplifies autoloading, reduces coupling, and makes the codebase more approachable.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.php`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In PHP, the barrel file pattern manifests as namespace files or service provider classes that import from many sub-namespaces via `use` declarations and register or expose them as a unified interface. Since PHP lacks re-export syntax, these files typically instantiate or reference each imported class, acting as an aggregation layer that adds indirection without meaningful logic.

### Bad Code (Anti-pattern)
```php
<?php
// src/Services/ServiceProvider.php -- aggregates all sub-namespace services
namespace App\Services;

use App\Services\Auth\AuthService;
use App\Services\Billing\BillingService;
use App\Services\Email\EmailService;
use App\Services\Reporting\ReportService;
use App\Services\Storage\StorageService;
use App\Services\Users\UserService;

class ServiceProvider
{
    public function auth(): AuthService { return new AuthService(); }
    public function billing(): BillingService { return new BillingService(); }
    public function email(): EmailService { return new EmailService(); }
    public function reporting(): ReportService { return new ReportService(); }
    public function storage(): StorageService { return new StorageService(); }
    public function users(): UserService { return new UserService(); }
}
```

### Good Code (Fix)
```php
<?php
// src/Controllers/PaymentController.php -- imports directly from source namespaces
namespace App\Controllers;

use App\Services\Auth\AuthService;
use App\Services\Billing\BillingService;

class PaymentController
{
    public function __construct(
        private AuthService $auth,
        private BillingService $billing,
    ) {}

    public function processPayment(string $token, string $cardId): void
    {
        $this->auth->validate($token);
        $this->billing->charge($cardId);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`, `method_declaration`, `class_declaration`
- **Detection approach**: Count `use` declarations in a single file. Flag as barrel if count >= 5 and the class consists primarily of one-line methods that instantiate or delegate to the imported classes. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture use declarations
(use_declaration
  (name) @use_path) @use_decl

;; Capture qualified use declarations
(use_declaration
  (qualified_name) @use_qualified_path) @use_decl

;; Capture thin delegation methods
(method_declaration
  name: (name) @method_name
  body: (compound_statement
    (return_statement
      (object_creation_expression
        (name) @instantiated_class)))) @method
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In PHP this appears as `use` declarations with many backslash-separated namespace segments.

### Bad Code (Anti-pattern)
```php
<?php
namespace App\Presentation\Http\Controllers\Api\V2;

use App\Infrastructure\Persistence\Repositories\Contracts\IOrderRepository;
use App\Infrastructure\Persistence\Repositories\Eloquent\OrderRepository;
use App\Domain\Aggregates\Orders\ValueObjects\OrderStatus;
use App\Application\Services\Orders\Handlers\CreateOrderHandler;

class OrderController
{
    public function __construct(
        private IOrderRepository $repo,
        private CreateOrderHandler $handler,
    ) {}

    public function store(Request $request): Response
    {
        return $this->handler->handle($request->validated());
    }
}
```

### Good Code (Fix)
```php
<?php
namespace App\Http\Controllers;

use App\Persistence\Contracts\IOrderRepository;
use App\Domain\Orders\OrderStatus;
use App\Services\CreateOrderHandler;

class OrderController
{
    public function __construct(
        private IOrderRepository $repo,
        private CreateOrderHandler $handler,
    ) {}

    public function store(Request $request): Response
    {
        return $this->handler->handle($request->validated());
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `use_declaration`, `qualified_name`
- **Detection approach**: Parse the namespace path in each `use` declaration and count backslash-separated segments. Flag if depth >= 4 (excluding the root `App` segment). Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
(use_declaration
  (qualified_name) @namespace_path) @use_decl
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
