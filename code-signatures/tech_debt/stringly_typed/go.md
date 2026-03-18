# Stringly Typed -- Go

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .go

---

## Pattern 1: String Comparisons for State/Status

### Description
Using string equality checks to determine state transitions, roles, or status instead of `type Status int` with `iota` constants or a typed string constant group.

### Bad Code (Anti-pattern)
```go
func processOrder(order *Order) error {
    if order.Status == "active" {
        return startFulfillment(order)
    } else if order.Status == "pending" {
        return notifyCustomer(order)
    } else if order.Status == "cancelled" {
        return refundPayment(order)
    } else if order.Status == "shipped" {
        return trackDelivery(order)
    } else if order.Status == "delivered" {
        return requestReview(order)
    }
    return fmt.Errorf("unknown status: %s", order.Status)
}

func getStatusColor(status string) string {
    switch status {
    case "active":
        return "green"
    case "pending":
        return "yellow"
    case "cancelled":
        return "red"
    case "shipped":
        return "blue"
    case "delivered":
        return "gray"
    default:
        return "white"
    }
}
```

### Good Code (Fix)
```go
type Status int

const (
    StatusActive    Status = iota
    StatusPending
    StatusCancelled
    StatusShipped
    StatusDelivered
)

func processOrder(order *Order) error {
    switch order.Status {
    case StatusActive:
        return startFulfillment(order)
    case StatusPending:
        return notifyCustomer(order)
    case StatusCancelled:
        return refundPayment(order)
    case StatusShipped:
        return trackDelivery(order)
    case StatusDelivered:
        return requestReview(order)
    default:
        return fmt.Errorf("unknown status: %d", order.Status)
    }
}

func (s Status) Color() string {
    switch s {
    case StatusActive:
        return "green"
    case StatusPending:
        return "yellow"
    case StatusCancelled:
        return "red"
    case StatusShipped:
        return "blue"
    case StatusDelivered:
        return "gray"
    default:
        return "white"
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `==`), `interpreted_string_literal`, `if_statement`, `expression_switch_statement`, `expression_case`
- **Detection approach**: Find equality comparisons (`==`) where one operand is a string literal. Also detect `switch` statements where 3+ `expression_case` clauses use string literals. Flag when the same variable is compared against 3+ different string literals.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (selector_expression
    field: (field_identifier) @field)
  right: (interpreted_string_literal) @string_val)

(expression_case
  value: (expression_list
    (interpreted_string_literal) @case_string))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed_config`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys to access configuration from `map[string]string` or `map[string]interface{}` instead of typed configuration structs.

### Bad Code (Anti-pattern)
```go
func setupApp(config map[string]string) {
    dbHost := config["database_host"]
    dbPort := config["database_port"]
    dbName := config["database_name"]
    redisURL := config["redis_url"]
    apiKey := config["api_key"]
    logLevel := config["log_level"]

    connectDB(dbHost, dbPort, dbName)
    connectRedis(redisURL)
    initLogger(logLevel)
}

func dispatchEvent(event string, data interface{}) {
    switch event {
    case "user_created":
        handleUserCreated(data)
    case "user_deleted":
        handleUserDeleted(data)
    case "order_placed":
        handleOrderPlaced(data)
    case "order_shipped":
        handleOrderShipped(data)
    case "payment_received":
        handlePaymentReceived(data)
    }
}
```

### Good Code (Fix)
```go
type DatabaseConfig struct {
    Host string `json:"host"`
    Port int    `json:"port"`
    Name string `json:"name"`
}

type AppConfig struct {
    Database DatabaseConfig `json:"database"`
    RedisURL string         `json:"redis_url"`
    APIKey   string         `json:"api_key"`
    LogLevel string         `json:"log_level"`
}

func setupApp(config *AppConfig) {
    connectDB(config.Database.Host, config.Database.Port, config.Database.Name)
    connectRedis(config.RedisURL)
    initLogger(config.LogLevel)
}

type EventType int

const (
    EventUserCreated EventType = iota
    EventUserDeleted
    EventOrderPlaced
    EventOrderShipped
    EventPaymentReceived
)

func dispatchEvent(event EventType, data interface{}) {
    switch event {
    case EventUserCreated:
        handleUserCreated(data)
    case EventUserDeleted:
        handleUserDeleted(data)
    case EventOrderPlaced:
        handleOrderPlaced(data)
    case EventOrderShipped:
        handleOrderShipped(data)
    case EventPaymentReceived:
        handlePaymentReceived(data)
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `index_expression` with `interpreted_string_literal`, `identifier`
- **Detection approach**: Find repeated map access patterns where string literals are used as keys via bracket notation (`config["key"]`). Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(index_expression
  operand: (identifier) @obj
  index: (interpreted_string_literal) @key)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed_config`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
