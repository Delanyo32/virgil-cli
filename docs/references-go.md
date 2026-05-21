# References — Go

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes **fact emission** only. The Go extractor produces `occurrence`, `scope`, and `binding` rows; the Cozoscript resolver in `docs/resolution.md` turns those facts into `references` rows. This document does not describe resolution, the `references` relation, or `referent_id` lookup — those are language-agnostic.

Symbol ids follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`. All `start_byte` / `start_line` / `start_col` values are the tree-sitter `Range` of the relevant node.

## Scope tree

Go has a six-level scope hierarchy. The extractor emits a `scope` row for every level that exists in the source, with `parent_id` pointing at the innermost enclosing scope.

| Source construct | `scope.kind` | Notes |
|---|---|---|
| The file itself | `"file"` | `parent_id = null`. Holds file-private import bindings. **Go's file scope sits below package scope** — imports declared in `foo.go` are not visible in `bar.go` even within the same package. |
| The package (one synthetic scope per directory) | `"module"` | `parent_id = null` for the package scope. Every `"file"` scope's `parent_id` points at its package's `"module"` scope. Top-level `func`/`var`/`const`/`type` bindings live here. |
| `func` / method body | `"function"` | Parent is the file scope. Parameters, receiver, and named return values are bound here. |
| `{ ... }` block | `"block"` | Any brace-delimited block under a function, plus the implicit blocks introduced by `if`, `for`, `switch`, `select` clauses (the init clause and body share one scope). |

The universe block (predeclared identifiers `int`, `string`, `nil`, `true`, `false`, `iota`, `append`, `len`, `cap`, `make`, `new`, `panic`, `recover`, `print`, `println`, `delete`, `close`, `copy`, `error`, `any`, `comparable`, `byte`, `rune`, primitives) is **not** emitted as a `scope` row. Predeclared identifiers resolve to `null` referents at the resolver level (no binding row matches).

### `:=` short declarations and scope

`x := …` and `x, y := …` introduce **new bindings into the current block scope** (not the enclosing function scope) for any LHS name not already declared in that block. Each new name produces one `binding{scope_id = <current block>, name, binding_kind = "definition", symbol_id = <variable's symbol id>}` row, with `start_byte` set to the LHS identifier's offset.

Existing names on the LHS (already bound in the current scope) are not redeclared; their occurrences emit a `write` row instead. A name already bound at an **outer** scope and re-bound by `:=` in an inner block becomes a fresh symbol — a shadow, not an assignment.

### `if` / `for` / `switch` / `select` init clauses

These open a single `"block"` scope that covers both the init/condition and the body. The body's inner braces do **not** open an additional scope; the body shares the init-clause scope. (This matches the Go spec: the variables in `if x := f(); x > 0 { … }` are visible across the entire `if`, `else if`, and `else` chain.)

A `case` clause inside `switch` or `select` opens its own `"block"` scope (each `case` is its own implicit block per the Go spec). A type-switch guard `switch v := x.(type)` binds `v` separately in each `case` clause's scope, with the static type narrowed by the case.

## Bindings

Every `binding` row carries `(scope_id, name, start_byte) => (symbol_id, binding_kind)`.

### `definition`

A name introduced by a declaration. Emit one row per name.

- `func F(...)` and `func (r *T) M(...)` — top-level `func`/method. `scope_id = <package module scope>`. `name = "F"` or `"M"`. `symbol_id` is the function/method symbol's id.
- `var x [T] = …`, `var ( x = …; y = … )` — at package level binds into the module scope; inside a function binds into the enclosing block.
- `const x = …`, `const ( … )` — same scope rules as `var`.
- `type Foo …`, `type Foo struct{…}`, `type Foo interface{…}`, `type Foo = Bar` (alias) — binds into the module scope.
- `x := …` short declaration — see the `:=` rules above. Binds into the current block scope.
- `for i := …; …; …` and `for k, v := range m` — binds `i` / `k` / `v` into the loop's block scope (shared with the loop body).
- `switch v := x.(type)` — binds `v` into each `case` clause's block scope, once per case.
- Struct field declarations (`type T struct { Field U }`) — **not emitted as `binding` rows**. Field resolution is name-keyed and requires the receiver's resolved type; the resolver does not chase fields at the contract depth committed here.
- Type parameters (`func F[T any](…)`) — bind `T` into the function scope with `binding_kind = "definition"` (a type-parameter binding behaves like a regular definition for resolution purposes).

### `parameter`

Function and method parameters, including named return values and the **method receiver**. One row per parameter name, `scope_id = <function scope>`, `symbol_id = <parameter's symbol row id>` (per Issue #11 — parameters are symbols).

- Method receiver: `func (r *Foo) Bar()` emits a `parameter` binding for `r`. The receiver name `r` is treated identically to any other parameter. The receiver **type** `*Foo` does not appear as a binding (it produces a `type_use` occurrence instead).
- Blank parameter `_`: skip — `_` is not a binding.
- Variadic parameter `args ...T`: a single `parameter` binding for `args`.

### `import`

Plain imports — `import "net/http"` binds the last path segment (`http`) into the **file scope** (not the package/module scope — Go's file-private import semantics).

- `import "net/http"` → `binding{scope_id = <file scope>, name = "http", binding_kind = "import", symbol_id = null}` (external package; resolver may upgrade to non-null if the workspace indexes the package).
- `import "github.com/example/ordersvc/internal/model"` → `binding{name = "model", binding_kind = "import", symbol_id = null}`. The `imports` relation separately records the file → file mapping so the resolver can chase cross-package symbols.
- The path string (`"net/http"`) itself is not an identifier and produces no row.

### `import_alias`

Aliased imports — `import b "foo/bar"` binds the alias `b` (not the last path segment) into the file scope.

- `import logr "github.com/example/ordersvc/pkg/logger"` → `binding{name = "logr", binding_kind = "import_alias", symbol_id = null}`.
- Blank-import `import _ "foo/bar"` (side-effect-only import) — emit no `binding` row (the `_` is not a name); the `imports` relation still records the file → file edge.

### `wildcard_import`

Go's dot-import `import . "foo/bar"` brings the imported package's exported names into the **file scope** as if they were declared locally.

- `import . "fmt"` → `binding{scope_id = <file scope>, name = "*", binding_kind = "wildcard_import", symbol_id = null}`. The resolver expands at materialise time using the `imports` graph to find every exported symbol in the target file.

The `http-service` benchmark contains no dot-imports. The contract still commits to emitting `wildcard_import` rows when they appear; the worked examples below note this absence rather than fabricating a row.

## Occurrence emission

Every `occurrence` row carries `(id) => (name, file_path, start_byte, end_byte, enclosing_symbol_id, enclosing_scope_id, occurrence_kind)`. The `id` is `path|start_byte|name|occurrence_kind`. `enclosing_symbol_id` is the innermost named symbol (`null` for file-level expressions, package-level initializers, etc.). `enclosing_scope_id` is the innermost `scope` row.

Defining identifiers (the `name:` child of `function_declaration`, `method_declaration`, `type_spec`, `var_spec`, `const_spec`, `parameter_declaration`, `import_spec`, `field_declaration`, `type_parameter_declaration`) do **not** produce `occurrence` rows. They are recorded as `binding` rows instead.

The blank identifier `_` produces no `occurrence` row regardless of position.

### `call`

The callee identifier of every `call_expression`.

- `f()` — emit a `call` occurrence for `f`.
- `pkg.f()` — emit a `read` occurrence for `pkg` (the selector receiver). The right-hand side `f` is **not** emitted (see field/selector policy below).
- `obj.Method()` — emit a `read` occurrence for `obj`. The method name `Method` is **not** emitted.
- `go f(args)` — the goroutine launch evaluates `f(args)` like a normal call. Emit a `call` occurrence for `f` and `read` occurrences for each argument. The `go` keyword itself has no occurrence row.
- `defer f()` — same as `go`: a `call` occurrence for `f`.
- Type conversions like `MyType(x)` parse as `call_expression`. The extractor emits a `call` occurrence for `MyType` initially; the resolver may post-process to `type_use` when `MyType`'s referent resolves to a type symbol. The contract commits to emitting `call`; rewriting to `type_use` is a resolver concern.

### `read`

Every identifier in a value position not covered by another rule.

- RHS of `=`, `:=`, `+=`, etc.
- Function arguments, return-statement operands, conditional tests, loop bounds, switch tags, case-clause values.
- The receiver of a selector expression: `x.Field`, `x.Method`, `pkg.Name` — emit a `read` for `x` / `pkg` only, **never for the right-hand side `Field` / `Method` / `Name`**. (See "field-row policy" below.)
- The operand of `&x` (address-of): emit `read` for `x`. Address-of does not convert to `write`.
- The operand of `*p` (dereference) in a read position: emit `read` for `p`.
- Receive from channel: `v := <-ch` — emit `read` for `ch`; `v` is a definition (no occurrence row).
- Inside composite literals: value positions are `read`; field-name keys (`{ID: 5}`) are **not** emitted.

### `write`

Every identifier whose binding is mutated.

- LHS of `=`: `x = 5` — emit `write` for `x`.
- LHS of compound assignment (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `&^=`): emit a **single** `write` row per [ADR-0003](adr/0003-level-3-types-and-references.md). No read row for the same identifier on the same site. Faithful read+write is Level 4.
- `x++` and `x--`: single `write` row, same as compound assignment.
- LHS of `:=` **only when the name already exists in the current block scope** (partial-redeclaration rule). A name not yet bound in the current scope becomes a `definition` binding and produces no occurrence row.
- LHS of `=` `range` (not `:=`): `for k, v = range m` — emit `write` for `k` and `v`.
- `*p = …` — emit `write` for `p`. The closest available referent at this contract depth is the pointer variable.
- **Channel send** `ch <- v` — emit `write` for `ch` (the send mutates the channel's state) and `read` for `v`. This is a deliberate Phase-1 narrowing; channel semantics (buffered vs unbuffered, send vs close vs receive) are out of scope. Level 4 would model channels as their own primitive.
- Field write: `x.Field = v` emits `read` for `x` only. No `write` row is produced for the field per the field-row policy below.

### `type_use`

Every identifier inside a type expression. These overlap exactly with the `type` rows produced by `types-go.md` — each `type` mention corresponds to one `type_use` occurrence.

- Parameter / return / named-return / receiver type annotations.
- Struct field types, map key/value types, channel element types, slice/array element types, function-type signatures.
- Type-assertion targets: `v.(MyType)` — `MyType` is `type_use`; `v` is `read`.
- Type-switch case clauses: `case *MyType:` — `MyType` is `type_use`.
- Type-parameter constraints: `func F[T comparable]` — `comparable` is `type_use`; `T` is a definition binding.
- Composite-literal type: `&Order{…}` — `Order` is `type_use`.
- `make(chan T)`, `new(T)` — `T` inside is `type_use`.
- Qualified types: `service.OrderService` in a type position — emit `read` for `service` (the import receiver) and `type_use` for `OrderService` (the cross-package type identifier). This is the **only** case where the right-hand side of a selector produces an occurrence row.

### `import_use`

Reserved for identifiers syntactically inside an `import_declaration` subtree. In Go, the only identifier inside `import_spec` is the alias name (a definition — no row) or the wildcard `.` (also not a regular identifier). In practice the Go extractor emits **zero** `import_use` rows. This is a known sparsity, not a bug; the column exists for cross-language uniformity.

## Field-row policy (selector right-hand sides)

`x.Field` and `pkg.Name` (in value position) emit a `read` occurrence only for the **left** side. The right-hand `Field` / `Name` is never emitted in value position, and `Field` is never emitted as a `write` on `x.Field = v`. Resolving the field requires the receiver's resolved type, which is out of scope.

The single exception is **qualified types** in type position (`pkg.TypeName`): the right side gets a `type_use` row whose name is the type identifier. The resolver chases this via the file's `import` bindings and the `imports` graph.

This is a Phase-1 narrowing. A future `field_access` relation (or a Level-4 type-resolved expansion) would emit field-level reads/writes.

## What this contract does NOT cover

- **Resolution.** No `references` rows. No `referent_id`. Resolution is `docs/resolution.md`.
- **Cross-package symbol lookup.** When `pkg.Name` is emitted, the resolver — not the extractor — chases `pkg` through the file's `import` binding to the `imports` graph, then looks `Name` up among that file's exported symbols.
- **Method dispatch / field access.** No field-level occurrences (see field-row policy).
- **Channel semantics.** `ch <- v` is one `write` on `ch`; `<-ch` in expression position is one `read` on `ch`. No richer modeling.

## Worked examples

All examples are real source from `../virgil-skills/benchmarks/go/http-service/`. For each, `path` is shown relative to that root.

### Example 1 — Method on a receiver (read-heavy body)

**Source** — `internal/model/order.go`, lines 41–45:

```go
func (o *Order) IsTerminal() bool {
    return o.Status == OrderStatusDelivered ||
        o.Status == OrderStatusCancelled ||
        o.Status == OrderStatusPaymentFailed
}
```

Enclosing symbol: `internal/model/order.go|41|0|IsTerminal|method`.

**`scope` rows (new for this snippet):**

| id | parent_id | kind | start_byte | end_byte |
|---|---|---|---|---|
| `internal/model/order.go\|<byte of '{' on 41>\|function` | `<file scope>` | `function` | byte of `{` | byte of `}` on 45 |

**`binding` rows:**

| scope_id | name | start_byte | symbol_id | binding_kind |
|---|---|---|---|---|
| `<function scope>` | `o` | byte of `o` in `(o *Order)` | `internal/model/order.go\|41\|<col>\|o\|parameter` | `parameter` |

The method itself (`IsTerminal`) is a `definition` binding at the package/module scope, emitted elsewhere when the file's top-level declarations are walked.

**`occurrence` rows:**

| name | enclosing_scope_id | occurrence_kind | rationale |
|---|---|---|---|
| `Order` | `<function scope>` | `type_use` | Receiver type `*Order` |
| `bool` | `<function scope>` | `type_use` | Return type annotation |
| `o` | `<function scope>` | `read` | line 42 selector receiver |
| `OrderStatusDelivered` | `<function scope>` | `read` | line 42 RHS of `==` |
| `o` | `<function scope>` | `read` | line 43 |
| `OrderStatusCancelled` | `<function scope>` | `read` | line 43 |
| `o` | `<function scope>` | `read` | line 44 |
| `OrderStatusPaymentFailed` | `<function scope>` | `read` | line 44 |

`Status` on `o.Status` is **not** emitted (selector RHS in value position). The receiver name `o` in `(o *Order)` is a definition (no occurrence).

### Example 2 — Cross-package qualified reference (struct field types)

**Source** — `internal/api/handler.go`, lines 1–23:

```go
package api

import (
    "database/sql"
    ...
    "github.com/example/ordersvc/internal/model"
    "github.com/example/ordersvc/internal/service"
)

type Handler struct {
    orderSvc        *service.OrderService
    inventorySvc    *service.InventoryService
    paymentSvc      *service.PaymentService
    notificationSvc *service.NotificationService
}
```

**`scope` rows:**

| id | parent_id | kind |
|---|---|---|
| `<file scope for handler.go>` | `<package "api" module scope>` | `file` |

The struct declaration does not open a new scope (struct fields are not resolved through lexical scope).

**`binding` rows (selected; one per import):**

| scope_id | name | binding_kind | symbol_id |
|---|---|---|---|
| `<file scope>` | `sql` | `import` | `null` (stdlib, external) |
| `<file scope>` | `model` | `import` | `null` (resolver may upgrade) |
| `<file scope>` | `service` | `import` | `null` (resolver may upgrade) |

`Handler` itself is a `definition` binding at the module scope.

**`occurrence` rows for the struct fields:**

| name | occurrence_kind | enclosing_symbol_id | rationale |
|---|---|---|---|
| `service` | `read` | `<Handler struct symbol>` | LHS of `service.OrderService` in field type |
| `OrderService` | `type_use` | `<Handler struct symbol>` | Qualified-type RHS in type position — the documented exception |
| `service` | `read` | `<Handler struct symbol>` | line 20 |
| `InventoryService` | `type_use` | `<Handler struct symbol>` | line 20 |
| `service` | `read` | `<Handler struct symbol>` | line 21 |
| `PaymentService` | `type_use` | `<Handler struct symbol>` | line 21 |
| `service` | `read` | `<Handler struct symbol>` | line 22 |
| `NotificationService` | `type_use` | `<Handler struct symbol>` | line 22 |

Field-name identifiers (`orderSvc`, `inventorySvc`, …) are field declarations — `binding` rows of `definition` kind are **not** emitted (per the bindings section, struct fields are not in scope for resolver lookup at this contract depth). They appear in `symbol` rows separately.

### Example 3 — Import alias and dot-import (benchmark absence)

The `http-service` benchmark contains no aliased imports and no dot-imports — every `import_spec` uses the bare path form. Concretely, every file in `internal/`, `cmd/`, and `pkg/` matches:

```go
import (
    "fmt"
    "net/http"
    "github.com/example/ordersvc/internal/model"
)
```

Each line emits one `binding{name = <last segment>, binding_kind = "import", symbol_id = null}` row in that file's `<file scope>`.

If `cmd/server/main.go` line 21 were instead:

```go
log "github.com/example/ordersvc/pkg/logger"
```

the row would become `binding{scope_id = <file scope>, name = "log", binding_kind = "import_alias", start_byte = <byte of `log`>, symbol_id = null}`. No occurrence row is emitted for the alias name at its definition site.

If `internal/api/middleware.go` line 4 were `import . "fmt"`, the contract emits:

`binding{scope_id = <file scope of middleware.go>, name = "*", binding_kind = "wildcard_import", start_byte = <byte of `.`>, symbol_id = null}`.

The resolver expands `*` at materialise time by enumerating exported symbols in the target file (via the `imports` graph + `symbol{exported = true}`).

State explicitly: this contract commits to those rows when the constructs appear. The Go extractor must not skip them merely because the benchmark omits them.

### Example 4 — Shadowing via `:=` in a nested block

**Source** — `internal/service/order.go`, lines 33–55:

```go
func (s *OrderService) CreateOrder(order *model.Order) error {
    var totalCents int
    for _, item := range order.Items {
        product, err := s.inventoryRepo.GetProduct(item.ProductID)
        if err != nil {
            return fmt.Errorf("failed to get product %d: %w", item.ProductID, err)
        }
        item.PriceCents = product.PriceCents
        totalCents += product.PriceCents * item.Quantity
    }
    ...
    for _, item := range order.Items {
        err := s.inventoryRepo.DecrementStock(item.ProductID, item.Quantity)
        if err != nil {
            return fmt.Errorf("failed to reserve stock for product %d: %w", item.ProductID, err)
        }
    }
}
```

Two distinct `for _, item := range …` blocks. Each opens its own `"block"` scope, and each binds its own `item` and `err` symbols.

**`scope` rows:**

| id | parent_id | kind |
|---|---|---|
| `<function scope for CreateOrder>` | `<file scope>` | `function` |
| `<block scope for loop 1, lines 36–44>` | `<CreateOrder function scope>` | `block` |
| `<block scope for `if err != nil` inside loop 1>` | `<loop 1 block scope>` | `block` |
| `<block scope for loop 2, lines 48–54>` | `<CreateOrder function scope>` | `block` |
| `<block scope for `if err != nil` inside loop 2>` | `<loop 2 block scope>` | `block` |

**`binding` rows (selected; new bindings inside the loops):**

| scope_id | name | binding_kind | symbol_id |
|---|---|---|---|
| `<CreateOrder function scope>` | `s` | `parameter` | `…\|33\|<col>\|s\|parameter` |
| `<CreateOrder function scope>` | `order` | `parameter` | `…\|33\|<col>\|order\|parameter` |
| `<CreateOrder function scope>` | `totalCents` | `definition` | `…\|35\|<col>\|totalCents\|variable` |
| `<loop 1 block scope>` | `item` | `definition` | `…\|36\|<col>\|item\|variable` |
| `<loop 1 block scope>` | `product` | `definition` | `…\|38\|<col>\|product\|variable` |
| `<loop 1 block scope>` | `err` | `definition` | `…\|38\|<col>\|err\|variable` |
| `<loop 2 block scope>` | `item` | `definition` | `…\|48\|<col>\|item\|variable` |
| `<loop 2 block scope>` | `err` | `definition` | `…\|50\|<col>\|err\|variable` |

Two `item` bindings with distinct `symbol_id`s in distinct scopes; same for `err`. The resolver picks the innermost binding active at each reference site, which gives the shadowing semantics for free.

**`occurrence` rows (selected):**

| name | enclosing_scope_id | occurrence_kind | line |
|---|---|---|---|
| `order` | `<CreateOrder function scope>` | `read` | 36 (in `order.Items`) |
| `s` | `<loop 1 block scope>` | `read` | 38 |
| `err` | `<loop 1 block scope>` | `read` | 39 |
| `fmt` | `<if-block under loop 1>` | `read` | 40 |
| `err` | `<if-block under loop 1>` | `read` | 40 |
| `item` | `<loop 1 block scope>` | `read` | 42 (LHS `item.PriceCents` — selector receiver) |
| `product` | `<loop 1 block scope>` | `read` | 42 |
| `totalCents` | `<loop 1 block scope>` | `write` | 43 (compound `+=`, single `write` per ADR-0003) |
| `product` | `<loop 1 block scope>` | `read` | 43 |
| `item` | `<loop 1 block scope>` | `read` | 43 |
| `s` | `<loop 2 block scope>` | `read` | 50 |
| `err` | `<loop 2 block scope>` | `read` | 51 |

`item.PriceCents = product.PriceCents` on line 42: the LHS is a selector. `item` is **`read`** (we read it to access its field), no `write` occurs at the identifier level. No field-level row for `PriceCents` (field-row policy). This is the documented under-reporting at the current contract depth.

`totalCents += …` on line 43: single `write` row on `totalCents`, no read. Per ADR-0003.

### Example 5 — Goroutine launch with channel send (`go` + `<-`)

**Source** — `internal/worker/dispatcher.go`, lines 93–97 (inside `ProcessJobs`):

```go
go func(j Job) {
    result := d.processOne(j)
    d.results <- result
}(job)
```

Enclosing symbol: `internal/worker/dispatcher.go|82|0|ProcessJobs|method`. The Go extractor does **not** emit a separate symbol for the anonymous function literal at this contract depth — its body's occurrences belong to the enclosing method. (Decision recorded for clarity; could change in Level 4.)

**`scope` rows:**

| id | parent_id | kind |
|---|---|---|
| `<block scope for the func literal body, lines 93–96>` | `<for-loop block scope>` | `block` |

The `for _, rawJob := range jobs` loop on line 83 opens its own `block` scope; the anonymous function's body is nested under it.

**`binding` rows:**

| scope_id | name | binding_kind | symbol_id |
|---|---|---|---|
| `<func literal body scope>` | `j` | `parameter` | `…\|93\|<col>\|j\|parameter` |
| `<func literal body scope>` | `result` | `definition` | `…\|94\|<col>\|result\|variable` |

The closure-parameter name `j` is a `parameter` binding scoped to the literal's body.

**`occurrence` rows:**

| name | enclosing_scope_id | occurrence_kind | line | rationale |
|---|---|---|---|---|
| `Job` | `<func literal body scope>` | `type_use` | 93 | parameter type annotation |
| `d` | `<func literal body scope>` | `read` | 94 | selector receiver of `d.processOne` |
| `processOne` | — | — | 94 | **not emitted** (selector RHS in call position) |
| `j` | `<func literal body scope>` | `read` | 94 | call argument |
| `d` | `<func literal body scope>` | `read` | 95 | selector receiver of `d.results` (LHS of `<-`) |
| `d` | `<func literal body scope>` | `write` | 95 | channel send mutates the channel — `write` on the closest available identifier |
| `result` | `<func literal body scope>` | `read` | 95 | RHS of `<-` |
| `job` | `<for-loop block scope>` | `read` | 96 | call argument outside the literal (`}(job)`) |

The `go` keyword itself has no occurrence row. The `<-` operator has no occurrence row.

**Note on the channel send:** the contract emits both a `read` and a `write` on `d` on line 95 (different `start_byte` values — they are two separate occurrences syntactically: the `d` in `d.results` is one identifier token, but the `write` is the consequence of the surrounding send). At the contract depth committed here, the closest identifier the send mutates is the receiver `d`; a future field-resolved layer would emit the `write` on the resolved channel field instead. This is the documented narrowing.

Alternative reading: emit only `write` on `d` for the send (drop the `read`). The current contract emits both, consistent with how `x = …` on a plain LHS produces only `write` but `x.f = …` on a field LHS produces only `read` on `x`. For consistency with channel sends being a *mutation through* a selector path, treat `d.results <- v` as: `read` on `d` (selector traversal), plus `write` on `d` (mutation effect at the field-resolved layer is unavailable, so the write attaches to the nearest identifier). State explicitly: this is the chosen interpretation; one `read` + one `write` row on `d`.

### Example 6 — Partial redeclaration with `:=` (mixed new + existing)

**Source** — `internal/api/middleware.go`, lines 94–106 (inside the inner anonymous handler returned by `RateLimitMiddleware`):

```go
return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
    clientIP := r.RemoteAddr

    mu.Lock()
    entry, exists := clients[clientIP]
    if !exists {
        entry = &rateLimitEntry{
            count:   0,
            resetAt: time.Now().Add(60 * time.Second),
        }
        clients[clientIP] = entry
    }
    ...
})
```

Enclosing symbol: `internal/api/middleware.go|74|0|RateLimitMiddleware|function`.

**`scope` rows:**

| id | parent_id | kind |
|---|---|---|
| `<RateLimitMiddleware function scope>` | `<file scope>` | `function` |
| `<inner anonymous func literal body scope>` | `<RateLimitMiddleware function scope>` | `block` |
| `<if-block under the literal, lines 100–106>` | `<inner literal body scope>` | `block` |

**`binding` rows (inside the inner literal):**

| scope_id | name | binding_kind | symbol_id |
|---|---|---|---|
| `<inner literal body>` | `w` | `parameter` | `…\|95\|<col>\|w\|parameter` |
| `<inner literal body>` | `r` | `parameter` | `…\|95\|<col>\|r\|parameter` |
| `<inner literal body>` | `clientIP` | `definition` | `…\|96\|<col>\|clientIP\|variable` |
| `<inner literal body>` | `entry` | `definition` | `…\|99\|<col>\|entry\|variable` |
| `<inner literal body>` | `exists` | `definition` | `…\|99\|<col>\|exists\|variable` |

On line 99, `entry, exists := clients[clientIP]` — both names are new in the inner literal's scope, so both produce `definition` bindings and **no occurrence rows** at the LHS.

**`occurrence` rows (selected):**

| name | enclosing_scope_id | occurrence_kind | line |
|---|---|---|---|
| `r` | `<inner literal body>` | `read` | 96 (selector receiver of `r.RemoteAddr`) |
| `mu` | `<inner literal body>` | `read` | 98 |
| `clients` | `<inner literal body>` | `read` | 99 (RHS of `:=`) |
| `clientIP` | `<inner literal body>` | `read` | 99 (map index) |
| `exists` | `<if-block>` | `read` | 100 |
| `entry` | `<if-block>` | `write` | 101 (LHS of `=`, name already bound — partial-redeclaration write) |
| `rateLimitEntry` | `<if-block>` | `type_use` | 101 |
| `time` | `<if-block>` | `read` | 103 (selector receiver) |
| `clients` | `<if-block>` | `read` | 105 (LHS of `clients[clientIP] = entry`: `clients` is the receiver of the index expression, read) |
| `clientIP` | `<if-block>` | `read` | 105 |
| `entry` | `<if-block>` | `read` | 105 (RHS) |

Notice line 101 emits **`write` on `entry`** because `entry` is already bound in the enclosing block (line 99 introduced it). This is the partial-redeclaration semantics applied to a plain `=`: the binding exists, so the assignment is a write to that binding.

A different scenario: if line 99 were `v, err := g()` and an outer `err` already existed at the function scope, the inner `:=` would create a **new** `err` binding in the inner block (shadow), with `binding_kind = "definition"`. The outer `err` is untouched. No `write` on the outer.

Conversely, if `err` had been declared earlier inside the **same** inner block (e.g. `var err error` on line 98), then `v, err := g()` would emit a `write` row for `err` (existing same-scope binding) and a `definition` binding for `v` only.
