# Module Size Distribution -- C#

## Overview
Module size distribution measures how symbol definitions are distributed across source files in a C# codebase. Well-sized files align with the single responsibility principle, keep code reviews focused, and make the project navigable. Extremely large files indicate responsibility sprawl while extremely small files suggest over-fragmentation.

## Why It's an Architecture Concern
Oversized C# files that pack many classes, interfaces, and enums into a single file violate the common convention of one primary type per file. They become merge conflict hotspots, are difficult to navigate in IDEs, and make it harder to reason about which namespace or assembly owns a piece of functionality. Anemic modules containing a single trivial type add file system noise and force developers to jump between many files to understand simple workflows, reducing rather than improving clarity.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cs`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```csharp
// Utilities.cs -- kitchen sink file with unrelated types
namespace MyApp.Utilities
{
    public class StringHelper
    {
        public static string Trim(string s) { /* ... */ }
        public static string Capitalize(string s) { /* ... */ }
    }

    public class MathHelper
    {
        public static int Clamp(int val, int lo, int hi) { /* ... */ }
        public static double Lerp(double a, double b, double t) { /* ... */ }
    }

    public interface IValidator { bool Validate(object input); }
    public interface IFormatter { string Format(object data); }

    public enum LogLevel { Debug, Info, Warning, Error }
    public enum StatusCode { Ok, NotFound, ServerError }

    public delegate void EventCallback(object sender, EventArgs e);
    public struct Point { public int X; public int Y; }

    // ... 20 more types covering logging, caching, serialization, etc.
}
```

### Good Code (Fix)
```csharp
// StringHelper.cs -- focused on string operations
namespace MyApp.Utilities
{
    public class StringHelper
    {
        public static string Trim(string s) { /* ... */ }
        public static string Capitalize(string s) { /* ... */ }
    }
}
```

```csharp
// MathHelper.cs -- focused on math operations
namespace MyApp.Utilities
{
    public class MathHelper
    {
        public static int Clamp(int val, int lo, int hi) { /* ... */ }
        public static double Lerp(double a, double b, double t) { /* ... */ }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `interface_declaration`, `struct_declaration`, `enum_declaration`, `delegate_declaration`, `record_declaration`
- **Detection approach**: Count all type declarations within the file (at any nesting depth under namespace). Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
[
  (class_declaration name: (identifier) @name) @def
  (interface_declaration name: (identifier) @name) @def
  (struct_declaration name: (identifier) @name) @def
  (enum_declaration name: (identifier) @name) @def
  (delegate_declaration name: (identifier) @name) @def
  (record_declaration name: (identifier) @name) @def
]
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```csharp
// ServiceRegistry.cs -- too many public types in one file
namespace MyApp.Services
{
    public class UserService { /* ... */ }
    public class OrderService { /* ... */ }
    public class PaymentService { /* ... */ }
    public class NotificationService { /* ... */ }
    public class CacheService { /* ... */ }
    public interface IUserRepository { /* ... */ }
    public interface IOrderRepository { /* ... */ }
    public interface IPaymentGateway { /* ... */ }
    public enum PaymentMethod { CreditCard, PayPal, BankTransfer }
    public enum OrderStatus { Pending, Confirmed, Shipped, Delivered }
    public record UserDto(string Name, string Email);
    public record OrderDto(int Id, decimal Total);
    // ... 10 more public types
}
```

### Good Code (Fix)
```csharp
// UserService.cs
namespace MyApp.Services
{
    public class UserService { /* ... */ }
    public record UserDto(string Name, string Email);
}
```

```csharp
// OrderService.cs
namespace MyApp.Services
{
    public class OrderService { /* ... */ }
    public enum OrderStatus { Pending, Confirmed, Shipped, Delivered }
    public record OrderDto(int Id, decimal Total);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `interface_declaration`, `struct_declaration`, `enum_declaration`, `delegate_declaration`, `record_declaration`
- **Detection approach**: Count type declarations that have a `public` or `internal` modifier. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(class_declaration
  (modifier) @mod
  name: (identifier) @name
  (#match? @mod "^(public|internal)$")) @def

(interface_declaration
  (modifier) @mod
  name: (identifier) @name
  (#match? @mod "^(public|internal)$")) @def
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```csharp
// Constants.cs
namespace MyApp
{
    public static class Constants
    {
        public const int MaxRetries = 5;
    }
}
```

### Good Code (Fix)
```csharp
// AppConfig.cs -- merge the trivial constant class into a related module
namespace MyApp
{
    public static class AppConfig
    {
        public const int MaxRetries = 5;
        public const int TimeoutMs = 30000;
        public const string DefaultLocale = "en-US";
    }

    public class ConfigLoader
    {
        public AppSettings Load(string path) { /* ... */ }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `interface_declaration`, `struct_declaration`, `enum_declaration`, `delegate_declaration`, `record_declaration`
- **Detection approach**: Count type declarations in the file. Flag if count == 1, excluding test files (files ending in `Tests.cs` or containing test attributes) and `Program.cs` entry points.
- **S-expression query sketch**:
```scheme
[
  (class_declaration name: (identifier) @name) @def
  (interface_declaration name: (identifier) @name) @def
  (struct_declaration name: (identifier) @name) @def
  (enum_declaration name: (identifier) @name) @def
  (delegate_declaration name: (identifier) @name) @def
  (record_declaration name: (identifier) @name) @def
]
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
