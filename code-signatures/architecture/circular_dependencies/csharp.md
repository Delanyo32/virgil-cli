# Circular Dependencies -- C#

## Overview
Circular dependencies in C# occur when two or more namespaces, classes, or projects mutually reference each other through `using` directives or project references. While the C# compiler can handle intra-assembly circular `using` statements (since namespaces within a single assembly are resolved together), cross-project circular references are a build error. Even within a single assembly, mutual namespace dependencies indicate poor separation of concerns and make refactoring difficult.

## Why It's an Architecture Concern
Circular references between namespaces or projects make modules inseparable — extracting one into a separate assembly or NuGet package becomes impossible without breaking the cycle first. They prevent independent testing because mocking or stubbing one side requires loading the other. Dependency injection containers may encounter initialization ordering issues when services in different namespaces circularly depend on each other. Cycles indicate tangled responsibilities: if namespace A uses types from B and B uses types from A, the abstraction boundary between them is illusory, making the codebase harder to maintain, deploy, and evolve.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.cs`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```csharp
// --- Services/OrderService.cs ---
using MyApp.Repositories;  // OrderService depends on Repositories

namespace MyApp.Services
{
    public class OrderService
    {
        private readonly CustomerRepository _customerRepo;

        public OrderService(CustomerRepository repo) => _customerRepo = repo;

        public void PlaceOrder(int customerId, decimal amount)
        {
            var customer = _customerRepo.GetById(customerId);
            // process order...
        }
    }
}

// --- Repositories/CustomerRepository.cs ---
using MyApp.Services;  // Repository depends on Services -- CIRCULAR

namespace MyApp.Repositories
{
    public class CustomerRepository
    {
        private readonly OrderService _orderService;

        public CustomerRepository(OrderService svc) => _orderService = svc;

        public Customer GetById(int id)
        {
            // fetches customer, then eagerly loads recent orders
            var orders = _orderService.GetRecentOrders(id);
            return new Customer { Id = id, RecentOrders = orders };
        }
    }
}
```

### Good Code (Fix)
```csharp
// --- Contracts/IOrderQuery.cs --- (shared interface breaks the cycle)
namespace MyApp.Contracts
{
    public interface IOrderQuery
    {
        List<Order> GetRecentOrders(int customerId);
    }
}

// --- Services/OrderService.cs ---
using MyApp.Contracts;
using MyApp.Repositories;

namespace MyApp.Services
{
    public class OrderService : IOrderQuery
    {
        private readonly CustomerRepository _customerRepo;
        public List<Order> GetRecentOrders(int customerId) { /* ... */ }
    }
}

// --- Repositories/CustomerRepository.cs ---
using MyApp.Contracts;  // depends on interface, not on Services

namespace MyApp.Repositories
{
    public class CustomerRepository
    {
        private readonly IOrderQuery _orderQuery;
        public CustomerRepository(IOrderQuery query) => _orderQuery = query;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `using_directive`
- **Detection approach**: Per-file: extract all `using` namespace references from each file. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each file's namespace to its used namespaces, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files whose namespace is referenced by a file that they also reference.
- **S-expression query sketch**:
```scheme
(using_directive
  (qualified_name) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `mutual_import`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Hub Module (Bidirectional)

### Description
A module with high fan-in (many dependents) AND high fan-out (many dependencies), acting as a nexus that participates in or enables dependency cycles.

### Bad Code (Anti-pattern)
```csharp
// --- Common/ServiceLocator.cs ---
using MyApp.Auth;
using MyApp.Billing;
using MyApp.Notifications;
using MyApp.Logging;
using MyApp.Data;
using MyApp.Caching;
using MyApp.Messaging;

namespace MyApp.Common
{
    // High fan-out (7 usings) AND high fan-in (every namespace above uses Common)
    public static class ServiceLocator
    {
        public static T Resolve<T>() => (T)_services[typeof(T)];
        public static void Register<T>(T instance) => _services[typeof(T)] = instance;
        private static readonly Dictionary<Type, object> _services = new();
    }
}
```

### Good Code (Fix)
```csharp
// --- Contracts/IAuthService.cs --- (focused interface)
namespace MyApp.Contracts
{
    public interface IAuthService
    {
        bool ValidateToken(string token);
    }
}

// --- Contracts/IBillingService.cs --- (focused interface)
namespace MyApp.Contracts
{
    public interface IBillingService
    {
        void ChargeCustomer(int customerId, decimal amount);
    }
}

// Use standard DI container (Microsoft.Extensions.DependencyInjection)
// Each service depends only on the interfaces it needs
// No central hub required
```

### Tree-sitter Detection Strategy
- **Target node types**: `using_directive`
- **Detection approach**: Per-file: count `using` directives to estimate fan-out. Cross-file: query imports.parquet to count how many other files reference this file's namespace (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(using_directive
  (qualified_name) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
