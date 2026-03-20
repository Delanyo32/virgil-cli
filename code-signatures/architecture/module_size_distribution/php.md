# Module Size Distribution -- PHP

## Overview
Module size distribution measures how symbol definitions are spread across source files in a PHP codebase. PHP follows PSR conventions that recommend one class per file, and balanced file sizes make autoloading predictable, keep code reviews focused, and simplify navigation. Files that grow excessively large or contain only trivial definitions indicate structural problems worth addressing.

## Why It's an Architecture Concern
Oversized PHP files that pack many classes, functions, and traits into a single file break autoloading conventions (PSR-4 expects one class per file), make it difficult to find specific symbols, and increase merge conflict frequency. Since PHP is often used in large web applications with deep dependency trees, a bloated file becomes a coupling magnet that many other files depend on. Anemic modules containing a single trivial function or empty interface add file system clutter and autoloading overhead without providing meaningful organizational value.

## Applicability
- **Relevance**: high
- **Languages covered**: `.php`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```php
<?php
// helpers.php -- a dumping ground for unrelated code

namespace App\Utils;

class StringHelper {
    public static function trim(string $s): string { /* ... */ }
    public static function slug(string $s): string { /* ... */ }
}

class ArrayHelper {
    public static function flatten(array $arr): array { /* ... */ }
    public static function unique(array $arr): array { /* ... */ }
}

class DateHelper {
    public static function format(\DateTime $d): string { /* ... */ }
    public static function parse(string $s): \DateTime { /* ... */ }
}

interface Cacheable { /* ... */ }
interface Renderable { /* ... */ }
trait HasTimestamps { /* ... */ }
trait SoftDeletes { /* ... */ }
enum Status: string { case Active = 'active'; case Inactive = 'inactive'; }

function config(string $key): mixed { /* ... */ }
function env(string $key): string { /* ... */ }
function dd(mixed ...$args): void { /* ... */ }
// ... 15 more functions, classes, and traits
```

### Good Code (Fix)
```php
<?php
// StringHelper.php -- focused on string operations
namespace App\Utils;

class StringHelper {
    public static function trim(string $s): string { /* ... */ }
    public static function slug(string $s): string { /* ... */ }
    public static function capitalize(string $s): string { /* ... */ }
}
```

```php
<?php
// ArrayHelper.php -- focused on array operations
namespace App\Utils;

class ArrayHelper {
    public static function flatten(array $arr): array { /* ... */ }
    public static function unique(array $arr): array { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_declaration`, `interface_declaration`, `trait_declaration`, `enum_declaration`, `namespace_definition`
- **Detection approach**: Count all top-level symbol definitions (children of `program` or direct children of `namespace_definition` body). Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(program
  [
    (function_definition name: (name) @name) @def
    (class_declaration name: (name) @name) @def
    (interface_declaration name: (name) @name) @def
    (trait_declaration name: (name) @name) @def
    (enum_declaration name: (name) @name) @def
    (namespace_definition name: (namespace_name) @name) @def
  ])

(namespace_definition
  body: (compound_statement
    [
      (function_definition name: (name) @name) @def
      (class_declaration name: (name) @name) @def
      (interface_declaration name: (name) @name) @def
      (trait_declaration name: (name) @name) @def
      (enum_declaration name: (name) @name) @def
    ]))
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```php
<?php
// ServiceProvider.php -- registering too many public bindings
namespace App\Providers;

class AppServiceProvider extends ServiceProvider {
    public function register(): void { /* ... */ }
    public function boot(): void { /* ... */ }
}

class UserService { public function find(int $id): User { /* ... */ } }
class OrderService { public function create(array $data): Order { /* ... */ } }
class PaymentService { public function charge(float $amount): bool { /* ... */ } }
class NotificationService { public function send(string $msg): void { /* ... */ } }
class CacheService { public function get(string $key): mixed { /* ... */ } }
class ReportService { public function generate(): Report { /* ... */ } }
class AuditService { public function log(string $action): void { /* ... */ } }

interface UserRepositoryInterface { /* ... */ }
interface OrderRepositoryInterface { /* ... */ }
trait Loggable { /* ... */ }
trait Auditable { /* ... */ }
// ... 10 more public types
```

### Good Code (Fix)
```php
<?php
// UserService.php -- focused on user operations
namespace App\Services;

class UserService {
    public function find(int $id): User { /* ... */ }
    public function create(array $data): User { /* ... */ }
    public function delete(int $id): void { /* ... */ }
}
```

```php
<?php
// OrderService.php -- focused on order operations
namespace App\Services;

class OrderService {
    public function create(array $data): Order { /* ... */ }
    public function cancel(int $id): void { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_declaration`, `interface_declaration`, `trait_declaration`, `enum_declaration`, `method_declaration`, `property_declaration`
- **Detection approach**: Count top-level public symbols. In PHP, top-level functions, classes, interfaces, traits, and enums are public by default. For class members, check for `public` visibility modifier. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(program
  (class_declaration name: (name) @name) @def)

(program
  (function_definition name: (name) @name) @def)

(program
  (interface_declaration name: (name) @name) @def)

(program
  (trait_declaration name: (name) @name) @def)
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```php
<?php
// EmptyCartException.php
namespace App\Exceptions;

class EmptyCartException extends \RuntimeException {}
```

### Good Code (Fix)
```php
<?php
// CartExceptions.php -- group related exceptions together
namespace App\Exceptions;

class EmptyCartException extends \RuntimeException {}
class CartLimitExceededException extends \RuntimeException {}
class InvalidCartItemException extends \RuntimeException {}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_declaration`, `interface_declaration`, `trait_declaration`, `enum_declaration`
- **Detection approach**: Count top-level symbol definitions. Flag if count == 1, excluding test files, service providers, and artisan command files.
- **S-expression query sketch**:
```scheme
(program
  [
    (function_definition name: (name) @name) @def
    (class_declaration name: (name) @name) @def
    (interface_declaration name: (name) @name) @def
    (trait_declaration name: (name) @name) @def
    (enum_declaration name: (name) @name) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
