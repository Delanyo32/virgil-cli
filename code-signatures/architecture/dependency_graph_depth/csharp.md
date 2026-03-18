# Dependency Graph Depth -- C#

## Overview
Dependency graph depth measures how many layers of namespace references and assembly dependencies a compilation unit must resolve. In C#, deep dependency chains manifest as deeply nested namespace hierarchies and type-forwarding assemblies that add indirection without adding value, making the codebase harder to navigate and more fragile to refactoring.

## Why It's an Architecture Concern
Deep dependency chains in C# increase the blast radius of changes -- renaming or restructuring a namespace buried several layers deep can ripple across dozens of projects in a solution. Type forwarding via `[assembly: TypeForwardedTo(...)]` adds invisible redirection layers that obscure where types actually live. Excessive namespace nesting creates long `using` directives that signal over-engineered layering. Keeping namespace hierarchies shallow and dependencies direct reduces coupling, simplifies IDE navigation, and makes the architecture easier to reason about.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.cs`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In C#, the barrel file pattern manifests as type forwarding assemblies or "facade" files that aggregate types from sub-namespaces using `[assembly: TypeForwardedTo(...)]` attributes or static wrapper classes that delegate every call to an inner namespace. These files add a layer of indirection without meaningful logic, making it harder to trace where functionality actually resides.

### Bad Code (Anti-pattern)
```csharp
// Facades/ServiceFacade.cs -- delegates to sub-namespaces
using MyApp.Services.Auth;
using MyApp.Services.Billing;
using MyApp.Services.Notifications;
using MyApp.Services.Reporting;
using MyApp.Services.Storage;
using MyApp.Services.Users;

namespace MyApp.Facades
{
    public static class ServiceFacade
    {
        public static void Authenticate(string token) => AuthService.Validate(token);
        public static void ChargeCard(string id) => BillingService.Charge(id);
        public static void SendEmail(string to) => NotificationService.Email(to);
        public static void GenerateReport() => ReportingService.Generate();
        public static void UploadFile(byte[] data) => StorageService.Upload(data);
        public static void CreateUser(string name) => UserService.Create(name);
    }
}
```

### Good Code (Fix)
```csharp
// Consumer.cs -- imports directly from source namespaces
using MyApp.Services.Auth;
using MyApp.Services.Billing;

namespace MyApp.Controllers
{
    public class PaymentController
    {
        public void ProcessPayment(string token, string cardId)
        {
            AuthService.Validate(token);
            BillingService.Charge(cardId);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `using_directive`, `method_declaration`, `class_declaration`
- **Detection approach**: Identify files where a class contains many short methods (single expression body) that each call into a different namespace imported at the top. Flag if the file has >= 5 `using` directives and the class consists primarily of one-line delegation methods. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture using directives for counting
(using_directive
  (qualified_name) @namespace_path) @using_stmt

;; Capture delegation methods (expression-bodied members)
(method_declaration
  name: (identifier) @method_name
  body: (arrow_expression_clause
    (invocation_expression) @delegated_call)) @method
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In C# this appears as `using` directives with many dot-separated namespace segments.

### Bad Code (Anti-pattern)
```csharp
using MyApp.Infrastructure.Persistence.Repositories.Abstractions;
using MyApp.Infrastructure.Persistence.Repositories.Implementations;
using MyApp.Domain.Aggregates.Orders.ValueObjects;
using MyApp.Application.Services.Orders.Handlers.Commands;

namespace MyApp.Presentation.Api.Controllers.V2
{
    public class OrderController
    {
        private readonly IOrderRepository _repo;
        private readonly CreateOrderHandler _handler;

        public OrderController(IOrderRepository repo, CreateOrderHandler handler)
        {
            _repo = repo;
            _handler = handler;
        }
    }
}
```

### Good Code (Fix)
```csharp
using MyApp.Persistence.Abstractions;
using MyApp.Domain.Orders;
using MyApp.Services.Orders;

namespace MyApp.Api.Controllers
{
    public class OrderController
    {
        private readonly IOrderRepository _repo;
        private readonly CreateOrderHandler _handler;

        public OrderController(IOrderRepository repo, CreateOrderHandler handler)
        {
            _repo = repo;
            _handler = handler;
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `using_directive`, `qualified_name`
- **Detection approach**: Parse the namespace in each `using` directive and count dot-separated segments. Flag if depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
(using_directive
  (qualified_name) @namespace_path) @using_stmt
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
