# References — Go

This contract maps Go identifier occurrences to rows in the `references` relation defined in `docs/virgil-datalog-schema.md`, at the Level-3 depth committed to in [ADR-0003](adr/0003-level-3-types-and-references.md). Symbol ids follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

`references` is keyed by `(referrer_id, site_file, site_start_byte, match_index)` with `referent_id` and `ref_kind` in the value position. `match_index = 0` for the primary/only candidate; Go has no overloading, so every Go row uses `match_index = 0`. Unresolvable referents emit a single row with `referent_id = null` (not skipped, not a sentinel string).

A `references` row is emitted for every identifier *use* in source. Identifier *definitions* (the `name:` capture inside a `function_declaration`, `type_spec`, `var_spec`, `const_spec`, struct field, parameter list, etc.) do not produce `references` rows — they are recorded by the `symbol` relation instead.

## Lexical scope rules

Go's scope rules, in order from innermost to outermost:

1. **Block scope** — every `{ ... }` introduces a scope. `if`, `for`, `switch`, and `select` introduce an *implicit* block whose scope covers the condition/init clause and the body together (e.g. `if x := f(); x > 0 { … }` — `x` is visible in both the condition and the body but not after the `if`).
2. **Function scope** — parameters, named return values, and type parameters are visible across the entire function body.
3. **File scope** — names imported via `import` are visible only inside the file that imports them. (This is the key Go-specific scope: imports are *not* package-wide.)
4. **Package scope** — every top-level declaration (`func`, `type`, `var`, `const`, `method`) in any file under the same directory is visible from every other file in that directory. Order does not matter; forward references are legal.
5. **Universe block** — predeclared identifiers (`int`, `string`, `nil`, `true`, `false`, `iota`, `append`, `len`, `cap`, `make`, `new`, `panic`, `recover`, `print`, `println`, `delete`, `close`, `copy`, `error`, `any`, `comparable`, `byte`, `rune`, and all primitive type names).

### Lookup

A bare identifier `x` looks up scopes 1 → 5 and binds to the first definition. A qualified identifier `pkg.x` resolves `pkg` via scope 3 (imports of the current file only), then looks up `x` in the resolved package's scope-4 namespace.

### Shadowing

Go allows shadowing freely: an inner-scope `:=` or `var` with the same name as an outer binding creates a new binding. The earlier binding is *not* an error and remains live for any code lexically outside the inner scope. The resolver records the innermost binding active at the reference site — outer bindings are not preserved as alternate referents.

### `:=` short declarations — the partial-redeclaration rule

`x, y := f()` in a scope introduces *new* bindings for any name on the LHS not already declared in the *current* scope. Names already declared in the current scope are assigned to, not redeclared. At least one name on the LHS must be new for `:=` to be legal. The contract:

- For every LHS name, the resolver checks the current scope's declarations. If the name is already bound at this scope, the occurrence is a `write` reference to that existing symbol. If not, a new symbol is created (`kind = "variable"`, parent = enclosing function or block) and *no* `references` row is emitted for that occurrence (it's a definition).
- A name declared at an *outer* scope and re-bound by `:=` at the inner scope is a *new symbol* (shadow); no `write` reference to the outer is recorded.

Example: in `internal/api/middleware.go` line 99, `entry, exists := clients[clientIP]` — both `entry` and `exists` are new bindings in the inner `func(w, r)` block, so no `references` rows are emitted for either LHS name; the row count for this line comes from `clients` and `clientIP` (both `read`).

### Method receivers

A method declaration `func (r *Foo) Bar()` introduces `r` into the function scope. The receiver name is a *definition* (no `references` row), but the receiver *type* `*Foo` produces a `type_use` row whose `referrer_id` is the method symbol and `referent_id` is the symbol id of the `Foo` type declaration. Uses of `r` inside the method body are `read` references to the receiver parameter (synthesized symbol id: `<file>|<method_line>|<method_col>|<receiver_name>|parameter`).

### Pointer dereferencing

`*p` (dereference) and `&x` (address-of) do not change the referent. The identifier inside (`p`, `x`) produces one `references` row with `ref_kind` determined by the surrounding context (write if the dereferenced location is assigned to, read otherwise). The `*` / `&` tokens themselves carry no row.

`p.field` where `p` is a pointer is treated the same as `p.field` where `p` is a value: a `read` of `p`, followed by a field-access whose target is *not* recorded as a `references` row (Go field accesses are name-keyed and the field symbol resolution requires the receiver's resolved type — see "field access" below).

## `ref_kind` decision tree

### `read`

Every identifier evaluation:
- An `identifier` or `field_identifier` that is the *receiver* of a `selector_expression` (the `x` in `x.Field`).
- An `identifier` appearing as an argument to a `call_expression`.
- An `identifier` on the RHS of an assignment or `:=`.
- An `identifier` in a conditional (`if x > 0`), loop bound, switch tag, return value, composite-literal value position, type-assertion expression, etc.
- An `identifier` inside `&x` (address-of read) — the address-of operator does *not* convert a read to a write.
- The function position of a call: `f()` produces a `read` of `f`. `pkg.f()` produces a `read` of `pkg` (the import name), and the `f` after the dot is *not* emitted as a `references` row at this contract depth — see "field access and selector expressions" below.

Exceptions (no `references` row):
- The defining occurrence inside a `function_declaration name:`, `method_declaration name:`, `type_spec name:`, `var_spec name:`, `const_spec name:`, `field_declaration name:`, `parameter_declaration name:`, `type_parameter_declaration name:`, or `import_spec name:` (import alias).
- The struct field name on the LHS of a composite-literal key (`Order{ID: 5, Items: ...}` — `ID` and `Items` are not emitted; they require resolved-receiver-type information).
- A blank identifier `_`.

### `write`

Every identifier that is *assigned to* or *mutated*:
- LHS of `=` assignment: `x = 5` emits `write` for `x`.
- LHS of `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `&^=`: emits a single `write` row for the LHS identifier. Updated per `docs/contract-review.md`: compound assignment is a single `write` at Level 3; faithful read+write semantics is Level 4.
- `x++` and `x--`: single `write` row, same as compound assignment.
- LHS of `:=` *only when the name already exists in the current scope* (the partial-redeclaration rule above).
- Dereferenced write: `*p = 5` emits a `write` row for `p`. Rationale: at Level 3 we have no points-to analysis, so the closest available referent is the pointer variable itself.
- LHS of `range` clause when used with `=` (not `:=`): `for k, v = range m { … }` emits `write` for `k` and `v`. With `:=` the partial-redeclaration rule applies.
- Channel send: `ch <- v` is a `write` on `ch` and a `read` of `v`. A send mutates the channel's state.
- Field write: `x.Field = v` — `x` is `read`. A `write` row for `Field` is emitted **only** when `Field` has a known `symbol_id` in the store (per the standardized field-tracking policy in `docs/contract-review.md`). Most local-struct fields are not extracted as symbols today and therefore produce no field-level row. Once the Phase 2 symbol pass extracts struct fields as symbols, those writes become resolvable. The previous rule that synthesized writes on the receiver pointer (`d.stats.Processed++` → `write` on `d`) is dropped in favor of "write on the field when its symbol_id is known, otherwise no field-level row".

### `type_use`

Every identifier occurrence inside a type expression as enumerated in `types-go.md`:
- Parameter type annotations: `x int` → `int` is `type_use`.
- Return types: the return-type tree of every function/method.
- Struct field types.
- Map key and value types.
- Channel element types.
- Type-assertion targets: `v.(MyType)` → `MyType` is `type_use`. The `v` itself is a `read`.
- Type-conversion targets: `MyType(x)` is parsed by tree-sitter as a `call_expression`, but if the callee identifier resolves to a *type* symbol (not a function), the row's `ref_kind` is `type_use`, not `read`. This requires a post-resolution rewrite — the resolver emits `read` initially, then upgrades to `type_use` when `referent_id` resolves to a symbol of kind `struct`/`interface`/`type_alias`.
- Type-parameter constraints: `func F[T comparable]` → `comparable` is `type_use`, `T` is a definition.
- Type-switch case clauses: `case *MyType:` → `*MyType` produces a `type_use` row for `MyType`.

Each `type_use` reference's `referent_id` points to the *symbol* of the named type, not to the `type` relation row. The corresponding `type` row's `id` lives in the `parameter`/`returns_type` rows, separately.

### `import_use`

Identifiers occurring inside `import_spec` nodes:
- The path string `"net/http"` is *not* an identifier — no row. (The `raw_import` / `imports` rows in the existing import-extraction pipeline cover this.)
- An import alias (`http "net/http"`) — the `http` token before the path is a *definition* of a file-scope binding; no `references` row.
- Subsequent uses of the imported name *outside* the `import_spec` are `read`, not `import_use`. **Decision: `import_use` is reserved exclusively for occurrences syntactically inside an `import_declaration` subtree.** In Go this is rare — typically only the alias and the path string appear there, neither of which generates a `references` row. In practice the Go extractor will emit `import_use` rows only for dot-imported names *if and when* the resolver elects to record them (see "unresolved" below); at Level 3 with the current corpus, `import_use` is effectively unused for Go. State explicitly: this is a known sparsity, not a bug.

## `referent_id` resolution

For each candidate occurrence:

1. **Walk scopes in order 1 → 5** as described above. The first match wins.
2. For `qualified_type` and `selector_expression` with package-qualified receiver (e.g. `model.Order`):
   - Resolve the package alias via the file's import table (`local_name` → import path).
   - If the import path resolves to a workspace-indexed package (see `resolve_import` in `src/languages/go/queries.rs`), search that package's symbols for `Name`. Match wins → `referent_id = <symbol id of Name>`.
   - If the package is not indexed (external), `referent_id = null`.
3. **No-match behavior:** emit the row with `referent_id = null`. Do *not* skip the row — downstream queries need to know "this identifier was referenced even if we don't know what it points to" for unresolved-symbol audits.
4. **Multiple matches:** Go's scope rules forbid same-scope ambiguity, so multi-match can only happen across the package/universe boundary (e.g. a package-level `int` would shadow the universe `int` — which Go actually allows). Pick the innermost match. If two candidates are at the same scope level (only possible via build-tag-conditional compilation producing duplicate package-level decls), pick the lexicographically-first `file_id` and record it; the contract does *not* require us to enumerate alternatives.

The resolver uses the existing `symbols_by_name` index from `src/graph/builder.rs`, filtered by `language = "go"`, plus a per-file import table built once during extraction. Block-scope and function-scope bindings are tracked via a per-file `Vec<Scope>` stack walked during the extractor pass; this stack is not shared across files.

### Field access and selector expressions

`x.Field` and `x.Method()` resolve the *receiver* (`x`) to a symbol and emit a `read` row for it. The field/method on the right side of the dot is *not* emitted as a `references` row at this contract depth, because resolving it requires:

- Resolving `x`'s type expression.
- Following named-type chains (including pointer dereference, type aliases, embedded fields).
- Looking up the method/field on the resulting type.

That is out of scope for Level 3 as currently specified. **Decision:** Go's `references` rows cover identifiers, not member-access right-hand sides. A separate `field_access` or `method_call` relation can be added later. The exception is `pkg.Name` where `pkg` is an import — the `pkg` side resolves to the import; the `Name` side, when it appears in a *type position*, is handled by the `qualified_type` branch above and *does* emit a `type_use` row whose `referent_id` is the cross-package symbol.

## Worked examples

### Example 1 — Block scope, `:=` introducing new bindings

Source: `internal/api/middleware.go`, lines 94–115 (the inner anonymous handler returned by `RateLimitMiddleware`).

```go
return func(next http.Handler) http.Handler {
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
}
```

Referrer for these rows is the symbol of the *outer* `RateLimitMiddleware` function, `internal/api/middleware.go|74|0|RateLimitMiddleware|function` — Go anonymous function literals do not produce their own symbol rows at this contract level; their bodies count as part of the enclosing named symbol. (Decision recorded for clarity.)

Selected rows (site_file = `internal/api/middleware.go`, site_start_byte = tree-sitter byte offset of the identifier):

| referent_id | ref_kind | identifier | rationale |
|---|---|---|---|
| `mu` (the `var mu sync.Mutex` at line 75 of same file) | `read` | `mu` on line 98 | bare identifier, package-local var |
| `clients` (the `clients := make(...)` at line 76, which is a function-local `:=` — *new binding* inside `RateLimitMiddleware`) | `read` | `clients` on line 99 | package-style closure capture; resolves to the outer function's local |
| `clientIP` (defined on line 96 via `:=`) | `read` | `clientIP` on line 99 | local in inner anonymous func |
| no row | (definition) | `entry` on line 99 | LHS of `:=`, name not yet bound in this block → definition, new symbol `entry` of kind `variable` |
| no row | (definition) | `exists` on line 99 | same as above |
| symbol of `exists` defined on line 99 | `read` | `exists` on line 100 | inside `if !exists` |
| symbol of `entry` defined on line 99 | `write` | `entry` on line 101 | LHS of `=` assignment (not `:=`), name already bound → write |
| `rateLimitEntry` (type defined on line 66) | `type_use` | `rateLimitEntry` on line 101 | inside composite literal after `&` |
| `time` (import alias) | `read` | `time` on line 103 | LHS of selector `time.Now` |

The `time.Now` right side (`Now`) is *not* emitted (selector RHS rule).

### Example 2 — Shadowing across nested blocks via `:=`

Source: `internal/service/order.go`, lines 33–55 (the `CreateOrder` method body).

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

Shadowing is explicit: each `for _, item := range ...` introduces a *new* `item` symbol in the loop's block scope. The two `item`s have different symbol ids:

- Loop 1: `internal/service/order.go|36|6|item|variable` (definition at line 36; tree-sitter column of `item` in `for _, item := range`).
- Loop 2: `internal/service/order.go|48|6|item|variable` (definition at line 48).

Similarly, `err` is declared *twice* via `:=` — once on line 38, once on line 50. Two separate symbols, two distinct ids. References to `err` inside the first loop body resolve to symbol 1; inside the second loop body to symbol 2.

Rows for the inner block of loop 1 (selected):

| referent_id | ref_kind | identifier | line |
|---|---|---|---|
| `s` (parameter receiver, definition at line 33 col after `(`) | `read` | `s` | 38 |
| no row | (definition) | `product` | 38 |
| no row | (definition) | `err` (first) | 38 |
| `err` (line 38) | `read` | `err` | 39 |
| `err` (line 38) | `read` | `err` | 40 (inside `fmt.Errorf`) |
| `item` (line 36) | `read` | `item` | 40 |
| `item` (line 36) | `write` | `item.PriceCents` — *row is on `item`*, ref_kind `read` not `write` | 42 |

**Edge case clarified:** `item.PriceCents = product.PriceCents` — the LHS is `item.PriceCents`. `item` itself is *read* (we read it to access its field), not written. The `.PriceCents` member is not a `references` row (selector RHS rule). So no `write` row is emitted for this assignment at this contract depth. State explicitly: this is a known under-reporting at Level 3; a future "field-access" relation would resolve it.

### Example 3 — `type_use` for receiver, parameter, and return types

Source: `internal/model/order.go`, lines 41–45.

```go
func (o *Order) IsTerminal() bool {
    return o.Status == OrderStatusDelivered ||
        o.Status == OrderStatusCancelled ||
        o.Status == OrderStatusPaymentFailed
}
```

The method symbol is `internal/model/order.go|41|0|IsTerminal|method`. Rows emitted from the signature alone:

| referent_id | ref_kind | identifier | site_start_byte |
|---|---|---|---|
| `Order` (symbol at `internal/model/order.go|8|0|Order|struct`) | `type_use` | `Order` in `*Order` receiver | tree-sitter byte of `Order` token |
| `bool` (universe-block primitive — referent_id is `null` because the universe block has no symbol rows) | `type_use` | `bool` | tree-sitter byte of `bool` |

The receiver name `o` is a definition (no row).

Rows from the body:

| referent_id | ref_kind | identifier | line |
|---|---|---|---|
| `o` (receiver, synthesized symbol) | `read` | `o` | 42 |
| `OrderStatusDelivered` (const at line 35 of same file) | `read` | `OrderStatusDelivered` | 42 |
| `o` | `read` | `o` | 43 |
| `OrderStatusCancelled` (line 36) | `read` | `OrderStatusCancelled` | 43 |
| `o` | `read` | `o` | 44 |
| `OrderStatusPaymentFailed` (line 37) | `read` | `OrderStatusPaymentFailed` | 44 |

Note `bool` resolves to the universe block. The contract emits the row with `referent_id = null` — universe-block primitives have no synthesized symbol. Queries that want "is this a primitive type-use" should join against `type` rows on `display_name`, not chase the `referent_id`.

### Example 4 — Cross-package qualified reference (`qualified_type` → `type_use`)

Source: `internal/api/handler.go`, lines 32–37.

```go
return &Handler{
    orderSvc:        orderSvc,
    inventorySvc:    inventorySvc,
    paymentSvc:      paymentSvc,
    notificationSvc: notificationSvc,
}
```

Plus, earlier on lines 18–23:

```go
type Handler struct {
    orderSvc        *service.OrderService
    inventorySvc    *service.InventoryService
    paymentSvc      *service.PaymentService
    notificationSvc *service.NotificationService
}
```

Rows for the struct's field-type expressions (referrer is the `Handler` *type* symbol at `internal/api/handler.go|18|0|Handler|struct`):

| referent_id | ref_kind | identifier | rationale |
|---|---|---|---|
| `service` (file-scope import binding) | `read` | `service` (LHS of `service.OrderService`) | The `service` token resolves to the import declared on line 14. |
| `OrderService` (symbol at `internal/service/order.go|14|0|OrderService|struct`) | `type_use` | `OrderService` (RHS of selector inside type position) | Selector RHS in a *type position* is the documented exception — emit a `type_use` row. Resolves cross-package via the file's import table. |

Repeat for `InventoryService`, `PaymentService`, `NotificationService`.

For the constructor body on lines 32–37 (referrer is `NewHandler`, `internal/api/handler.go|26|0|NewHandler|function`):

| referent_id | ref_kind | identifier | line |
|---|---|---|---|
| `Handler` (struct symbol on line 18) | `type_use` | `Handler` after `&` | 32 |
| no row | (composite-literal field name) | `orderSvc:` | 33 |
| `orderSvc` (parameter at line 27) | `read` | `orderSvc` (RHS of `:`) | 33 |
| ... same pattern for the other three fields | | | |

### Example 5 — Write to a non-local (package-level mutation through pointer)

Source: `internal/worker/dispatcher.go`, lines 101–115.

```go
func (d *Dispatcher) processOne(job Job) JobResult {
    log.Printf("processing job %s (type: %s)", job.ID, job.Type)

    time.Sleep(100 * time.Millisecond)

    d.mu.Lock()
    d.stats.Processed++
    d.mu.Unlock()

    return JobResult{
        JobID:   job.ID,
        Success: true,
    }
}
```

Selected rows (referrer = `internal/worker/dispatcher.go|101|0|processOne|method`):

| referent_id | ref_kind | identifier | line | rationale |
|---|---|---|---|---|
| `log` (file-scope import) | `read` | `log` | 102 | selector receiver |
| `job` (parameter, definition at line 101) | `read` | `job` | 102 | `job.ID` — read of `job` |
| `time` (file-scope import) | `read` | `time` | 105 | selector receiver |
| `d` (receiver, synthesized) | `read` | `d` | 107 | `d.mu` |
| `d` | `read` | `d` | 108 | `d.stats.Processed++` — `d` is read |
| `d` | `read` | `d` | 109 | `d.mu.Unlock()` |
| `JobResult` (symbol at line 21) | `type_use` | `JobResult` | 111 | composite-literal type |
| `job` | `read` | `job` | 112 | `job.ID` |

**Field writes through receiver (updated per `docs/contract-review.md`, policy 5):** `d.stats.Processed++` on line 108. The compound increment writes the `Processed` field. A `write` row is emitted **only** if `Processed` has a known `symbol_id`. When struct fields are extracted as symbols (Phase 2), the resolver emits:

| `<Stats.Processed field symbol>` | `write` | `Processed` | 108 | field-level write |

Pre-Phase 2, no `write` row is produced for the field. The previously-documented synthetic `write` on the base pointer `d` is dropped: receiver `d` produces only a `read` row, as on the surrounding lines.

### Example 6 — Channel send classified as `write`

Source: `internal/worker/dispatcher.go`, line 95.

```go
go func(j Job) {
    result := d.processOne(j)
    d.results <- result
}(job)
```

The send statement `d.results <- result` (line 95) emits:

| referent_id | ref_kind | identifier | line | rationale |
|---|---|---|---|---|
| `d` (receiver of enclosing method `ProcessJobs`) | `read` | `d` | 95 | selector receiver |
| `d` | `write` | `d` | 95 | channel send mutates `d.results`; rule "channel send is write on LHS identifier" |
| `result` (`:=` definition on line 94) | `read` | `result` | 95 | RHS of send |

Note: the closure parameter `j` is a definition (no row); inside the closure, `j` would be a `read` if used. `processOne` call on line 94 emits a `read` of `d` and a `read` of `j` (via `d.processOne(j)`).

### Example 7 — `:=` partial redeclaration (mixed new + existing)

Synthetic example not present verbatim in benchmark, but committed here for clarity. The corpus contains the simpler case at `internal/api/middleware.go` line 47 (`parts := strings.SplitN(...)`) — a clean introduction with no existing binding. For partial redeclaration:

```go
err := f()
if err != nil { return err }
v, err := g()  // err already bound; v is new
```

Rows for line 3:
- `v`: no row (definition, new symbol).
- `err`: `write` row to the existing `err` from line 1.
- `g`: `read` (callee).

This is the canonical Go ambiguity. The contract is: same-scope `:=` partially redeclares; outer-scope `:=` shadows.
