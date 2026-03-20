# Stringly Typed -- PHP

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .php

---

## Pattern 1: String Comparisons for State/Status

### Description
Using string equality checks to determine state transitions, roles, or status instead of PHP 8.1 enums or class constants.

### Bad Code (Anti-pattern)
```php
class OrderProcessor
{
    public function processOrder(Order $order): void
    {
        if ($order->status === 'active') {
            $this->startFulfillment($order);
        } elseif ($order->status === 'pending') {
            $this->notifyCustomer($order);
        } elseif ($order->status === 'cancelled') {
            $this->refundPayment($order);
        } elseif ($order->status === 'shipped') {
            $this->trackDelivery($order);
        } elseif ($order->status === 'delivered') {
            $this->requestReview($order);
        }
    }

    public function getStatusColor(string $status): string
    {
        return match ($status) {
            'active' => 'green',
            'pending' => 'yellow',
            'cancelled' => 'red',
            'shipped' => 'blue',
            'delivered' => 'gray',
            default => 'white',
        };
    }
}
```

### Good Code (Fix)
```php
enum Status: string
{
    case Active = 'active';
    case Pending = 'pending';
    case Cancelled = 'cancelled';
    case Shipped = 'shipped';
    case Delivered = 'delivered';

    public function color(): string
    {
        return match ($this) {
            self::Active => 'green',
            self::Pending => 'yellow',
            self::Cancelled => 'red',
            self::Shipped => 'blue',
            self::Delivered => 'gray',
        };
    }
}

class OrderProcessor
{
    public function processOrder(Order $order): void
    {
        match ($order->status) {
            Status::Active => $this->startFulfillment($order),
            Status::Pending => $this->notifyCustomer($order),
            Status::Cancelled => $this->refundPayment($order),
            Status::Shipped => $this->trackDelivery($order),
            Status::Delivered => $this->requestReview($order),
        };
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `===` or `==`), `string` (encapsed_string), `if_statement`, `match_expression`, `match_condition_list`
- **Detection approach**: Find equality comparisons (`===`, `==`) where one operand is a string literal. Flag when the same variable is compared against 3+ different string literals across `if`/`elseif` chains. Also detect `match` expressions with 3+ arms using string literal conditions.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (member_access_expression
    name: (name) @prop)
  right: (string) @string_val)

(match_condition_list
  (string) @match_string)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for configuration access via associative arrays (`$config['key']`) instead of typed configuration classes or value objects.

### Bad Code (Anti-pattern)
```php
class AppInitializer
{
    public function setup(array $config): void
    {
        $dbHost = $config['database_host'];
        $dbPort = $config['database_port'];
        $dbName = $config['database_name'];
        $redisUrl = $config['redis_url'];
        $apiKey = $config['api_key'];
        $logLevel = $config['log_level'];

        $this->connectDb($dbHost, (int) $dbPort, $dbName);
        $this->connectRedis($redisUrl);
        $this->initLogger($logLevel);
    }

    public function dispatchEvent(string $event, mixed $data): void
    {
        match ($event) {
            'user_created' => $this->handleUserCreated($data),
            'user_deleted' => $this->handleUserDeleted($data),
            'order_placed' => $this->handleOrderPlaced($data),
            'order_shipped' => $this->handleOrderShipped($data),
            'payment_received' => $this->handlePaymentReceived($data),
        };
    }
}
```

### Good Code (Fix)
```php
class DatabaseConfig
{
    public function __construct(
        public readonly string $host,
        public readonly int $port,
        public readonly string $name,
    ) {}
}

class AppConfig
{
    public function __construct(
        public readonly DatabaseConfig $database,
        public readonly string $redisUrl,
        public readonly string $apiKey,
        public readonly string $logLevel,
    ) {}
}

class AppInitializer
{
    public function setup(AppConfig $config): void
    {
        $this->connectDb($config->database->host, $config->database->port, $config->database->name);
        $this->connectRedis($config->redisUrl);
        $this->initLogger($config->logLevel);
    }
}

enum EventType: string
{
    case UserCreated = 'user_created';
    case UserDeleted = 'user_deleted';
    case OrderPlaced = 'order_placed';
    case OrderShipped = 'order_shipped';
    case PaymentReceived = 'payment_received';
}

class EventDispatcher
{
    public function dispatch(EventType $event, mixed $data): void
    {
        match ($event) {
            EventType::UserCreated => $this->handleUserCreated($data),
            EventType::UserDeleted => $this->handleUserDeleted($data),
            EventType::OrderPlaced => $this->handleOrderPlaced($data),
            EventType::OrderShipped => $this->handleOrderShipped($data),
            EventType::PaymentReceived => $this->handlePaymentReceived($data),
        };
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `subscript_expression` with `string` literal, `variable_name`
- **Detection approach**: Find repeated array access patterns where string literals are used as keys via bracket notation (`$config['key']`). Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(subscript_expression
  (variable_name) @obj
  (string) @key)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
