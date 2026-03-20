# Encapsulation Leaks -- PHP

## Overview
Encapsulation leaks in PHP occur when functions rely on the `global` keyword to access and modify variables from the global scope, or when classes expose public properties without accessor methods, allowing any code to bypass validation and invariant enforcement. Both patterns create tightly coupled, hard-to-test code.

## Why It's a Tech Debt Concern
Using `global` creates invisible dependencies between functions and the global scope — any function can silently read or modify a global variable, making call graphs unpredictable and unit testing nearly impossible without manipulating global state. Public properties without accessors allow any code to set invalid values, and adding validation later requires modifying every access site across the codebase. PHP's default public visibility makes this especially pervasive.

## Applicability
- **Relevance**: high (`global` keyword and public properties are widespread in legacy PHP)
- **Languages covered**: `.php`
- **Frameworks/libraries**: WordPress (heavy global usage), Laravel (Eloquent model attributes), legacy PHP applications

---

## Pattern 1: Global Variable Usage

### Description
Functions use the `global` keyword to import variables from the global scope, creating hidden dependencies that are not visible in the function signature. This makes functions impure — their behavior depends on and modifies state outside their parameter list.

### Bad Code (Anti-pattern)
```php
<?php

$dbConnection = null;
$appConfig = [];
$requestCount = 0;
$logger = null;
$cache = [];

function initApp() {
    global $dbConnection, $appConfig, $logger;
    $appConfig = parse_ini_file('/etc/app/config.ini');
    $dbConnection = new PDO($appConfig['dsn'], $appConfig['user'], $appConfig['pass']);
    $logger = new FileLogger($appConfig['log_path']);
}

function getUser(int $id): ?array {
    global $dbConnection, $cache, $requestCount, $logger;
    $requestCount++;
    $logger->debug("Fetching user $id (request #$requestCount)");

    if (isset($cache["user:$id"])) {
        return $cache["user:$id"];
    }

    $stmt = $dbConnection->prepare('SELECT * FROM users WHERE id = ?');
    $stmt->execute([$id]);
    $user = $stmt->fetch(PDO::FETCH_ASSOC);

    $cache["user:$id"] = $user;
    return $user;
}

function saveUser(array $data): int {
    global $dbConnection, $cache, $logger;
    $logger->info("Saving user: " . $data['name']);
    $stmt = $dbConnection->prepare('INSERT INTO users (name, email) VALUES (?, ?)');
    $stmt->execute([$data['name'], $data['email']]);
    $id = (int)$dbConnection->lastInsertId();
    unset($cache["user:$id"]);
    return $id;
}

function resetCache(): void {
    global $cache;
    $cache = [];
}
```

### Good Code (Fix)
```php
<?php

class UserRepository {
    private PDO $db;
    private LoggerInterface $logger;
    private array $cache = [];

    public function __construct(PDO $db, LoggerInterface $logger) {
        $this->db = $db;
        $this->logger = $logger;
    }

    public function findById(int $id): ?array {
        $this->logger->debug("Fetching user $id");

        if (isset($this->cache["user:$id"])) {
            return $this->cache["user:$id"];
        }

        $stmt = $this->db->prepare('SELECT * FROM users WHERE id = ?');
        $stmt->execute([$id]);
        $user = $stmt->fetch(PDO::FETCH_ASSOC);

        if ($user) {
            $this->cache["user:$id"] = $user;
        }
        return $user;
    }

    public function save(array $data): int {
        $this->logger->info("Saving user: " . $data['name']);
        $stmt = $this->db->prepare('INSERT INTO users (name, email) VALUES (?, ?)');
        $stmt->execute([$data['name'], $data['email']]);
        $id = (int)$this->db->lastInsertId();
        unset($this->cache["user:$id"]);
        return $id;
    }

    public function clearCache(): void {
        $this->cache = [];
    }
}

// Usage with dependency injection
$db = new PDO($config['dsn'], $config['user'], $config['pass']);
$logger = new FileLogger($config['log_path']);
$repo = new UserRepository($db, $logger);
```

### Tree-sitter Detection Strategy
- **Target node types**: `global_declaration` inside `function_definition` or `method_declaration`
- **Detection approach**: Find `global_declaration` nodes inside function or method bodies. Each `global_declaration` lists one or more variable names being imported from the global scope. Flag every function that contains a `global_declaration`. Stronger signal when the same global variable appears in 3+ different functions.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    name: (name) @func_name
    body: (compound_statement
      (global_declaration
        (variable_name) @global_var)))

  (method_declaration
    name: (name) @method_name
    body: (compound_statement
      (global_declaration
        (variable_name) @global_var)))
  ```

### Pipeline Mapping
- **Pipeline name**: `encapsulation_leaks`
- **Pattern name**: `global_keyword_usage`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Public Properties Without Accessors

### Description
A class declares properties as `public`, allowing any code to read and modify them directly. Since PHP's default visibility is public, this is especially common in classes where visibility is simply omitted. Internal state should use `private` or `protected` visibility with getter/setter methods that can enforce validation.

### Bad Code (Anti-pattern)
```php
<?php

class Order {
    public int $id;
    public string $status;
    public float $total;
    public float $tax;
    public float $discount;
    public array $items = [];
    public ?string $couponCode = null;
    public string $currency = 'USD';
    public ?\DateTimeInterface $createdAt = null;
    public ?\DateTimeInterface $completedAt = null;

    public function __construct(int $id) {
        $this->id = $id;
        $this->status = 'draft';
        $this->createdAt = new \DateTimeImmutable();
    }
}

// Any code can set invalid state
$order = new Order(1);
$order->status = 'invalid_status';  // no validation
$order->total = -100.0;             // negative total
$order->tax = -50.0;                // negative tax
$order->items = 'not an array';     // type coercion issues at runtime
$order->createdAt = null;           // destroy audit trail
$order->completedAt = new \DateTimeImmutable('1900-01-01'); // nonsensical date
```

### Good Code (Fix)
```php
<?php

class Order {
    private int $id;
    private string $status;
    private float $total = 0.0;
    private float $tax = 0.0;
    private float $discount = 0.0;
    private array $items = [];
    private ?string $couponCode = null;
    private string $currency;
    private \DateTimeInterface $createdAt;
    private ?\DateTimeInterface $completedAt = null;

    private const VALID_STATUSES = ['draft', 'pending', 'confirmed', 'shipped', 'completed', 'cancelled'];

    public function __construct(int $id, string $currency = 'USD') {
        $this->id = $id;
        $this->status = 'draft';
        $this->currency = $currency;
        $this->createdAt = new \DateTimeImmutable();
    }

    public function getId(): int { return $this->id; }
    public function getStatus(): string { return $this->status; }
    public function getTotal(): float { return $this->total; }
    public function getTax(): float { return $this->tax; }
    public function getItems(): array { return $this->items; }
    public function getCreatedAt(): \DateTimeInterface { return $this->createdAt; }
    public function getCompletedAt(): ?\DateTimeInterface { return $this->completedAt; }

    public function setStatus(string $status): void {
        if (!in_array($status, self::VALID_STATUSES, true)) {
            throw new \InvalidArgumentException("Invalid status: $status");
        }
        $this->status = $status;
        if ($status === 'completed') {
            $this->completedAt = new \DateTimeImmutable();
        }
    }

    public function addItem(OrderItem $item): void {
        $this->items[] = $item;
        $this->recalculateTotal();
    }

    public function applyCoupon(string $code, float $discount): void {
        if ($discount < 0 || $discount > $this->total) {
            throw new \InvalidArgumentException("Invalid discount amount");
        }
        $this->couponCode = $code;
        $this->discount = $discount;
        $this->recalculateTotal();
    }

    private function recalculateTotal(): void {
        $subtotal = array_reduce($this->items, fn($sum, $i) => $sum + $i->getPrice(), 0.0);
        $this->tax = $subtotal * 0.08;
        $this->total = $subtotal + $this->tax - $this->discount;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `property_declaration` inside `class_declaration` or `declaration_list` with `public` `visibility_modifier`
- **Detection approach**: Find `property_declaration` nodes inside class bodies whose `visibility_modifier` is `public` (or have no visibility modifier, since PHP defaults to public). Exclude `static` properties and constants. Flag classes with 3+ public instance properties. Stronger signal when the class also has methods that modify those properties without validation.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (name) @class_name
    body: (declaration_list
      (property_declaration
        (visibility_modifier) @vis
        (property_element
          (variable_name) @prop_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `encapsulation_leaks`
- **Pattern name**: `public_properties_without_accessors`
- **Severity**: warning
- **Confidence**: high
