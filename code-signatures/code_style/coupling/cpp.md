# Coupling -- C++

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file including many different headers (high fan-in), indicating it depends on too many parts of the system. In C++, `#include` directives are textual inclusion — each header transitively pulls in its own headers, creating deep dependency chains that dramatically slow compilation. Template-heavy headers amplify this problem since they cannot be forward-declared and pull in full definitions.

### Bad Code (Anti-pattern)
```cpp
// controllers/order_controller.cpp
#include "auth/auth_service.hpp"
#include "auth/token_validator.hpp"
#include "users/user_service.hpp"
#include "users/preferences_service.hpp"
#include "orders/order_service.hpp"
#include "orders/order_validator.hpp"
#include "billing/tax_service.hpp"
#include "billing/payment_gateway.hpp"
#include "billing/discount_engine.hpp"
#include "notifications/email_service.hpp"
#include "notifications/push_service.hpp"
#include "logging/event_logger.hpp"
#include "analytics/tracker.hpp"
#include "cache/cache_manager.hpp"
#include "queue/job_queue.hpp"
#include "utils/formatters.hpp"
#include "database/connection.hpp"

#include <string>
#include <vector>
#include <memory>
#include <optional>

class OrderController {
public:
    Response createOrder(const Request& req) {
        auto user = AuthService::authenticate(req);
        TokenValidator::validate(req.bearerToken());
        auto prefs = PreferencesService::get(user.id);
        OrderValidator::validate(req.body());
        auto tax = TaxService::calculate(req.amount());
        auto discount = DiscountEngine::apply(req.coupon());
        auto payment = PaymentGateway::charge(req.amount() + tax - discount);
        auto order = OrderService::create(user, req.body(), payment);
        EmailService::sendConfirmation(user, order);
        PushService::notify(user, "Order created");
        EventLogger::log("order_created", {{"order_id", order.id}});
        Tracker::track("purchase", {{"amount", std::to_string(order.total)}});
        CacheManager::invalidate("user_orders_" + user.id);
        JobQueue::enqueue("process_order", order.id);
        return Response{order};
    }
};
```

### Good Code (Fix)
```cpp
// controllers/order_controller.hpp
#pragma once

#include <memory>

// Forward declarations — minimal header coupling
class OrderService;
class AuthService;
class EventLogger;
struct Request;
struct Response;

class OrderController {
public:
    OrderController(std::shared_ptr<OrderService> orderService,
                    std::shared_ptr<AuthService> authService,
                    std::shared_ptr<EventLogger> logger);
    Response createOrder(const Request& req);

private:
    std::shared_ptr<OrderService> orderService_;
    std::shared_ptr<AuthService> authService_;
    std::shared_ptr<EventLogger> logger_;
};

// controllers/order_controller.cpp — includes only in implementation
#include "controllers/order_controller.hpp"
#include "auth/auth_service.hpp"
#include "orders/order_service.hpp"
#include "logging/event_logger.hpp"

OrderController::OrderController(
    std::shared_ptr<OrderService> orderService,
    std::shared_ptr<AuthService> authService,
    std::shared_ptr<EventLogger> logger)
    : orderService_(std::move(orderService))
    , authService_(std::move(authService))
    , logger_(std::move(logger)) {}

Response OrderController::createOrder(const Request& req) {
    auto user = authService_->authenticate(req);
    auto order = orderService_->create(user, req.body());
    logger_->log("order_created", {{"order_id", order.id}});
    return Response{order};
}

// orders/order_service.cpp — encapsulates billing, notifications
#include "orders/order_service.hpp"
#include "orders/order_validator.hpp"
#include "billing/billing_service.hpp"
#include "notifications/notification_service.hpp"

Order OrderService::create(const User& user, const OrderData& data) {
    OrderValidator::validate(data);
    auto total = billing_->processOrder(data);
    notifications_->sendOrderConfirmation(user, total);
    return Order{data, total};
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Count unique `#include` directives per file. Extract the header path from the `string_literal` or `system_lib_string` child. Flag files exceeding threshold (e.g., 15+ unique includes). Distinguish between system headers (`<header>`) and project headers (`"header"`). Headers (.hpp/.hxx/.hh) should be checked more strictly since their includes propagate to all includers.
- **S-expression query sketch**:
```scheme
;; Project-local includes
(preproc_include
  path: (string_literal) @local_include)

;; System/library includes
(preproc_include
  path: (system_lib_string) @system_include)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more files that include each other's headers (directly or transitively), creating a dependency cycle. In C++, circular includes cause compilation errors or require complex workarounds with forward declarations. Circular header dependencies dramatically increase compilation times because modifying any header in the cycle triggers recompilation of all files in the cycle. Forward declarations can break cycles for pointers/references but not for value types or templates.

### Bad Code (Anti-pattern)
```cpp
// models/user.hpp
#pragma once
#include "models/order.hpp"  // user.hpp depends on order.hpp

#include <string>
#include <vector>

class User {
public:
    std::string id;
    std::string name;
    std::vector<Order> orders;  // Needs full Order definition

    void addOrder(const Order& order) {
        orders.push_back(order);
    }

    double totalSpent() const {
        double sum = 0;
        for (const auto& o : orders) sum += o.amount;
        return sum;
    }
};

// models/order.hpp
#pragma once
#include "models/user.hpp"  // order.hpp depends on user.hpp — circular!

#include <string>

class Order {
public:
    double amount;
    User owner;  // Needs full User definition — cannot forward-declare

    Order(const User& user, double amt) : owner(user), amount(amt) {
        // Cannot call user.addOrder(*this) here — would be recursive
    }
};
```

### Good Code (Fix)
```cpp
// models/user_fwd.hpp — forward declarations
#pragma once

#include <string>

class User;
class Order;

// models/user.hpp — no dependency on order.hpp
#pragma once

#include <string>
#include <vector>

class User {
public:
    std::string id;
    std::string name;

    const std::string& getId() const { return id; }
};

// models/order.hpp — uses user ID, not full User object
#pragma once

#include <string>

class Order {
public:
    double amount;
    std::string ownerId;  // ID reference instead of full object

    Order(const std::string& ownerId, double amt)
        : ownerId(ownerId), amount(amt) {}
};

// services/order_service.hpp — composes User and Order without circular deps
#pragma once

#include "models/user.hpp"
#include "models/order.hpp"
#include <vector>

class OrderService {
public:
    Order createOrder(const User& user, double amount) {
        return Order(user.getId(), amount);
    }

    double totalSpent(const std::vector<Order>& orders) {
        double sum = 0;
        for (const auto& o : orders) sum += o.amount;
        return sum;
    }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Build a directed graph of header-to-header includes by extracting the header path from each `#include` directive. Resolve relative paths to build the full dependency graph. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Also detect forward declarations (`class Foo;`) as indicators that the developer may have already attempted to break a cycle.
- **S-expression query sketch**:
```scheme
;; Collect all include paths to build dependency graph
(preproc_include
  path: (string_literal) @include_path)

;; System includes
(preproc_include
  path: (system_lib_string) @system_path)

;; Forward declarations — may indicate existing cycle workarounds
(declaration
  type: (type_identifier) @fwd_decl_type
  (#match? @fwd_decl_type "^[A-Z]"))

;; Detect pragma once / include guards
(preproc_call
  directive: (preproc_directive) @directive
  (#eq? @directive "pragma"))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
