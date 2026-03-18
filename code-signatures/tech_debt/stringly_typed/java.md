# Stringly Typed -- Java

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .java

---

## Pattern 1: String Comparisons for State/Status

### Description
Using `.equals()` string comparisons to determine state transitions, roles, or status instead of Java's rich `enum` types.

### Bad Code (Anti-pattern)
```java
public class OrderProcessor {
    public void processOrder(Order order) {
        if (order.getStatus().equals("active")) {
            startFulfillment(order);
        } else if (order.getStatus().equals("pending")) {
            notifyCustomer(order);
        } else if (order.getStatus().equals("cancelled")) {
            refundPayment(order);
        } else if (order.getStatus().equals("shipped")) {
            trackDelivery(order);
        } else if (order.getStatus().equals("delivered")) {
            requestReview(order);
        }
    }

    public String getStatusColor(String status) {
        switch (status) {
            case "active": return "green";
            case "pending": return "yellow";
            case "cancelled": return "red";
            case "shipped": return "blue";
            case "delivered": return "gray";
            default: return "white";
        }
    }
}
```

### Good Code (Fix)
```java
public enum Status {
    ACTIVE, PENDING, CANCELLED, SHIPPED, DELIVERED;

    public String getColor() {
        return switch (this) {
            case ACTIVE -> "green";
            case PENDING -> "yellow";
            case CANCELLED -> "red";
            case SHIPPED -> "blue";
            case DELIVERED -> "gray";
        };
    }
}

public class OrderProcessor {
    public void processOrder(Order order) {
        switch (order.getStatus()) {
            case ACTIVE -> startFulfillment(order);
            case PENDING -> notifyCustomer(order);
            case CANCELLED -> refundPayment(order);
            case SHIPPED -> trackDelivery(order);
            case DELIVERED -> requestReview(order);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation` (`.equals()`), `string_literal`, `if_statement`, `switch_expression`, `switch_label`
- **Detection approach**: Find `method_invocation` nodes calling `.equals()` where the argument is a string literal. Flag when the same receiver variable calls `.equals()` with 3+ different string literals across an `if`/`else if` chain. Also detect `switch` statements with 3+ `case` labels using string literals.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (method_invocation) @receiver
  name: (identifier) @method_name
  arguments: (argument_list
    (string_literal) @string_val))

(switch_label
  (string_literal) @case_string)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys to access configuration from `Map<String, String>` or `Properties` instead of typed configuration classes or records.

### Bad Code (Anti-pattern)
```java
public class AppInitializer {
    public void setup(Map<String, String> config) {
        String dbHost = config.get("database_host");
        String dbPort = config.get("database_port");
        String dbName = config.get("database_name");
        String redisUrl = config.get("redis_url");
        String apiKey = config.get("api_key");
        String logLevel = config.get("log_level");

        connectDb(dbHost, Integer.parseInt(dbPort), dbName);
        connectRedis(redisUrl);
        initLogger(logLevel);
    }

    public void dispatchEvent(String eventName, Object data) {
        switch (eventName) {
            case "user_created" -> handleUserCreated(data);
            case "user_deleted" -> handleUserDeleted(data);
            case "order_placed" -> handleOrderPlaced(data);
            case "order_shipped" -> handleOrderShipped(data);
            case "payment_received" -> handlePaymentReceived(data);
        }
    }
}
```

### Good Code (Fix)
```java
public record DatabaseConfig(String host, int port, String name) {}
public record AppConfig(DatabaseConfig database, String redisUrl, String apiKey, String logLevel) {}

public class AppInitializer {
    public void setup(AppConfig config) {
        connectDb(config.database().host(), config.database().port(), config.database().name());
        connectRedis(config.redisUrl());
        initLogger(config.logLevel());
    }
}

public enum EventType {
    USER_CREATED, USER_DELETED, ORDER_PLACED, ORDER_SHIPPED, PAYMENT_RECEIVED
}

public class EventDispatcher {
    public void dispatch(EventType event, Object data) {
        switch (event) {
            case USER_CREATED -> handleUserCreated(data);
            case USER_DELETED -> handleUserDeleted(data);
            case ORDER_PLACED -> handleOrderPlaced(data);
            case ORDER_SHIPPED -> handleOrderShipped(data);
            case PAYMENT_RECEIVED -> handlePaymentReceived(data);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation` (`.get()`), `string_literal` argument, `identifier`
- **Detection approach**: Find repeated `.get("key")` calls on the same `Map` or `Properties` variable where string literals are used as arguments. Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @obj
  name: (identifier) @method
  arguments: (argument_list
    (string_literal) @key))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
