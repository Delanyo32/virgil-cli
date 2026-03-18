# Coupling -- Go

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file importing from many different packages (high fan-in), indicating it depends on too many parts of the system. Go's strict unused-import rule means every import is actually used, making high import counts a reliable signal of excessive coupling. Typically a "god file" that orchestrates everything.

### Bad Code (Anti-pattern)
```go
// controllers/order_controller.go
package controllers

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"time"

	"myapp/auth"
	"myapp/billing/discount"
	"myapp/billing/payment"
	"myapp/billing/tax"
	"myapp/cache"
	"myapp/database"
	"myapp/logging"
	"myapp/models"
	"myapp/notifications/email"
	"myapp/notifications/push"
	"myapp/orders"
	"myapp/orders/validation"
	"myapp/queue"
	"myapp/users"
	"myapp/users/preferences"
	"myapp/utils"
)
```

### Good Code (Fix)
```go
// controllers/order_controller.go
package controllers

import (
	"encoding/json"
	"net/http"

	"myapp/auth"
	"myapp/logging"
	"myapp/orders"
)

type OrderController struct {
	orderService orders.Service
	logger       logging.Logger
}

func NewOrderController(svc orders.Service, logger logging.Logger) *OrderController {
	return &OrderController{orderService: svc, logger: logger}
}

func (c *OrderController) CreateOrder(w http.ResponseWriter, r *http.Request) {
	user, err := auth.Authenticate(r)
	if err != nil {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	order, err := c.orderService.Create(r.Context(), user, r.Body)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	c.logger.Info("order_created", "order_id", order.ID)
	json.NewEncoder(w).Encode(order)
}

// orders/service.go — encapsulates billing, notifications, caching
package orders

import (
	"myapp/billing"
	"myapp/notifications"
	"myapp/orders/validation"
)

type Service struct {
	billing       billing.Service
	notifications notifications.Service
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`, `import_spec`
- **Detection approach**: Count unique import paths per file. Extract the `path` string from each `import_spec` within `import_declaration` blocks. Flag files exceeding threshold (e.g., 15+ unique package imports). Distinguish between standard library imports (no dot in path) and third-party/internal imports (contain dots or module prefix).
- **S-expression query sketch**:
```scheme
;; Single import
(import_declaration
  (import_spec
    path: (interpreted_string_literal) @import_path))

;; Grouped imports
(import_declaration
  (import_spec_list
    (import_spec
      path: (interpreted_string_literal) @import_path)))

;; Aliased imports
(import_declaration
  (import_spec_list
    (import_spec
      name: (package_identifier) @alias
      path: (interpreted_string_literal) @import_path)))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more packages that import each other (directly or transitively), creating a dependency cycle. Go enforces no import cycles at compile time — the build will fail. However, detecting near-cycles and tightly coupled package pairs helps prevent developers from hitting this wall. Large interfaces that mirror concrete types across packages are a sign of hidden coupling.

### Bad Code (Anti-pattern)
```go
// models/user.go
package models

import "myapp/services"  // models depends on services

type User struct {
	ID   string
	Name string
}

func (u *User) ActiveOrders() []services.Order {
	return services.GetOrdersForUser(u.ID)  // Tight coupling to services
}


// services/order.go
package services

import "myapp/models"  // services depends on models — cycle!

type Order struct {
	ID     string
	Amount float64
}

func GetOrdersForUser(userID string) []Order {
	// ... query database
	return nil
}

func ProcessOrder(user models.User, amount float64) Order {
	return Order{Amount: amount}
}
```

### Good Code (Fix)
```go
// models/user.go — no dependency on services
package models

type User struct {
	ID   string
	Name string
}

// services/order.go — depends on models (one direction only)
package services

import "myapp/models"

type Order struct {
	ID      string
	UserID  string
	Amount  float64
}

func GetOrdersForUser(userID string) []Order {
	return nil
}

func ProcessOrder(user models.User, amount float64) Order {
	return Order{UserID: user.ID, Amount: amount}
}

// If models needs order-like behavior, define an interface in models:
// models/interfaces.go
package models

type OrderProvider interface {
	GetOrdersForUser(userID string) ([]OrderSummary, error)
}

type OrderSummary struct {
	ID     string
	Amount float64
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`, `import_spec`
- **Detection approach**: Build a directed graph of package-to-package imports by extracting import paths and mapping them to packages. Since Go forbids import cycles at compile time, focus on detecting tightly coupled package pairs (A imports B and B imports a sibling/neighbor of A) that are one refactor away from a cycle. Also detect large interfaces that duplicate concrete type methods across packages.
- **S-expression query sketch**:
```scheme
;; Collect all import paths to build dependency graph
(import_declaration
  (import_spec_list
    (import_spec
      path: (interpreted_string_literal) @import_path)))

;; Package declaration to identify current package
(package_clause
  (package_identifier) @package_name)

;; Detect interface declarations that might indicate dependency inversion needs
(type_declaration
  (type_spec
    name: (type_identifier) @type_name
    type: (interface_type) @iface))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
