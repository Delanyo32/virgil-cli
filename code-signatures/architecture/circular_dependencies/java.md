# Circular Dependencies -- Java

## Overview
Circular dependencies in Java occur when two or more packages or classes mutually import each other, forming a cycle in the dependency graph. While the Java compiler can handle intra-module circular imports (classes within the same compilation unit are resolved together), circular dependencies between packages or modules indicate poor architectural layering. They complicate build tooling, prevent clean modularization with the Java Platform Module System (JPMS), and make code harder to understand and test.

## Why It's an Architecture Concern
Circular imports between packages make modules inseparable — extracting one package into a separate JAR or Maven module becomes impossible without breaking the cycle first. They prevent independent testing because each package's tests require the other package to compile. Dependency injection frameworks like Spring may encounter circular bean creation errors when services in different packages depend on each other. JPMS strictly forbids circular module dependencies, so cycles block modularization. Cycles indicate tangled responsibilities: if package A uses classes from B and B uses classes from A, the package boundary provides no meaningful encapsulation.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.java`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```java
// --- com/app/service/OrderService.java ---
package com.app.service;

import com.app.repository.OrderRepository;  // service imports repository

public class OrderService {
    private final OrderRepository repository;

    public OrderService(OrderRepository repo) {
        this.repository = repo;
    }

    public void processOrder(int orderId) {
        Order order = repository.findById(orderId);
        order.setStatus("PROCESSED");
        repository.save(order);
    }
}

// --- com/app/repository/OrderRepository.java ---
package com.app.repository;

import com.app.service.OrderService;  // repository imports service -- CIRCULAR

public class OrderRepository {
    private final OrderService orderService;

    public OrderRepository(OrderService svc) {
        this.orderService = svc;
    }

    public Order findById(int id) {
        Order order = db.query(id);
        // Eager validation via service — creates the cycle
        orderService.validate(order);
        return order;
    }
}
```

### Good Code (Fix)
```java
// --- com/app/domain/OrderValidator.java --- (shared logic extracted)
package com.app.domain;

public class OrderValidator {
    public boolean isValid(Order order) {
        return order.getTotal() > 0 && order.getStatus() != null;
    }
}

// --- com/app/repository/OrderRepository.java ---
package com.app.repository;

import com.app.domain.OrderValidator;  // depends on domain, not service

public class OrderRepository {
    private final OrderValidator validator;

    public Order findById(int id) {
        Order order = db.query(id);
        validator.isValid(order);  // no service dependency needed
        return order;
    }
}

// --- com/app/service/OrderService.java ---
package com.app.service;

import com.app.repository.OrderRepository;  // unidirectional

public class OrderService {
    private final OrderRepository repository;
    public void processOrder(int orderId) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`
- **Detection approach**: Per-file: extract all import paths from each Java file. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each file (or package) to its imported packages, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files whose package is imported by a file they also import.
- **S-expression query sketch**:
```scheme
(import_declaration
  (scoped_identifier) @import_source)
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
```java
// --- com/app/util/AppContext.java ---
package com.app.util;

import com.app.auth.AuthManager;
import com.app.billing.BillingService;
import com.app.cache.CacheManager;
import com.app.config.ConfigLoader;
import com.app.data.DatabasePool;
import com.app.logging.LogFactory;
import com.app.messaging.EventBus;

// High fan-out (7 imports) AND high fan-in (every package above imports AppContext)
public class AppContext {
    private static AppContext instance;

    public AuthManager getAuth() { return AuthManager.getInstance(); }
    public BillingService getBilling() { return new BillingService(); }
    public CacheManager getCache() { return CacheManager.getInstance(); }
    public DatabasePool getDb() { return DatabasePool.getInstance(); }
    public LogFactory getLogger() { return LogFactory.getInstance(); }
    public EventBus getEventBus() { return EventBus.getInstance(); }
}
```

### Good Code (Fix)
```java
// --- com/app/auth/AuthService.java --- (focused interface)
package com.app.auth;

public interface AuthService {
    boolean validateToken(String token);
    User getCurrentUser(String token);
}

// --- com/app/billing/BillingService.java --- (focused interface)
package com.app.billing;

public interface BillingService {
    void charge(int customerId, BigDecimal amount);
}

// Wire via dependency injection (Spring, Guice, or manual)
// Each service declares only the interfaces it needs
// No central hub class required
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`
- **Detection approach**: Per-file: count `import` declarations to estimate fan-out. Cross-file: query imports.parquet to count how many other files import classes from this file's package (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(import_declaration
  (scoped_identifier) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
