# References — Rust

Per [ADR-0005](adr/0005-datalog-resolution.md), the Rust extractor is a
**fact emitter**. It produces `scope`, `binding`, and `occurrence` rows
that the Cozoscript resolver in [`docs/resolution.md`](resolution.md)
consumes to materialise the `references` relation. This contract
specifies *what* the extractor emits; *how* a name occurrence is mapped
to a `referent_id` is the resolver's job and is **not** described here.

Symbol IDs follow [ADR-0002](adr/0002-symbol-id-scheme.md):
`path|start_line|start_col|name|kind`. `start_byte`, `end_byte`, and
`start_col` values are the tree-sitter `Range` of the relevant node
(no trivia adjustment). `start_line` is 1-indexed, `start_col` is
0-indexed (matches the existing `SymbolInfo` convention).

---

## Scope tree

A `scope` row is emitted for every lexical region that can hold its own
name bindings. The `kind` column uses the schema enum
(`"file" | "module" | "namespace" | "class" | "function" | "block"`);
Rust has no `namespace` and only uses `class` for the impl-block /
trait-block construct described below.

| AST node                  | `scope.kind` | Notes                                                                |
|---------------------------|--------------|----------------------------------------------------------------------|
| `source_file`             | `file`       | One per file. `parent_id = null`.                                    |
| `mod_item` body           | `module`     | Inline `mod foo { ... }`. Parent is the enclosing file/module scope. |
| `function_item` body      | `function`   | The body `block` is folded into this scope, **not** a nested block.  |
| `closure_expression` body | `function`   | Each closure introduces a function-kind scope.                       |
| `impl_item` body          | `class`      | Holds generic parameters declared on `impl<T>` and associated items. |
| `trait_item` body         | `class`      | Holds trait generics and associated items.                           |
| `struct_item` body        | `class`      | Visible to field type expressions; holds struct generics.            |
| `enum_item` body          | `class`      | Holds enum generics; visible to variant body types.                  |
| `union_item` body         | `class`      | Holds union generics; visible to field types.                        |
| `block` (non-function)    | `block`      | Every standalone `{ ... }` not already counted above.                |
| `if_let_expression` then  | `block`      | The pattern's bindings are in scope only inside `then` (not `else`). |
| `while_let_expression`    | `block`      | The pattern's bindings are in scope only inside the loop body.       |
| `for_expression`          | `block`      | The pattern binding is in scope inside the loop body.                |
| `match_arm`               | `block`      | The arm's pattern bindings are in scope inside the arm's expression. |

`parent_id` is the innermost enclosing emitted `scope.id`. Scope `id`
format is `file_path|start_byte|kind` (per the schema).

### Rust-specific edge cases

- **Function body folding.** A `function_item`'s body `block` does *not*
  get a separate `block` scope on top of the `function` scope. The
  function scope spans the whole `function_item` body.
- **`if let` / `while let` else arms.** Pattern bindings are emitted
  with `scope_id` set to the `then`/loop-body block scope, **not** the
  outer scope. The `else` arm uses the outer scope only.
- **`match` arms.** Each `match_arm` is its own `block` scope. Bindings
  introduced by the arm's pattern live in that arm's scope.
- **Single-statement bodies.** Rust does not have implicit-block
  single-statement bodies the way C does; every body is already a
  `block`.
- **`macro_rules!` bodies.** Identifiers inside a `macro_definition` are
  not parsed identifiers in any meaningful scope — no scope or binding
  rows are emitted for the macro body.

---

## Bindings

A `binding` row introduces a name into a scope. The composite key is
`(scope_id, name, start_byte)`; the `start_byte` disambiguator allows
Rust's `let` shadowing (multiple bindings of the same name in the same
scope, ordered by source position).

### `definition`

Emitted for every top-level/nested item that introduces a name in its
enclosing scope. The `symbol_id` is identical to the `symbol` row's id
for the same definition site.

| AST node            | `symbol.kind` |
|---------------------|---------------|
| `function_item`     | `function` (or `method` inside `impl_item` / `trait_item`) |
| `struct_item`       | `struct`      |
| `enum_item`         | `enum`        |
| `union_item`        | `union`       |
| `trait_item`        | `trait`       |
| `type_item`         | `type_alias`  |
| `const_item`        | `constant`    |
| `static_item`       | `variable`    |
| `mod_item`          | `module`      |
| `macro_definition`  | `macro`       |

Also emitted as `definition` (not in the existing `symbol` extractor's
output today, but required for resolution):

- `let_declaration` — every `let` pattern. Each identifier in the
  pattern produces one `definition` binding with `symbol.kind =
  "variable"`. Shadowing produces multiple rows with the same `name`
  and `scope_id` but distinct `start_byte`.
- `for_expression` pattern — loop variable; `symbol.kind = "variable"`.
- `if_let_expression` / `while_let_expression` / `match_arm` pattern
  variables — each pattern identifier; `symbol.kind = "variable"`.

The pattern bindings above live in the **inner body scope** (the
then-block, loop body, or match arm), not in the construct's outer
scope.

### `parameter`

Emitted for every parameter identifier in a function, method, or
closure parameter list. `symbol.kind = "parameter"`. Parameters bind
into the enclosing `function` scope (function body or closure body).

- `function_item` parameters → bound in the function-body scope.
- `closure_expression` parameters → bound in the closure-body scope.
- `self` parameter (`&self`, `&mut self`, `self`, `&mut Box<Self>`,
  etc.) is emitted as a `parameter` binding with `name = "self"`.
- Tuple/struct destructuring in a parameter pattern produces one
  `parameter` binding per identifier inside the pattern.

### `import`

Emitted for plain (non-aliased, non-glob) `use_declaration` paths. The
`name` is the rightmost segment of the path; the `scope_id` is the
file-module scope of the importing file.

- `use std::collections::HashMap;` → `binding{name: "HashMap",
  binding_kind: "import"}`.
- `use crate::utils::hash::hash_bytes;` → `binding{name: "hash_bytes",
  binding_kind: "import"}`.

`symbol_id` is:
- The `symbol_id` of the target definition when the import is internal
  and resolves to a file the extractor indexed (uses the existing
  `resolve_import` machinery in `src/languages/rust_lang/queries.rs`).
- `null` when the target is external (unindexed crate / stdlib /
  unresolved path).

Grouped imports (`use foo::{bar, baz}`) expand to one `binding` row per
inner item, mirroring the existing `ImportInfo` expansion.

### `import_alias`

Emitted for aliased imports. The `name` is the alias.

- `use foo::bar as baz;` → `binding{name: "baz", binding_kind:
  "import_alias", symbol_id: <id of foo::bar's definition>}`.
- `use foo as bar;` → `binding{name: "bar", binding_kind:
  "import_alias"}`.
- `use foo::{bar as qux};` → `binding{name: "qux", binding_kind:
  "import_alias"}`.

`symbol_id` follows the original definition. Transitive re-exports
(`pub use foo::bar` chains) are resolved by `resolve_import` during
import resolution; the resolver does not chase them again.

### `wildcard_import`

Emitted once per `use foo::*;` declaration. `name = "*"`. `symbol_id =
null` — the resolver expands the wildcard at materialise time using
the `imports` graph + the target file's exported symbols (`resolution.md`,
the `wildcard_target` rule).

- `use super::*;` → one `wildcard_import` row in the file-module scope.
- `use std::io::*;` → one row.

The leading-path segments and the `*` token of a wildcard import do
not separately become `binding` rows — only the single
`wildcard_import` row per declaration.

---

## Occurrence emission

An `occurrence` row is emitted for every identifier token of interest in
the parsed file. `enclosing_symbol_id` is the innermost `symbol` row
containing the occurrence (`null` for file/module-level expressions,
e.g. tokens inside a `use_declaration` at file scope).
`enclosing_scope_id` is the innermost `scope` row containing the
occurrence's byte range. `occurrence.id` is
`file_path|start_byte|name|occurrence_kind` (per the schema).

### `call`

Every call-position identifier:

- Free-function call: `foo(x)` → one `call` occurrence on `foo`.
- Method call: `obj.method()` → one `call` occurrence on `method`. The
  receiver `obj` is a separate `read` occurrence.
- Associated function call: `Type::assoc()` → one `call` occurrence on
  the rightmost segment `assoc`. The head segment (`Type`) is a `read`
  occurrence (see "Module-qualified paths" below).
- Macro invocation: `println!(...)`, `vec![...]`, `assert_eq!(...)` →
  one `call` occurrence on the macro name.
- `?`-suffixed calls (`f(x)?`) emit a `call` on `f`; the `?` operator
  itself produces no occurrence.

The arguments inside a call are recursively walked and emit their own
occurrences according to their own positions.

### `read`

Every value-position identifier. Specifically:

- Bare identifier in expression position: `foo`, `result`, `count`.
- The receiver of a method call: `obj.method()` → `obj` is `read`.
- The function position of a call expression: `f(x)` is the `call`
  rule above; non-call value-position uses (`let g = f;`) are `read`.
- Path expressions in value position: `crate::utils::hash::hash_bytes`
  in `let h = crate::utils::hash::hash_bytes;` — see "Module-qualified
  paths" for which segments emit which rows.
- RHS of `let`, condition of `if`, operands of unary/binary
  expressions, elements of a tuple/array literal, call/macro arguments.
- Indexing: `xs[i]` — `xs` and `i` are both `read`. (The container
  becomes `write` only when on an assignment LHS; see `write` below.)
- The discriminant of a `match`: `match foo { ... }` → `foo` is `read`.
- The iterator expression of a `for`: `for x in xs.iter()` → `xs` is
  `read`. (The `x` pattern is a `definition` binding, not an
  occurrence.)
- The matched expression in `if let pattern = expr` / `while let
  pattern = expr` → `expr` is `read`.
- `self` and field accesses through `self`: `self.field` → `self` is a
  `read` occurrence with `name = "self"`. The trailing `.field` is a
  field access (see below).
- Field access: `obj.field` → `obj` is `read`. The trailing `.field`
  identifier emits a `read` occurrence with `name = "field"` only when
  the field is a tree-sitter `field_identifier` node (i.e. struct
  field access, not tuple indexing). The resolver will produce
  `referent_id = null` for any field the schema doesn't model as a
  symbol; the extractor emits the occurrence row regardless.

**Non-emitting cases** (no `occurrence` row):

- Identifiers in **attribute paths**: `#[derive(Clone)]`, `#[cfg(test)]`,
  `#[serde(rename = "x")]` — the `Clone`, `cfg`, `serde`, `rename`,
  `test` tokens emit no occurrence. (Attribute content is captured in
  `rust_attrs` per `attrs-rust.md`.)
- Identifiers **inside macro definition bodies** (`macro_rules! foo
  { ... }`): the tokens between `{ ... }` are not parsed identifiers.
- **Lifetime identifiers** (`'a`, `'static`): no occurrence row, ever.
- **Identifiers consumed by macros as tokens** (e.g. inside `format!`'s
  format string `"{:?}"` — the `?` and `:` are not identifiers; the
  format-string body is a string literal so nothing is emitted from
  inside it). Identifiers passed as macro arguments *outside* the
  format string (`format!("{}", x)` — the `x`) are normal expressions
  and emit `read` occurrences.
- The `_` placeholder pattern: no occurrence.
- The identifier *being declared* by a `let` / `fn` / `struct` / etc. —
  those produce `binding` rows (and `symbol` rows), not occurrences.

### `write`

Per ADR-0003, a `write` occurrence captures structural writes only.

- Assignment LHS: `x = y` → `x` is `write`, `y` is `read`.
- Compound assignment: `x += 1`, `x -= 1`, `x *= 1`, `x /= 1`,
  `x %= 1`, `x |= 1`, `x &= 1`, `x ^= 1`, `x <<= 1`, `x >>= 1` →
  **single `write` occurrence on the LHS, no separate `read`**.
- Index assignment: `xs[i] = v` → `xs` is `write`, `i` and `v` are
  `read`.
- Field assignment: `obj.field = v` → `obj` is `read`; the trailing
  `.field` identifier is `write`.
- The LHS of a `let` is **not** a write — it is a `definition`
  binding.
- `&mut x` (mutable borrow): `x` is `read`, **not** `write`. ADR-0005's
  resolver-driven model treats borrows as reads; only assignment-shaped
  syntax produces a `write`. (This is a change from the previous
  per-language commitment, which conservatively marked `&mut x` as a
  write.)
- Method calls do **not** produce a write on the receiver. Without
  type info we cannot tell `&self` from `&mut self`.

### `type_use`

Every identifier in type position. These should overlap exactly with
the `type` rows emitted by the Rust extractor's `types-rust.md`
contract — every `type` row's head identifier is also a `type_use`
occurrence.

- Parameter type annotations: `fn f(x: Foo) -> Bar` → `Foo` and `Bar`
  are `type_use`.
- Field types in `struct` / `union` / `enum`-variant.
- Type-alias RHS: `type Foo = Bar<u8>;` → `Bar` and `u8` are `type_use`.
- Generic argument lists: `Vec<T>` → `Vec` and `T` are `type_use`.
- Trait bounds: `T: Display + Send` → `Display` and `Send` are
  `type_use`.
- Trait objects: `dyn Trait`, `impl Trait`.
- `as` casts: `x as u32` → `u32` is `type_use`.
- Turbofish: `parse::<f64>()` → `f64` is `type_use`.
- `impl` headers: `impl Foo for Bar` → both `Foo` and `Bar` are
  `type_use`.
- `where` clauses.
- Reference / pointer wrappers (`&Foo`, `&mut Bar`, `*const Baz`,
  `Box<Qux>`) — the inner head identifier is `type_use`; the wrapper
  itself produces no extra occurrence beyond what `types-rust.md`
  records.

### `import_use`

Every identifier inside a `use_declaration`. The rightmost segment is
already represented by a `binding` (kind `import` / `import_alias` /
`wildcard_import`); `import_use` occurrences pin the surface-syntax
identifiers to their bytes so cross-reference queries can locate
"every file that mentions `HashMap`" without re-parsing.

- `use std::collections::HashMap;` → `import_use` occurrences on
  `std`, `collections`, and `HashMap`.
- `use std::collections::{HashMap, HashSet};` → `import_use`
  occurrences on `std`, `collections`, `HashMap`, and `HashSet`.
- `use foo::bar as baz;` → `import_use` occurrences on `foo` and
  `bar`. The alias `baz` is a binding name, not an `import_use`
  occurrence.
- `use std::io::*;` → `import_use` occurrences on `std` and `io`. The
  `*` token is not an identifier and emits nothing.

### Module-qualified paths

For a `scoped_identifier` or `scoped_type_identifier` like `a::b::c` in
non-`use` position:

- The **leading segment** `a` emits a `read` occurrence (or `type_use`
  if the path is in type position).
- **Mid-path segments** (`b`) emit no occurrence — Rust treats module
  segments as path fragments rather than addressable references.
- The **rightmost segment** `c` emits an occurrence whose `kind`
  reflects the path's role:
  - `read` if the path is in value position outside a call: `let g =
    a::b::c;` → `c` is `read`.
  - `call` if the path is the callee of a call: `a::b::c(x)` → `c` is
    `call`.
  - `type_use` if the path is in type position: `fn f() -> a::b::c { }`
    → `c` is `type_use`.

This rule preserves a single addressable occurrence per identifier
chain and avoids the ambiguity of emitting rows for module name
fragments that the schema does not model as symbols.

`enclosing_symbol_id`/`enclosing_scope_id` apply uniformly to every
emitted occurrence — these columns are populated from the
deepest-containing symbol / scope at the occurrence's byte range.

---

## What this contract does NOT cover

- **Resolution algorithm.** Walking scopes outward, chasing
  re-exports, expanding wildcard imports, picking between shadowed
  bindings — all of this is the Cozoscript resolver's job in
  [`docs/resolution.md`](resolution.md). The extractor does not produce
  `references` rows.
- **Prelude resolution.** The extractor does not know which names
  belong to `std`'s prelude. `Option`, `Result`, `Vec`, `i32`, etc.,
  appear as `occurrence` rows the resolver resolves (or leaves null if
  the resolver's prelude table doesn't list them).
- **Per-occurrence `referent_id`.** Worked examples below stop at
  `occurrence` / `scope` / `binding` rows.
- **`calls` relation.** The Phase 1 `calls` relation continues to be
  populated by a separate extractor pass (per ADR-0005's
  "Consequences"); this contract is about facts that feed the
  resolver, not the call graph.

---

## Worked examples

Each example cites a real path inside
`../virgil-skills/benchmarks/rust/systems-cli/`. Symbol IDs follow
ADR-0002. Concrete `start_byte` values are illustrative — the extractor
emits whatever the tree-sitter `Range.start_byte` is for each node.

For brevity:
- `<F>` abbreviates the path prefix `src/utils/hash.rs|` etc. within
  the same example.
- `scope.id` values are written `<F>|<sb>|<kind>` per the schema.
- We list scope/binding/occurrence rows separately. `parent_id` /
  `enclosing_*` fields point at the relevant id strings.

### Example 1 — `hash_bytes` body (shadowing-free, `read` + `write` mix)

**Source.** `benchmarks/rust/systems-cli/src/utils/hash.rs:29-41`

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

**`scope` rows** (file-module scope omitted for brevity; assume `file`-kind scope at byte 0):

| id (`file_path\|start_byte\|kind`) | parent_id     | kind       |
|------------------------------------|---------------|------------|
| `src/utils/hash.rs\|<sb of fn body>\|function` | `<file scope>` | `function` |
| `src/utils/hash.rs\|<sb of for body>\|block`   | `<function scope>` | `block` |

**`binding` rows** (all `binding_kind = "definition"` unless noted, all in the function or for-body scope):

| name         | binding_kind  | symbol_id                                         | scope        |
|--------------|---------------|---------------------------------------------------|--------------|
| `data`       | `parameter`   | `src/utils/hash.rs\|29\|22\|data\|parameter`       | function     |
| `FNV_OFFSET` | `definition`  | `src/utils/hash.rs\|31\|10\|FNV_OFFSET\|constant`  | function     |
| `FNV_PRIME`  | `definition`  | `src/utils/hash.rs\|32\|10\|FNV_PRIME\|constant`   | function     |
| `hash`       | `definition`  | `src/utils/hash.rs\|34\|12\|hash\|variable`        | function     |
| `byte`       | `definition`  | `src/utils/hash.rs\|35\|8\|byte\|variable`         | for-body block |

**`occurrence` rows** (`enclosing_symbol_id` is `hash_bytes`'s symbol id throughout):

| name         | occurrence_kind | source position                                                  |
|--------------|-----------------|------------------------------------------------------------------|
| `FNV_OFFSET` | `read`          | line 34, RHS of `let mut hash = FNV_OFFSET;`                     |
| `data`       | `read`          | line 35, RHS of `for &byte in data`                              |
| `hash`       | `write`         | line 36, LHS of compound `^=` (single occurrence; no `read`)     |
| `byte`       | `read`          | line 36, RHS                                                     |
| `u64`        | `type_use`      | line 36, the `as u64` cast                                       |
| `hash`       | `write`         | line 37, LHS of `=`                                              |
| `hash`       | `read`          | line 37, RHS — receiver of `.wrapping_mul`                       |
| `wrapping_mul` | `call`        | line 37, method call name                                        |
| `FNV_PRIME`  | `read`          | line 37, argument to `wrapping_mul`                              |
| `ContentHash`| `read`          | line 40, head of struct literal `ContentHash { ... }`            |
| `hash`       | `read`          | line 40, inside `ContentHash { value: hash }`                    |

Notes:
- The `u8` inside `&[u8]` (parameter type) and `ContentHash` (return
  type) at line 29 are `type_use` occurrences on the signature; they
  belong to the function's signature scope, not the body. Omitted from
  this table to keep it focused on the body.
- `wrapping_mul` is a method call; `hash` is its receiver. Receiver is
  `read`, callee is `call`. No `write` is inferred from the method
  name.
- The `mut` modifier on `let mut hash` does not produce a `write`
  occurrence on `hash` — `hash` is a fresh `definition` binding.

### Example 2 — `let entry = entry` shadowing

**Source.** `benchmarks/rust/systems-cli/src/utils/fs.rs:86-88`

```rust
    for entry in entries {
        let entry = entry.map_err(|e| format!("Directory entry error: {}", e))?;
        let path = entry.path();
```

**`scope` rows** (innermost relevant):

| id (`file_path\|start_byte\|kind`)                  | kind  |
|-----------------------------------------------------|-------|
| `src/utils/fs.rs\|<sb of for body>\|block`           | `block` |

**`binding` rows** (showing the two `entry` rows that exercise shadowing):

| name    | binding_kind | symbol_id                                    | scope     | start_byte (key) |
|---------|--------------|----------------------------------------------|-----------|------------------|
| `entry` | `definition` | `src/utils/fs.rs\|86\|8\|entry\|variable`     | for-block | `<sb of pat>`    |
| `entry` | `definition` | `src/utils/fs.rs\|87\|12\|entry\|variable`    | for-block | `<sb of let>`    |
| `path`  | `definition` | `src/utils/fs.rs\|88\|12\|path\|variable`     | for-block | `<sb of let>`    |
| `e`     | `parameter`  | `src/utils/fs.rs\|87\|39\|e\|parameter`       | closure   | `<sb of `|e|`>`  |

Both `entry` bindings live in the same for-body block scope. The
ordering key `start_byte` separates them so the resolver can pick
"latest binding before the occurrence" per Rust shadowing semantics.

**`occurrence` rows** (`enclosing_symbol_id` is `list_files_inner`,
`src/utils/fs.rs|70|3|list_files_inner|function`):

| name      | occurrence_kind | source position                                          |
|-----------|-----------------|----------------------------------------------------------|
| `entries` | `read`          | line 86, RHS of `for entry in entries`                   |
| `entry`   | `read`          | line 87, **RHS of `let entry = entry.map_err(...)`** — starts before the new `let entry` binding completes |
| `map_err` | `call`          | line 87, method on the RHS `entry`                       |
| `e`       | `read`          | line 87, inside the closure body `format!("...", e)`     |
| `entry`   | `read`          | line 88, receiver of `entry.path()`                      |
| `path`    | `call`          | line 88, method call name                                |

The extractor emits two distinct `binding` rows and the four `entry`
occurrences. The resolver — *not the extractor* — picks which binding
each occurrence resolves to by comparing `start_byte` values.

### Example 3 — Wildcard import (`use super::*;`) inside a test module

**Source.** `benchmarks/rust/systems-cli/src/cli/args.rs:93-96`

```rust
#[cfg(test)]
mod tests {
    use super::*;
```

**`scope` rows** (showing the test module and file scope):

| id                                              | parent_id | kind     |
|-------------------------------------------------|-----------|----------|
| `src/cli/args.rs\|0\|file`                       | `null`    | `file`   |
| `src/cli/args.rs\|<sb of mod tests>\|module`     | file      | `module` |

**`binding` rows**:

| name      | binding_kind       | symbol_id | scope                                                 | start_byte |
|-----------|--------------------|-----------|-------------------------------------------------------|------------|
| `tests`   | `definition`       | `src/cli/args.rs\|94\|4\|tests\|module` | file scope            | `<sb of mod>` |
| `*`       | `wildcard_import`  | `null`    | `src/cli/args.rs\|<sb of mod tests>\|module`           | `<sb of use>` |

**`occurrence` rows** for the `use super::*;` declaration
(`enclosing_symbol_id` is the `tests` module symbol):

| name    | occurrence_kind | source position                |
|---------|-----------------|--------------------------------|
| `super` | `import_use`    | line 95, leading path segment  |

`super` is the only identifier inside `use super::*;` (the `*` token
is not an identifier). No occurrence for `*`. The resolver expands
the wildcard at materialise time by joining `imports` (which records
`super` → the parent file) with `symbol{exported: true}` rows.

### Example 4 — Plain import + `self.field` access (`Cache::get`)

**Source.** `benchmarks/rust/systems-cli/src/core/cache.rs:1-10` (imports)
and `:41-61` (the `get` method body).

```rust
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
```

```rust
    pub fn get(&mut self, key: &str) -> Option<&CacheEntry> {
        let now = SystemTime::now();

        // Check if entry exists and is still valid
        if let Some(entry) = self.entries.get(key) {
            if let Ok(elapsed) = now.duration_since(entry.created_at) {
                if elapsed < self.ttl {
                    // Update access count via raw pointer to avoid borrow issues
                    // This is safe because we hold a mutable reference
                    if let Some(entry_mut) = self.entries.get_mut(key) {
                        entry_mut.access_count += 1;
                    }
                    return self.entries.get(key);
                }
            }
        }
```

**`binding` rows** for the imports (file-module scope):

| name        | binding_kind | symbol_id           | scope |
|-------------|--------------|---------------------|-------|
| `HashMap`   | `import`     | `null` (external)   | file  |
| `Duration`  | `import`     | `null` (external)   | file  |
| `SystemTime`| `import`     | `null` (external)   | file  |

**`binding` rows** inside `get`'s function scope and its inner blocks
(only the load-bearing ones for the snippet shown):

| name        | binding_kind | symbol_id                                       | scope         |
|-------------|--------------|-------------------------------------------------|---------------|
| `self`      | `parameter`  | `src/core/cache.rs\|41\|15\|self\|parameter`     | get-fn        |
| `key`       | `parameter`  | `src/core/cache.rs\|41\|27\|key\|parameter`      | get-fn        |
| `now`       | `definition` | `src/core/cache.rs\|42\|12\|now\|variable`       | get-fn        |
| `entry`     | `definition` | `src/core/cache.rs\|45\|20\|entry\|variable`     | if-let-block (line 45) |
| `elapsed`   | `definition` | `src/core/cache.rs\|46\|24\|elapsed\|variable`   | if-let-block (line 46) |
| `entry_mut` | `definition` | `src/core/cache.rs\|50\|29\|entry_mut\|variable` | if-let-block (line 50) |

**`occurrence` rows** (`enclosing_symbol_id` is `get`,
`src/core/cache.rs|41|11|get|method`):

| name           | occurrence_kind | source position / notes                                                         |
|----------------|-----------------|---------------------------------------------------------------------------------|
| `SystemTime`   | `read`          | line 42, leading segment of path `SystemTime::now()`                            |
| `now`          | `call`          | line 42, rightmost path segment used as call target                             |
| `self`         | `read`          | line 45, receiver of `self.entries`                                             |
| `entries`      | `read`          | line 45, field name in `self.entries`                                           |
| `get`          | `call`          | line 45, method call on `self.entries`                                          |
| `key`          | `read`          | line 45, argument                                                               |
| `now`          | `read`          | line 46, receiver of `.duration_since`                                          |
| `duration_since` | `call`        | line 46                                                                         |
| `entry`        | `read`          | line 46, receiver of `.created_at`                                              |
| `created_at`   | `read`          | line 46, field name                                                             |
| `elapsed`      | `read`          | line 47, LHS of `<` comparison                                                  |
| `self`         | `read`          | line 47, receiver of `.ttl`                                                     |
| `ttl`          | `read`          | line 47, field name                                                             |
| `self`         | `read`          | line 50, receiver                                                               |
| `entries`      | `read`          | line 50, field name                                                             |
| `get_mut`      | `call`          | line 50                                                                         |
| `key`          | `read`          | line 50, argument                                                               |
| `entry_mut`    | `read`          | line 51, receiver                                                               |
| `access_count` | `write`         | line 51, LHS of compound `+=` on a field access (single `write`, no `read`)     |
| `self`         | `read`          | line 53, receiver                                                               |
| `entries`      | `read`          | line 53                                                                         |
| `get`          | `call`          | line 53                                                                         |
| `key`          | `read`          | line 53                                                                         |

Notes:
- `self` is emitted as a normal `read` occurrence; the resolver looks
  it up against the `self` `parameter` binding in the function scope.
- `SystemTime::now()` — leading segment `SystemTime` is `read` (path
  head); rightmost segment `now` is `call`. Mid-path-style emission
  does not apply here (only two segments).
- `entry_mut.access_count += 1;` — the field name `access_count` is
  the LHS of the compound assignment, so it's a single `write`
  occurrence (no separate `read`). The receiver `entry_mut` is `read`.
- `now.duration_since(...)` — `now` is `read` (receiver); the resolver
  resolves `now` to the function-scoped `let now` binding from line
  42. `duration_since` is a `call`; the resolver will produce
  `referent_id = null` because no in-scope binding matches.

### Example 5 — Method call with non-statically-resolvable receiver

**Source.** `benchmarks/rust/systems-cli/src/plugins/registry.rs:41-49`

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

**`binding` rows** (load-bearing for the snippet):

| name       | binding_kind | symbol_id                                          | scope        |
|------------|--------------|----------------------------------------------------|--------------|
| `info`     | `parameter`  | `src/plugins/registry.rs\|41\|23\|info\|parameter`  | register_plugin |
| `plugins`  | `definition` | `src/plugins/registry.rs\|43\|24\|plugins\|variable`| if-let-block (line 43) |
| `p`        | `parameter`  | `src/plugins/registry.rs\|45\|34\|p\|parameter`     | closure body |

Note that `ref mut plugins` introduces a `definition` binding for
`plugins` — `ref mut` is a pattern modifier, not a separate construct.

**`occurrence` rows** (`enclosing_symbol_id =
src/plugins/registry.rs|41|7|register_plugin|function`):

| name       | occurrence_kind | source position / notes                                                         |
|------------|-----------------|---------------------------------------------------------------------------------|
| `PluginInfo` | `type_use`    | line 41, parameter type                                                         |
| `Result`   | `type_use`      | line 41, return type head                                                       |
| `String`   | `type_use`      | line 41, return type generic argument                                           |
| `Some`     | `call`          | line 43, pattern `Some(ref mut plugins)` — `Some` is a tuple-struct constructor in the pattern. Per the contract, the constructor identifier in a pattern is treated as `read`. (Patterns are not call expressions; see "Edge case" below.) |
| `PLUGINS`  | `read`          | line 43, RHS of `if let Some(...) = PLUGINS`                                    |
| `plugins`  | `read`          | line 45, receiver of `.iter()`                                                  |
| `iter`     | `call`          | line 45, method call                                                            |
| `any`      | `call`          | line 45, method call (chained)                                                  |
| `p`        | `read`          | line 45, closure body, receiver of `.name`                                      |
| `name`     | `read`          | line 45, field name                                                             |
| `info`     | `read`          | line 45, receiver of `.name` on the RHS of `==`                                 |
| `name`     | `read`          | line 45, field name on the RHS                                                  |
| `Err`      | `call`          | line 46, tuple-struct constructor call                                          |
| `format`   | `call`          | line 46, macro invocation (`format!`)                                           |
| `info`     | `read`          | line 46, argument receiver                                                      |
| `name`     | `read`          | line 46, field name                                                             |
| `plugins`  | `read`          | line 48, receiver of `.push(info)`                                              |
| `push`     | `call`          | line 48, method call                                                            |
| `info`     | `read`          | line 48, argument                                                               |
| `Ok`       | `call`          | line 49, tuple-struct constructor call                                          |

Edge-case correction: the `Some(...)` *inside the `if let` pattern* on
line 43 is a pattern, not a call expression. Per the contract, **only
expressions** emit `call` occurrences; in pattern position, the
constructor identifier emits a `read` instead. The example above is
intentional — the extractor must distinguish `expression_call` from
`tuple_struct_pattern`. (The Cozoscript resolver treats both as name
lookups; the kind discriminator matters mainly for taint/audit
queries.)

Non-statically-resolvable callees:
- `plugins.iter()` — `iter` resolves to nothing in this file's
  bindings; the resolver emits `references{referent_id: null,
  ref_kind: "call"}`. The receiver `plugins` resolves to the
  `if-let`-block binding.
- `plugins.push(info)` — same shape. `push` is a `call` occurrence
  with no scoped binding match; resolver yields `null`. The extractor
  does **not** infer mutation on `plugins` from the name `push`, so
  `plugins` is `read`, not `write`.
- `PLUGINS` (line 43) — `read` occurrence. The resolver walks scopes
  out to the file-module scope, finds the `static_item` `PLUGINS`
  definition, and resolves to its symbol id. The extractor does not do
  this lookup itself.
