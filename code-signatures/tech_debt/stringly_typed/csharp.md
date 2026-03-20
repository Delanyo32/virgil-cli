# Stringly Typed -- C#

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs

---

## Pattern 1: String Comparisons for State/Status

### Description
Using string equality checks to determine state transitions, roles, or status instead of C# `enum` types.

### Bad Code (Anti-pattern)
```csharp
public class OrderProcessor
{
    public void ProcessOrder(Order order)
    {
        if (order.Status == "active")
        {
            StartFulfillment(order);
        }
        else if (order.Status == "pending")
        {
            NotifyCustomer(order);
        }
        else if (order.Status == "cancelled")
        {
            RefundPayment(order);
        }
        else if (order.Status == "shipped")
        {
            TrackDelivery(order);
        }
        else if (order.Status == "delivered")
        {
            RequestReview(order);
        }
    }

    public string GetStatusColor(string status)
    {
        return status switch
        {
            "active" => "green",
            "pending" => "yellow",
            "cancelled" => "red",
            "shipped" => "blue",
            "delivered" => "gray",
            _ => "white",
        };
    }
}
```

### Good Code (Fix)
```csharp
public enum Status
{
    Active,
    Pending,
    Cancelled,
    Shipped,
    Delivered
}

public class OrderProcessor
{
    public void ProcessOrder(Order order)
    {
        switch (order.Status)
        {
            case Status.Active:
                StartFulfillment(order);
                break;
            case Status.Pending:
                NotifyCustomer(order);
                break;
            case Status.Cancelled:
                RefundPayment(order);
                break;
            case Status.Shipped:
                TrackDelivery(order);
                break;
            case Status.Delivered:
                RequestReview(order);
                break;
        }
    }

    public string GetStatusColor(Status status) => status switch
    {
        Status.Active => "green",
        Status.Pending => "yellow",
        Status.Cancelled => "red",
        Status.Shipped => "blue",
        Status.Delivered => "gray",
        _ => "white",
    };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` (with `==`), `string_literal`, `if_statement`, `switch_expression`, `switch_expression_arm`
- **Detection approach**: Find equality comparisons (`==`) where one operand is a string literal. Flag when the same variable is compared against 3+ different string literals across `if`/`else if` chains. Also detect `switch` expressions with 3+ arms using string literal patterns.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (member_access_expression
    name: (identifier) @prop)
  operator: "=="
  right: (string_literal) @string_val)

(switch_expression_arm
  pattern: (constant_pattern
    (string_literal) @arm_string))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for configuration access via `Dictionary<string, string>` or `IConfiguration["key"]` instead of strongly-typed options classes bound via `IOptions<T>`.

### Bad Code (Anti-pattern)
```csharp
public class AppInitializer
{
    public void Setup(Dictionary<string, string> config)
    {
        var dbHost = config["database_host"];
        var dbPort = config["database_port"];
        var dbName = config["database_name"];
        var redisUrl = config["redis_url"];
        var apiKey = config["api_key"];
        var logLevel = config["log_level"];

        ConnectDb(dbHost, int.Parse(dbPort), dbName);
        ConnectRedis(redisUrl);
        InitLogger(logLevel);
    }

    public void DispatchEvent(string eventName, object data)
    {
        switch (eventName)
        {
            case "user_created": HandleUserCreated(data); break;
            case "user_deleted": HandleUserDeleted(data); break;
            case "order_placed": HandleOrderPlaced(data); break;
            case "order_shipped": HandleOrderShipped(data); break;
            case "payment_received": HandlePaymentReceived(data); break;
        }
    }
}
```

### Good Code (Fix)
```csharp
public record DatabaseConfig(string Host, int Port, string Name);
public record AppConfig(DatabaseConfig Database, string RedisUrl, string ApiKey, string LogLevel);

public class AppInitializer
{
    public void Setup(AppConfig config)
    {
        ConnectDb(config.Database.Host, config.Database.Port, config.Database.Name);
        ConnectRedis(config.RedisUrl);
        InitLogger(config.LogLevel);
    }
}

public enum EventType
{
    UserCreated, UserDeleted, OrderPlaced, OrderShipped, PaymentReceived
}

public class EventDispatcher
{
    public void Dispatch(EventType eventType, object data)
    {
        switch (eventType)
        {
            case EventType.UserCreated: HandleUserCreated(data); break;
            case EventType.UserDeleted: HandleUserDeleted(data); break;
            case EventType.OrderPlaced: HandleOrderPlaced(data); break;
            case EventType.OrderShipped: HandleOrderShipped(data); break;
            case EventType.PaymentReceived: HandlePaymentReceived(data); break;
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `element_access_expression` with `string_literal`, `identifier`
- **Detection approach**: Find repeated dictionary/indexer access patterns where string literals are used as keys via bracket notation (`config["key"]`). Flag when 5+ different string keys access the same variable.
- **S-expression query sketch**:
```scheme
(element_access_expression
  expression: (identifier) @obj
  subscript: (bracket_argument_list
    (argument
      (string_literal) @key)))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
