# Types — C++

This document is the contract for how C++ type expressions map to the `type` relation defined in [`virgil-datalog-schema.md`](virgil-datalog-schema.md). It commits to Level-3 extraction per [ADR-0003](adr/0003-level-3-types-and-references.md). `symbol_id` strings follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

Scope: C++ source files (`.cpp`, `.cc`, `.cxx`) and C++ headers (`.hpp`, `.hxx`, `.hh`). The bare `.h` extension maps to C, not C++ — see the C contract for `.h`.

C++ is the most syntactically complex of the supported languages. Full template instantiation, ADL, and SFINAE resolution are explicit non-goals; the boundary is stated under "Resolution boundary" below.

---

## Tree-sitter node kinds

Every tree-sitter node kind that can appear as a type expression. The schema `kind` column is one of `primitive`, `named`, `generic`, `union`, `intersection`, `function`, `tuple`, `array`.

| Node kind | What it represents | `kind` |
|---|---|---|
| `primitive_type` | Built-in fundamental type (`int`, `char`, `bool`, `void`, `float`, `double`, `size_t` when produced by grammar as primitive) | `primitive` |
| `sized_type_specifier` | `unsigned int`, `long long`, `signed char`, etc. (size/sign modifiers on a primitive) | `primitive` |
| `type_identifier` | Bare named type (`Stage`, `string`, `MemoryPool`) | `named` |
| `qualified_identifier` (in a type position) | Namespace-qualified type name (`std::string`, `dataforge::Stage`) | `named` |
| `template_type` | Templated type instantiation (`std::vector<Stage*>`, `std::map<std::string, std::string>`) | `generic` |
| `pointer_declarator` (carrying `*` onto an inner type) | `T*` — a pointer to `T` | `generic` (see "Pointers and references") |
| `reference_declarator` (carrying `&` or `&&` onto an inner type) | `T&`, `T&&` — lvalue/rvalue reference | `generic` (see "Pointers and references") |
| `abstract_pointer_declarator` | `*` in an unnamed type position (e.g. parameter `void*`) | folded into the named pointer type |
| `abstract_reference_declarator` | `&` / `&&` in an unnamed type position | folded into the named reference type |
| `array_declarator` / `abstract_array_declarator` | `T[]`, `T[N]` | `array` |
| `function_declarator` (in a type position, e.g. `int (*)(int)`) | C-style function pointer / function type | `function` |
| `auto` | `auto` placeholder | `named` (special — see below) |
| `decltype` | `decltype(expr)` placeholder | `named` (special — see below) |
| `dependent_type` | `typename T::value_type` inside a template | `named`, `canonical_name = null` |
| `placeholder_type_specifier` | `auto` / `decltype(auto)` produced by the grammar in some contexts | `named` (special — see below) |
| `enum_specifier` (used as a type, not a definition) | `enum Color` used as a parameter type | `named` |
| `struct_specifier` / `class_specifier` / `union_specifier` (used as a type, not a definition) | `struct Point` used as a parameter type | `named` |
| `type_descriptor` | Wrapper produced by `sizeof`/casts/template args | unwrapped; the inner type produces the row |

### Single node kind, multiple schema kinds

- `template_type` always maps to `generic` regardless of its arguments. `std::vector<int>` is `generic`, not `array`.
- A `pointer_declarator` over a `function_declarator` (e.g. `int (*)(int)`) maps to `function`, *not* a pointer wrapping a `function` row. Function pointers and function types collapse into one `function` row whose `display_name` carries the `(*)` shape verbatim.
- `array_declarator` over a `pointer_declarator` (`int* xs[10]`) maps to `array`. The element type (`int*`) becomes a separate `generic` row (per policy 2 above) referenced by `display_name` only — array element typing is not relational in this schema.
- `type_identifier` is `named`. `template_type` whose `name` field is the same identifier is `generic` — never both. The relationship is captured by `display_name` containing the `<...>`.

### Pointers and references

C++ pointer/reference declarators are part of the declarator subtree, not the type subtree. The extractor must walk up the declarator chain to build the full `display_name`.

Updated per `docs/contract-review.md` (policy 2): pointer / reference types across Rust/C/C++/Go all map to `kind = "generic"` with one type argument (the referent). The `display_name` keeps the `*` / `&` / `&&` punctuation, and `canonical_name` includes the punctuation too.

- `T*`, `T**`, `T***` → `kind = "generic"`, `display_name = "T*"` / `"T**"` / `"T***"`. Each level of indirection is its own row whose single type argument is the inner type.
- `T&`, `T&&` → `kind = "generic"`, `display_name = "T&"` / `"T&&"`.
- `const T*`, `T* const`, `const T* const` → cv-qualifiers preserved in `display_name`; see normalization below.

### `auto` and `decltype`

Both `auto` and `decltype(...)` are emitted as `kind = "named"` rows with:

- `display_name = "auto"` (or the literal source text of the `decltype` expression, normalized — e.g. `"decltype(x + y)"`)
- `canonical_name = null` — resolution requires real type inference, which is out of scope.

Rationale: `auto` is syntactically a type expression, so we must produce a row to keep `parameter.type_id` non-null where the source has a type annotation. Marking `canonical_name = null` is honest — we do not know what it resolves to.

### Dependent types

`typename T::value_type` inside a template body is `kind = "named"`, `display_name = "T::value_type"` (or the source text), `canonical_name = null`. We do not chase dependent member types; they remain unresolved by design.

---

## `display_name` construction

`display_name` is built by walking the AST and concatenating tokens with a fixed whitespace policy. Goal: `Vec<i32>` and `Vec< i32 >` produce identical `display_name`.

### Rules

1. Walk the type subtree (including any declarator wrapping needed for `*`/`&`/`[]`/function-pointer shape).
2. Emit each token's source text, separated by single ASCII spaces only where C++ grammar requires them between identifiers/keywords (e.g. `unsigned int`, `const T`).
3. Collapse all runs of whitespace inside a type subtree to a single space.
4. No spaces around: `<`, `>`, `,` inside template arguments, `::`, `*`, `&`, `&&`, `[`, `]`, `(`, `)`.
5. One space between cv-qualifiers and the type they qualify: `const T`, `volatile T`, `T const` (preserved positionally — we do not move qualifiers; `const T` stays `const T`, `T const` stays `T const`).
6. Function-pointer syntax `int (*)(int, char)` keeps its parentheses and inner commas with no space after commas.
7. Reference qualifiers `&`, `&&` and pointer `*` glue directly to the preceding type token, e.g. `Stage*`, `std::string&`.

### Examples (informational; canonical examples are at the end)

| Source | `display_name` |
|---|---|
| `int` | `int` |
| `unsigned long long` | `unsigned long long` |
| `std::string` | `std::string` |
| `std::string&` | `std::string&` |
| `const std::string&` | `const std::string&` |
| `std::vector<Stage*>` | `std::vector<Stage*>` |
| `std::map<std::string, std::string>` | `std::map<std::string,std::string>` |
| `void*` | `void*` |
| `int (*)(int, char)` | `int(*)(int,char)` |
| `auto` | `auto` |
| `decltype(x + y)` | `decltype(x+y)` |

Note: inner expression whitespace inside `decltype(...)` is normalized the same way (single-space-between-identifiers, no spaces around operators).

### Dedup

Per [ADR-0003](adr/0003-level-3-types-and-references.md), `type` rows dedup per `(language, file_id, display_name)`. Multiple occurrences of `std::string` in the same file produce one row.

---

## `canonical_name` resolution

`canonical_name` is the fully-qualified name the `display_name` resolves to, or `null` when unresolved. C++ scope walk follows the standard lookup order.

### Scope walk order

For a `type_identifier` or `qualified_identifier` appearing at byte offset `b` in file `f`:

1. **Local scope**: type aliases (`using Foo = ...;`, `typedef ... Foo;`) declared in the enclosing block before `b`.
2. **Enclosing function template parameter list**: template type parameters (`template <typename T>`) bind `T` to itself; `canonical_name = T` (local name, not fully qualified, since instantiation isn't tracked).
3. **Enclosing class scope**: nested type aliases, nested classes/structs, member typedefs.
4. **Enclosing namespace scope(s)**: walk outward through every enclosing `namespace_definition`.
5. **`using namespace` directives** in the current scope chain: introduce names from the named namespace into the current scope.
6. **`using` declarations** (`using std::string;`): introduce a single name from another namespace.
7. **File scope**: top-level typedefs, using-declarations, `#include`-ed declarations *if* the included header was indexed in the same workspace. External (system) headers contribute nothing.
8. **Anonymous global**: built-in primitive types short-circuit at step 0 — they do not walk.

The walk stops at the first match. Multiple matches at the same scope level are a C++ ambiguity error; we record the first one in tree-sitter source order and leave detecting the ambiguity to a higher-level audit.

### Unresolved cases (`canonical_name = null`)

- The grammar produced an `ERROR` node anywhere in the type subtree.
- The type refers to a symbol declared in a header outside the indexed workspace (e.g. `std::string` when `<string>` is not indexed).
- Generic / template parameters (`T` inside a `template <typename T>`): `canonical_name = T` (the local parameter name), not `null` — they are resolved, just not to a non-parameter symbol.
- `auto` and `decltype(...)` (always `null`).
- `dependent_type` (`typename T::value_type`): always `null`.
- A `qualified_identifier` whose leading namespace cannot be located in the indexed workspace.

### Aliases

C++ has two alias forms: `typedef OldType NewName;` and `using NewName = OldType;`. The decision: **aliases canonicalize to themselves**, not through to the aliased type.

- `using Buf = std::vector<char>;` then later `Buf x;` → the type row for `Buf` has `display_name = "Buf"`, `canonical_name = "<namespace>::Buf"`.
- Walking the alias chain is left to query-time joins. A future query helper can chase `using NewName = OldType;` declarations via a separate `type_alias` table if needed; not in scope.

Rationale: alias-chasing is a query concern, not an extraction concern. Eagerly chasing would collapse two distinct source-level entities into one row and lose information.

### Template parameters in `canonical_name`

When `T` is the parameter of an enclosing template, `canonical_name = T` (local). No mangling, no fully-qualified path. Reason: there is no global identity for an un-instantiated parameter.

### Templates as their unspecialized form

`std::vector<int>` and `std::vector<std::string>` both have `display_name` containing the argument, but the `canonical_name` of the *generic* template itself is `std::vector` — i.e. we resolve the template name, not the instantiation. The arguments live only in `display_name`. There is no `type_arguments` relation. **This is the templates boundary.**

### Resolution boundary

The extractor does *not* attempt:

- Template instantiation, partial-specialization matching, or SFINAE.
- ADL (argument-dependent lookup) at call sites — only direct scope walk.
- Two-phase name lookup inside templates.
- `auto` / `decltype` type inference.
- Overload resolution for type-dependent expressions.

These produce `canonical_name = null` (or, in the case of templates-as-types, the unspecialized name).

---

## Field types — `field_type` relation

Per the schema, every class/struct/union data-member symbol with a typed declaration emits a `field_type {symbol_id, type_id}` row linking the field symbol to its `type` row. Covers C++ class fields (including `public`/`private`/`protected` data members), `struct` members, and `union` members. Local variables and function parameters are not field declarations and use `parameter` / `references` wiring instead.

## Identity

Per [ADR-0003](adr/0003-level-3-types-and-references.md), `type.id = blake3(language | file_id | display_name)`. The hash inputs are joined with the literal `|` separator. `language` is the string `"cpp"`.

C++-specific normalization applied to `display_name` *before* hashing:

- All whitespace collapsed per the rules above.
- No reordering of cv-qualifiers — `const T` and `T const` are distinct rows (intentional; mirroring source intent).
- Pointer/reference shapes (`*`, `&`, `&&`) are part of `display_name` and therefore part of the hash.

---

## Worked examples

All examples are drawn from `../virgil-skills/benchmarks/cpp/data-processor/`. Line/byte ranges are tree-sitter `Range`s. `file_id` is the workspace-relative path (per ADR-0002).

### Example 1 — `primitive`: `int` return type of `Stage::process`

**Source:** `src/core/stage.cpp:35`

```cpp
int Stage::process(void* input, void* output) {
```

**`type` row(s):**

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|src/core/stage.cpp\|int")` | `primitive` | `cpp` | `int` | `int` |
| `blake3("cpp\|src/core/stage.cpp\|void*")` | `generic` | `cpp` | `void*` | `void*` |
| `blake3("cpp\|src/core/stage.cpp\|void")` | `primitive` | `cpp` | `void` | `void` |

**`returns_type` row:**

| function_id | type_id |
|---|---|
| `src/core/stage.cpp\|35\|0\|process\|function` | id of the `int` row above |

**`parameter` rows:**

| function_id | index | name | type_id | is_optional | has_default |
|---|---|---|---|---|---|
| `src/core/stage.cpp\|35\|0\|process\|function` | 0 | `input` | id of `void*` row | false | false |
| `src/core/stage.cpp\|35\|0\|process\|function` | 1 | `output` | id of `void*` row | false | false |

`void*` resolves: `void` is a primitive (its own row), `*` wraps it as a `generic` row with one type argument. Updated per `docs/contract-review.md` (policy 2): pointer types are `kind = "generic"`. `canonical_name = "void*"` because the base is built-in.

---

### Example 2 — `generic`: `std::vector<Stage*>` field

**Source:** `include/dataforge/core/pipeline.hpp:27`

```cpp
    std::vector<Stage*> stages_;
```

**`type` rows emitted (one per file × display_name):**

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|include/dataforge/core/pipeline.hpp\|std::vector<Stage*>")` | `generic` | `cpp` | `std::vector<Stage*>` | `std::vector` |
| `blake3("cpp\|include/dataforge/core/pipeline.hpp\|Stage*")` | `generic` | `cpp` | `Stage*` | `dataforge::Stage*` |
| `blake3("cpp\|include/dataforge/core/pipeline.hpp\|Stage")` | `named` | `cpp` | `Stage` | `dataforge::Stage` |

The `std::vector<Stage*>` row has `canonical_name = "std::vector"` — the unspecialized template name. Arguments live in `display_name` only.

`Stage*` resolves to a `generic` row (one type argument: the inner `Stage` `named` row). `Stage` is a forward-declared class in the same `dataforge` namespace at line 8 of the same header; canonical name `dataforge::Stage`. The pointer shape `*` is appended in both `display_name` and `canonical_name` (per policy 2).

There is no `parameter` row pointing at `std::vector<Stage*>` directly — it appears as the type of a *field* (data member). The schema's `parameter` table is for function parameters only; field type wiring is via `references` rows with `ref_kind = "type_use"` (see [`references-cpp.md`](references-cpp.md)).

---

### Example 3 — `generic`: `std::map<std::string, std::string>` parameter

**Source:** `include/dataforge/core/stage.hpp:16`

```cpp
    virtual bool initialize(const std::map<std::string, std::string>& params);
```

**`type` rows:**

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|include/dataforge/core/stage.hpp\|bool")` | `primitive` | `cpp` | `bool` | `bool` |
| `blake3("cpp\|include/dataforge/core/stage.hpp\|const std::map<std::string,std::string>&")` | `generic` | `cpp` | `const std::map<std::string,std::string>&` | `const std::map&` (the reference `&` wrapper is preserved per policy 2; the inner `std::map<...>` row has `canonical_name = "std::map"`) |
| `blake3("cpp\|include/dataforge/core/stage.hpp\|std::map<std::string,std::string>")` | `generic` | `cpp` | `std::map<std::string,std::string>` | `std::map` |
| `blake3("cpp\|include/dataforge/core/stage.hpp\|std::string")` | `named` | `cpp` | `std::string` | `null` |

The outer parameter type is the full `const std::map<std::string, std::string>&`. Its `kind` is `generic` because the reference `&` is now encoded as a one-arg generic wrapper (per `docs/contract-review.md` policy 2). The wrapped `std::map<std::string,std::string>` is also `generic` (template instantiation). `canonical_name` preserves the `&` and `const` markers — see the worked example above.

`std::string` has `canonical_name = null` because `<string>` is not indexed in this workspace (system header).

**`returns_type`:**

| function_id | type_id |
|---|---|
| `include/dataforge/core/stage.hpp\|16\|17\|initialize\|method` | id of the `bool` row |

**`parameter`:**

| function_id | index | name | type_id |
|---|---|---|---|
| `include/dataforge/core/stage.hpp\|16\|17\|initialize\|method` | 0 | `params` | id of the `const std::map<std::string,std::string>&` row |

---

### Example 4 — `function` (callable type): `FactoryFunc` using-alias

**Source:** `include/dataforge/core/registry.hpp:16`

```cpp
    using FactoryFunc = std::function<std::unique_ptr<Stage>()>;
```

This declares a type alias. The right-hand side is a `template_type` (`std::function<...>`) whose single argument is a callable shape `std::unique_ptr<Stage>()`.

**`type` rows for the right-hand side:**

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|include/dataforge/core/registry.hpp\|std::function<std::unique_ptr<Stage>()>")` | `generic` | `cpp` | `std::function<std::unique_ptr<Stage>()>` | `std::function` |
| `blake3("cpp\|include/dataforge/core/registry.hpp\|std::unique_ptr<Stage>()")` | `function` | `cpp` | `std::unique_ptr<Stage>()` | `null` |
| `blake3("cpp\|include/dataforge/core/registry.hpp\|std::unique_ptr<Stage>")` | `generic` | `cpp` | `std::unique_ptr<Stage>` | `std::unique_ptr` |
| `blake3("cpp\|include/dataforge/core/registry.hpp\|Stage")` | `named` | `cpp` | `Stage` | `dataforge::Stage` |

Note: the inner `std::unique_ptr<Stage>()` is emitted as `kind = "function"` because syntactically it is a parenthesized callable signature `R()` — the canonical "function type" shape. `canonical_name = null` (the type has no name; it is a structural type).

The alias name `FactoryFunc` itself becomes a `symbol` (`kind = "type_alias"`), not a `type` row. Downstream uses of `FactoryFunc` in this file (line 22) emit a separate `type` row with `display_name = "FactoryFunc"`, `canonical_name = "dataforge::Registry::FactoryFunc"`.

---

### Example 5 — `array` and pointer in same parameter list

**Source:** `src/utils/memory_pool.cpp:34`

```cpp
void* MemoryPool::allocate(size_t size) {
```

**`type` rows:**

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|src/utils/memory_pool.cpp\|void*")` | `generic` | `cpp` | `void*` | `void*` |
| `blake3("cpp\|src/utils/memory_pool.cpp\|size_t")` | `named` | `cpp` | `size_t` | `null` |

`size_t` has `canonical_name = null` because `<cstddef>` is a system header not indexed in this workspace. If the workspace included an indexed `<cstddef>`, the resolver would set `canonical_name = "std::size_t"`.

**`returns_type` / `parameter`:**

| function_id | index | name | type_id |
|---|---|---|---|
| (returns) `src/utils/memory_pool.cpp\|34\|6\|allocate\|method` | — | — | id of `void*` |
| `src/utils/memory_pool.cpp\|34\|6\|allocate\|method` | 0 | `size` | id of `size_t` |

---

### Example 6 — `named` with `auto` placeholder

**Source:** `src/core/pipeline.cpp:42`

```cpp
    for (auto it = stages_.begin(); it != stages_.end(); ++it) {
```

The `auto` here appears in a local variable declaration. The extractor emits a `type` row:

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|src/core/pipeline.cpp\|auto")` | `named` | `cpp` | `auto` | `null` |

The variable `it` is a `symbol` (`kind = "variable"`); type wiring runs through a `references` row with `ref_kind = "type_use"` pointing at this `type` row (the `referent_id` field is unused for `auto` since `canonical_name = null`).

Local `auto` variables are a common case and not all queries need them; downstream queries can filter with `kind = "named" AND display_name = "auto"`.

---

### Example 7 — `named` with namespace qualifier resolving locally

**Source:** `include/dataforge/io/csv_reader.hpp:10`

```cpp
class CsvReader : public Reader {
```

The base-class name `Reader` is a `type_identifier` in the `base_class_clause` of the `class_specifier`.

**`type` row:**

| id | kind | language | display_name | canonical_name |
|---|---|---|---|---|
| `blake3("cpp\|include/dataforge/io/csv_reader.hpp\|Reader")` | `named` | `cpp` | `Reader` | `dataforge::Reader` |

Resolution: `CsvReader` is inside `namespace dataforge { ... }`. The scope walk finds `class Reader` declared at `include/dataforge/io/reader.hpp:11` (assumed indexed in this workspace via the `#include "dataforge/io/reader.hpp"` at line 3). Canonical name = enclosing-namespace + class name.

The `extends` row emitted alongside:

| child_id | parent_id |
|---|---|
| `include/dataforge/io/csv_reader.hpp\|10\|6\|CsvReader\|class` | `include/dataforge/io/reader.hpp\|11\|6\|Reader\|class` |

And a `references` row of `ref_kind = "type_use"` (see [`references-cpp.md`](references-cpp.md)).
