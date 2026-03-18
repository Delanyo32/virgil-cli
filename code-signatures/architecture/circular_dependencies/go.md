# Circular Dependencies -- Go

## Overview
Circular dependencies between Go packages are a compile-time error — the Go compiler strictly prohibits import cycles. As a result, true circular imports cannot exist in a compiled Go program. However, near-cycles and excessive cross-package coupling remain architectural concerns. Packages that are tightly interconnected through many imports, or that require convoluted workarounds to avoid cycles (such as interface packages existing solely to break dependency loops), signal design problems that merit attention.

## Why It's an Architecture Concern
Although the Go compiler enforces acyclic package imports, the underlying design pressure that causes cycles in other languages still applies. Developers may resort to merging logically distinct packages, introducing artificial interface-only packages, or using `internal/` workarounds to circumvent cycle errors — all of which obscure the true dependency structure. Excessive cross-package coupling makes packages harder to test independently, increases compilation times, and indicates tangled responsibilities. Detecting near-cycle patterns (pairs of packages with high mutual interaction) helps identify modules that should be restructured before they force awkward workarounds.

## Applicability
- **Relevance**: low (compiler prevents import cycles, but near-cycle detection is still valuable)
- **Languages covered**: `.go`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently. In Go, this is a compile error, so this pattern detects near-mutual dependencies: two packages where package A imports package B and package B imports a third package C that re-exports or proxies types from A.

### Bad Code (Anti-pattern)
```go
// --- service/handler.go ---
package service

import (
    "myapp/repo"  // service imports repo
)

func ProcessOrder(id int) error {
    order, err := repo.FindOrder(id)
    if err != nil {
        return err
    }
    return repo.UpdateStatus(order, "processed")
}

// --- repo/order.go ---
package repo

// This would cause a cycle:
// import "myapp/service"  // COMPILE ERROR

// Workaround: duplicates logic from service to avoid the import
func UpdateStatus(order *Order, status string) error {
    // duplicated validation logic that belongs in service
    if status == "processed" && order.Total <= 0 {
        return fmt.Errorf("invalid order total")
    }
    order.Status = status
    return db.Save(order)
}
```

### Good Code (Fix)
```go
// --- domain/order.go --- (shared types in a leaf package)
package domain

type Order struct {
    ID     int
    Total  float64
    Status string
}

func (o *Order) ValidateForProcessing() error {
    if o.Total <= 0 {
        return fmt.Errorf("invalid order total")
    }
    return nil
}

// --- repo/order.go ---
package repo

import "myapp/domain"

func FindOrder(id int) (*domain.Order, error) { /* ... */ }
func Save(order *domain.Order) error           { /* ... */ }

// --- service/handler.go ---
package service

import (
    "myapp/domain"
    "myapp/repo"
)

func ProcessOrder(id int) error {
    order, _ := repo.FindOrder(id)
    if err := order.ValidateForProcessing(); err != nil {
        return err
    }
    order.Status = "processed"
    return repo.Save(order)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_spec`
- **Detection approach**: Per-file: extract all import paths from each Go file. Full cycle detection requires cross-file analysis — build a package-level adjacency list from imports.parquet, then look for near-cycles: pairs of packages (A, B) where A imports B and B imports a package C that A also imports heavily. Since Go prevents true cycles, focus on high mutual coupling rather than strict cycles. Per-file proxy: flag packages with unusually high cross-package import counts.
- **S-expression query sketch**:
```scheme
(import_spec
  path: (interpreted_string_literal) @import_source)
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
```go
// --- pkg/common/common.go ---
package common

import (
    "myapp/pkg/auth"
    "myapp/pkg/billing"
    "myapp/pkg/cache"
    "myapp/pkg/config"
    "myapp/pkg/db"
    "myapp/pkg/logging"
    "myapp/pkg/messaging"
)

// High fan-out (7 imports) AND high fan-in (every package above imports common)
func Init() error {
    logging.Setup()
    config.Load()
    db.Connect(config.GetDSN())
    cache.Init()
    auth.Configure(config.GetAuthConfig())
    billing.SetupStripe(config.GetStripeKey())
    messaging.Connect(config.GetMQURL())
    return nil
}

func GetLogger() *logging.Logger   { return logging.Default() }
func GetDB() *db.Connection        { return db.Default() }
```

### Good Code (Fix)
```go
// --- cmd/server/main.go --- (composition at the entry point)
package main

import (
    "myapp/pkg/auth"
    "myapp/pkg/config"
    "myapp/pkg/db"
    "myapp/pkg/logging"
)

func main() {
    cfg := config.Load()
    logger := logging.New(cfg.LogLevel)
    conn := db.Connect(cfg.DSN)
    authSvc := auth.New(cfg.AuthConfig, logger)
    // wire dependencies explicitly — no hub package needed
    server := NewServer(conn, authSvc, logger)
    server.Run()
}

// Each package has clear, unidirectional dependencies
// No shared "common" package acting as a hub
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_spec`
- **Detection approach**: Per-file: count import specs to estimate fan-out. Cross-file: query imports.parquet to count how many other files import this package (fan-in). Flag packages where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(import_spec
  path: (interpreted_string_literal) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
