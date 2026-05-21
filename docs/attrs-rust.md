# Language attributes — Rust

Contract for `rust_attrs` rows. Conforms to the schema in
[virgil-datalog-schema.md](virgil-datalog-schema.md). Symbol IDs in
all worked examples follow [ADR-0002](adr/0002-symbol-id-scheme.md):
`path|start_line|start_col|name|kind`.

## Schema

```
:create rust_attrs {
    symbol_id: String =>
    is_unsafe:        Bool      default false,
    is_const:         Bool      default false,
    derives:          [String]  default [],
    is_extern:        Bool      default false,
    abi:              String?   default null,
    is_test:          Bool      default false,
    is_ignored_test:  Bool      default false,
    cfg:              [String]  default [],
    type_parameters:  [String]  default [],
    lifetime_parameters: [String] default [],
    visibility_kind:  String    default "private",
    is_proc_macro:    Bool      default false,
}
```

The base `symbol` relation already carries `is_async`, `is_static`,
`is_mutable`, and a `visibility` enum — those are not duplicated here.
`rust_attrs` rows exist **only** for symbols whose `language = "rust"`.

### Applicability per column

| column                | applies to                                                                 |
|-----------------------|----------------------------------------------------------------------------|
| `is_unsafe`           | functions, methods, blocks-as-attribute-of-enclosing-function, traits, impls (`unsafe impl`), modules (`unsafe mod`) |
| `is_const`            | functions, methods, constants (`const` items), associated constants       |
| `derives`             | structs, enums, unions                                                    |
| `is_extern`           | functions, statics (`extern "C" fn`, `extern "C" { static ... }`)         |
| `abi`                 | functions and function types declared `extern`; value is the ABI string (`"C"`, `"Rust"`, `"system"`, …); `null` for non-extern symbols |
| `is_test`             | functions annotated `#[test]`                                             |
| `is_ignored_test`     | functions annotated `#[test]` *and* `#[ignore]`                           |
| `cfg`                 | any symbol carrying one or more `#[cfg(...)]` attributes; stores the **raw textual predicate** strings, one per `#[cfg]` |
| `type_parameters`     | functions, methods, structs, enums, unions, traits, type aliases, impls — names of generic type parameters, **in source order** |
| `lifetime_parameters` | same scope as `type_parameters` — names of lifetime parameters (`'a`, `'b`, ...) in source order |
| `visibility_kind`     | every symbol — coarse-grained kind: `"private"`, `"pub"`, `"pub(crate)"`, `"pub(super)"`, `"pub(in <path>)"`. (`symbol.visibility` already carries a coarse public/private bit; `rust_attrs.visibility_kind` preserves the **textual restriction**.) |
| `is_proc_macro`       | functions annotated with `#[proc_macro]`, `#[proc_macro_derive(...)]`, or `#[proc_macro_attribute]` |

A row is emitted for every Rust symbol regardless of whether any
non-default column is populated — rationale: cheaper to keep
`rust_attrs` 1:1 with Rust symbols than to require every query to
left-join and coalesce defaults. Storage cost is negligible
(small fixed columns plus two usually-empty lists).

## Extraction rules

### `is_unsafe`

- `function_item` with an `unsafe` keyword child (`unsafe fn foo` /
  `unsafe extern "C" fn foo`) → `true`.
- `impl_item` whose `unsafe` keyword is present (`unsafe impl Trait
  for T`) → `true`. The impl block contributes one `symbol` row
  (kind `"impl"`), which carries `is_unsafe = true`. Methods inside
  do **not** automatically inherit; each method's own `is_unsafe`
  is independent.
- `trait_item` with `unsafe` keyword (`unsafe trait T`) → `true`.
- A function that **contains** an `unsafe { ... }` block but is
  not itself marked `unsafe` → `is_unsafe = false`. Rationale: the
  function's *signature* is safe; downstream queries that want
  "functions that use `unsafe` internally" join through a separate
  `unsafe_block` indicator (out of scope for this attrs table).
- Default: `false`.

### `is_const`

- `function_item` / `function_signature_item` with `const` keyword
  → `true`.
- `const_item` (a constant definition like `const FNV_OFFSET: u64
  = ...`) → `true`. (The `is_const` flag is redundant with the
  `symbol.kind = "constant"` for `const_item`s; we set it anyway
  for uniform queries — "any const-callable thing".)
- Associated constants in trait/impl bodies → `true`.
- `static_item` is **not** `is_const` (statics are not
  const-evaluable in the same sense).
- Default: `false`.

### `derives`

- For each `#[derive(...)]` attribute on a `struct_item`,
  `enum_item`, `union_item`, take the comma-separated trait list
  inside the parens and append each trimmed identifier to the
  `derives` list.
- Multiple `#[derive(...)]` attributes on the same item are
  concatenated **in source order**.
- Path-qualified derives (`#[derive(serde::Serialize)]`) are
  stored verbatim as `serde::Serialize` (rationale: queries
  matching `derives` against external proc-macro crates need the
  full path; trimming to the rightmost segment would collide
  with same-named built-in derives).
- Default: `[]`.

### `is_extern` / `abi`

- `function_item` whose first modifier is `extern` (with or
  without an explicit ABI literal) → `is_extern = true`. `abi`
  is the literal string from the `extern "..."` clause if present
  (e.g. `"C"`, `"system"`, `"Rust"`), else `"C"` (the default ABI
  for bare `extern fn`).
- `function_signature_item` declared inside a `foreign_mod_item`
  (`extern "C" { fn ...; }`) → `is_extern = true`, `abi` is the
  parent block's ABI string.
- `static_item` inside a `foreign_mod_item` → `is_extern = true`,
  `abi` is the parent block's ABI.
- Otherwise: `is_extern = false`, `abi = null`.

### `is_test` / `is_ignored_test`

- `function_item` with any attribute path equal to `test` or
  `tokio::test` or `async_std::test` (whitelist of common test
  attributes; entries match the rightmost path segment when
  unqualified, full path when qualified) → `is_test = true`.
- A `#[test]` function additionally annotated with `#[ignore]` (or
  `#[ignore = "reason"]`) → `is_ignored_test = true`.
- Default: both `false`.

### `cfg`

- For each `#[cfg(...)]` attribute on the symbol, store the **raw
  textual content** between the outer parens, after whitespace
  normalization (collapse runs of whitespace to a single space).
  Example: `#[cfg(target_os = "linux")]` → store
  `"target_os = \"linux\""`.
- `#[cfg_attr(predicate, inner_attr)]` is **not** added to the
  `cfg` list. Rationale: `cfg_attr` is a *conditional attribute
  application*, not a "this item exists under predicate" gate;
  modeling it here would mislead consumers. Such attributes are
  ignored by this column.
- Multiple `#[cfg]` attributes on the same item are stored as
  separate list entries in source order (rationale: ambiguous
  ANDing/ORing across multiple attributes is left to the
  consumer; we record what was written, not a synthesized
  predicate). This is the "pick the first cfg / union of all"
  decision: **union of all**, in source order, raw text.
- Default: `[]`.

### `type_parameters` / `lifetime_parameters`

- Walk the symbol's `type_parameters` AST child if present
  (`function_item`, `impl_item`, `trait_item`, struct/enum/union/
  type-alias items). For each child:
  - `lifetime` node → append the verbatim lifetime name
    (including the leading `'`) to `lifetime_parameters`.
  - `type_identifier` node → append the parameter name to
    `type_parameters`.
  - `constrained_type_parameter` (a type parameter with bounds,
    `T: Display`) → append only the parameter name; bounds are
    not stored on `rust_attrs`. (Rationale: bound information is
    available through `references` rows of `ref_kind = type_use`
    against the same symbol's signature; duplicating it here
    would create a second source of truth.)
  - `const_parameter` (`const N: usize`) → append the parameter
    name to `type_parameters` (we do not currently separate
    `const_parameter` from `type_parameter`; this is a
    deliberate simplification — downstream queries that need to
    distinguish const generics can join through `parameter`/
    `type` rows).
- Both lists in source-declaration order.
- Default: `[]`.

### `visibility_kind`

- Read the optional `visibility_modifier` child of the symbol's
  definition node.
- Absent → `"private"`.
- `pub` (bare) → `"pub"`.
- `pub(crate)` → `"pub(crate)"`.
- `pub(super)` → `"pub(super)"`.
- `pub(self)` → `"pub(self)"` (treated separately from `"private"`
  even though they have identical semantics — rationale: the
  source author wrote it explicitly).
- `pub(in path::to::module)` → `"pub(in path::to::module)"` (with
  whitespace normalized).
- Default: `"private"`.

### `is_proc_macro`

- `function_item` with an attribute matching any of the procedural-
  macro markers (rightmost path segment equals one of
  `proc_macro`, `proc_macro_derive`, `proc_macro_attribute`) →
  `true`.
- Default: `false`.

## Worked examples

Each example cites a real path inside
`/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/rust/systems-cli/`.

### Example 1 — `derives` on a struct (plain `#[derive(...)]`)

**Source.** `src/utils/hash.rs:9-12`

```rust
/// A computed content hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentHash {
    value: u64,
}
```

`symbol_id`: `src/utils/hash.rs|11|11|ContentHash|struct`.

**`rust_attrs` row:**

| column                 | value                                                  |
|------------------------|--------------------------------------------------------|
| `symbol_id`            | `"src/utils/hash.rs\|11\|11\|ContentHash\|struct"`      |
| `is_unsafe`            | `false`                                                |
| `is_const`             | `false`                                                |
| `derives`              | `["Clone", "Debug", "PartialEq", "Eq"]`                |
| `is_extern`            | `false`                                                |
| `abi`                  | `null`                                                 |
| `is_test`              | `false`                                                |
| `is_ignored_test`      | `false`                                                |
| `cfg`                  | `[]`                                                   |
| `type_parameters`      | `[]`                                                   |
| `lifetime_parameters`  | `[]`                                                   |
| `visibility_kind`      | `"pub"`                                                |
| `is_proc_macro`        | `false`                                                |

Notes: derive order matches source. `visibility_kind = "pub"` is
the load-bearing distinction from `private`.

### Example 2 — `is_unsafe` and `visibility_kind` on a function with mutable static access

**Source.** `src/plugins/registry.rs:32-38` and `41-54`

```rust
pub fn init_registry() {
    unsafe {
        if PLUGINS.is_none() {
            PLUGINS = Some(Vec::new());
        }
    }
}
```

```rust
pub fn register_plugin(info: PluginInfo) -> Result<(), String> {
    unsafe {
        ...
    }
}
```

For `init_registry`, `symbol_id` =
`src/plugins/registry.rs|32|7|init_registry|function`:

| column            | value                                                    |
|-------------------|----------------------------------------------------------|
| `symbol_id`       | `"src/plugins/registry.rs\|32\|7\|init_registry\|function"` |
| `is_unsafe`       | `false` (function signature is safe — the `unsafe { ... }` is *inside* the body) |
| `is_const`        | `false`                                                  |
| `derives`         | `[]`                                                     |
| `is_extern`       | `false`                                                  |
| `abi`             | `null`                                                   |
| `is_test`         | `false`                                                  |
| `is_ignored_test` | `false`                                                  |
| `cfg`             | `[]`                                                     |
| `type_parameters` | `[]`                                                     |
| `lifetime_parameters` | `[]`                                                 |
| `visibility_kind` | `"pub"`                                                  |
| `is_proc_macro`   | `false`                                                  |

The **non-obvious** point: even though the body uses `unsafe`,
`is_unsafe` is `false`. This is the decision called out in the
extraction rules — `is_unsafe` tracks the *signature*, not the
body. Queries that want "functions with `unsafe` blocks inside"
must use a separate signal.

### Example 3 — `is_const` on a `const_item`

**Source.** `src/utils/hash.rs:31-32` (inside `hash_bytes`)

```rust
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
```

For `FNV_OFFSET`, `symbol_id` =
`src/utils/hash.rs|31|10|FNV_OFFSET|constant`:

| column                | value                                                       |
|-----------------------|-------------------------------------------------------------|
| `symbol_id`           | `"src/utils/hash.rs\|31\|10\|FNV_OFFSET\|constant"`          |
| `is_unsafe`           | `false`                                                     |
| `is_const`            | `true`                                                      |
| `derives`             | `[]`                                                        |
| `is_extern`           | `false`                                                     |
| `abi`                 | `null`                                                      |
| `is_test`             | `false`                                                     |
| `is_ignored_test`     | `false`                                                     |
| `cfg`                 | `[]`                                                        |
| `type_parameters`     | `[]`                                                        |
| `lifetime_parameters` | `[]`                                                        |
| `visibility_kind`     | `"private"` (no visibility modifier; function-local consts are not addressable from outside anyway) |
| `is_proc_macro`       | `false`                                                     |

`is_const = true` is redundant with `symbol.kind = "constant"`
here; the column is set so a single predicate
`*rust_attrs{symbol_id, is_const: true}` returns both `const fn`
items and `const` items in one query.

### Example 4 — Path-qualified derive (non-obvious extraction)

**Source.** `src/core/cache.rs:10-11`

```rust
#[derive(Clone, Debug)]
pub struct CacheEntry {
```

For `CacheEntry`, `symbol_id` =
`src/core/cache.rs|11|11|CacheEntry|struct`:

| column                | value                                              |
|-----------------------|----------------------------------------------------|
| `symbol_id`           | `"src/core/cache.rs\|11\|11\|CacheEntry\|struct"`   |
| `derives`             | `["Clone", "Debug"]`                               |
| `visibility_kind`     | `"pub"`                                            |
| (others)              | defaults                                           |

Non-obvious bit: when a derive is path-qualified like
`#[derive(serde::Serialize)]` (no such case exists in this
corpus, but the rule applies), the **full path** is stored
verbatim. This worked example uses the simple form found in the
corpus; the contract rule is documented in the extraction-rules
section above.

### Example 5 — `type_parameters` on a type alias

**Source.** `src/core/processor.rs:17-20`

```rust
pub type DataMap = HashMap<String, String>;

/// Intermediate representation for structured data rows.
pub type DataTable = Vec<HashMap<String, String>>;
```

(Both type aliases are non-generic in this corpus.)

For `DataMap`, `symbol_id` =
`src/core/processor.rs|17|9|DataMap|type_alias`:

| column                | value                                                    |
|-----------------------|----------------------------------------------------------|
| `symbol_id`           | `"src/core/processor.rs\|17\|9\|DataMap\|type_alias"`     |
| `derives`             | `[]`                                                     |
| `type_parameters`     | `[]`                                                     |
| `lifetime_parameters` | `[]`                                                     |
| `visibility_kind`     | `"pub"`                                                  |
| (others)              | defaults                                                 |

To show the *populated* shape: the existing-code test in
`src/languages/rust_lang/queries.rs` exercises
`type Result<T> = std::result::Result<T, Error>;`. If that
construct appeared at, say, `src/example.rs:1:0`, the row would be:

| column                | value                                              |
|-----------------------|----------------------------------------------------|
| `symbol_id`           | `"src/example.rs\|1\|5\|Result\|type_alias"`        |
| `type_parameters`     | `["T"]`                                            |
| `visibility_kind`     | `"private"`                                        |
| (others)              | defaults                                           |

This synthesized variation is shown only to demonstrate the
populated shape — it is **not** from the benchmark corpus and is
flagged as such. The real corpus rows for the two `DataMap` /
`DataTable` aliases have empty `type_parameters`.

### Example 6 — `cfg` raw-textual storage (non-obvious AST)

The benchmark corpus does not contain `#[cfg(...)]` attributes on
its symbols. The contract for `cfg` is therefore exercised by the
shape rule alone: if a symbol carried, in the benchmark,

```rust
#[cfg(target_os = "linux")]
#[cfg(feature = "fast-hash")]
pub fn linux_fast_hash() -> u64 { ... }
```

the row would store:

```
cfg = ["target_os = \"linux\"", "feature = \"fast-hash\""]
```

Order: source order. Whitespace inside each predicate: collapsed
to single spaces. `cfg_attr` attributes elsewhere on the same
symbol: not included. This is the "union of all, in source order,
raw text" decision.

(Flagged as a non-corpus example: included to nail down the `cfg`
contract; no row of this shape will be emitted from the current
benchmark.)
