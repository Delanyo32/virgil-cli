# Type Safety Gaps -- C#

## Overview
C# has a strong type system with nullable reference types (NRTs) introduced in C# 8.0, but codebases that do not enable NRTs or fail to add null checks on reference parameters risk `NullReferenceException` at runtime. These gaps undermine the compiler's ability to catch null-related bugs at compile time.

## Why It's a Tech Debt Concern
`NullReferenceException` is the most common runtime exception in C# applications. Without nullable reference type annotations, the compiler cannot distinguish between parameters that may be null and those that should never be null, allowing null values to flow unchecked through the call chain. Missing null checks on reference parameters in public/internal methods means invalid null arguments crash deep inside the implementation rather than failing fast at the API boundary with a clear error message.

## Applicability
- **Relevance**: high (null safety is critical in all C# codebases)
- **Languages covered**: `.cs`
- **Frameworks/libraries**: All .NET codebases; ASP.NET Core, Entity Framework, WPF, Blazor

---

## Pattern 1: Not Using Nullable Reference Types

### Description
Reference type parameters, return types, and fields declared without nullable annotations (`?`) in codebases that have not enabled the `<Nullable>enable</Nullable>` project setting, or that suppress nullable warnings. Without NRTs, the compiler treats all reference types as implicitly nullable, providing no compile-time protection against null dereferences.

### Bad Code (Anti-pattern)
```csharp
// No #nullable enable directive, no nullable annotations
public class UserService
{
    private readonly IUserRepository _repository;
    private readonly ILogger _logger;

    public User GetUser(string id)
    {
        var user = _repository.FindById(id);
        return user;  // Could be null, caller has no indication
    }

    public string GetDisplayName(User user)
    {
        return user.FirstName + " " + user.LastName;  // NRE if user is null
    }

    public List<User> SearchUsers(string query, UserFilter filter)
    {
        // Both query and filter could be null, no compiler warning
        var results = _repository.Search(query, filter.Category);
        return results;
    }
}
```

### Good Code (Fix)
```csharp
#nullable enable

public class UserService
{
    private readonly IUserRepository _repository;
    private readonly ILogger _logger;

    public User? GetUser(string id)
    {
        var user = _repository.FindById(id);
        return user;  // Caller knows this may be null
    }

    public string GetDisplayName(User user)
    {
        // Compiler enforces that 'user' is non-null here
        return user.FirstName + " " + user.LastName;
    }

    public List<User> SearchUsers(string query, UserFilter? filter)
    {
        var category = filter?.Category ?? "all";
        var results = _repository.Search(query, category);
        return results;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `parameter`, `type_identifier`, `nullable_type`, `method_declaration`
- **Detection approach**: Find `method_declaration` nodes with `public` or `internal` visibility. For each parameter, check whether the type is a reference type (class, interface, string, array) without a `nullable_type` wrapper (`?` suffix). Also check return types. Flag reference-type parameters and returns that are not marked nullable in files lacking a `#nullable enable` directive. The absence of any `nullable_type` nodes in a file is a strong signal that NRTs are not enabled.
- **S-expression query sketch**:
```scheme
(method_declaration
  returns: (type_identifier) @return_type
  name: (identifier) @method_name
  parameters: (parameter_list
    (parameter
      type: (type_identifier) @param_type
      name: (identifier) @param_name)))
```

### Pipeline Mapping
- **Pipeline name**: `null_reference_risk`
- **Pattern name**: `missing_nullable_annotations`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Missing Null Checks on Reference Parameters

### Description
Public and internal methods that accept reference-type parameters without validating them for null at the method entry point. When null is passed, the `NullReferenceException` occurs deep inside the method body rather than at the boundary, making the error harder to diagnose.

### Bad Code (Anti-pattern)
```csharp
public class OrderService
{
    public decimal CalculateTotal(Order order)
    {
        // No null check -- NRE thrown on order.Items access
        decimal total = 0;
        foreach (var item in order.Items)
        {
            total += item.Price * item.Quantity;
        }
        return total;
    }

    public void ProcessPayment(Customer customer, PaymentInfo payment)
    {
        // No null checks -- NRE on customer.Email or payment.CardNumber
        var receipt = _gateway.Charge(payment.CardNumber, payment.Amount);
        _emailService.Send(customer.Email, receipt);
    }

    public string FormatAddress(Address address)
    {
        return $"{address.Street}, {address.City}, {address.State} {address.Zip}";
    }
}
```

### Good Code (Fix)
```csharp
public class OrderService
{
    public decimal CalculateTotal(Order order)
    {
        ArgumentNullException.ThrowIfNull(order);
        decimal total = 0;
        foreach (var item in order.Items)
        {
            total += item.Price * item.Quantity;
        }
        return total;
    }

    public void ProcessPayment(Customer customer, PaymentInfo payment)
    {
        ArgumentNullException.ThrowIfNull(customer);
        ArgumentNullException.ThrowIfNull(payment);
        var receipt = _gateway.Charge(payment.CardNumber, payment.Amount);
        _emailService.Send(customer.Email, receipt);
    }

    public string FormatAddress(Address address)
    {
        ArgumentNullException.ThrowIfNull(address);
        return $"{address.Street}, {address.City}, {address.State} {address.Zip}";
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `parameter`, `type_identifier`, `if_statement`, `invocation_expression`
- **Detection approach**: Find `public` or `internal` `method_declaration` nodes with reference-type parameters. Check whether the method body starts with null checks (`if (param == null)`, `if (param is null)`, `ArgumentNullException.ThrowIfNull(param)`, or null-coalescing throw `param ?? throw`). Flag methods where reference-type parameters lack corresponding null guards in the first few statements of the body.
- **S-expression query sketch**:
```scheme
(method_declaration
  (modifier) @visibility
  returns: (_) @return_type
  name: (identifier) @method_name
  parameters: (parameter_list
    (parameter
      type: (type_identifier) @param_type
      name: (identifier) @param_name))
  body: (block) @body)
```

### Pipeline Mapping
- **Pipeline name**: `null_reference_risk`
- **Pattern name**: `missing_null_guard`
- **Severity**: warning
- **Confidence**: medium
