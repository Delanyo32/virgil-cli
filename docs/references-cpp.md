# References — C++

This document is the contract for how C++ identifier occurrences map to the `references` relation defined in [`virgil-datalog-schema.md`](virgil-datalog-schema.md). Level-3 extraction per [ADR-0003](adr/0003-level-3-types-and-references.md). `symbol_id` strings follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

Scope: C++ source files (`.cpp`, `.cc`, `.cxx`) and C++ headers (`.hpp`, `.hxx`, `.hh`). `.h` maps to C.

Per `docs/virgil-datalog-schema.md`, `references` is keyed by `(referrer_id, site_file, site_start_byte, match_index)` with `referent_id` and `ref_kind` in the value position. `referrer_id` is the enclosing symbol containing the occurrence. `referent_id` is the symbol the occurrence names, or `null` when unresolved. `ref_kind` is one of `read`, `write`, `type_use`, `import_use`. `match_index = 0` for the primary/only candidate; overload resolution emits additional rows at `match_index = 1, 2, ...` sharing the same `(referrer_id, site_file, site_start_byte)` (per `docs/contract-review.md`, policy 1).

---

## Lexical scope rules

C++ has block, function, class, namespace, and translation-unit (file) scope, plus template-parameter scope.

### Scopes (innermost outward)

1. **Block scope** — every `compound_statement` (`{ ... }`). New declarations live until the closing brace.
2. **Function parameter scope** — parameter names visible inside the function body.
3. **Function scope** — labels (e.g. for `goto`); rarely relevant.
4. **Class scope** — members of the enclosing `class_specifier`, `struct_specifier`, or `union_specifier`. Includes inherited members from base classes named in the `base_class_clause`.
5. **Enclosing namespace(s)** — names declared in every `namespace_definition` from innermost outward.
6. **Anonymous namespace** — treated like an enclosing namespace whose name is the empty string; names have internal linkage.
7. **`using namespace X;` directives** — re-export `X`'s names into the scope containing the directive (transparent: visible at the same scope level).
8. **`using N::name;` declarations** — introduce a single name from `N` at the scope containing the directive.
9. **File scope** — top-level declarations in the translation unit, plus declarations from `#include`-ed headers that are indexed in the workspace.
10. **Template parameter scope** — `template <typename T, int N>` binds `T` and `N` over the entity that follows.

### Lookup walk

For an unqualified identifier at byte `b` in file `f`:

1. Start at the innermost enclosing block.
2. Walk outward through enclosing blocks → function parameters → enclosing class (including base classes, depth-first) → enclosing namespace chain → `using` declarations/directives at each level → file scope → indexed-header file scopes via transitive `#include`.
3. Stop at the first match.

For a qualified identifier `A::B::C`:

1. Resolve the leading qualifier `A` via the unqualified-lookup rules above.
2. If `A` is a namespace or class, look up `B` *within* `A`'s declarations only (no further outer walk).
3. Repeat for `C`.
4. If any step fails, the entire qualified name is unresolved.

### Shadowing rules

- **Block shadowing**: a name declared in an inner block hides the same name in any outer block. Earlier binding wins for occurrences before the inner declaration; the inner binding wins after its point of declaration.
- **Parameter vs. member**: a parameter shadows a class member of the same name inside the function body. Inside such a function, `this->x` is required to reach the member.
- **Class member vs. namespace name**: class scope wins inside member-function bodies.
- **`using namespace` is not shadowing**: names introduced by `using namespace` lose to any name declared directly in the target scope. Ambiguity across multiple `using namespace` directives is recorded as multiple candidates — see "Multiple candidates".
- **Function overloading is not shadowing**: multiple functions with the same name at the same scope are all candidates; see "Overload resolution".

### `this->`, smart pointers, member access

- `this->member` and `obj.member` and `ptr->member` are all `field_expression`s in tree-sitter. The base (`this`, `obj`, `ptr`) is a `read` reference to the variable. A `read`/`write` row for the *member* is emitted **only** when the member has a known `symbol_id` in the store (per `docs/contract-review.md`, policy 5). Class data members extracted as `field` symbols qualify; members on unindexed external types produce no member-level row.
- Smart-pointer dereferencing (`unique_ptr->foo`, `(*shared_ptr).foo`) is treated identically to raw-pointer dereferencing. The extractor does not special-case `std::unique_ptr` / `std::shared_ptr` — they parse as `field_expression`s exactly like raw pointers.
- Pointer-to-member access (`obj.*pmf`, `obj->*pmf`) is detected as a `pointer_expression` with `.*` / `->*`. The base is `read`; the right operand is `read`. We do not attempt to resolve the actual member being pointed at.

---

## `ref_kind` decision tree

For each AST occurrence of an identifier, decide one `ref_kind`. If multiple kinds would apply (rare), the first matching rule below wins.

### `read`

An identifier whose value is being evaluated. AST patterns:

- An `identifier` node appearing as a sub-expression of any expression: function arguments, RHS of assignment, condition of `if`/`while`/`for`, `return` value, initializer, etc.
- A `field_expression`'s `field` identifier (`obj.foo`, `this->foo`, `ptr->foo`).
- A `qualified_identifier` used as a value (e.g. `std::cout`, `Pipeline::stages_` when read).
- The callee identifier in a `call_expression` is `read`. (We do not invent a separate `call` ref_kind; `calls` already has its own relation. The `references` row complements it.)
- `sizeof(x)` where `x` is an identifier (yes, `sizeof` evaluates the type, but the identifier reads the entity).

Exceptions (no `references` row emitted):

- Macro names inside `#define` / `#ifdef` / `#ifndef` / `#if defined(...)` — preprocessor names are out of scope for `references`.
- Attribute paths (`[[nodiscard]]`, `[[gnu::pure]]`).
- Identifiers inside string literals or character literals (they are not identifiers in the AST).

### `write`

An identifier that is being assigned to or mutated. AST patterns:

- LHS of an `assignment_expression` with operator `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`. The full LHS-tail identifier is `write`; intermediate identifiers in the LHS chain (e.g. the `obj` in `obj.foo = 1`) are `read`. Compound assignments emit a single `write` row at the LHS site — no separate `read` (per `docs/contract-review.md`, policy 3: faithful read+write semantics is Level 4, deferred).
- Pre- or post-increment / decrement (`++x`, `x++`, `--x`, `x--`): one `write` row at the operand identifier.
- An identifier passed by non-const lvalue reference to a function — we do **not** mark this as `write`. Reason: detecting it requires looking at the callee signature, which is not always available. Convention: lvalue-reference parameters stay `read`. (This is a known false-negative; downstream audits handle it.)
- An identifier passed to a method that mutates by language convention (e.g. `vec.push_back(x)`): treat `vec` as `read`, not `write`. We do not enumerate STL mutating methods.

### `type_use`

An identifier appearing in a type position. AST patterns:

- Parameter type annotation (the `type` field of a `parameter_declaration`).
- Return type of a `function_definition` / `function_declarator`.
- Cast target in `(T)x`, `static_cast<T>(x)`, `dynamic_cast<T>(x)`, `reinterpret_cast<T>(x)`, `const_cast<T>(x)`.
- Generic argument inside a `template_type` (`std::vector<Stage*>` → both `std::vector` and `Stage` get `type_use` rows; the pointer `*` is purely syntactic).
- Base class names in a `base_class_clause` (`: public Reader`).
- Type alias right-hand side (`using Foo = Bar;` → `Bar` is `type_use`).
- `sizeof(T)` and `alignof(T)` operand when it is a type.
- `new T(...)` allocation type.
- Class-member declarations: the *type* of a `field_declaration` produces `type_use`; the field *name* itself defines a symbol, not a reference.

Every `type_use` row ties back to a `type` row emitted per [`types-cpp.md`](types-cpp.md). The relationship: the `type` row owns `display_name` and `canonical_name`; the `references` row owns the byte-offset occurrence.

### `import_use`

Identifiers inside an `#include` directive. AST patterns:

- The `path` child of a `preproc_include` — string literal or system path. The `raw_import` / `imports` rows already capture this; the `references` row exists as a uniform hook so audits don't have to special-case includes.

`referrer_id` for `import_use` is the file-level symbol (the synthetic file symbol per ADR-0002 with kind = `"file"`). `referent_id` is the imported file's id if the header is indexed; otherwise `null` (see resolution below).

Note: there is no `import_use` row for `using namespace std;` or `using std::string;` — those are scope-modification declarations, not imports. The names they introduce produce ordinary `read`/`type_use` rows at the usage site.

---

## `referent_id` resolution

Algorithm for mapping an identifier occurrence to the `referent_id` column.

### Precedence

For an unqualified identifier:

1. Local block scope (innermost first).
2. Enclosing function's parameters.
3. Enclosing function template parameters (`template <typename T>` → `T` is a symbol of kind `type_parameter` if present in the symbol table; today this kind is not emitted, so `referent_id = null` for template parameter usages).
4. Enclosing class members. Walk inherited members through `base_class_clause` (`extends`) depth-first.
5. Enclosing namespace chain (innermost outward).
6. `using` declarations visible at each scope level encountered along the walk.
7. File scope.
8. Names from indexed `#include`-ed headers (transitive — header `A` includes header `B`; symbols in `B` are reachable when including `A`).

For a `qualified_identifier`: resolve the leading qualifier via the above, then perform *only* in-scope lookup within the qualifier's namespace/class for each subsequent segment.

The resolver uses the `symbols_by_name` index that already exists in `src/graph/builder.rs`. A per-file scope tree is built during reference extraction; the `symbols_by_name` index supplies the cross-file candidates for steps 5–8.

### Multiple candidates (overload resolution)

C++ allows multiple symbols with the same name at the same scope (overloaded free functions, overloaded member functions, function vs. variable in disjoint scopes pulled together by `using namespace`).

**Decision: record all candidates as separate `references` rows, distinguished by `match_index`.**

Each row shares `referrer_id`, `site_file`, and `site_start_byte`; `match_index` runs `0, 1, 2, ...` across candidates (per the updated schema in `docs/virgil-datalog-schema.md`). `match_index = 0` is the primary candidate (typically the first match in lookup order); additional overload candidates get `match_index = 1, 2, ...`. Each row may carry a different `referent_id` and `ref_kind`.

Rationale: heuristic overload resolution requires argument-type matching, which requires reliable type info on every argument expression. We have type info at type-annotation sites, not at arbitrary expression sites. Recording all candidates is honest. Downstream audits that care can prune candidates by signature when they have signature data.

### No candidate

If lookup yields zero matches: emit a single `references` row with `referent_id = null` (updated per `docs/contract-review.md`, policy 1: the schema now declares `referent_id: String?` in the value position; rows for unresolved identifiers use the SQL null, not a sentinel string). Do not skip the row. A `read` of an unknown name is itself useful for audits (e.g. "what symbols are referenced but never defined here?").

### Templates

References inside a template body that depend on a template parameter (`T x;` where `T` is a parameter): no `referent_id` — they are unresolved by design. The `type` row exists with `canonical_name = T` (the local parameter name), but no `references` row points at it as a defining symbol because the parameter has no symbol id in the current schema.

---

## Worked examples

All examples are drawn from `../virgil-skills/benchmarks/cpp/data-processor/`. `referrer_id` and `referent_id` use ADR-0002 ids: `path|start_line|start_col|name|kind`. `site_start_byte` is the tree-sitter byte offset of the identifier occurrence.

For brevity, ids below are written as `<path>|L|C|<name>|<kind>`; full symbol ids are pipe-joined verbatim.

### Example 1 — `read`, `write`, and `type_use` in one function body

**Source:** `src/utils/memory_pool.cpp:34-57`

```cpp
void* MemoryPool::allocate(size_t size) {
    if (size == 0) {
        return nullptr;
    }

    size_t alloc_size = size;
    if (alloc_size < block_size_) {
        alloc_size = block_size_;
    }

    void* block = std::malloc(alloc_size);
    if (block == nullptr) {
        std::cerr << "MemoryPool: allocation failed for " << alloc_size << " bytes" << std::endl;
        return nullptr;
    }

    std::memset(block, 0, alloc_size);
    blocks_.push_back(block);
    total_allocated_ += alloc_size;
    allocation_count_++;

    return block;
}
```

Let `R = src/utils/memory_pool.cpp|34|6|allocate|method` (the referrer for every row in this body).

| referrer_id | referent_id | ref_kind | site_file | site_start_byte (occurrence) | note |
|---|---|---|---|---|---|
| R | `src/utils/memory_pool.cpp\|34\|0\|void*\|type` (via `type` row) | `type_use` | `src/utils/memory_pool.cpp` | byte of `void*` at L34 | return type |
| R | `src/utils/memory_pool.cpp\|34\|26\|size_t\|type` (via `type` row) | `type_use` | `src/utils/memory_pool.cpp` | byte of `size_t` at L34 | parameter type |
| R | `src/utils/memory_pool.cpp\|34\|33\|size\|parameter` | `read` | … | byte of `size` at L35 (in `size == 0`) | |
| R | (local `alloc_size` defined at L39) | `write` | … | byte of `alloc_size` at L39 | declaration is a definition, not a write; **do not emit** a `write` for the declarator itself |
| R | (local `alloc_size`) | `read` | … | byte of `alloc_size` at L40 | RHS read in `< block_size_` |
| R | `include/dataforge/utils/memory_pool.hpp\|26\|11\|block_size_\|field` | `read` | … | byte of `block_size_` at L40 | member access via implicit `this` |
| R | (local `alloc_size`) | `write` | … | byte of LHS `alloc_size` at L41 | assignment |
| R | `include/dataforge/utils/memory_pool.hpp\|26\|11\|block_size_\|field` | `read` | … | byte of `block_size_` at L41 | RHS read |
| R | (local `block` defined at L44) | (declaration; no row) | … | … | |
| R | `null` (`std::malloc` — system header not indexed) | `read` | … | byte of `std::malloc` at L44 | unresolvable → single row with `referent_id = null` per policy 1 |
| R | (local `alloc_size`) | `read` | … | byte at L44 (call arg) | |
| R | `include/dataforge/utils/memory_pool.hpp\|25\|22\|blocks_\|field` | `read` | … | byte of `blocks_` at L51 | base of method call; **read**, not write |
| R | `null` for `push_back` | `read` | … | byte of `push_back` at L51 | method on `std::vector`; system header |
| R | `include/dataforge/utils/memory_pool.hpp\|27\|11\|total_allocated_\|field` | `write` | … | byte of `total_allocated_` at L52 | compound assignment `+=` |
| R | (local `alloc_size`) | `read` | … | byte at L52 RHS | |
| R | `include/dataforge/utils/memory_pool.hpp\|28\|11\|allocation_count_\|field` | `write` | … | byte of `allocation_count_` at L53 | post-increment |

Notes:
- The declarator-as-definition rule: an identifier whose AST position is the declarator's `name` (i.e. it *introduces* the symbol) does not produce a `references` row. The `symbol` row plus its `span` already capture the definition.
- `blocks_.push_back(block)`: `blocks_` is `read`, not `write` (see `ref_kind.write` rule about STL mutating methods). Audits that care can post-process.

### Example 2 — `this->` access and `field_expression`

**Source:** `src/core/pipeline.cpp:21-26`

```cpp
Pipeline::~Pipeline() {
    // DEBT: stages_ contains raw pointers ...
    stages_.clear();
}
```

Let `R = src/core/pipeline.cpp|21|0|~Pipeline|method`.

| referrer_id | referent_id | ref_kind | site_start_byte | note |
|---|---|---|---|---|
| R | `include/dataforge/core/pipeline.hpp\|27\|24\|stages_\|field` | `read` | byte of `stages_` at L25 | implicit `this->stages_` |
| R | `null` for `clear` | `read` | byte of `clear` at L25 | `std::vector<Stage*>::clear` — not indexed |

Resolution: `stages_` is a member of `Pipeline` (the enclosing class of `~Pipeline`). Lookup walks: block (empty) → function params (none) → class `Pipeline` (declared at `include/dataforge/core/pipeline.hpp:11`) → finds member `stages_` at L27. Canonical match.

### Example 3 — Shadowing: parameter vs. member

**Source:** `src/core/stage.cpp:77-79`

```cpp
void Stage::set_name(const std::string& name) {
    name_ = name;
}
```

Let `R = src/core/stage.cpp|77|0|set_name|method`.

| referrer_id | referent_id | ref_kind | site_start_byte | note |
|---|---|---|---|---|
| R | `include/dataforge/core/stage.hpp\|16\|0\|std::string\|type` (via `type` row, file scope) | `type_use` | byte of `std::string` at L77 | parameter type |
| R | `include/dataforge/core/stage.hpp\|25\|16\|name_\|field` | `write` | byte of `name_` at L78 | LHS of `=` |
| R | `src/core/stage.cpp\|77\|34\|name\|parameter` | `read` | byte of `name` at L78 | RHS of `=` |

Resolution for `name` on the RHS: lookup walks innermost-out → function parameter `name` matches first → parameter wins. The class member `name_` (different name) is not a competitor here.

If the parameter had been named `name_` (collision with the member), the parameter would still win for that identifier in the body. Reaching the member would require `this->name_`. We do not warn on this here — that's an audit concern.

### Example 4 — Multiple overload candidates

**Source:** `include/dataforge/core/pipeline.hpp:13-14`, call site `src/main.cpp` (synthesized — the constructor is declared twice as an overload):

```cpp
class Pipeline {
public:
    Pipeline();
    explicit Pipeline(const std::string& name);
```

Suppose a `main.cpp` contains:

```cpp
    Pipeline p("ingest");
```

Two `Pipeline` constructors are visible. The call-target resolver records both as candidates.

Let `R = src/main.cpp|<L>|<C>|main|function` (the enclosing function).

| referrer_id | match_index | referent_id | ref_kind | site_start_byte | note |
|---|---|---|---|---|---|
| R | 0 | `include/dataforge/core/pipeline.hpp\|11\|6\|Pipeline\|class` | `type_use` | byte of `Pipeline` at call site | the *type* `Pipeline` is referenced |
| R | 0 | `include/dataforge/core/pipeline.hpp\|13\|4\|Pipeline\|constructor` | `read` | byte of `Pipeline` at call site | overload candidate #1 (default ctor); primary |
| R | 1 | `include/dataforge/core/pipeline.hpp\|14\|13\|Pipeline\|constructor` | `read` | byte of `Pipeline` at call site | overload candidate #2 (string-arg ctor) |

The two `read` rows share `(referrer_id, site_file, site_start_byte)` and are distinguished by `match_index` (per the updated schema key in `docs/virgil-datalog-schema.md`). Note the `type_use` row at `match_index = 0` does not collide with the constructor `read` at `match_index = 0` because their `ref_kind` differs and the `(referrer_id, site_file, site_start_byte, match_index)` tuple alone is the relation key — `ref_kind` lives in the value position. In practice the extractor allocates `match_index` per identifier role: type-use occurrences use their own `match_index = 0`; overload-resolved value occurrences use their own `match_index` series starting at 0. Implementation note: when type-use and value-use occurrences would otherwise share `(site_file, site_start_byte)`, allocate a fresh `match_index` to keep rows distinct.

Pruning the correct overload by argument type is the job of downstream queries with full type info — not this extractor.

### Example 5 — `using` declaration and qualified-name resolution

**Source:** `src/core/registry.cpp:21`

```cpp
    factories_[name] = std::move(factory);
```

Context: this is inside `Registry::register_stage` (`src/core/registry.cpp:17-22`). The `std::move` is a qualified identifier; no `using namespace std;` is present in this file.

Let `R = src/core/registry.cpp|17|0|register_stage|method`.

| referrer_id | referent_id | ref_kind | site_start_byte | note |
|---|---|---|---|---|
| R | `include/dataforge/core/registry.hpp\|37\|34\|factories_\|field` | `write` | byte of `factories_` at L21 | LHS via `factories_[name] = ...`; the *subscript* is the write target. `factories_` itself is read; the `[name]` write is on the resulting reference. **Practical rule:** emit one `write` row keyed on `factories_` and one `read` row for `name`. The subscript identity is not in the schema. |
| R | `src/core/registry.cpp\|17\|56\|name\|parameter` | `read` | byte of `name` (subscript arg) at L21 | |
| R | `null` for `std::move` | `read` | byte of `std::move` at L21 | system header `<utility>` not indexed |
| R | `src/core/registry.cpp\|17\|71\|factory\|parameter` | `read` | byte of `factory` at L21 | |

The `<unresolved>` row for `std::move` is honest. If `<utility>` were indexed in the workspace, the resolver would walk: file scope → no local `move` → file imports `<algorithm>` → not there → `<stdexcept>` → not there → finally indexed `<utility>` → match `std::move`. Resolution succeeds only when the qualifier and the leaf both resolve.

### Example 6 — `type_use` chain through a template

**Source:** `include/dataforge/core/registry.hpp:16`

```cpp
    using FactoryFunc = std::function<std::unique_ptr<Stage>()>;
```

Let `R = include/dataforge/core/registry.hpp|16|10|FactoryFunc|type_alias`.

| referrer_id | referent_id | ref_kind | site_start_byte | note |
|---|---|---|---|---|
| R | `null` for `std::function` | `type_use` | byte of `std::function` | `<functional>` not indexed |
| R | `null` for `std::unique_ptr` | `type_use` | byte of `std::unique_ptr` | `<memory>` not indexed |
| R | `include/dataforge/core/stage.hpp\|10\|6\|Stage\|class` | `type_use` | byte of `Stage` | resolved via `using` of namespace `dataforge`; `Stage` is in the same namespace |

Three `type_use` rows for one source line — one per identifier-in-type-position. Tree-sitter exposes each as a distinct node; the extractor walks the whole `template_type` subtree and emits one row per `type_identifier` / `qualified_identifier` it encounters.

### Example 7 — `import_use`

**Source:** `src/core/pipeline.cpp:4-7`

```cpp
#include "dataforge/core/pipeline.hpp"
#include "dataforge/core/stage.hpp"
#include "dataforge/utils/logger.hpp"

#include <iostream>
```

Let `R = src/core/pipeline.cpp|0|0|<file>|file` (synthetic file symbol).

| referrer_id | referent_id | ref_kind | site_start_byte | note |
|---|---|---|---|---|
| R | `include/dataforge/core/pipeline.hpp\|0\|0\|<file>\|file` | `import_use` | byte of L4 `#include ...` | resolved local include |
| R | `include/dataforge/core/stage.hpp\|0\|0\|<file>\|file` | `import_use` | byte of L5 | |
| R | `include/dataforge/utils/logger.hpp\|0\|0\|<file>\|file` | `import_use` | byte of L6 | |
| R | `null` for `<iostream>` | `import_use` | byte of L8 | system header |

These rows duplicate information in the `imports` relation but provide a uniform reference-shaped view. Queries can join either way.
