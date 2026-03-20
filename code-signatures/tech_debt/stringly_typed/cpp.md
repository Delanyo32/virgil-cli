# Stringly Typed -- C++

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh

---

## Pattern 1: String Comparisons for State/Status

### Description
Using `==` on `std::string` or `strcmp()` to determine state transitions, roles, or status instead of `enum class` (scoped enums).

### Bad Code (Anti-pattern)
```cpp
#include <string>

void processOrder(Order& order) {
    if (order.status == "active") {
        startFulfillment(order);
    } else if (order.status == "pending") {
        notifyCustomer(order);
    } else if (order.status == "cancelled") {
        refundPayment(order);
    } else if (order.status == "shipped") {
        trackDelivery(order);
    } else if (order.status == "delivered") {
        requestReview(order);
    }
}

std::string getStatusColor(const std::string& status) {
    if (status == "active") return "green";
    if (status == "pending") return "yellow";
    if (status == "cancelled") return "red";
    if (status == "shipped") return "blue";
    if (status == "delivered") return "gray";
    return "white";
}
```

### Good Code (Fix)
```cpp
enum class Status {
    Active,
    Pending,
    Cancelled,
    Shipped,
    Delivered
};

void processOrder(Order& order) {
    switch (order.status) {
    case Status::Active:
        startFulfillment(order);
        break;
    case Status::Pending:
        notifyCustomer(order);
        break;
    case Status::Cancelled:
        refundPayment(order);
        break;
    case Status::Shipped:
        trackDelivery(order);
        break;
    case Status::Delivered:
        requestReview(order);
        break;
    }
}

std::string getStatusColor(Status status) {
    switch (status) {
    case Status::Active:    return "green";
    case Status::Pending:   return "yellow";
    case Status::Cancelled: return "red";
    case Status::Shipped:   return "blue";
    case Status::Delivered: return "gray";
    default:                return "white";
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `==`), `string_literal`, `if_statement`, `call_expression` (`strcmp`)
- **Detection approach**: Find equality comparisons (`==`) where one operand is a string literal (comparing `std::string` to a C string literal). Also detect `strcmp()` calls in `if`/`else if` chains. Flag when the same variable is compared against 3+ different string literals.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (field_expression
    field: (field_identifier) @field)
  right: (string_literal) @string_val)

(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @var
    (string_literal) @string_val))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for configuration access via `std::map<std::string, std::string>` or `std::unordered_map` instead of typed configuration structs.

### Bad Code (Anti-pattern)
```cpp
#include <map>
#include <string>

void setupApp(const std::map<std::string, std::string>& config) {
    auto dbHost = config.at("database_host");
    auto dbPort = config.at("database_port");
    auto dbName = config.at("database_name");
    auto redisUrl = config.at("redis_url");
    auto apiKey = config.at("api_key");
    auto logLevel = config.at("log_level");

    connectDb(dbHost, std::stoi(dbPort), dbName);
    connectRedis(redisUrl);
    initLogger(logLevel);
}

void dispatchEvent(const std::string& event, void* data) {
    if (event == "user_created") handleUserCreated(data);
    else if (event == "user_deleted") handleUserDeleted(data);
    else if (event == "order_placed") handleOrderPlaced(data);
    else if (event == "order_shipped") handleOrderShipped(data);
    else if (event == "payment_received") handlePaymentReceived(data);
}
```

### Good Code (Fix)
```cpp
struct DatabaseConfig {
    std::string host;
    int port;
    std::string name;
};

struct AppConfig {
    DatabaseConfig database;
    std::string redisUrl;
    std::string apiKey;
    std::string logLevel;
};

void setupApp(const AppConfig& config) {
    connectDb(config.database.host, config.database.port, config.database.name);
    connectRedis(config.redisUrl);
    initLogger(config.logLevel);
}

enum class EventType {
    UserCreated,
    UserDeleted,
    OrderPlaced,
    OrderShipped,
    PaymentReceived
};

void dispatchEvent(EventType event, void* data) {
    switch (event) {
    case EventType::UserCreated:     handleUserCreated(data);     break;
    case EventType::UserDeleted:     handleUserDeleted(data);     break;
    case EventType::OrderPlaced:     handleOrderPlaced(data);     break;
    case EventType::OrderShipped:    handleOrderShipped(data);    break;
    case EventType::PaymentReceived: handlePaymentReceived(data); break;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression` with `field_expression` (`.at()`, `.find()`, `[]`), `string_literal` argument
- **Detection approach**: Find repeated `.at("key")` or `.find("key")` or `operator[]("key")` calls on the same map variable where string literals are used as arguments. Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    argument: (identifier) @obj
    field: (field_identifier) @method)
  arguments: (argument_list
    (string_literal) @key))

(subscript_expression
  argument: (identifier) @obj
  index: (string_literal) @key)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
