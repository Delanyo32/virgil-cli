# Legacy API Usage -- C#

## Overview
Legacy API usage in C# refers to architectural anti-patterns and misused language features that reduce maintainability and clarity. Common examples include anemic domain models -- classes that hold data but contain no behavior -- and using exceptions for control flow instead of proper conditional logic or result types.

## Why It's a Tech Debt Concern
Anemic domain models scatter business logic across service classes, making it impossible to enforce invariants at the type level and violating encapsulation. As the codebase grows, the same validation and transformation logic gets duplicated across multiple services. Using exceptions for control flow is expensive (stack trace capture costs 1-5ms per throw), obscures the happy path, and makes it difficult to distinguish expected business outcomes from actual errors. Both patterns degrade code readability and make refactoring risky.

## Applicability
- **Relevance**: high (both patterns are extremely common in enterprise C# codebases, especially those following older "service layer" architectures)
- **Languages covered**: `.cs`
- **Frameworks/libraries**: ASP.NET, Entity Framework, MediatR (anemic models common in CQRS-without-DDD setups)

---

## Pattern 1: Anemic Domain Model

### Description
Data classes that consist entirely of auto-properties with public getters and setters but contain no methods, validation, or business logic. All behavior lives in separate "service" classes that manipulate these passive data bags, leading to scattered invariants and duplicated validation.

### Bad Code (Anti-pattern)
```csharp
// Anemic model: just a data bag
public class Order
{
    public int Id { get; set; }
    public string CustomerId { get; set; }
    public List<OrderLine> Lines { get; set; }
    public decimal Total { get; set; }
    public string Status { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime? ShippedAt { get; set; }
    public string ShippingAddress { get; set; }
    public decimal DiscountPercent { get; set; }
}

public class OrderLine
{
    public int ProductId { get; set; }
    public int Quantity { get; set; }
    public decimal UnitPrice { get; set; }
}

// All logic lives in service classes
public class OrderService
{
    public void AddLine(Order order, int productId, int quantity, decimal price)
    {
        order.Lines.Add(new OrderLine { ProductId = productId, Quantity = quantity, UnitPrice = price });
        order.Total = order.Lines.Sum(l => l.Quantity * l.UnitPrice);
        order.Total *= (1 - order.DiscountPercent / 100);
    }

    public void Ship(Order order, string trackingNumber)
    {
        if (order.Status != "Confirmed")
            throw new InvalidOperationException("Cannot ship unconfirmed order");
        if (order.Lines.Count == 0)
            throw new InvalidOperationException("Cannot ship empty order");
        order.Status = "Shipped";
        order.ShippedAt = DateTime.UtcNow;
    }

    public void Cancel(Order order)
    {
        if (order.Status == "Shipped")
            throw new InvalidOperationException("Cannot cancel shipped order");
        order.Status = "Cancelled";
    }
}
```

### Good Code (Fix)
```csharp
public class Order
{
    public int Id { get; private set; }
    public string CustomerId { get; private set; }
    private readonly List<OrderLine> _lines = new();
    public IReadOnlyList<OrderLine> Lines => _lines.AsReadOnly();
    public decimal Total { get; private set; }
    public OrderStatus Status { get; private set; }
    public DateTime CreatedAt { get; private set; }
    public DateTime? ShippedAt { get; private set; }

    public Order(string customerId)
    {
        CustomerId = customerId ?? throw new ArgumentNullException(nameof(customerId));
        Status = OrderStatus.Draft;
        CreatedAt = DateTime.UtcNow;
    }

    public void AddLine(int productId, int quantity, decimal unitPrice)
    {
        if (quantity <= 0) throw new ArgumentException("Quantity must be positive");
        if (unitPrice < 0) throw new ArgumentException("Price cannot be negative");
        _lines.Add(new OrderLine(productId, quantity, unitPrice));
        RecalculateTotal();
    }

    public void Ship()
    {
        if (Status != OrderStatus.Confirmed)
            throw new InvalidOperationException("Cannot ship unconfirmed order");
        if (_lines.Count == 0)
            throw new InvalidOperationException("Cannot ship empty order");
        Status = OrderStatus.Shipped;
        ShippedAt = DateTime.UtcNow;
    }

    public void Cancel()
    {
        if (Status == OrderStatus.Shipped)
            throw new InvalidOperationException("Cannot cancel shipped order");
        Status = OrderStatus.Cancelled;
    }

    private void RecalculateTotal()
    {
        Total = _lines.Sum(l => l.Quantity * l.UnitPrice);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `property_declaration` (auto-properties), `method_declaration`
- **Detection approach**: Find `class_declaration` nodes whose `declaration_list` body contains only `property_declaration` nodes (auto-properties with `get`/`set` accessors) and no `method_declaration` nodes. Flag classes with 5+ auto-properties and zero methods as anemic data classes. Exclude record types and DTOs (classes ending in `Dto`, `Request`, `Response`, `ViewModel`).
- **S-expression query sketch**:
```scheme
(class_declaration
  name: (identifier) @class_name
  body: (declaration_list
    (property_declaration
      accessors: (accessor_list
        (accessor_declaration) @getter
        (accessor_declaration) @setter))))
```

### Pipeline Mapping
- **Pipeline name**: `anemic_domain_model`
- **Pattern name**: `data_class_no_behavior`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Exceptions Used for Control Flow

### Description
Throwing and catching exceptions to control normal program flow -- for example, throwing a `NotFoundException` to indicate a missing record, or catching `FormatException` to validate input instead of using `TryParse`. Exceptions are expensive (stack trace capture) and obscure the distinction between expected outcomes and actual errors.

### Bad Code (Anti-pattern)
```csharp
public class UserService
{
    public User GetUserOrThrow(int id)
    {
        var user = _repository.Find(id);
        if (user == null)
            throw new NotFoundException($"User {id} not found");
        return user;
    }

    public bool IsValidAge(string input)
    {
        try
        {
            int age = int.Parse(input);
            return age >= 0 && age <= 150;
        }
        catch (FormatException)
        {
            return false;
        }
        catch (OverflowException)
        {
            return false;
        }
    }

    public decimal CalculateDiscount(string couponCode)
    {
        try
        {
            var coupon = _couponRepository.GetByCode(couponCode);
            if (coupon.ExpiresAt < DateTime.UtcNow)
                throw new ExpiredException("Coupon expired");
            return coupon.DiscountPercent;
        }
        catch (NotFoundException)
        {
            return 0m;
        }
        catch (ExpiredException)
        {
            return 0m;
        }
    }

    public void ProcessBatch(List<string> emails)
    {
        foreach (var email in emails)
        {
            try
            {
                ValidateEmailOrThrow(email);
                SendEmail(email);
            }
            catch (ValidationException)
            {
                // Skip invalid emails silently
                continue;
            }
        }
    }
}
```

### Good Code (Fix)
```csharp
public class UserService
{
    public User? FindUser(int id)
    {
        return _repository.Find(id);
    }

    public bool IsValidAge(string input)
    {
        return int.TryParse(input, out int age) && age >= 0 && age <= 150;
    }

    public decimal CalculateDiscount(string couponCode)
    {
        var coupon = _couponRepository.FindByCode(couponCode);
        if (coupon == null || coupon.ExpiresAt < DateTime.UtcNow)
            return 0m;
        return coupon.DiscountPercent;
    }

    public void ProcessBatch(List<string> emails)
    {
        foreach (var email in emails)
        {
            if (!IsValidEmail(email))
            {
                _logger.LogWarning("Skipping invalid email: {Email}", email);
                continue;
            }
            SendEmail(email);
        }
    }

    private bool IsValidEmail(string email)
    {
        return !string.IsNullOrWhiteSpace(email)
            && email.Contains('@')
            && email.Contains('.');
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `throw_statement`, `catch_clause`, `try_statement`
- **Detection approach**: Find `throw_statement` nodes inside methods that are not error-handling boundaries (e.g., not in middleware or global handlers). Flag when a `throw` and its corresponding `catch` are in the same class or the same call chain (indicating the exception is used for flow control rather than error propagation). Also flag `catch` clauses that contain only a `return` statement or `continue` -- these indicate the exception was expected, not exceptional.
- **S-expression query sketch**:
```scheme
(catch_clause
  (catch_declaration
    type: (identifier) @exception_type)
  body: (block
    (return_statement) @return_in_catch))

(try_statement
  body: (block
    (expression_statement
      (invocation_expression) @try_call))
  (catch_clause
    body: (block
      (continue_statement))))
```

### Pipeline Mapping
- **Pipeline name**: `exception_control_flow`
- **Pattern name**: `exception_for_flow_control`
- **Severity**: warning
- **Confidence**: medium
