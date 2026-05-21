# Types — Go

This contract maps Go type expressions to rows in the `type` relation defined in `docs/virgil-datalog-schema.md`, at the Level-3 depth committed to in [ADR-0003](adr/0003-level-3-types-and-references.md). Symbol ids in worked examples follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

The extractor walks every node that appears in a type position (parameter types, return types, struct field types, map key/value types, channel element types, type-spec bodies, type-assertion targets, conversion targets, composite-literal types, type-parameter bounds). A single `type` row is emitted per occurrence, deduplicated per file by `id = blake3("go" | file_id | display_name)`.

## Tree-sitter node kinds

| Node kind | Source it represents | Schema `kind` |
|---|---|---|
| `type_identifier` | A bare named type (`int`, `Order`, `time.Duration` after the `qualified_type` wrapper). | `primitive` if name is a Go built-in (see list below); otherwise `named`. |
| `qualified_type` | `pkg.Name` (e.g. `model.Order`, `http.Handler`). | `named`. |
| `pointer_type` | `*T`. | `generic`. One type argument: the referent type `T`. Updated per `docs/contract-review.md` (policy 2) — pointer / reference types across Rust/C/C++/Go all map to `kind = "generic"` with a single type arg, keeping the schema's 7-kind closed set intact. `display_name` retains the `*` punctuation; `canonical_name` includes the `*` as well. |
| `slice_type` | `[]T`. | `array`. Length is `-1` (the schema doesn't carry it; the `display_name` preserves the textual form). |
| `array_type` | `[N]T` and `[...]T`. | `array`. |
| `map_type` | `map[K]V`. | `generic`. Two type arguments in canonical order: key first, value second. |
| `channel_type` | `chan T`, `chan<- T`, `<-chan T`. | `generic`. One type argument. Direction is recorded in `display_name` only. |
| `function_type` | `func(A, B) (C, error)`. | `function`. |
| `interface_type` | Anonymous interface bodies (`interface { Read() }`) and the right-hand side of `type X interface { ... }` declarations. | `named`. |
| `struct_type` | Anonymous struct types (`struct { X int }`) and the RHS of `type X struct { ... }`. | `named`. |
| `generic_type` | Instantiated generic (`Vec[int]`, `sync.Map[string, int]`). | `generic`. |
| `parenthesized_type` | `(T)`. | Transparent — emit the row for the inner type only; the parentheses are stripped from `display_name`. |
| `type_parameter_list` children (`type_parameter_declaration` → `type_identifier`) | `T` inside `func F[T any](...)`. | `named`, with `canonical_name = null` (see "unresolved" below). |

**Primitive list (treated as `kind = "primitive"` regardless of import):** `bool`, `byte`, `rune`, `string`, `error`, `int`, `int8`, `int16`, `int32`, `int64`, `uint`, `uint8`, `uint16`, `uint32`, `uint64`, `uintptr`, `float32`, `float64`, `complex64`, `complex128`, `any`, `comparable`. `error` and `any` are predeclared identifiers; they are treated as primitives even though `error` is technically an interface, because they need no scope walk and no import.

Go has no union or intersection types; those `kind` variants are never emitted for Go.

Tuples: Go return-type tuples (`(int, error)`) are *not* emitted as a single `tuple` row. Each return type produces its own `returns_type` row (see Signatures below) with `index` set on a per-language extension if needed — at Level 3 the schema's `returns_type` is single-keyed by `function_id`, so Go records only the *first* return value's `type_id` in `returns_type`; additional returns are stored as `parameter` rows with negative `index` (`-1`, `-2`, …) and `name` set to `_ret<n>`. State explicitly: this is the Go-specific convention for multi-return.

## `display_name` construction

`display_name` is the source text of the type expression, with the following normalizations applied in order:

1. All run-of-whitespace (including newlines) collapses to a single space.
2. No space inside `[]`, `[...]`, `[N]`, `*`, `&`, `<-`, `chan<-`, `<-chan` — i.e. spaces are removed adjacent to these tokens (`[ ]T` → `[]T`).
3. Exactly one space after each comma in parameter/argument lists, none before (`map[ K , V ]` → `map[K, V]`).
4. Exactly one space between `func` and `(`, none between `(` and the first parameter, none between the last parameter and `)`.
5. Field tags inside struct types (the backtick-quoted strings after a field) are stripped from `display_name`. They live on `parameter`/`field` rows, not on the `type` row, because they are not part of the type identity in Go's type system.
6. Comments inside the type expression are stripped.
7. Qualified types (`pkg.Name`) keep the dot with no spaces: `time.Duration`, not `time . Duration`.

This guarantees round-trip: `map[ string ] int` and `map[string]int` both produce `display_name = "map[string]int"`.

## `canonical_name` resolution

Per ADR-0003 every resolvable `type` row gets a `canonical_name`. The Go resolver works as follows.

### Scope walk order (most-local first)

1. Function/method type parameters introduced by the enclosing `type_parameter_list`. These resolve to `<package>.<containing_func>.<T>` — but per "unresolved" below, type parameters get `canonical_name = null`, so this lookup primarily exists to *prevent* a later step from matching `T` against a package-level type.
2. The current file's `type` and `const`/`var` declarations of named types (Go does not allow nested type declarations inside functions, so this step is per-file but flat).
3. The current package's exported and unexported symbols across all files in the same directory (Go's package scope is directory-wide). The resolver uses the existing `symbols_by_name` index from `src/graph/builder.rs`, filtered by `language = "go"` and `file_id` having the same parent directory as the type's `file_id`.
4. The current file's imports. A `qualified_type` `pkg.Name` resolves by matching `pkg` against either (a) the import's `local_name` (when aliased: `import foo "github.com/.../bar"` makes `pkg` = `foo`) or (b) the last `/`-segment of the import path (when unaliased). If a match is found, `canonical_name = "<full-import-path>.<Name>"`.
5. The Go built-in universe block (`int`, `error`, `string`, `any`, …): canonical name is the bare identifier, no package prefix.

### What counts as "unresolved" (`canonical_name = null`)

- Generic type parameters introduced by the enclosing function (`T` in `func F[T any](x T)`).
- Types from imports whose package is not indexed in this workspace (e.g. `sql.DB` when `database/sql` is an external standard-library package).
- Types from a dot-import (`import . "pkg"`) when the underlying package is not in the workspace — Go's dot-import injects names into the current file's scope; without the package indexed we cannot enumerate them.
- Types nested inside an anonymous `struct_type` or `interface_type` whose enclosing symbol has no name (e.g. the inline `var req struct { … }` in `handler.go`) — the anonymous parent has no canonical path, so its members are also unresolvable.
- Parse failures (tree-sitter `ERROR` node in the type subtree).

### Aliases

Go has two forms:

- `type Foo = Vec[uint8]` (true alias, `=` present): the alias *and* its underlying type both produce `type` rows. The alias row gets `display_name = "Foo"`, `canonical_name = "<pkg>.Foo"`, `kind = "named"`. The underlying-type row is emitted separately by the RHS walk. **References that name `Foo` resolve to the alias row, not to `Vec[uint8]`.** No collapsing.
- `type Foo Vec[uint8]` (defined type, no `=`): a brand-new named type. Same row shape as above; the underlying type is still emitted as a separate row.

### Generic argument rendering

`canonical_name` for a `generic_type` instantiation uses the canonical names of the arguments where resolvable, separated by `, `: `canonical_name("Vec[int]")` is `<pkg-of-Vec>.Vec[int]` (built-in `int` keeps its bare name). If any argument is unresolvable, the whole `canonical_name` is `null` — partial canonicalization is not supported.

## Identity

Per ADR-0003: `type.id = blake3("go" | file_id | display_name)`. The pre-hash inputs are joined with `\x00` separators. No additional normalization beyond the `display_name` rules above. Dedup is per file: two occurrences of `*Order` in the same file produce one row; the same `*Order` in a different file produces a second row with a different id but (when resolution succeeds) the same `canonical_name`.

## Field types — `field_type` relation

Per the schema, every struct-field symbol with a typed declaration emits a `field_type {symbol_id, type_id}` row linking the field symbol to its `type` row. This is the field-level analogue of `parameter` and `returns_type`. Go struct fields (`type T struct { F int }`) qualify; map and channel element types do not (they're already covered as type arguments). Untyped or anonymous-struct fields whose containing struct has no name produce no row.

## Multi-return signatures

Go functions can return tuples. The schema's `returns_type` is single-valued. Go-specific convention:

- The *first* return type is recorded in `returns_type{function_id, type_id}`.
- Additional return types are recorded as `parameter` rows with `index = -1, -2, …`, `name = "_ret1"`, `"_ret2"`, …, `type_id = <id of that type's row>`, `is_optional = false`, `has_default = false`.
- If a function returns nothing, no `returns_type` row is emitted.
- Named return values (`func f() (n int, err error)`) keep their source name on the parameter rows (`name = "n"`, `name = "err"`) instead of `_retN`.

## Worked examples

### Example 1 — `pointer_type` wrapping a `qualified_type`

Source: `internal/api/handler.go`, lines 26–31 (the `NewHandler` signature).

```go
func NewHandler(
    orderSvc        *service.OrderService,
    inventorySvc    *service.InventoryService,
    paymentSvc      *service.PaymentService,
    notificationSvc *service.NotificationService,
) *Handler {
```

`type` rows emitted for this signature (file_id is `internal/api/handler.go`):

| id (truncated) | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `<hash>` | generic | go | `*service.OrderService` | `*github.com/example/ordersvc/internal/service.OrderService` |
| `<hash>` | generic | go | `*service.InventoryService` | `*github.com/example/ordersvc/internal/service.InventoryService` |
| `<hash>` | generic | go | `*service.PaymentService` | `*github.com/example/ordersvc/internal/service.PaymentService` |
| `<hash>` | generic | go | `*service.NotificationService` | `*github.com/example/ordersvc/internal/service.NotificationService` |
| `<hash>` | generic | go | `*Handler` | `*github.com/example/ordersvc/internal/api.Handler` |

**Note on the `*` in `canonical_name`:** updated per `docs/contract-review.md` (policy 2). The leading `*` is preserved in both `display_name` and `canonical_name`. Pointer types are encoded as `kind = "generic"` over a single type argument (the referent). Queries that need pointer-vs-value can filter on the leading `*` in either column; queries that want "every use of `OrderService` regardless of pointer-ness" join on the inner type's `canonical_name`.

Corresponding `parameter` rows on the symbol `NewHandler` (`function_id = internal/api/handler.go|26|0|NewHandler|function`):

| index | name | type_id |
|---|---|---|
| 0 | `orderSvc` | id of `*service.OrderService` |
| 1 | `inventorySvc` | id of `*service.InventoryService` |
| 2 | `paymentSvc` | id of `*service.PaymentService` |
| 3 | `notificationSvc` | id of `*service.NotificationService` |

`returns_type{function_id: "internal/api/handler.go|26|0|NewHandler|function", type_id: <id of *Handler>}`.

### Example 2 — `slice_type` and `array_type`

Source: `internal/model/order.go`, line 14 (the `Items []OrderItem` field of `Order`).

```go
Items           []OrderItem `json:"items,omitempty"`
```

`type` row for `[]OrderItem` (file_id = `internal/model/order.go`):

```
id:             blake3("go" \x00 "internal/model/order.go" \x00 "[]OrderItem")
kind:           "array"
language:       "go"
display_name:   "[]OrderItem"
canonical_name: "github.com/example/ordersvc/internal/model.OrderItem[]"
```

Canonical-name convention for slices: the element's canonical name followed by `[]`. Fixed arrays (`[N]T`) canonicalize as `<element>[N]`.

The `OrderItem` symbol itself produces its own named-type row via the `type X struct { ... }` declaration on line 20 of the same file; its `canonical_name` is `github.com/example/ordersvc/internal/model.OrderItem`. The slice's canonical-name lookup goes through that row.

### Example 3 — `map_type` (`kind = "generic"`)

Source: `internal/repository/category_repo.go`, line 13.

```go
type CategoryRepository struct {
    mu         sync.RWMutex
    categories map[int]model.Category
    nextID     int
}
```

`type` row for `map[int]model.Category` (file_id = `internal/repository/category_repo.go`):

```
id:             blake3("go" \x00 "internal/repository/category_repo.go" \x00 "map[int]model.Category")
kind:           "generic"
language:       "go"
display_name:   "map[int]model.Category"
canonical_name: "map[int]github.com/example/ordersvc/internal/model.Category"
```

Also emitted for the same struct body: a row for `sync.RWMutex` (kind `named`, canonical_name `sync.RWMutex` if `sync` is not indexed → `null`; in this workspace `sync` is external → `canonical_name = null`), and a row for `int` (kind `primitive`, canonical_name `int`).

### Example 4 — `channel_type` (`kind = "generic"`)

Source: `internal/worker/dispatcher.go`, lines 38 and 41 (the `Dispatcher` struct fields).

```go
results    chan JobResult
stopCh     chan struct{}
```

Two `type` rows:

```
id:             blake3("go" \x00 "internal/worker/dispatcher.go" \x00 "chan JobResult")
kind:           "generic"
language:       "go"
display_name:   "chan JobResult"
canonical_name: "chan github.com/example/ordersvc/internal/worker.JobResult"
```

```
id:             blake3("go" \x00 "internal/worker/dispatcher.go" \x00 "chan struct{}")
kind:           "generic"
language:       "go"
display_name:   "chan struct{}"
canonical_name: null
```

The second has `canonical_name = null` because the element type is an anonymous `struct_type` with no name to canonicalize against. The anonymous-struct row itself is also emitted with `display_name = "struct{}"` and `canonical_name = null`.

Directional channels: `chan<- T` produces `display_name = "chan<- T"`, canonical `chan<- <T>`; `<-chan T` produces `display_name = "<-chan T"`, canonical `<-chan <T>`. Direction is part of identity — bidirectional and unidirectional channels are different types.

### Example 5 — `function_type` (`kind = "function"`)

Source: `internal/api/middleware.go`, line 15 (the return type of `LoggingMiddleware`).

```go
func LoggingMiddleware(logr *logger.Logger) func(http.Handler) http.Handler {
```

The return-type expression is `func(http.Handler) http.Handler`. `type` row:

```
id:             blake3("go" \x00 "internal/api/middleware.go" \x00 "func(http.Handler) http.Handler")
kind:           "function"
language:       "go"
display_name:   "func(http.Handler) http.Handler"
canonical_name: "func(net/http.Handler) net/http.Handler"
```

The parameter type `*logger.Logger` emits a separate `generic` row (per policy 2) with `display_name = "*logger.Logger"` and `canonical_name = "*github.com/example/ordersvc/pkg/logger.Logger"`.

This function returns *another* function whose return type is `http.Handler` — that is reflected only in the nested function-type's `display_name`/`canonical_name`. The schema does not flatten nested function types into separate rows; the whole `func(...) ...` text is one row.

### Example 6 — `struct_type` declaration (`kind = "named"`)

Source: `internal/model/order.go`, lines 8–17.

```go
type Order struct {
    ID              int         `json:"id"`
    UserID          int         `json:"user_id"`
    Status          string      `json:"status"`
    TotalCents      int         `json:"total_cents"`
    ShippingAddress string      `json:"shipping_address"`
    Items           []OrderItem `json:"items,omitempty"`
    CreatedAt       time.Time   `json:"created_at"`
    UpdatedAt       time.Time   `json:"updated_at"`
}
```

The named-type declaration produces *one* row for `Order` itself:

```
id:             blake3("go" \x00 "internal/model/order.go" \x00 "Order")
kind:           "named"
language:       "go"
display_name:   "Order"
canonical_name: "github.com/example/ordersvc/internal/model.Order"
```

Each field type produces its own row in the same file:
- `int` (kind `primitive`, dedup'd across fields with the same display_name).
- `string` (primitive).
- `[]OrderItem` (array — same row as Example 2 because it's in a different file's namespace; here a new row with file_id `internal/model/order.go`).
- `time.Time` (named, canonical `time.Time` → `null` because `time` is external).

Each struct-field symbol gets a `field_type` row linking it to its type row (per `docs/virgil-datalog-schema.md`). For example, the field symbol for `Items` gets `field_type {symbol_id: "<...Items field id>", type_id: <id of "[]OrderItem">}`. The earlier convention of repurposing `parameter` rows for struct fields is dropped — `parameter` is for function parameters only.

### Example 7 — `interface_type` declaration

Source (synthetic from the codebase pattern; the closest in-corpus example is the dispatcher's `Job` struct — the benchmark has no top-level named interface, so this example uses the test pattern):

Not present in `benchmarks/go/http-service/` — instead, the contract commits to the following behavior, verified by extractor unit tests:

```go
type Reader interface { Read() }
```

would produce:

```
id:             blake3("go" \x00 <file_id> \x00 "Reader")
kind:           "named"
language:       "go"
display_name:   "Reader"
canonical_name: "<pkg>.Reader"
```

The method set inside the interface body produces `symbol` rows of kind `method` whose `parent_id` is the `Reader` symbol; their parameter and return types follow the rules above. Method-set entries do not emit `type` rows for the interface body itself.
