# Types — Rust

Contract for the `type` relation rows emitted by the Rust extractor.
Conforms to [ADR-0003](adr/0003-level-3-types-and-references.md) (Level 3:
full kind decomposition + canonical resolution) and the schema in
[virgil-datalog-schema.md](virgil-datalog-schema.md). Symbol IDs in all
worked examples follow [ADR-0002](adr/0002-symbol-id-scheme.md):
`path|start_line|start_col|name|kind`.

## Tree-sitter node kinds

Every tree-sitter node that can appear in a *type position* in Rust
(parameter type, return type, field type, type alias body, generic
argument, cast target, type-ascription, `as` cast, `dyn`/`impl` trait
object). Mapped to the seven schema `kind` variants
(`primitive | named | generic | union | intersection | function | tuple | array`).

| tree-sitter node kind     | source-level form                              | schema `kind`        |
|---------------------------|------------------------------------------------|----------------------|
| `primitive_type`          | `i32`, `u64`, `bool`, `char`, `f64`, `usize`, `str`, `()` (unit), `!` (never) | `primitive`         |
| `type_identifier`         | a bare named type like `String`, `Duration`, `Config` | `named`             |
| `scoped_type_identifier`  | path-qualified name like `std::time::Duration`, `toml::Value` | `named`             |
| `generic_type`            | name with type args: `Vec<u8>`, `Option<String>`, `Result<T, E>`, `HashMap<K, V>` | `generic`           |
| `reference_type`          | `&T`, `&mut T`, `&'a T`                        | `reference` → modeled as `generic` with `display_name` prefix `&` / `&mut` (see below) |
| `pointer_type`            | `*const T`, `*mut T`                           | `generic` (prefix `*const` / `*mut`) |
| `tuple_type`              | `(T1, T2, ...)` with arity ≥ 2; the unit `()` is `primitive` instead | `tuple`             |
| `array_type`              | `[T; N]` (fixed-size)                          | `array`             |
| `slice_type`              | `[T]` (unsized slice)                          | `array`             |
| `function_type`           | `fn(A, B) -> R`, `unsafe fn(...) -> R`, `extern "C" fn(...)` | `function`          |
| `dynamic_type`            | `dyn Trait`, `dyn Trait1 + Trait2 + 'a`        | `intersection` when bound list has ≥ 2 traits; otherwise `named` |
| `bounded_type`            | `T + Send + Sync`, generic bound lists in `impl` and `where` clauses | `intersection`      |
| `abstract_type`           | `impl Trait`, `impl Trait1 + Trait2`           | `intersection` when bound list has ≥ 2 traits; otherwise `named` |
| `lifetime` (alone)        | `'a` appearing as a type argument             | not emitted as a `type` row — folded into the parent `display_name` |
| `removed_trait_bound`     | `?Sized` etc.                                  | not emitted; folded into parent |
| `never_type`              | `!`                                            | `primitive` |
| `unit_type`               | `()`                                           | `primitive` |
| `macro_invocation` in type position | `Foo!(...)`                          | `named`, `canonical_name = null` (macro expansion not resolved) |
| `metavariable`            | `$T` inside macro bodies                       | not emitted (macro body, not real type) |

Rust does not have TS-style structural `union` types. Tagged enums
(`enum Foo { ... }`) are *symbols* of kind `enum` — their use in a type
position emits a `named` row, **not** `union`. The `union` schema variant
is therefore unused by the Rust extractor; this is a deliberate choice
(rationale: Rust enums are nominal and resolve through the symbol table,
not via a structural-union encoding).

When a node kind splits across two schema kinds (`dynamic_type` /
`abstract_type` / `bounded_type`) the decision is based on **bound
count** of the parsed node: ≥ 2 trait/lifetime bounds → `intersection`;
exactly 1 → `named` (collapsing `dyn Trait` / `impl Trait` to the
underlying trait name in `canonical_name`).

## `display_name` construction

`display_name` is a normalized textual rendering of the type
expression. Rules:

1. **Whitespace normalization.** All internal runs of ASCII whitespace
   (space, tab, newline) collapse to a single space. Leading/trailing
   whitespace trimmed. `Vec< i32 >` → `Vec<i32>`. `Vec<\n    i32\n>` →
   `Vec<i32>`.
2. **Punctuation has no inner spaces.** `<`, `>`, `,`, `;`, `(`, `)`,
   `[`, `]`, `::`, `&`, `*` carry no internal whitespace; one space
   *after* a `,` or `;` separating list elements (`Result<T, E>`,
   `[u8; 16]`).
3. **Reference modifiers.** `&T` → `&T`; `&mut T` → `&mut T`; `&'a T`
   → `&'a T`. The lifetime is kept verbatim in `display_name`.
4. **Pointer modifiers.** `*const T` → `*const T`; `*mut T` → `*mut T`.
5. **Generic args render with full inner display.** `Vec<HashMap<String, u8>>`
   round-trips byte-for-byte after normalization.
6. **Lifetime-only arguments are preserved.** `Cow<'a, str>` keeps
   `'a` in the rendered output (rationale: stripping lifetimes loses
   information that downstream lifetime audits will want; cost is
   negligible since lifetime names are part of the source).
7. **`dyn` / `impl` keywords preserved.** `dyn Display` → `dyn Display`;
   `impl Iterator<Item = u32>` → `impl Iterator<Item = u32>` (the
   `Item = u32` associated-type binding is preserved verbatim).
8. **Function-type signatures.** `fn(i32, &str) -> String` round-trips
   with `, ` separators and ` -> ` around the return type.
9. **`unsafe` / `extern` qualifiers on function types are preserved.**
   `unsafe extern "C" fn(*const u8) -> i32` round-trips with one space
   between qualifiers.
10. **Array sizes are textual.** `[u8; 16]` keeps `16`; `[u8; N]`
    keeps the identifier `N` as written.
11. **Trait-bound order is preserved.** `T + Send + Sync` is *not*
    sorted (rationale: trait-bound order is meaningful for human
    readers and re-ordering would make `display_name` diverge from
    source intent for no analytical benefit).

`display_name` round-trips the source's *textual intent* under those
rules: `Vec<i32>` and `Vec< i32 >` produce the same `display_name`.
Byte spans and node identity still differ (so two writes still produce
two distinct rows when they happen across different files).

## `canonical_name` resolution

Per ADR-0003, every `type` row carries a `canonical_name` when the
extractor can resolve the head symbol. The head symbol is:

- the `type_identifier` / `scoped_type_identifier` of a `named` row,
- the *constructor* of a `generic` row (e.g. `Vec` in `Vec<u8>`,
  `Option` in `Option<&str>`),
- the *single trait* of a single-bound `dyn`/`impl` row (e.g.
  `Display` in `dyn Display`),
- the *outermost reference target* head of a `reference`/`pointer`
  row (the head of `T` in `&T`).

Scope-walk order for resolving the head symbol:

1. **In-scope generic parameters.** Items declared in the enclosing
   `function_item` / `impl_item` / `trait_item` / `struct_item` /
   `enum_item` / `type_item` generic parameter lists.
   *Outcome:* `canonical_name = null` (rationale: generic parameters
   are not first-class symbols and have no resolvable definition
   site; downstream queries filter on `canonical_name IS NULL` to
   skip them).
2. **Local `use` aliases.** If the head is renamed by a
   `use X as Y;` in the current file or any in-scope module, follow
   the alias to the imported name. The canonical form is the
   **target** of the alias (`std::collections::HashMap`), not the
   local alias `Y` (rationale: cross-file aggregation joins on the
   thing being *imported*, not the local nickname; the
   `import_use` reference row records the alias itself).
3. **Local `use` bindings.** If the head matches a non-aliased
   `use a::b::Name;` in the current file, the canonical form is
   `a::b::Name`.
4. **Same-file definitions.** If the head matches a `struct_item` /
   `enum_item` / `trait_item` / `type_item` / `union_item` in the
   same file, canonical form is `<crate>::<module-path>::<name>`
   where `<module-path>` is derived from the file path under
   `src/` (e.g. `src/core/cache.rs` → `crate::core::cache`,
   `src/utils/mod.rs` → `crate::utils`). For a binary crate the
   prefix is `crate`; library crates use `crate` too (we do not
   resolve the actual crate name from `Cargo.toml` in this phase).
5. **Sibling-module definitions.** If the head matches a symbol
   in another file in the same project (resolved through the
   `imports` relation already emitted), use the same
   `crate::<module-path>::<name>` form.
6. **`crate::` / `self::` / `super::` paths.** Already-qualified
   paths resolve directly against the workspace file tree using the
   same mapping that `resolve_import` in
   `src/languages/rust_lang/queries.rs` already does.
7. **`std::` / `core::` / `alloc::` paths.** Preserved verbatim as
   `canonical_name` (rationale: standard-library types are
   universally unambiguous; we do not need to index `std` source to
   resolve them).
8. **External crate paths.** If the head's first segment matches a
   `use external_crate::...;` import (an `is_external = true` import
   in the existing `imports` relation), `canonical_name` is the
   import path verbatim (e.g. `serde_json::Value`).
9. **Unresolved.** Anything else (parse failure, macro-expanded
   type, primitive-shaped identifier we somehow miss):
   `canonical_name = null`.

### Type aliases

`type Foo = Vec<u8>;` introduces an alias. **`Foo` canonicalizes to
its own definition site, not to `Vec<u8>`.** Rationale: the alias *is*
the name a Rust author writes and reasons about; rewriting every use
of `Foo` to `Vec<u8>` would lose the abstraction boundary downstream
queries are explicitly trying to surface (e.g. "find every place
`DataMap` is used"). Following the alias chain would also force a
multi-pass resolver, which contradicts the "single-pass per file"
constraint of this extractor.

Consequence: a `Vec<u8>` field and a `DataMap` field of the same
underlying shape get *different* `type.id` and different
`canonical_name`. Queries that need the underlying shape can join
through `symbol` rows of `kind = "type_alias"` separately.

### Primitive types

`canonical_name` for primitives is the primitive name itself: `i32`,
`bool`, `()`, `!`, `str`. There is no module-qualification.

### Generic parameters

`fn foo<T>(...)`: the `T` rows inside the function body get
`canonical_name = null`. The `T` *declaration* on `foo` is recorded
in `rust_attrs` (see `attrs-rust.md`), not in `type`.

## Identity

`type.id = blake3(language | file_id | display_name)` per ADR-0003.

Rust-specific normalization applied to `display_name` *before* hashing:

- All rules from "`display_name` construction" above are already
  baked in — no additional pre-hash step.
- `language` is the literal string `"rust"`.
- `file_id` is the workspace-relative file path (Cozo `file.id`,
  which is the path per ADR-0002).

Two textually-identical type expressions in the same file
deduplicate to one row. The same expression in different files
produces two rows (per ADR-0003).

## Worked examples

Each example cites the real path inside
`/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/rust/systems-cli/`
with `src/` already on the workspace root.

### Example 1 — `primitive` (`u64` field)

**Source.** `src/utils/hash.rs:11`

```rust
pub struct ContentHash {
    value: u64,
}
```

**`type` row for `u64`:**

| column          | value                                       |
|-----------------|---------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "u64")` |
| `kind`          | `"primitive"`                               |
| `language`      | `"rust"`                                    |
| `display_name`  | `"u64"`                                     |
| `canonical_name`| `"u64"`                                     |

**Referenced by:** no `parameter` row (it's a struct field, not a
function parameter). The struct-field type wiring lives on a
`field_type` row keyed by the field symbol:

```
field_type {
    symbol_id: "src/utils/hash.rs|11|10|value|field",
    type_id:   blake3("rust" | "src/utils/hash.rs" | "u64"),
}
```

The corresponding `type_use` row in `references` records the
identifier occurrence at the `u64` token (see
`references-rust.md`).

### Example 2 — `named` (struct used as parameter)

**Source.** `src/utils/hash.rs:65`

```rust
pub fn verify_hash(path: &str, expected: &ContentHash) -> Result<bool, String> {
```

**`type` rows emitted for this signature.** Note the `&str` and
`&ContentHash` are `reference` types (Example 4 covers these); the
inner `ContentHash` is the `named` row of interest here:

| column          | value                                                                  |
|-----------------|------------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "ContentHash")`               |
| `kind`          | `"named"`                                                              |
| `language`      | `"rust"`                                                               |
| `display_name`  | `"ContentHash"`                                                        |
| `canonical_name`| `"crate::utils::hash::ContentHash"` (resolved against same-file definition at line 10) |

**Referenced by `parameter`:**

```
parameter {
    function_id:  "src/utils/hash.rs|65|7|verify_hash|function",
    index:        1,
    name:         "expected",
    type_id:      blake3("rust" | "src/utils/hash.rs" | "&ContentHash"),  -- the reference wrapper
    is_optional:  false,
    has_default:  false,
}
```

The parameter's `type_id` points at the **outer reference type**
(`&ContentHash`); the inner `ContentHash` row stands on its own and is
referred to by the `references` row of `ref_kind = type_use` at byte
offset of the `ContentHash` token (covered in `references-rust.md`).

### Example 3 — `generic` (`Result<bool, String>` return type)

**Source.** `src/utils/hash.rs:65` (same line — the `->` clause)

```rust
pub fn verify_hash(path: &str, expected: &ContentHash) -> Result<bool, String> {
```

**`type` rows.** Three rows are emitted (one per nested type node):

Outer `Result<bool, String>`:

| column          | value                                                          |
|-----------------|----------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "Result<bool, String>")` |
| `kind`          | `"generic"`                                                    |
| `language`      | `"rust"`                                                       |
| `display_name`  | `"Result<bool, String>"`                                       |
| `canonical_name`| `"std::result::Result"` (head `Result` resolves via prelude — recorded as `std::result::Result`; generic args are *not* embedded in `canonical_name`) |

Inner `bool`:

| column          | value                                              |
|-----------------|----------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "bool")`  |
| `kind`          | `"primitive"`                                      |
| `display_name`  | `"bool"`                                           |
| `canonical_name`| `"bool"`                                           |

Inner `String`:

| column          | value                                                |
|-----------------|------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "String")`  |
| `kind`          | `"named"`                                            |
| `display_name`  | `"String"`                                           |
| `canonical_name`| `"alloc::string::String"` (prelude-resolved)         |

**Referenced by `returns_type`:**

```
returns_type {
    function_id: "src/utils/hash.rs|65|7|verify_hash|function",
    type_id:     blake3("rust" | "src/utils/hash.rs" | "Result<bool, String>"),
}
```

Generic args do not appear in `canonical_name`; the constructor head
is what aggregates across files. Queries that want "every use of
`Result` regardless of args" join on `canonical_name =
'std::result::Result'`.

### Example 4 — `reference` (modeled as `generic`)

**Source.** `src/utils/hash.rs:65` again, the first parameter `path: &str`.

**`type` rows.**

The `&str`:

| column          | value                                              |
|-----------------|----------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "&str")`  |
| `kind`          | `"generic"`                                        |
| `language`      | `"rust"`                                           |
| `display_name`  | `"&str"`                                           |
| `canonical_name`| `null`                                             |

Rationale for `kind = "generic"` and `canonical_name = null`:
references are not first-class types in the seven-variant schema;
modeling them as `generic` (one type "argument", the referent)
preserves the constructor/argument relationship without adding an
eighth variant. The referent `str` resolves separately (next row).

The inner `str`:

| column          | value                                              |
|-----------------|----------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/hash.rs" \| "str")`   |
| `kind`          | `"primitive"`                                      |
| `display_name`  | `"str"`                                            |
| `canonical_name`| `"str"`                                            |

The same shape applies to `&mut T` (`display_name = "&mut T"`),
`*const T`, `*mut T`. `&'a T` keeps the lifetime in `display_name`.

### Example 5 — `tuple` and `array` (`&[(String, bool, u64)]`)

**Source.** `src/cli/output.rs:63`

```rust
pub fn print_batch_summary(results: &[(String, bool, u64)]) {
```

**`type` rows.** Four are emitted:

Outer `&[(String, bool, u64)]`:

| column          | value                                                                 |
|-----------------|-----------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/cli/output.rs" \| "&[(String, bool, u64)]")`   |
| `kind`          | `"generic"` (the `&` reference wrapper)                               |
| `display_name`  | `"&[(String, bool, u64)]"`                                            |
| `canonical_name`| `null`                                                                |

The slice `[(String, bool, u64)]`:

| column          | value                                                                 |
|-----------------|-----------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/cli/output.rs" \| "[(String, bool, u64)]")`    |
| `kind`          | `"array"` (per the mapping table — slice and fixed array both map here)|
| `display_name`  | `"[(String, bool, u64)]"`                                             |
| `canonical_name`| `null`                                                                |

The tuple `(String, bool, u64)`:

| column          | value                                                                 |
|-----------------|-----------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/cli/output.rs" \| "(String, bool, u64)")`      |
| `kind`          | `"tuple"`                                                             |
| `display_name`  | `"(String, bool, u64)"`                                               |
| `canonical_name`| `null` (tuples are structural — no head symbol; rationale: per ADR-0003 unresolved targets get `null`) |

Inner primitives/named (`String`, `bool`, `u64`) each get their own
row, identical in shape to Examples 1–3 above. They dedup per file.

**Referenced by `parameter`:**

```
parameter {
    function_id: "src/cli/output.rs|63|7|print_batch_summary|function",
    index:       0,
    name:        "results",
    type_id:     blake3("rust" | "src/cli/output.rs" | "&[(String, bool, u64)]"),
    is_optional: false,
    has_default: false,
}
```

### Example 6 — `named` enum used in a type position (clarifying the no-`union` choice)

**Source.** `src/utils/errors.rs:27` (the `kind: ErrorKind` field of `FmtoolError`)

```rust
pub struct FmtoolError {
    pub kind: ErrorKind,
    ...
}
```

**`type` row for `ErrorKind`:**

| column          | value                                                                  |
|-----------------|------------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/utils/errors.rs" \| "ErrorKind")`               |
| `kind`          | `"named"` (**not** `"union"` — Rust enums are nominal)                 |
| `display_name`  | `"ErrorKind"`                                                          |
| `canonical_name`| `"crate::utils::errors::ErrorKind"` (resolved against same-file `enum_item` at line 9) |

The enum's tag set (`Io`, `Parse`, `Conversion`, …) lives in the
`symbol` relation as separate rows of kind `"constant"`-equivalent
variants; the `type` relation does not encode them.

### Example 7 — `generic` constructor with same head used twice (intra-file dedup)

**Source.** `src/core/cache.rs:20`

```rust
pub struct Cache {
    entries: HashMap<String, CacheEntry>,
    ...
}
```

and `src/core/processor.rs:17`

```rust
pub type DataMap = HashMap<String, String>;
```

**`type` rows.**

In `cache.rs`:

| column          | value                                                                          |
|-----------------|--------------------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/core/cache.rs" \| "HashMap<String, CacheEntry>")`       |
| `kind`          | `"generic"`                                                                    |
| `display_name`  | `"HashMap<String, CacheEntry>"`                                                |
| `canonical_name`| `"std::collections::HashMap"` (resolved via `use std::collections::HashMap;` at line 6) |

In `processor.rs`:

| column          | value                                                                          |
|-----------------|--------------------------------------------------------------------------------|
| `id`            | `blake3("rust" \| "src/core/processor.rs" \| "HashMap<String, String>")`       |
| `kind`          | `"generic"`                                                                    |
| `display_name`  | `"HashMap<String, String>"`                                                    |
| `canonical_name`| `"std::collections::HashMap"`                                                  |

Two distinct rows (different `id` because `file_id` and
`display_name` differ), same `canonical_name`. Cross-file
aggregation queries join on `canonical_name = 'std::collections::HashMap'`
to find every `HashMap` use across the project.

Also note: `DataMap` itself, when used as a parameter type elsewhere
(e.g. `pub fn process_file(path: &str) -> DataTable`), produces a
`named` row with `display_name = "DataMap"` and `canonical_name =
"crate::core::processor::DataMap"` — **not** `HashMap<String,
String>`. This is the alias-canonicalization choice from above.
