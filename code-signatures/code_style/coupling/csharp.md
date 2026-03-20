# Coupling -- C#

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file importing from many different namespaces (high fan-in), indicating it depends on too many parts of the system. In C#, `using` directives at the top of a file reveal namespace-level coupling. Project references in `.csproj` files add another layer of coupling that `using` statements alone do not capture. Global using directives (C# 10+) can hide coupling by making it implicit.

### Bad Code (Anti-pattern)
```csharp
// Controllers/OrderController.cs
using MyApp.Auth;
using MyApp.Auth.Tokens;
using MyApp.Users;
using MyApp.Users.Preferences;
using MyApp.Orders;
using MyApp.Orders.Validation;
using MyApp.Billing.Tax;
using MyApp.Billing.Payments;
using MyApp.Billing.Discounts;
using MyApp.Notifications.Email;
using MyApp.Notifications.Push;
using MyApp.Logging;
using MyApp.Analytics;
using MyApp.Cache;
using MyApp.Queue;
using MyApp.Utils.Formatting;
using MyApp.Database;
using Microsoft.AspNetCore.Mvc;
```

### Good Code (Fix)
```csharp
// Controllers/OrderController.cs
using MyApp.Auth;
using MyApp.Orders;
using MyApp.Logging;
using Microsoft.AspNetCore.Mvc;

namespace MyApp.Controllers;

[ApiController]
[Route("api/orders")]
public class OrderController : ControllerBase
{
    private readonly IOrderService _orderService;
    private readonly IAuthService _authService;
    private readonly IEventLogger _logger;

    public OrderController(IOrderService orderService, IAuthService authService, IEventLogger logger)
    {
        _orderService = orderService;
        _authService = authService;
        _logger = logger;
    }

    [HttpPost]
    public async Task<IActionResult> CreateOrder([FromBody] OrderRequest request)
    {
        var user = await _authService.AuthenticateAsync(HttpContext);
        var order = await _orderService.CreateAsync(user, request);
        _logger.LogEvent("order_created", new { OrderId = order.Id });
        return Ok(order);
    }
}

// Orders/OrderService.cs — encapsulates billing, notifications
using MyApp.Billing;
using MyApp.Notifications;
using MyApp.Orders.Validation;

namespace MyApp.Orders;

public class OrderService : IOrderService
{
    private readonly IBillingService _billing;
    private readonly INotificationService _notifications;

    public OrderService(IBillingService billing, INotificationService notifications)
    {
        _billing = billing;
        _notifications = notifications;
    }

    public async Task<Order> CreateAsync(User user, OrderRequest data)
    {
        OrderValidator.Validate(data);
        var total = await _billing.ProcessOrderAsync(data);
        await _notifications.SendOrderConfirmationAsync(user, total);
        return new Order(data, total);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `using_directive`
- **Detection approach**: Count unique namespace prefixes per file. Extract the namespace identifier from each `using_directive`. Flag files exceeding threshold (e.g., 15+ unique namespace imports). Distinguish between framework namespaces (`System.`, `Microsoft.`) and project-internal namespaces. Flag `using static` directives separately as they create tighter coupling to specific types.
- **S-expression query sketch**:
```scheme
;; Regular using directives
(using_directive
  (qualified_name) @using_namespace)

;; Using with alias
(using_directive
  (name_equals (identifier) @alias)
  (qualified_name) @using_namespace)

;; Using static
(using_directive
  "static"
  (qualified_name) @static_using)

;; Global using (C# 10+)
(global_statement
  (using_directive
    (qualified_name) @global_using))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more namespaces or projects that reference each other (directly or transitively), creating a dependency cycle. C# allows circular `using` directives between namespaces within the same project, but circular project references cause build failures. Even within a project, circular namespace dependencies indicate poor architecture and make it impossible to later split into separate assemblies.

### Bad Code (Anti-pattern)
```csharp
// Models/User.cs
using MyApp.Services;  // Models depends on Services

namespace MyApp.Models;

public class User
{
    public string Id { get; set; }
    public string Name { get; set; }

    public List<Order> GetActiveOrders()
    {
        return OrderService.Instance.GetOrdersForUser(Id);  // Tight coupling
    }
}

// Services/OrderService.cs
using MyApp.Models;  // Services depends on Models — circular

namespace MyApp.Services;

public class OrderService
{
    public static OrderService Instance { get; } = new();

    public Order ProcessOrder(User user, decimal amount)
    {
        return new Order { UserId = user.Id, Amount = amount };
    }

    public List<Order> GetOrdersForUser(string userId)
    {
        return new List<Order>();
    }
}
```

### Good Code (Fix)
```csharp
// Models/User.cs — no dependency on Services
namespace MyApp.Models;

public class User
{
    public string Id { get; set; }
    public string Name { get; set; }
}

// Models/IOrderRepository.cs — interface in Models namespace
namespace MyApp.Models;

public interface IOrderRepository
{
    Task<List<Order>> GetOrdersForUserAsync(string userId);
}

// Services/OrderService.cs — implements interface, depends on Models (one direction)
using MyApp.Models;

namespace MyApp.Services;

public class OrderService : IOrderRepository
{
    public async Task<List<Order>> GetOrdersForUserAsync(string userId)
    {
        return new List<Order>();
    }

    public Order ProcessOrder(User user, decimal amount)
    {
        return new Order { UserId = user.Id, Amount = amount };
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `using_directive`, `namespace_declaration`
- **Detection approach**: Build a directed graph of namespace-to-namespace imports by extracting the namespace from each `using_directive` and the current namespace from `namespace_declaration`. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Also detect project reference cycles if `.csproj` files are available.
- **S-expression query sketch**:
```scheme
;; Collect all using directives to build dependency graph
(using_directive
  (qualified_name) @using_path)

;; Current namespace to identify the source node
(namespace_declaration
  name: (qualified_name) @namespace_name)

;; File-scoped namespace (C# 10+)
(file_scoped_namespace_declaration
  name: (qualified_name) @namespace_name)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
