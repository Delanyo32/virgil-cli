# API Surface Area -- PHP

## Overview
API surface area in PHP is managed through visibility modifiers on class members: `public`, `protected`, and `private`, with `public` as the default when no modifier is specified. This permissive default means classes often end up with large public surfaces by accident. Tracking the ratio of public to total members identifies classes that expose too much, tightening coupling between components and making refactoring risky.

## Why It's an Architecture Concern
PHP's default-public behavior means that developers who omit visibility modifiers inadvertently expand the class's public contract. Consumers of a class can call any public method or access any public property, creating dependencies on implementation details that were never intended as API. A large public surface makes it difficult to rename methods, change signatures, or restructure class hierarchies without breaking dependents. In frameworks like Laravel or Symfony, where classes are often resolved through dependency injection containers, the distinction between intended API and internal plumbing is especially important. Disciplined use of `private` and `protected` keeps the surface narrow and intentions clear.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.php`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```php
class PaymentGateway
{
    public function charge(float $amount): bool { return true; }
    public function refund(string $txId): bool { return true; }
    public function authorize(float $amount): string { return ''; }
    public function capture(string $authId): bool { return true; }
    public function void(string $txId): bool { return true; }
    public function validateCard(array $card): bool { return true; }
    public function tokenizeCard(array $card): string { return ''; }
    public function buildRequest(array $params): array { return []; }
    public function sendRequest(array $req): array { return []; }
    public function parseResponse(string $raw): array { return []; }
    public function logTransaction(array $tx): void { }
    public function retryOnFailure(callable $fn): mixed { return null; }
}
```

### Good Code (Fix)
```php
class PaymentGateway
{
    public function charge(float $amount): bool { return true; }
    public function refund(string $txId): bool { return true; }
    public function authorize(float $amount): string { return ''; }
    public function capture(string $authId): bool { return true; }
    public function void(string $txId): bool { return true; }

    private function validateCard(array $card): bool { return true; }
    private function tokenizeCard(array $card): string { return ''; }
    private function buildRequest(array $params): array { return []; }
    private function sendRequest(array $req): array { return []; }
    private function parseResponse(string $raw): array { return []; }
    private function logTransaction(array $tx): void { }
    private function retryOnFailure(callable $fn): mixed { return null; }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration` with `visibility_modifier`, `property_declaration`
- **Detection approach**: Count all method and property declarations within a class. A member is exported if it has a `public` visibility modifier or no visibility modifier (PHP default is public). Flag classes where total members >= 10 and exported/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match public methods (explicit public)
(class_declaration
  name: (name) @class.name
  body: (declaration_list
    (method_declaration
      (visibility_modifier) @vis
      name: (name) @method.name
      (#eq? @vis "public"))))

;; Match methods without visibility modifier (default public)
(class_declaration
  body: (declaration_list
    (method_declaration
      name: (name) @default.method.name)))

;; Post-process: methods without a visibility_modifier child are default-public
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```php
class UserService
{
    public PDO $db;
    public array $queryCache = [];
    public Logger $logger;
    public int $maxRetries = 3;
    public float $timeoutSeconds = 5.0;
    public bool $debugMode = false;

    public function findUser(int $id): ?User { return null; }
    public function saveUser(User $user): void { }
}
```

### Good Code (Fix)
```php
class UserService
{
    private PDO $db;
    private array $queryCache = [];
    private LoggerInterface $logger;
    private int $maxRetries;
    private float $timeoutSeconds;

    public function __construct(
        PDO $db,
        LoggerInterface $logger,
        int $maxRetries = 3,
        float $timeoutSeconds = 5.0
    ) {
        $this->db = $db;
        $this->logger = $logger;
        $this->maxRetries = $maxRetries;
        $this->timeoutSeconds = $timeoutSeconds;
    }

    public function findUser(int $id): ?User { return null; }
    public function saveUser(User $user): void { }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `property_declaration` with `visibility_modifier` inside `class_declaration`
- **Detection approach**: Find classes and check for public property declarations. Public properties expose internal state directly to consumers. Flag classes with 2+ public properties, especially those typed with concrete classes (e.g., `PDO`, `Logger`) rather than interfaces.
- **S-expression query sketch**:
```scheme
;; Match public properties in classes
(class_declaration
  name: (name) @class.name
  body: (declaration_list
    (property_declaration
      (visibility_modifier) @vis
      (property_element
        (variable_name) @prop.name)
      (#eq? @vis "public"))))

;; Match properties with type declarations
(property_declaration
  (visibility_modifier) @vis
  type: (named_type (name) @prop.type)
  (property_element
    (variable_name) @prop.name)
  (#eq? @vis "public"))
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
