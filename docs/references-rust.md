# References — Rust

Contract for the `references` relation rows emitted by the Rust
extractor. Conforms to [ADR-0003](adr/0003-level-3-types-and-references.md)
(full lexical scope walking with shadowing) and the schema in
[virgil-datalog-schema.md](virgil-datalog-schema.md). Symbol IDs in all
worked examples follow [ADR-0002](adr/0002-symbol-id-scheme.md):
`path|start_line|start_col|name|kind`. `site_start_byte` values are the
tree-sitter `Range.start_byte` of the referring identifier node
(no trivia adjustment).

## Lexical scope rules

Rust has the following nested scopes, walked from innermost outward:

1. **Block scope** (`{ ... }`). Every `block` AST node introduces a
   new scope. `let` bindings are visible from the `let` site to the
   end of the enclosing block.
2. **Pattern scope** for `if let` / `while let` / `for` / `match`
   arms. Pattern bindings are visible only inside the *body* of that
   construct (the `match arm` block, the `if let` `then`-block, the
   `for` body, the `while let` body — **not** the `else` arm).
3. **Function scope.** Parameters are visible throughout the
   function body but not in any other item. Generic parameters
   declared on the function are also in scope here for `type_use`
   references.
4. **`impl` / `trait` scope.** Generic parameters declared on
   `impl<T> Foo<T>` or `trait Bar<T>` are visible inside method
   bodies and associated-type declarations of that impl/trait.
5. **`struct` / `enum` / `union` / `type_alias` scope.** Generic
   parameters declared on the item are visible in field/variant/body
   type expressions.
6. **Module scope** (`mod foo { ... }`). Items declared at module
   level are visible to siblings within the same module. Nested
   modules introduce their own scope; a child module sees parent
   items only through `super::` or `crate::` paths.
7. **File-module scope.** Top-level items in a file are at the file's
   module level. `use` statements at module top are in scope for the
   whole file.
8. **Crate root.** `crate::` paths resolve from `src/lib.rs` or
   `src/main.rs`.
9. **Prelude.** `Option`, `Result`, `String`, `Vec`, `Box`, `Iterator`,
   `Copy`, `Clone`, `Debug`, `Default`, `Drop`, `Eq`, `Hash`, `Ord`,
   `PartialEq`, `PartialOrd`, `Send`, `Sync`, the primitive type
   names (`i32`, `bool`, `str`, …), and the standard macros (`println!`,
   `format!`, `vec!`, `panic!`, `assert!`, `assert_eq!`, etc.) are
   considered always-in-scope and resolve to their `std`/`core`
   canonical names.

### Lookup walk

Resolution of an identifier occurrence:

1. Start at the innermost block; check `let` bindings, then enclosing
   pattern bindings.
2. Walk outward through blocks until the enclosing function.
3. Check function parameters and function-level generic parameters.
4. Check enclosing `impl`/`trait` generic parameters.
5. Check items declared in the enclosing module (file-level), including
   `use` aliases.
6. Check `crate::` / `super::` / `self::` qualified candidates by
   walking the workspace file tree (using `resolve_import` in
   `src/languages/rust_lang/queries.rs`).
7. Check the prelude.
8. Otherwise, unresolved.

### Shadowing

**Later binding wins.** Rust permits a binding to shadow an outer
binding of the same name. A new `let x = ...` introduces a *new*
symbol; later references in the same block resolve to the most
recent binding visible at the reference site.

The extractor models each shadowing `let` as a *distinct* `symbol`
row (kind `"variable"`) with its own `symbol_id` derived from its
own `start_line` / `start_col` / `name`. References resolve to the
nearest enclosing binding by source-byte order: a reference at
byte `B` resolves to the most-recent `let` whose `start_byte ≤ B`
in the same lexical scope.

Type ascriptions and `mut` modifiers on a shadowing binding do not
affect the rule — the new binding is still its own symbol.

### Module-qualified names

`a::b::c`:

1. The leading segment (`a`) resolves as a single identifier per the
   normal walk above. If it matches a `use a::...;` or a sibling
   module, follow that.
2. Each subsequent segment indexes into the resolved target
   (sub-module, then named item).
3. `crate::`, `self::`, `super::` short-circuit segment 1 to a known
   anchor in the workspace.

The leading segment of a module path emits an `import_use` row when
the path appears inside a `use_declaration`, and a `read` or
`type_use` row when it appears elsewhere (depending on position).
Subsequent path segments do **not** emit their own `references`
rows; only the rightmost segment's referent is recorded (rationale:
mid-path segments are not addressable as standalone symbols in our
schema — they are module-name fragments — so emitting rows for
them would create dangling `referent_id = null` entries that
queries would need to filter out).

## `ref_kind` decision tree

### `read`

Every AST pattern where an identifier is *evaluated*. Specifically:

- Bare identifier in expression position: `foo`, `result`.
- The receiver of a method call: `self.entries.get(...)` — `self`
  and `entries` are both `read` (one for the receiver, one for the
  field access through the dot).
- The function position of a call expression: `f(x)` — `f` is `read`.
- The right-hand side of `let`, the condition of `if`, the operand
  of unary/binary expressions, the elements of a tuple/array
  literal, the arguments of a call/macro.
- Path expressions in value position: `crate::utils::hash::hash_bytes`
  (the rightmost segment is the `read` target).
- Indexing: `xs[i]` — both `xs` and `i` are `read`.
- The discriminant of a `match`.
- The iterator expression of a `for` loop: `for x in xs.iter()` —
  `xs` is `read`; `x` is a pattern *binding* and does not emit a
  reference row (it emits a `symbol` row instead).
- The matched expression in `if let` / `while let`.

**Exceptions** (no `read` row emitted):

- The identifier *being declared* by `let`, `fn`, `struct`, `enum`,
  parameter list, pattern, `for`-loop binding. These are *symbol
  definitions*, recorded in the `symbol` relation, not references.
- Macro argument tokens that are not parsed identifiers (e.g. the
  format-string body in `format!("{}", x)` — only `x` is a `read`;
  the literal `"{}"` is not).
- Attribute path identifiers (`#[derive(Clone)]` — `Clone` does
  *not* emit a `read`; it is captured in `rust_attrs.derives`
  instead, per `attrs-rust.md`).
- Lifetime identifiers (`'a`) — never emit a reference row.
- Identifiers inside macro definition bodies (`macro_rules! { ... }`)
  are skipped — they are not real references until expansion.
- The `_` placeholder pattern.

### `write`

Every AST pattern where an identifier is *assigned to* or
*mutated through a known-mutating construct*:

- Assignment: `x = y` — `x` is `write`, `y` is `read`.
- Compound assignment: `x += 1`, `x -= 1`, `x *= ...`, `x /= ...`,
  `x %= ...`, `x |= ...`, `x &= ...`, `x ^= ...`, `x <<= ...`,
  `x >>= ...`, plus `x++`-style updates if Rust supported them —
  one `write` row per site, no separate `read`. Faithful
  read+write semantics is Level 4; Level 3 records only the
  dominant kind (per `docs/contract-review.md`).
- The left-hand side of an `=` inside a `let` is *not* a write —
  it is a *symbol definition*.
- `&mut x` — `x` is `read`-then-mutably-borrowed; this is
  conservatively recorded as **`write`** (rationale: the borrow
  is the entry point for mutation through the reference and
  downstream taint/safety queries want to see it as a write
  site).
- Method calls: the receiver identifier (`obj` in `obj.push(x)`)
  is recorded as `read`. We do not infer `&mut self`-style mutation
  from method name. Updated per `docs/contract-review.md`: the
  stdlib mutator-name whitelist is dropped. Only structural writes
  (assignment LHS, compound-op LHS, `&mut x` borrows) produce
  `write` rows.
- Field assignment: `obj.field = v` — `obj` is `read`. `field`
  produces a `write` row **only** when the field has a known
  `symbol_id` in the store (per the standardized field-tracking
  policy in `docs/contract-review.md`). Local-struct fields and
  other fields not extracted as symbols produce no row.
- Index assignment: `xs[i] = v` — `xs` is `write` (the container
  is mutated), `i` is `read`, `v` is `read`.

### `type_use`

Every identifier appearing in a *type position* (the same positions
that produce `type` rows in `types-rust.md`):

- Parameter type annotations.
- Return type after `->`.
- Field types in `struct` / `union` / `enum-variant`.
- Type-alias right-hand side: `type Foo = Bar<u8>;` — `Bar` is
  `type_use`, `u8` is `type_use`.
- Generic argument lists: `Vec<T>` — `Vec` is `type_use`, `T` is
  `type_use`.
- Trait bounds: `T: Display + Send` — `Display` and `Send` are
  `type_use`.
- Trait object types: `dyn Trait`, `impl Trait`.
- `as` casts: `x as u32` — `u32` is `type_use`.
- Turbofish: `parse::<f64>()` — `f64` is `type_use`.
- `impl` headers: `impl Foo for Bar` — both `Foo` and `Bar` are
  `type_use`.
- `where` clauses.

The same `references` row references the type's *head symbol* —
the `type_id` lives on the `type` row, but `references.referent_id`
points at the **symbol** (`struct`/`enum`/`trait`/`type_alias`)
that defines the head, not at the type row.

### `import_use`

Every identifier *inside a `use_declaration`*:

- `use std::collections::HashMap;` — the leading `std` and the
  rightmost `HashMap` both emit `import_use` rows. Mid-path
  segments (`collections`) do **not** emit rows (same rule as
  "module-qualified names" above).
- `use std::collections::{HashMap, HashSet};` — both `HashMap`
  and `HashSet` get rows.
- `use std::io::*;` — the glob `*` emits no row; the leading
  `std` and the closest-named segment (`io`) emit rows.
- `use foo::bar as baz;` — `bar` is `import_use`; `baz` is a
  symbol definition (alias binding), not a reference.

Tie-in with the existing `raw_import` / `imports` relations: every
`use_declaration` already produces an `imports` row. The
`import_use` references are *additional* rows that pin the
identifier occurrences to their site bytes so cross-reference
queries (e.g. "find every file that imports `HashMap`") can join
through `references` instead of re-parsing.

## `referent_id` resolution

Per the schema, `references` is keyed by
`(referrer_id, site_file, site_start_byte, match_index)` and
`referent_id` is nullable in the value position. `match_index = 0`
for the primary/only candidate; overload candidates at the same
site use `match_index = 1, 2, ...`. Rust does not have C++-style
overloading, so every Rust row uses `match_index = 0` in practice.
Unresolvable referents emit a single row with `referent_id = null`
(not a sentinel string, not skipped).

Algorithm to map an identifier occurrence at byte `B` in file `F`
to a `referent_id`:

1. Build a per-file scope tree at extraction time (a fresh
   in-memory walk of the `file_item`'s children — **not** the
   global `symbols_by_name` index). Each scope holds:
   - symbol definitions introduced inside it (with their
     `start_byte`),
   - the parent scope.
   The scope tree is discarded after the file's references are
   emitted; it is not persisted.
2. To resolve `(name, B)`, find the innermost scope containing
   `B`. Walk outward:
   - For each scope, scan its definitions in *reverse* `start_byte`
     order, taking the first whose `start_byte ≤ B` and whose name
     matches. This implements the "later binding wins" shadowing
     rule.
   - If no match, ascend to the parent scope.
3. On reaching the file-module scope, also consult the file's
   `use` aliases (the existing `imports` rows for this file). A
   match against `local_name` resolves to the **imported** symbol
   id if the import is internal (`is_external = false`), or to a
   synthetic external id `<module_specifier>` if external.
4. On miss at file scope, consult the prelude table (a static map
   of `Option` → `core::option::Option`, etc.). A match resolves
   to a synthetic `referent_id` of the form `prelude:<canonical_name>`.
5. Otherwise, emit the row with `referent_id = null` (per the
   updated schema, `referent_id` is nullable in value position; we
   *keep the row* rather than dropping it — rationale: query
   authors filter on `referent_id IS NOT NULL` when they want
   resolved-only, and keep the unresolved rows for surface-area
   audits like "every external dep mentioned by name").

When multiple candidates exist at the same scope level (e.g. two
`let x =` shadowing each other above the same site), only the
**most-recent** one is recorded. The extractor does *not* emit
multiple `references` rows for a single identifier occurrence.

The resolver does **not** use the global `symbols_by_name` index
that `src/graph/builder.rs` exposes. Per-file scope trees are
required to honor block-local shadowing correctly; the global
index ignores scope and would resolve every `x` to whichever `x`
was indexed first. ADR-0003 specifies per-language scope owners
and this choice is consistent with that.

## Worked examples

Each example cites a real path inside
`/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/rust/systems-cli/`.
Bytes are computed against the corpus file as it stands.

### Example 1 — `read` and `write` inside a function body

**Source.** `src/utils/hash.rs:29-41` (function `hash_bytes`)

```rust
pub fn hash_bytes(data: &[u8]) -> ContentHash {
    // FNV-1a constants for 64-bit
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    ContentHash { value: hash }
}
```

Defined symbols inside the function body (per ADR-0002):

- `src/utils/hash.rs|31|10|FNV_OFFSET|constant`
- `src/utils/hash.rs|32|10|FNV_PRIME|constant`
- `src/utils/hash.rs|34|12|hash|variable` (the `let mut hash`)
- `src/utils/hash.rs|35|8|byte|variable` (the for-loop `&byte` pattern binding)

**`references` rows** (caller `referrer_id` is
`src/utils/hash.rs|29|7|hash_bytes|function`; `site_file` is
`src/utils/hash.rs` for every row):

| referent_id                                          | ref_kind | site_start_byte (token) |
|------------------------------------------------------|----------|--------------------------|
| `src/utils/hash.rs\|31\|10\|FNV_OFFSET\|constant`     | `read`   | byte of `FNV_OFFSET` on line 34 |
| `src/utils/hash.rs\|34\|12\|hash\|variable`           | `write`  | byte of `hash` on line 36 (compound `^=`) |
| `src/utils/hash.rs\|35\|8\|byte\|variable`            | `read`   | byte of `byte` on line 36 |
| `src/utils/hash.rs\|34\|12\|hash\|variable`           | `write`  | byte of `hash` on line 37 (LHS of `=`) |
| `src/utils/hash.rs\|34\|12\|hash\|variable`           | `read`   | byte of `hash` on line 37 (RHS, the receiver of `.wrapping_mul`) |
| `src/utils/hash.rs\|32\|10\|FNV_PRIME\|constant`      | `read`   | byte of `FNV_PRIME` on line 37 |
| `src/utils/hash.rs\|34\|12\|hash\|variable`           | `read`   | byte of `hash` on line 40 (inside `ContentHash { value: hash }`) |

Resolver decisions:

- `data` (the parameter) is not in this example because we focus
  on body references; a `data` read on line 35 (`for &byte in
  data`) is `read`, referent `src/utils/hash.rs|29|22|data|parameter`.
- Line 36 `hash ^= byte as u64;` — `hash` is the LHS of a
  compound op → single `write` row (no separate `read`).
- Line 37 `hash = hash.wrapping_mul(FNV_PRIME);` — LHS `hash` is
  `write`; RHS `hash` is a separate token and is `read`. Two
  rows.
- `wrapping_mul` is a method call on `hash`. `wrapping_mul` is
  not on the known-mutating whitelist (it returns a new value),
  so the receiver `hash` is `read`, not `write`.

### Example 2 — Shadowing (the `let entry = entry` pattern)

**Source.** `src/utils/fs.rs:86-88`

```rust
    for entry in entries {
        let entry = entry.map_err(|e| format!("Directory entry error: {}", e))?;
        let path = entry.path();
```

Defined symbols:

- `src/utils/fs.rs|86|8|entry|variable` — the for-loop pattern binding.
- `src/utils/fs.rs|87|12|entry|variable` — the shadowing `let entry`
  on line 87.

**`references` rows** inside this snippet (caller `referrer_id` is
the enclosing function `list_files_inner`,
`src/utils/fs.rs|70|3|list_files_inner|function`):

| referent_id                                          | ref_kind | source token                                    |
|------------------------------------------------------|----------|-------------------------------------------------|
| `src/utils/fs.rs\|83\|4\|entries\|variable`           | `read`   | `entries` on line 86 (RHS of `for entry in`)    |
| `src/utils/fs.rs\|86\|8\|entry\|variable`             | `read`   | `entry` on line 87, **RHS of `let entry =`**    |
| `src/utils/fs.rs\|87\|12\|entry\|variable`            | `read`   | `entry` on line 88 (`entry.path()`)             |

Resolver decisions for the shadowing:

- The RHS `entry` on line 87 sits *before* the `let entry` on the
  same line completes its binding. By the source-byte rule, a
  reference at byte `B` resolves to the most-recent binding with
  `start_byte ≤ B`. The `let entry` on line 87 starts at byte
  greater than the RHS `entry` — so the RHS resolves to the
  **outer** binding (the for-loop `entry` at line 86).
- The `entry` on line 88 sits *after* the `let entry` on line 87
  completes. It resolves to the **inner** (shadowing) binding.
- This pattern is exactly the example called out in the
  shadowing section: same name, two distinct `symbol` rows, two
  distinct `referent_id` values across consecutive lines.

### Example 3 — Write to a non-local (mutable static, via `unsafe`)

**Source.** `src/plugins/registry.rs:42-48`

```rust
pub fn register_plugin(info: PluginInfo) -> Result<(), String> {
    unsafe {
        if let Some(ref mut plugins) = PLUGINS {
            // Check for duplicate names
            if plugins.iter().any(|p| p.name == info.name) {
                return Err(format!("Plugin '{}' is already registered", info.name));
            }
            plugins.push(info);
            Ok(())
```

Defined symbols of interest:

- `src/plugins/registry.rs|26|0|PLUGINS|variable` — the `static mut`.
- `src/plugins/registry.rs|41|7|register_plugin|function`.
- `src/plugins/registry.rs|41|23|info|parameter`.
- `src/plugins/registry.rs|44|24|plugins|variable` — the `ref mut
  plugins` pattern binding from `if let`.
- `src/plugins/registry.rs|44|17|p|variable` — the closure parameter
  `|p|`.

**`references` rows** (`referrer_id =
src/plugins/registry.rs|41|7|register_plugin|function`):

| referent_id                                                | ref_kind   | source token                                            |
|------------------------------------------------------------|------------|---------------------------------------------------------|
| `src/plugins/registry.rs\|26\|0\|PLUGINS\|variable`         | `write`    | `PLUGINS` on line 44 — bound `ref mut` borrow of a non-local |
| `src/plugins/registry.rs\|44\|24\|plugins\|variable`        | `read`     | `plugins` on line 46 (`plugins.iter()`)                 |
| `src/plugins/registry.rs\|44\|17\|p\|variable`              | `read`     | `p` (twice) on line 46                                   |
| `src/plugins/registry.rs\|41\|23\|info\|parameter`          | `read`     | `info` on line 46 (`p.name == info.name`)                |
| `src/plugins/registry.rs\|44\|24\|plugins\|variable`        | `read`     | `plugins` on line 49 (`plugins.push(info)` — receiver is `read`; the mutator whitelist was dropped per `docs/contract-review.md`) |
| `src/plugins/registry.rs\|41\|23\|info\|parameter`          | `read`     | `info` on line 49 (argument to `push`)                   |

Resolver decisions:

- `PLUGINS` is a file-module-level `static mut`. The reference on
  line 44 (`ref mut plugins = PLUGINS`) is an `&mut`-equivalent
  borrow of a non-local. Per the `write` decision tree (`&mut x`
  → `write`), this records `PLUGINS` as `write`. This is the
  required "write to a non-local" example.
- The `plugins` binding is a *new local* (a mutable reference
  alias). The `push` call on line 49 is a method call, so
  `plugins` is `read` there — Level 3 does not infer
  `&mut self` mutation from method names.
- `name` on `p.name` is a *field access*. Per the standardized
  field-tracking policy (`docs/contract-review.md`), a `read` or
  `write` row for the field token is emitted **only** when the
  field has a known `symbol_id`. The `field` symbol on
  `PluginInfo.name` qualifies, so a `read` row is emitted with
  the field symbol as referent. For brevity the field rows are
  omitted from the table.

### Example 4 — `type_use` references in a function signature

**Source.** `src/utils/hash.rs:65`

```rust
pub fn verify_hash(path: &str, expected: &ContentHash) -> Result<bool, String> {
```

Defined symbols:

- `src/utils/hash.rs|65|7|verify_hash|function`.
- `src/utils/hash.rs|10|11|ContentHash|struct` (same-file definition).

**`references` rows** (`referrer_id =
src/utils/hash.rs|65|7|verify_hash|function`):

| referent_id                                              | ref_kind   | source token                  |
|----------------------------------------------------------|------------|-------------------------------|
| `prelude:str`                                            | `type_use` | `str` in `&str` (param 0)     |
| `src/utils/hash.rs\|10\|11\|ContentHash\|struct`          | `type_use` | `ContentHash` (param 1)       |
| `prelude:core::result::Result`                           | `type_use` | `Result` in return type       |
| `prelude:bool`                                           | `type_use` | `bool` in `Result<bool, _>`   |
| `prelude:alloc::string::String`                          | `type_use` | `String` in `Result<_, String>` |

Resolver decisions:

- `str` and `bool` are primitive type names → prelude resolution.
- `Result` and `String` come from the prelude.
- `ContentHash` is defined in the same file at line 10 → resolves
  to its definition `symbol_id`.
- Lifetimes (none here) would not produce rows.
- The reference modifiers `&str` and `&ContentHash` themselves do
  not get their own `references` rows; they affect the wrapper
  `type` rows (per `types-rust.md`) but reference resolution
  follows the *inner head symbol*.

### Example 5 — `import_use` rows for a `use_declaration`

**Source.** `src/core/cache.rs:6`

```rust
use std::collections::HashMap;
```

and `src/core/cache.rs:7`

```rust
use std::time::{Duration, SystemTime};
```

**`references` rows.** `referrer_id` is the file-module pseudo-symbol
for `cache.rs`: `src/core/cache.rs|0|0|cache|module` (the synthetic
file-level module symbol per the existing extractor's module
modeling).

For line 6:

| referent_id          | ref_kind     | site_start_byte (token)        |
|----------------------|--------------|--------------------------------|
| `prelude:std`        | `import_use` | byte of `std` on line 6        |
| `prelude:std::collections::HashMap` | `import_use` | byte of `HashMap` on line 6 |

For line 7 (the grouped import expands to two `import_use` rows for
the inner names plus the leading path segment):

| referent_id                          | ref_kind     | site_start_byte (token)         |
|--------------------------------------|--------------|---------------------------------|
| `prelude:std`                        | `import_use` | byte of `std` on line 7         |
| `prelude:std::time::Duration`        | `import_use` | byte of `Duration` on line 7    |
| `prelude:std::time::SystemTime`      | `import_use` | byte of `SystemTime` on line 7  |

Resolver decisions:

- The leading `std` segment matches no in-scope symbol; the
  prelude entry for `std` resolves it.
- Mid-path segment `collections` does **not** emit a row.
- Inside `{ ... }`, each comma-separated item is its own
  `import_use` row.
- The `module` synthetic file-symbol is the *referrer* (rationale:
  imports are file-level, not nested in any function — they need
  a stable `referrer_id` that isn't a real function).

### Example 6 — `read` on a method call and unresolved external

**Source.** `src/core/cache.rs:42`

```rust
        let now = SystemTime::now();
```

Defined symbols visible at this site (line 42 is inside
`Cache::get`, line 41 declares it):

- `src/core/cache.rs|41|11|get|method`.
- The file's `use std::time::{Duration, SystemTime};` import gives
  a local binding `SystemTime` mapping to `std::time::SystemTime`.

**`references` rows** (`referrer_id =
src/core/cache.rs|41|11|get|method`):

| referent_id                          | ref_kind   | source token                            |
|--------------------------------------|------------|-----------------------------------------|
| `prelude:std::time::SystemTime`      | `read`     | `SystemTime` on line 42                 |
| `null`                               | `read`     | `now` on line 42 (the *method* name on `SystemTime::now`) |

Resolver decisions:

- The path expression `SystemTime::now` resolves the head
  `SystemTime` via the file's import; it is a `read` of the type
  (used as a path-value head, not as a type annotation, so
  `read` not `type_use`).
- The rightmost segment `now` is an associated function we cannot
  resolve without indexing `std`. Per the algorithm step 5, we
  *emit the row* with `referent_id = null` so downstream
  surface-area queries can still aggregate.
- The new local `now` on the LHS of `let` is a symbol definition
  (not a reference) — it produces a `symbol` row at
  `src/core/cache.rs|42|12|now|variable`.
