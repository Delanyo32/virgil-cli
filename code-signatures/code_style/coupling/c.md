# Coupling -- C

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file including many different headers (high fan-in), indicating it depends on too many parts of the system. In C, `#include` directives are textual inclusion — each header transitively pulls in its own headers, creating deep dependency chains that slow compilation and increase rebuild times. A file with many includes is often doing too much.

### Bad Code (Anti-pattern)
```c
/* controllers/order_controller.c */
#include "auth/auth_service.h"
#include "auth/token_validator.h"
#include "users/user_service.h"
#include "users/preferences.h"
#include "orders/order_service.h"
#include "orders/order_validator.h"
#include "billing/tax_service.h"
#include "billing/payment_gateway.h"
#include "billing/discount_engine.h"
#include "notifications/email_service.h"
#include "notifications/push_service.h"
#include "logging/event_logger.h"
#include "analytics/tracker.h"
#include "cache/cache_manager.h"
#include "queue/job_queue.h"
#include "utils/formatters.h"
#include "database/connection.h"

int create_order(struct request *req, struct response *resp) {
    struct user *u = authenticate(req);
    validate_token(req->token);
    struct preferences *prefs = get_user_preferences(u->id);
    validate_order(req->body);
    double tax = calculate_tax(req->amount);
    double discount = apply_discount(req->coupon);
    struct payment *p = process_payment(req->amount + tax - discount);
    struct order *o = create_order_record(u, req->body, p);
    send_confirmation_email(u, o);
    send_push_notification(u, "Order created");
    log_event("order_created", o->id);
    track_analytics("purchase", o->total);
    invalidate_cache(u->id);
    enqueue_job("process_order", o->id);
    return 0;
}
```

### Good Code (Fix)
```c
/* controllers/order_controller.c */
#include "auth/auth_service.h"
#include "orders/order_service.h"
#include "logging/event_logger.h"

int create_order(struct request *req, struct response *resp) {
    struct user *u = authenticate(req);
    struct order *o = order_service_create(u, req->body);
    log_event("order_created", o->id);
    resp->body = order_to_json(o);
    return 0;
}

/* orders/order_service.c — encapsulates billing, notifications */
#include "orders/order_service.h"
#include "orders/order_validator.h"
#include "billing/billing_service.h"
#include "notifications/notification_service.h"

struct order *order_service_create(struct user *u, struct order_data *data) {
    validate_order(data);
    double total = billing_process_order(data);
    notification_send_confirmation(u, total);
    return order_new(data, total);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Count unique `#include` directives per file. Extract the header path from the `string_literal` or `system_lib_string` child. Flag files exceeding threshold (e.g., 15+ unique includes). Distinguish between system headers (`<header.h>`) and project headers (`"header.h"`). Consider transitive includes by analyzing included headers recursively.
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
Two or more files that include each other's headers (directly or transitively), creating a dependency cycle. In C, circular includes cause compilation errors (redefinition of types) unless every header has include guards (`#ifndef`/`#define`/`#endif`). Even with guards, circular header dependencies indicate tightly coupled modules and create fragile build ordering. Forward declarations can break the cycle but are not always possible.

### Bad Code (Anti-pattern)
```c
/* models/user.h */
#ifndef USER_H
#define USER_H

#include "models/order.h"  /* user.h depends on order.h */

typedef struct user {
    char *id;
    char *name;
    struct order **orders;  /* Needs full order type */
    int order_count;
} user_t;

user_t *user_create(const char *name);
double user_total_spent(user_t *u);

#endif

/* models/order.h */
#ifndef ORDER_H
#define ORDER_H

#include "models/user.h"  /* order.h depends on user.h — circular */

typedef struct order {
    double amount;
    user_t *owner;  /* Needs full user type */
} order_t;

order_t *order_create(user_t *user, double amount);

#endif
```

### Good Code (Fix)
```c
/* models/types.h — forward declarations, breaks the cycle */
#ifndef TYPES_H
#define TYPES_H

typedef struct user user_t;
typedef struct order order_t;

#endif

/* models/user.h — uses forward declaration for order */
#ifndef USER_H
#define USER_H

#include "models/types.h"

struct user {
    char *id;
    char *name;
    order_t **orders;  /* Pointer only — forward declaration sufficient */
    int order_count;
};

user_t *user_create(const char *name);

#endif

/* models/order.h — uses forward declaration for user */
#ifndef ORDER_H
#define ORDER_H

#include "models/types.h"

struct order {
    double amount;
    char *owner_id;  /* Use ID instead of pointer to break coupling */
};

order_t *order_create(const char *owner_id, double amount);

#endif
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Build a directed graph of header-to-header includes by extracting the header path from each `#include` directive. Resolve relative paths to build the full dependency graph. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Also detect missing include guards (`#ifndef`/`#pragma once`) which make circular includes immediately fatal.
- **S-expression query sketch**:
```scheme
;; Collect all include paths to build dependency graph
(preproc_include
  path: (string_literal) @include_path)

;; System includes (less likely to be circular, but track for completeness)
(preproc_include
  path: (system_lib_string) @system_path)

;; Detect include guards (their absence makes circular includes fatal)
(preproc_ifdef
  name: (identifier) @guard_name)

(preproc_ifndef
  name: (identifier) @guard_name)
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
