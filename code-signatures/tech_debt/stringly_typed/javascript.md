# Stringly Typed -- JavaScript

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx

---

## Pattern 1: String Comparisons for State/Status

### Description
Using string equality checks to determine state transitions, roles, or status instead of enums or constants.

### Bad Code (Anti-pattern)
```javascript
function processOrder(order) {
  if (order.status === "active") {
    startFulfillment(order);
  } else if (order.status === "pending") {
    notifyCustomer(order);
  } else if (order.status === "cancelled") {
    refundPayment(order);
  } else if (order.status === "shipped") {
    trackDelivery(order);
  } else if (order.status === "delivered") {
    requestReview(order);
  }
}

function getStatusColor(status) {
  switch (status) {
    case "active": return "green";
    case "pending": return "yellow";
    case "cancelled": return "red";
    case "shipped": return "blue";
    case "delivered": return "gray";
  }
}
```

### Good Code (Fix)
```typescript
const Status = {
  Active: "active",
  Pending: "pending",
  Cancelled: "cancelled",
  Shipped: "shipped",
  Delivered: "delivered",
} as const;
type Status = typeof Status[keyof typeof Status];

// Or with TypeScript enum:
// enum Status { Active = "active", Pending = "pending", ... }

function processOrder(order: { status: Status }) {
  switch (order.status) {
    case Status.Active:
      startFulfillment(order);
      break;
    case Status.Pending:
      notifyCustomer(order);
      break;
    case Status.Cancelled:
      refundPayment(order);
      break;
    case Status.Shipped:
      trackDelivery(order);
      break;
    case Status.Delivered:
      requestReview(order);
      break;
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `===` or `==`), `string` (string literal), `if_statement`, `switch_statement`, `switch_case`
- **Detection approach**: Find equality comparisons (`===`, `==`) where one operand is a string literal. Flag when the same variable is compared against 3+ different string literals (indicates an enum is appropriate). Also detect `switch_statement` cases where 3+ `switch_case` nodes use string literals.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (member_expression
    property: (property_identifier) @prop)
  operator: ["===" "=="]
  right: (string) @string_val)

(switch_case
  value: (string) @case_string)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for configuration access, event dispatch, or service lookup instead of typed accessors.

### Bad Code (Anti-pattern)
```javascript
function setupApp(config) {
  const dbHost = config["database_host"];
  const dbPort = config["database_port"];
  const dbName = config["database_name"];
  const redisUrl = config["redis_url"];
  const apiKey = config["api_key"];
  const logLevel = config["log_level"];

  connectDb(dbHost, dbPort, dbName);
  connectRedis(redisUrl);
  initLogger(logLevel);
}

class EventBus {
  emit(event, data) {
    if (event === "user:created") { /* ... */ }
    if (event === "user:deleted") { /* ... */ }
    if (event === "order:placed") { /* ... */ }
    if (event === "order:shipped") { /* ... */ }
    if (event === "payment:received") { /* ... */ }
  }
}
```

### Good Code (Fix)
```typescript
interface AppConfig {
  database: {
    host: string;
    port: number;
    name: string;
  };
  redis: { url: string };
  apiKey: string;
  logLevel: "debug" | "info" | "warn" | "error";
}

function setupApp(config: AppConfig) {
  connectDb(config.database.host, config.database.port, config.database.name);
  connectRedis(config.redis.url);
  initLogger(config.logLevel);
}

const Events = {
  UserCreated: "user:created",
  UserDeleted: "user:deleted",
  OrderPlaced: "order:placed",
  OrderShipped: "order:shipped",
  PaymentReceived: "payment:received",
} as const;
type EventName = typeof Events[keyof typeof Events];

class EventBus {
  emit(event: EventName, data: unknown) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `subscript_expression` with `string` literal, `member_expression`
- **Detection approach**: Find repeated access patterns where string literals are used as keys to the same map/dict/object via bracket notation (`config["key"]`). Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(subscript_expression
  object: (identifier) @obj
  index: (string) @key)
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
