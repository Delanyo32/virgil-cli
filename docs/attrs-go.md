# Language attributes — Go

This contract specifies how the `go_attrs` extension table from `docs/virgil-datalog-schema.md` is populated. Rows in `go_attrs` exist only for symbols whose `symbol.language = "go"`. Symbol ids follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

## Schema

```
:create go_attrs {
    symbol_id: String =>
    is_exported:  Bool   default false,    # capitalized name
    has_receiver: Bool   default false,    # function vs method
    build_tags:   [String] default [],     # //go:build / // +build constraints
}
```

| Column | Applies to | Default |
|---|---|---|
| `is_exported` | All Go symbol kinds (`function`, `method`, `struct`, `interface`, `type_alias`, `constant`, `variable`, and struct fields recorded as `parameter` rows — those get a parallel attribute via the symbol id of the field). | `false` |
| `has_receiver` | Only `function` and `method` symbols. Always `false` for non-callable kinds. | `false` |
| `build_tags` | All Go symbols. Identical value for every symbol declared in the same source file. | `[]` |

A row is emitted in `go_attrs` for *every* Go symbol, even when all three values are at default. This keeps the extension table aligned 1:1 with the Go subset of `symbol` and lets queries `LEFT JOIN`-style joins return present-but-default rows.

## Extraction rules

### `is_exported`

Source: the first rune of the symbol's `name` field as recorded on the `symbol` row.

- `true` iff the first rune is a Unicode uppercase letter (`unicode::IsUpper`).
- The blank identifier `_` is `false`.
- Generic type parameters introduced by `type_parameter_declaration` (e.g. `T` in `func F[T any]`) are not emitted as `symbol` rows, so no `go_attrs` row exists for them. (They appear instead in references-go's resolution; see `references-go.md`.)
- Package-level identifiers with leading underscores or digits are syntactically invalid in Go and never reach the extractor.
- Struct fields and interface method-set entries follow the same rule. `Order.ID` is exported; `Handler.orderSvc` is not.

### `has_receiver`

Source: the tree-sitter node kind of the symbol's `definition` capture in `GO_SYMBOL_QUERY` (see `src/languages/go/queries.rs`).

- `true` iff the definition node is `method_declaration`. This is precisely the Go AST distinction between functions and methods — `func F()` parses to `function_declaration`, `func (r *T) F()` parses to `method_declaration`.
- For all other symbol kinds: `false`.
- Constructor-style functions that *return* a pointer to a type (e.g. `NewHandler`) are *not* methods. `has_receiver = false`.

### `build_tags`

Source: comments at the top of the source file matching either of these forms:

- `//go:build <expr>` — the modern form (Go 1.17+). Captured by tree-sitter as a `comment` node whose text starts with `//go:build `.
- `// +build <constraints>` — the legacy form. Tree-sitter records this as a `comment` node whose text starts with `// +build `.

Both forms may appear; when both appear they must be semantically equivalent (the Go toolchain enforces this), and the extractor records *both* the modern and legacy forms in `build_tags` as separate strings, in source order.

The exact extraction rule:

1. Find the package's "file preamble": the contiguous run of top-level comments (line `//` and block `/* */`) that appears *before* the `package` clause, with at most a single blank line between any two comments and no other syntax in between.
2. For each preamble comment whose text (after stripping `//` or `/* */` markers and trimming whitespace) begins with `go:build ` or `+build `, record the *full expression text following the prefix*, normalized:
   - For `//go:build foo && (bar || baz)` → record `"foo && (bar || baz)"`.
   - For `// +build linux,amd64 darwin` → record `"linux,amd64 darwin"` (legacy syntax: space = OR, comma = AND, leading `!` = NOT). The extractor *does not* translate legacy form into modern boolean form; it preserves the literal expression.
3. A comment that is anywhere *after* the `package` clause is never a build tag, even if its text starts with `//go:build` — that's a regular comment per Go spec.
4. A `//go:build` line *after* a `// +build` line is still valid Go; both still go into `build_tags` in source order.
5. The same `build_tags` value is attached to every symbol in the file. The extractor computes it once per file and broadcasts.
6. If the preamble contains no `go:build` / `+build` lines, `build_tags = []` (the default).

**Edge case — multiple `go:build` lines:** the Go spec allows at most one `//go:build` constraint per file. If a file has two, the Go toolchain rejects it. The extractor does *not* reject; it records both verbatim. Downstream queries that care about validity should count and assert.

**Edge case — `_test.go` and architecture-suffixed filenames:** filenames like `foo_linux.go` or `foo_test.go` carry implicit build constraints in the Go toolchain. The extractor does *not* synthesize entries for these; only explicit comment-based constraints go into `build_tags`. (Decision recorded for clarity. A future enhancement could synthesize filename-implied tags into a separate column.)

**Edge case — CRLF line endings:** the comment-prefix check is on the trimmed text, so `\r` is stripped before the `go:build`/`+build` match.

## Worked examples

### Example 1 — Exported function with no receiver, no build tags

Source: `internal/api/handler.go`, line 26 (the `NewHandler` constructor).

```go
func NewHandler(
    orderSvc        *service.OrderService,
    inventorySvc    *service.InventoryService,
    paymentSvc      *service.PaymentService,
    notificationSvc *service.NotificationService,
) *Handler {
```

Symbol id: `internal/api/handler.go|26|0|NewHandler|function`.

`go_attrs` row:

```
symbol_id:    "internal/api/handler.go|26|0|NewHandler|function"
is_exported:  true
has_receiver: false
build_tags:   []
```

Rationale: `NewHandler` begins with uppercase `N` (`is_exported = true`); the AST node is `function_declaration` not `method_declaration` (`has_receiver = false`); `internal/api/handler.go` has no build-tag preamble (`build_tags = []`).

### Example 2 — Unexported method on a pointer receiver

Source: `internal/worker/dispatcher.go`, line 101 (the `processOne` method).

```go
func (d *Dispatcher) processOne(job Job) JobResult {
```

Symbol id: `internal/worker/dispatcher.go|101|0|processOne|method`.

`go_attrs` row:

```
symbol_id:    "internal/worker/dispatcher.go|101|0|processOne|method"
is_exported:  false
has_receiver: true
build_tags:   []
```

Rationale: `processOne` begins with lowercase `p` (`is_exported = false`); the AST node is `method_declaration` (`has_receiver = true`); no build-tag preamble in this file.

The receiver type itself, `Dispatcher`, has its own symbol row `internal/worker/dispatcher.go|35|0|Dispatcher|struct` with:

```
symbol_id:    "internal/worker/dispatcher.go|35|0|Dispatcher|struct"
is_exported:  true
has_receiver: false
build_tags:   []
```

(`Dispatcher` is uppercase; structs cannot have receivers — `has_receiver` is always `false` for `struct`/`interface`/`type_alias`/`constant`/`variable` kinds.)

### Example 3 — Constants with mixed export status declared in a `const ( … )` block

Source: `internal/model/order.go`, lines 30–38.

```go
const (
    OrderStatusPending       = "pending"
    OrderStatusConfirmed     = "confirmed"
    OrderStatusProcessing    = "processing"
    OrderStatusShipped       = "shipped"
    OrderStatusDelivered     = "delivered"
    OrderStatusCancelled     = "cancelled"
    OrderStatusPaymentFailed = "payment_failed"
)
```

Each `const_spec` becomes its own `symbol` row of kind `constant`. Sample rows in `go_attrs`:

```
symbol_id:    "internal/model/order.go|31|4|OrderStatusPending|constant"
is_exported:  true
has_receiver: false
build_tags:   []
```

```
symbol_id:    "internal/model/order.go|37|4|OrderStatusPaymentFailed|constant"
is_exported:  true
has_receiver: false
build_tags:   []
```

All seven constants are exported (uppercase first rune); `has_receiver` is structurally `false` for constants; no build-tag preamble.

Contrast with an unexported package-level binding from `internal/api/middleware.go` line 75 (`var mu sync.Mutex` is inside a function so isn't symbolized as package-level; the closest example in the corpus of an *unexported package-level constant* is absent). The contract still commits to: a hypothetical `const maxRetries = 5` at package scope would produce `is_exported = false`.

### Example 4 — Build tags from a `//go:build` preamble (synthetic; no in-corpus example)

The benchmark `benchmarks/go/http-service/` contains no files with build-constraint preambles. The contract commits to the following behavior, to be verified by extractor unit tests:

Source (hypothetical `internal/worker/cleanup_linux.go`):

```go
//go:build linux && !arm
// +build linux,!arm

package worker

func PlatformCleanup() {}
```

`go_attrs` row for the function:

```
symbol_id:    "internal/worker/cleanup_linux.go|6|0|PlatformCleanup|function"
is_exported:  true
has_receiver: false
build_tags:   ["linux && !arm", "linux,!arm"]
```

Note the source-order preservation: the modern `//go:build` form appears first in the file, so it appears first in the list. The legacy `// +build` form is recorded verbatim, not translated. Every other symbol in this file would receive *the same* `build_tags` value.

### Example 5 — Field of a struct (non-obvious source: struct field becomes a symbol)

Source: `internal/model/order.go`, line 11 (the `Status` field of `Order`).

```go
type Order struct {
    ID              int         `json:"id"`
    UserID          int         `json:"user_id"`
    Status          string      `json:"status"`
    ...
}
```

Struct fields are recorded primarily as `parameter` rows whose `function_id` is the struct symbol (per `types-go.md`'s convention). They are *also* materialized as `symbol` rows of kind `field` — this is the standard cross-language symbol shape — and therefore get a `go_attrs` row.

Symbol id: `internal/model/order.go|11|4|Status|field`.

`go_attrs` row:

```
symbol_id:    "internal/model/order.go|11|4|Status|field"
is_exported:  true
has_receiver: false
build_tags:   []
```

The non-obvious construct here: `Status` is *exported* because Go's export rule applies to identifier names regardless of whether they're top-level or nested in a struct. Querying for "all exported names in this file" returns struct fields too, not just package-level declarations.

### Example 6 — Anonymous struct fields don't reach this table

Source: `internal/api/handler.go`, lines 66–72.

```go
var req struct {
    Items []struct {
        ProductID int `json:"product_id"`
        Quantity  int `json:"quantity"`
    } `json:"items"`
    ShippingAddress string `json:"shipping_address"`
}
```

These are anonymous structs declared inside a function. The fields (`Items`, `ProductID`, `Quantity`, `ShippingAddress`) are *not* emitted as `symbol` rows at this contract level — the schema only materializes fields of *named* struct types. Therefore no `go_attrs` rows are emitted for them.

This is a deliberate scope choice. If a future revision wants to capture anonymous-struct fields, it would add `symbol` rows with `parent_id` pointing to the enclosing `var` symbol and a synthesized name; `go_attrs` would follow automatically because the extractor broadcasts the file's `build_tags` to every symbol-row-in-file regardless of nesting.
