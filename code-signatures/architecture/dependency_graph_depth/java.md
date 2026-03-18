# Dependency Graph Depth -- Java

## Overview
Dependency graph depth measures how many layers of package imports and class references a Java source file must traverse before all dependencies are resolved. In Java, deep dependency chains manifest as deeply nested package hierarchies and facade classes that aggregate sub-package functionality, adding layers of abstraction that increase coupling and make the codebase harder to navigate.

## Why It's an Architecture Concern
Deep dependency chains in Java increase the blast radius of changes -- restructuring a package nested several layers deep ripples across every class that imports from it. Facade classes that delegate to sub-packages create a false sense of encapsulation while adding an indirection layer that must be maintained in lockstep with the underlying implementations. Java's verbose package naming conventions can mask excessive nesting, normalizing paths like `com.company.project.module.sub.internal.impl` that signal over-engineered layering. Keeping package hierarchies shallow and imports direct reduces coupling and simplifies refactoring.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.java`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In Java, the barrel file pattern manifests as facade classes that import from many sub-packages and expose thin delegation methods, acting as a single entry point to a subsystem. While the Facade pattern has legitimate uses, files that do nothing but delegate every call to a different sub-package class add a maintenance burden and obscure the real dependency graph.

### Bad Code (Anti-pattern)
```java
// com/myapp/services/ServiceFacade.java
package com.myapp.services;

import com.myapp.services.auth.AuthService;
import com.myapp.services.billing.BillingService;
import com.myapp.services.email.EmailService;
import com.myapp.services.reporting.ReportService;
import com.myapp.services.storage.StorageService;
import com.myapp.services.users.UserService;

public class ServiceFacade {
    public static void authenticate(String token) { AuthService.validate(token); }
    public static void charge(String cardId) { BillingService.charge(cardId); }
    public static void sendEmail(String to) { EmailService.send(to); }
    public static void generateReport(int id) { ReportService.generate(id); }
    public static void uploadFile(byte[] data) { StorageService.upload(data); }
    public static void createUser(String name) { UserService.create(name); }
}
```

### Good Code (Fix)
```java
// com/myapp/api/PaymentController.java
package com.myapp.api;

import com.myapp.services.auth.AuthService;
import com.myapp.services.billing.BillingService;

public class PaymentController {
    public void processPayment(String token, String cardId) {
        AuthService.validate(token);
        BillingService.charge(cardId);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`, `method_declaration`, `class_declaration`
- **Detection approach**: Identify classes where the majority of methods are single-statement delegations to imported classes. Flag if the file has >= 5 import declarations from sibling sub-packages and most methods consist of a single expression statement delegating to an imported type. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture import declarations
(import_declaration
  (scoped_identifier) @import_path) @import_decl

;; Capture single-statement delegation methods
(method_declaration
  name: (identifier) @method_name
  body: (block
    (expression_statement
      (method_invocation
        object: (identifier) @delegate_target
        name: (identifier) @delegate_method)))) @method
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In Java this appears as import statements with many dot-separated package segments beyond the conventional root (e.g., beyond `com.company.project`).

### Bad Code (Anti-pattern)
```java
package com.myapp.presentation.api.controllers.v2;

import com.myapp.infrastructure.persistence.repositories.abstractions.IOrderRepository;
import com.myapp.infrastructure.persistence.repositories.implementations.OrderRepositoryImpl;
import com.myapp.domain.aggregates.orders.valueobjects.OrderStatus;
import com.myapp.application.services.orders.handlers.commands.CreateOrderHandler;

public class OrderController {
    private final IOrderRepository repo;
    private final CreateOrderHandler handler;

    public OrderController(IOrderRepository repo, CreateOrderHandler handler) {
        this.repo = repo;
        this.handler = handler;
    }
}
```

### Good Code (Fix)
```java
package com.myapp.api.controllers;

import com.myapp.persistence.IOrderRepository;
import com.myapp.domain.orders.OrderStatus;
import com.myapp.services.CreateOrderHandler;

public class OrderController {
    private final IOrderRepository repo;
    private final CreateOrderHandler handler;

    public OrderController(IOrderRepository repo, CreateOrderHandler handler) {
        this.repo = repo;
        this.handler = handler;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`, `scoped_identifier`
- **Detection approach**: Parse the fully qualified import path and count dot-separated segments. Subtract the conventional prefix depth (e.g., 3 for `com.company.project`). Flag if the remaining sub-package depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
(import_declaration
  (scoped_identifier) @import_path) @import_decl
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
