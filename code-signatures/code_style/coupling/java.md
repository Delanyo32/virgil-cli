# Coupling -- Java

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single class importing from many different packages (high fan-in), indicating it depends on too many parts of the system. Spring dependency injection can hide coupling by making it implicit rather than visible in imports, but the underlying problem remains. Wildcard imports (`import java.util.*`) mask the true breadth of coupling.

### Bad Code (Anti-pattern)
```java
// controllers/OrderController.java
package com.myapp.controllers;

import com.myapp.auth.AuthService;
import com.myapp.auth.TokenValidator;
import com.myapp.users.UserService;
import com.myapp.users.PreferencesService;
import com.myapp.orders.OrderService;
import com.myapp.orders.OrderValidator;
import com.myapp.billing.TaxService;
import com.myapp.billing.PaymentGateway;
import com.myapp.billing.DiscountEngine;
import com.myapp.notifications.EmailService;
import com.myapp.notifications.PushService;
import com.myapp.logging.EventLogger;
import com.myapp.analytics.Tracker;
import com.myapp.cache.CacheManager;
import com.myapp.queue.JobQueue;
import com.myapp.utils.CurrencyFormatter;
import com.myapp.database.ConnectionPool;
import javax.servlet.http.HttpServletRequest;
import javax.servlet.http.HttpServletResponse;
```

### Good Code (Fix)
```java
// controllers/OrderController.java
package com.myapp.controllers;

import com.myapp.auth.AuthService;
import com.myapp.orders.OrderService;
import com.myapp.logging.EventLogger;
import javax.servlet.http.HttpServletRequest;
import javax.servlet.http.HttpServletResponse;

public class OrderController {
    private final OrderService orderService;
    private final AuthService authService;
    private final EventLogger logger;

    public OrderController(OrderService orderService, AuthService authService, EventLogger logger) {
        this.orderService = orderService;
        this.authService = authService;
        this.logger = logger;
    }

    public void createOrder(HttpServletRequest req, HttpServletResponse resp) {
        var user = authService.authenticate(req);
        var order = orderService.create(user, parseBody(req));
        logger.logEvent("order_created", Map.of("orderId", order.getId()));
    }
}

// orders/OrderService.java — encapsulates billing, notifications
package com.myapp.orders;

import com.myapp.billing.BillingService;
import com.myapp.notifications.NotificationService;

public class OrderService {
    private final BillingService billing;
    private final NotificationService notifications;

    public OrderService(BillingService billing, NotificationService notifications) {
        this.billing = billing;
        this.notifications = notifications;
    }

    public Order create(User user, OrderData data) {
        OrderValidator.validate(data);
        var total = billing.processOrder(data);
        notifications.sendOrderConfirmation(user, total);
        return new Order(data, total);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`
- **Detection approach**: Count unique package prefixes per file. Extract the full qualified name from each `import_declaration` and group by top-level package (e.g., `com.myapp.billing`). Flag files exceeding threshold (e.g., 15+ unique package imports). Exclude standard library (`java.`, `javax.`) from internal coupling counts. Flag wildcard imports (`import pkg.*`) separately as they hide true coupling breadth.
- **S-expression query sketch**:
```scheme
;; Regular imports
(import_declaration
  (scoped_identifier) @import_path)

;; Wildcard imports
(import_declaration
  (asterisk) @wildcard)

;; Static imports
(import_declaration
  "static"
  (scoped_identifier) @static_import)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more classes/packages that import each other (directly or transitively), creating a dependency cycle. Java allows circular dependencies between classes in different packages (unlike Go), but they prevent independent compilation of modules, complicate testing, and indicate poor architecture. Spring's dependency injection can mask cycles that surface as `BeanCurrentlyInCreationException` at runtime.

### Bad Code (Anti-pattern)
```java
// models/User.java
package com.myapp.models;

import com.myapp.services.OrderService;  // models depends on services

public class User {
    private String id;
    private String name;

    public List<Order> getActiveOrders() {
        return OrderService.getInstance().getOrdersForUser(this.id);  // Tight coupling
    }
}

// services/OrderService.java
package com.myapp.services;

import com.myapp.models.User;  // services depends on models — circular

public class OrderService {
    private static OrderService instance;

    public static OrderService getInstance() { return instance; }

    public Order processOrder(User user, double amount) {
        return new Order(user, amount);
    }

    public List<Order> getOrdersForUser(String userId) {
        return List.of();
    }
}
```

### Good Code (Fix)
```java
// models/User.java — no dependency on services
package com.myapp.models;

public class User {
    private String id;
    private String name;

    public String getId() { return id; }
    public String getName() { return name; }
}

// models/OrderRepository.java — interface in models package
package com.myapp.models;

public interface OrderRepository {
    List<Order> getOrdersForUser(String userId);
}

// services/OrderService.java — implements interface, depends on models (one direction)
package com.myapp.services;

import com.myapp.models.Order;
import com.myapp.models.OrderRepository;
import com.myapp.models.User;

public class OrderService implements OrderRepository {
    @Override
    public List<Order> getOrdersForUser(String userId) {
        return List.of();
    }

    public Order processOrder(User user, double amount) {
        return new Order(user.getId(), amount);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`
- **Detection approach**: Build a directed graph of package-to-package imports by extracting the package prefix from each import path. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Also detect classes that reference each other's concrete types rather than interfaces, which is a precursor to tighter coupling.
- **S-expression query sketch**:
```scheme
;; Collect all import paths to build dependency graph
(import_declaration
  (scoped_identifier) @import_path)

;; Package declaration to identify current package
(package_declaration
  (scoped_identifier) @package_name)

;; Static imports also create coupling
(import_declaration
  "static"
  (scoped_identifier) @static_import_path)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
