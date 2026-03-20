# Module Size Distribution -- Java

## Overview
Module size distribution measures how symbol definitions are distributed across source files in a Java codebase. Java convention mandates one public class per file, so size issues typically manifest as classes with too many methods and inner types, or as files containing excessive non-public helper classes. Balanced file sizes keep classes focused, improve navigability, and reduce the cognitive load of code reviews.

## Why It's an Architecture Concern
Oversized Java files typically indicate god classes that have accumulated too many methods and responsibilities over time. Because Java enforces one public class per file, a bloated file means the class itself needs decomposition -- it likely violates the single responsibility principle and has become a dependency magnet. Changes to any method in the class risk affecting unrelated functionality. Anemic modules containing a single trivial type (like a constants-only class or a marker interface) add unnecessary file system overhead and force developers to navigate more files without gaining meaningful organizational benefit.

## Applicability
- **Relevance**: high
- **Languages covered**: `.java`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration. In Java, this primarily means a class with too many methods and inner types.

### Bad Code (Anti-pattern)
```java
// UserManager.java -- god class with too many responsibilities
public class UserManager {
    public void createUser(String name) { /* ... */ }
    public void deleteUser(int id) { /* ... */ }
    public void updateUser(int id, String name) { /* ... */ }
    public User findUser(int id) { /* ... */ }
    public List<User> searchUsers(String query) { /* ... */ }
    public void sendEmail(int userId, String subject) { /* ... */ }
    public void resetPassword(int userId) { /* ... */ }
    public void validateEmail(String email) { /* ... */ }
    public void exportToCsv(List<User> users) { /* ... */ }
    public void importFromCsv(String path) { /* ... */ }
    public void syncWithLdap() { /* ... */ }
    public void generateReport() { /* ... */ }
    // ... 20 more methods covering notifications, caching, auditing, etc.

    class UserCache { /* ... */ }
    class UserValidator { /* ... */ }
    enum UserStatus { ACTIVE, INACTIVE, SUSPENDED }
}
```

### Good Code (Fix)
```java
// UserRepository.java -- focused on persistence
public class UserRepository {
    public void create(String name) { /* ... */ }
    public void delete(int id) { /* ... */ }
    public void update(int id, String name) { /* ... */ }
    public User findById(int id) { /* ... */ }
    public List<User> search(String query) { /* ... */ }
}
```

```java
// UserNotificationService.java -- focused on notifications
public class UserNotificationService {
    public void sendEmail(int userId, String subject) { /* ... */ }
    public void resetPassword(int userId) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `interface_declaration`, `enum_declaration`, `record_declaration`, `annotation_type_declaration`, `method_declaration`, `field_declaration`, `constructor_declaration`
- **Detection approach**: Count top-level type declarations and their member definitions (methods, fields, constructors, inner types). Flag if total count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(program
  [
    (class_declaration name: (identifier) @name) @def
    (interface_declaration name: (identifier) @name) @def
    (enum_declaration name: (identifier) @name) @def
    (record_declaration name: (identifier) @name) @def
    (annotation_type_declaration name: (identifier) @name) @def
  ])

(class_body
  [
    (method_declaration name: (identifier) @name) @def
    (field_declaration) @def
    (constructor_declaration name: (identifier) @name) @def
    (class_declaration name: (identifier) @name) @def
  ])
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
```java
// ServiceFacade.java -- too many public methods
public class ServiceFacade {
    public void createUser(String name) { /* ... */ }
    public void deleteUser(int id) { /* ... */ }
    public void createOrder(int userId) { /* ... */ }
    public void cancelOrder(int orderId) { /* ... */ }
    public void processPayment(int orderId) { /* ... */ }
    public void refundPayment(int paymentId) { /* ... */ }
    public void sendNotification(int userId, String msg) { /* ... */ }
    public void generateInvoice(int orderId) { /* ... */ }
    public void updateInventory(int productId, int qty) { /* ... */ }
    public void syncExternalSystem() { /* ... */ }
    public Report generateDailyReport() { /* ... */ }
    public Report generateMonthlyReport() { /* ... */ }
    // ... 10 more public methods
}
```

### Good Code (Fix)
```java
// UserService.java -- focused service
public class UserService {
    public void createUser(String name) { /* ... */ }
    public void deleteUser(int id) { /* ... */ }
}
```

```java
// OrderService.java -- focused service
public class OrderService {
    public void createOrder(int userId) { /* ... */ }
    public void cancelOrder(int orderId) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `field_declaration`, `constructor_declaration`, `class_declaration`, `interface_declaration`, `enum_declaration`
- **Detection approach**: Count members with a `public` modifier inside the `modifiers` wrapper node. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(method_declaration
  (modifiers
    (modifier) @mod
    (#eq? @mod "public"))
  name: (identifier) @name) @def

(class_declaration
  (modifiers
    (modifier) @mod
    (#eq? @mod "public"))
  name: (identifier) @name) @def
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
```java
// AppConstants.java
public class AppConstants {
    public static final int MAX_RETRIES = 5;
}
```

### Good Code (Fix)
```java
// AppConfig.java -- merge the trivial constants class into a related module
public class AppConfig {
    public static final int MAX_RETRIES = 5;
    public static final int TIMEOUT_MS = 30000;
    public static final String DEFAULT_LOCALE = "en-US";

    public Settings load(String path) { /* ... */ }
    public void validate(Settings settings) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration`, `interface_declaration`, `enum_declaration`, `record_declaration`, `annotation_type_declaration`
- **Detection approach**: Count top-level type declarations (direct children of `program`). Flag if count == 1 and the type has very few members (e.g., <= 2). Exclude test files and entry point classes (containing a `public static void main` method).
- **S-expression query sketch**:
```scheme
(program
  [
    (class_declaration name: (identifier) @name) @def
    (interface_declaration name: (identifier) @name) @def
    (enum_declaration name: (identifier) @name) @def
    (record_declaration name: (identifier) @name) @def
    (annotation_type_declaration name: (identifier) @name) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
