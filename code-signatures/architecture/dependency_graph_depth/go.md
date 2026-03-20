# Dependency Graph Depth -- Go

## Overview
Dependency graph depth measures how many layers of package imports a Go source file must traverse before all dependencies are resolved. In Go, deep import chains typically surface as long module paths and wrapper packages that delegate to sub-packages without adding meaningful logic, increasing cognitive load and making the dependency tree harder to reason about.

## Why It's an Architecture Concern
Deep dependency chains in Go increase the blast radius of changes -- restructuring a package buried several layers deep forces updates in every transitive consumer. Wrapper packages that merely re-export sub-package functions add indirection without value, obscuring where logic actually lives. Go's explicit import model makes deep chains visible, but that visibility becomes a liability when developers must mentally trace through four or five levels of packages to understand a call path. Keeping the package hierarchy flat and dependencies direct aligns with Go's philosophy of simplicity.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.go`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In Go, the barrel file pattern manifests as wrapper packages that import sub-packages and re-export their functions by wrapping every call in a thin delegation function. Since Go lacks re-export syntax, these wrappers are actual functions that just call through to the real implementation, adding a layer of indirection without meaningful logic.

### Bad Code (Anti-pattern)
```go
// pkg/services/services.go -- wrapper package delegating to sub-packages
package services

import (
	"myapp/pkg/services/auth"
	"myapp/pkg/services/billing"
	"myapp/pkg/services/notifications"
	"myapp/pkg/services/reporting"
	"myapp/pkg/services/storage"
)

func Authenticate(token string) error     { return auth.Validate(token) }
func ChargeCard(cardID string) error       { return billing.Charge(cardID) }
func SendEmail(to, body string) error      { return notifications.Email(to, body) }
func GenerateReport(id int) ([]byte, error) { return reporting.Generate(id) }
func UploadFile(data []byte) (string, error) { return storage.Upload(data) }
```

### Good Code (Fix)
```go
// cmd/api/handler.go -- imports directly from source packages
package main

import (
	"myapp/pkg/services/auth"
	"myapp/pkg/services/billing"
)

func handlePayment(token, cardID string) error {
	if err := auth.Validate(token); err != nil {
		return err
	}
	return billing.Charge(cardID)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_declaration`, `import_spec`, `function_declaration`
- **Detection approach**: Identify files where the majority of exported functions are single-expression delegations to imported packages. Flag if the file has >= 5 imports and most functions consist of a single `return pkg.Func(...)` call. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture import paths
(import_spec
  path: (interpreted_string_literal) @import_path) @import

;; Capture thin wrapper functions (single return statement body)
(function_declaration
  name: (identifier) @func_name
  body: (block
    (return_statement
      (expression_list
        (call_expression
          function: (selector_expression) @delegated_call))))) @func
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In Go this appears as import paths with many slash-separated segments beyond the module root.

### Bad Code (Anti-pattern)
```go
package handler

import (
	"github.com/myorg/myapp/internal/platform/database/postgres/migrations"
	"github.com/myorg/myapp/internal/platform/database/postgres/queries"
	"github.com/myorg/myapp/internal/domain/orders/aggregates/events"
	"github.com/myorg/myapp/internal/infrastructure/messaging/kafka/producers"
)

func ProcessOrder(orderID string) error {
	event := events.NewOrderCreated(orderID)
	producers.Publish("orders", event)
	return queries.InsertOrder(orderID)
}
```

### Good Code (Fix)
```go
package handler

import (
	"github.com/myorg/myapp/database"
	"github.com/myorg/myapp/domain/orders"
	"github.com/myorg/myapp/messaging"
)

func ProcessOrder(orderID string) error {
	event := orders.NewCreatedEvent(orderID)
	messaging.Publish("orders", event)
	return database.InsertOrder(orderID)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_spec`, `interpreted_string_literal`
- **Detection approach**: Parse the import path string and count slash-separated segments after the module root (first three segments for `github.com/org/repo`). Flag if the sub-package depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
(import_spec
  path: (interpreted_string_literal) @import_path)
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
