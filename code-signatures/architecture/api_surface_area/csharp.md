# API Surface Area -- C#

## Overview
API surface area in C# is controlled through explicit access modifiers: `public`, `internal`, `protected`, and `private`. A well-designed assembly exposes a minimal `public` surface while keeping implementation details `internal` or `private`. Tracking the ratio of public to total members identifies classes and namespaces that over-expose their internals, creating tight coupling between assemblies and making version upgrades risky.

## Why It's an Architecture Concern
In C#, `public` types and members form the contractual surface of a library or service. A large public API means more symbols that external consumers can depend on, which directly limits the freedom to refactor, rename, or restructure internals. Every public member is a potential breaking change if modified. Exposing fields directly instead of using properties with controlled accessors makes it impossible to add validation, logging, or lazy initialization later without breaking callers. Disciplined use of `internal` and `private` keeps the public surface narrow and maintainable.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cs`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```csharp
public class OrderService
{
    public void CreateOrder(Order order) { }
    public void UpdateOrder(int id, Order order) { }
    public void DeleteOrder(int id) { }
    public void ValidateOrder(Order order) { }
    public void CalculateTotal(Order order) { }
    public void ApplyDiscount(Order order, decimal pct) { }
    public void ApplyTax(Order order) { }
    public void NotifyCustomer(Order order) { }
    public void GenerateInvoice(Order order) { }
    public void ArchiveOrder(Order order) { }
    public void RestoreOrder(int id) { }
    public List<Order> SearchOrders(string query) { return null; }
}
```

### Good Code (Fix)
```csharp
public class OrderService
{
    public void CreateOrder(Order order) { }
    public void UpdateOrder(int id, Order order) { }
    public void DeleteOrder(int id) { }
    public List<Order> SearchOrders(string query) { return null; }

    internal void ValidateOrder(Order order) { }
    private void CalculateTotal(Order order) { }
    private void ApplyDiscount(Order order, decimal pct) { }
    private void ApplyTax(Order order) { }
    private void NotifyCustomer(Order order) { }
    private void GenerateInvoice(Order order) { }
    private void ArchiveOrder(Order order) { }
    private void RestoreOrder(int id) { }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `property_declaration`, `field_declaration` with `modifier`
- **Detection approach**: Count all method, property, and field declarations within a class. A symbol is exported if it has a `public` or `internal` modifier. Flag classes where total members >= 10 and exported/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match public method declarations
(class_declaration
  name: (identifier) @class.name
  body: (declaration_list
    (method_declaration
      (modifier) @mod
      name: (identifier) @method.name
      (#eq? @mod "public"))))

;; Match all method declarations for total count
(class_declaration
  body: (declaration_list
    (method_declaration
      name: (identifier) @all.method.name)))
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
```csharp
public class UserRepository
{
    public SqlConnection Connection;
    public List<User> CachedUsers;
    public Dictionary<int, DateTime> LastAccessTimes;
    public string ConnectionString;
    public int RetryCount;
    public bool IsConnected;

    public User GetById(int id) { return null; }
    public void Save(User user) { }
}
```

### Good Code (Fix)
```csharp
public class UserRepository : IUserRepository
{
    private readonly SqlConnection _connection;
    private readonly List<User> _cachedUsers;
    private readonly Dictionary<int, DateTime> _lastAccessTimes;

    public bool IsConnected { get; private set; }

    public UserRepository(string connectionString) { }
    public User GetById(int id) { return null; }
    public void Save(User user) { }
}

public interface IUserRepository
{
    bool IsConnected { get; }
    User GetById(int id);
    void Save(User user);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` with `public` modifier inside `class_declaration`
- **Detection approach**: Find public classes that have public field declarations (not properties). Public fields expose raw storage to consumers. Flag classes with 2+ public fields alongside public methods, as this indicates leaked implementation.
- **S-expression query sketch**:
```scheme
;; Match public fields in public classes
(class_declaration
  (modifier) @class.mod
  name: (identifier) @class.name
  body: (declaration_list
    (field_declaration
      (modifier) @field.mod
      (variable_declaration
        (variable_declarator
          (identifier) @field.name)))
    (#eq? @class.mod "public")
    (#eq? @field.mod "public")))
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
