# References — C++

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes **fact emission** only. The C++ extractor emits `scope` / `binding` / `occurrence` rows; the Cozoscript resolver in [`docs/resolution.md`](resolution.md) consumes them and materialises `references` rows. Resolution is not described here.

Symbol IDs follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`. `start_byte` / `end_byte` / `start_col` are the tree-sitter `Range` of the relevant node.

## Scope tree

C++ has a five-level scope hierarchy emitted as `scope` rows:

| Source construct | `scope.kind` | Notes |
|---|---|---|
| The file itself (translation unit) | `"file"` | `parent_id = null`. |
| `namespace foo { ... }` | `"namespace"` | Parent is enclosing namespace / file. Anonymous namespaces emit a `namespace` scope with a synthetic name (`"<anon@<byte>>"`). |
| `class` / `struct` / `union` body | `"class"` | Parent is the enclosing namespace / class / file. |
| `function` body / `method` body / `lambda` body | `"function"` | Parameters and template parameters bind here. |
| `{ ... }` block | `"block"` | Inside function bodies. `for(init;…)`, `if(init;…)`, `switch(init;…)` open a single block scope spanning init + body. |

`using namespace foo;` does NOT open a new scope. It introduces names from namespace `foo` into the current scope via a `wildcard_import` binding (see below).

Template parameters bind in a special scope just inside the function/class scope. The extractor emits this as part of the enclosing `function` or `class` scope (no separate template scope) — template parameters bind at the start of that scope's `start_byte`.

## Bindings

### `definition`

One row per definition:
- Function / method / constructor / destructor
- Class / struct / union / enum (typed or scoped)
- Variable (file scope, namespace scope, block scope)
- Typedef / `using X = Y` (the alias name; see also `import_alias` below)
- Template definitions (the template's name binds at the enclosing scope)
- Namespace declarations themselves (`namespace foo` binds `foo` in the parent scope)

`using-declaration` `using foo::bar;` emits a `binding{name: "bar", kind: "import_alias", symbol_id: <foo::bar's id>}` — see `import_alias` below.

### `parameter`

Function/method/constructor/lambda parameters. Template parameters (`template<typename T>`) also emit `parameter` bindings against the template's enclosing function or class scope. Receivers (`this`) are implicit and emitted by the resolver, not as a `binding` here.

### `import`

C++ has no `import` keyword pre-C++20 modules. `#include` is treated as `wildcard_import` (see below). C++20 modules (`import foo;`) are **not** supported in Phase 1 — they require module-resolution machinery that doesn't exist yet.

### `import_alias`

Three sources:
- `using X = Y;` — alias declaration. Binds `X` in current scope; `symbol_id` points at `Y`'s definition's id when resolvable.
- `using foo::bar;` — using-declaration. Binds `bar` in current scope; `symbol_id` points at `foo::bar`.
- `namespace ns = foo::bar;` — namespace alias. Binds `ns` in current scope; `symbol_id` points at the `foo::bar` namespace's synthetic id.

### `wildcard_import`

Two sources:
- `#include "foo.hpp"` / `#include <vector>` — emit one row per include at the file scope, `name: "*"`, `symbol_id: null`. Same shape as C.
- `using namespace foo;` — emit one row in the enclosing scope, `name: "*"`, `symbol_id` pointing at `foo`'s namespace id. The resolver expands by joining through bindings in the target namespace.

## Occurrence emission

### `call`

Every `call_expression` whose callee is an identifier or qualified identifier:

```cpp
foo(x);              // call: "foo"
obj.method(x);       // read: "obj"; field "method" not emitted
ns::function(x);     // call: "function"; "ns" not separately emitted
std::move(v);        // call: "move"
```

Constructor calls (`Foo(x)`, `Foo{x}`) emit `call` of `Foo`. Operator overloads invoked via syntax (`a + b`) do NOT emit a `call` occurrence — `a` and `b` get `read` occurrences only. (Operator-overload resolution requires type info Phase 1 doesn't have.)

Overloaded function calls emit one `call` `occurrence`. The Cozoscript resolver finds multiple candidates in the binding lookup and emits them as separate `references` rows at `match_index = 0, 1, 2, …` per ADR-0003.

### `read`

Every identifier in value position. Includes `this`, `super`-equivalent (`__super` MSVC ext: ignore), the operand of `&`, the operand of `*`, the LHS of `.` / `->` (field name not emitted per field-row policy), subscript bases, range-for variable expressions.

### `write`

Every assignment LHS that's a plain identifier. Compound `+=` etc. → single `write` per ADR-0003. Pointer/reference writes (`*p = x`, `r = x` where `r` is `T&`) emit `read` of the pointer name; the assignment to the pointee/referent is not separately attributed. Field writes via `s.field` / `p->field` follow the field-row policy: emit `read` of `s` / `p` only.

### `type_use`

Every identifier in type position:
- Declaration type specifiers (`Foo x;` → `type_use` of `Foo`)
- Template arguments (`std::vector<int>` → `type_use` of `vector`; the inner `int` is its own `type_use` (primitive))
- Return type, parameter types
- Cast targets (`static_cast<T>(x)` → `type_use` of `T`)
- Inheritance lists (`class D : public Base` → `type_use` of `Base`)
- `decltype` operand emits `read` (the operand is an expression)
- `auto` placeholder emits NO `type_use` (no name to resolve)

`override` and `final` keywords change no extractor behavior — they change attrs (`cpp_attrs.is_override`, `cpp_attrs.is_final`). No occurrence emitted for these keywords.

### `import_use`

The path string inside `#include` is NOT emitted (no identifier).

## What this contract does NOT cover

- Resolution (in [`docs/resolution.md`](resolution.md))
- ADL (argument-dependent lookup) — handled at resolver level
- Template instantiation — only the unspecialized template name emits `type_use`
- Macro expansion (no `#define` bindings)
- Operator overload resolution (operator syntax emits no `call` occurrence)
- C++20 modules

## Worked examples

All examples drawn from `../virgil-skills/benchmarks/cpp/data-processor/`.

### Example 1 — `using` declaration

```cpp
namespace data {
    using std::string;
    string lookup(const string& key);
}
```

**`scope`:**
| id | parent_id | kind |
|---|---|---|
| `<file>\|0\|file` | null | file |
| `<file>\|<ns byte>\|namespace` (named `data`) | `<file>\|0\|file` | namespace |

**`binding`** (in `data` namespace scope):
| name | kind | symbol_id |
|---|---|---|
| `string` | import_alias | `<std::string's id>` |
| `lookup` | definition | `<lookup's id>` |

The using-declaration sources `std::string` into the `data` namespace.

### Example 2 — Namespace alias

```cpp
namespace fs = std::filesystem;
fs::path p;
```

**`binding`** (in file scope):
| name | kind | symbol_id |
|---|---|---|
| `fs` | import_alias | `<std::filesystem's namespace id>` |

**`occurrence`** (for `fs::path p`):
| name | kind |
|---|---|
| `fs` | type_use |
| `path` | type_use |

The resolver chains `fs` → `std::filesystem` → looks up `path` within that namespace.

### Example 3 — Overloaded function call

```cpp
double area(Circle c);
double area(Rectangle r);
double total = area(c) + area(r);
```

**`occurrence`** (for the two `area(...)` calls):
| name | kind | enclosing_symbol_id |
|---|---|---|
| `area` | call | `<containing function>` |
| `area` | call | `<containing function>` |

The extractor emits **two** `call` occurrences, one per call site. The resolver finds both `area` overloads in scope and emits `references` rows at `match_index = 0, 1, …` per call site.

### Example 4 — Virtual method override

```cpp
class Pipeline {
    virtual void run() = 0;
};
class FastPipeline : public Pipeline {
    void run() override;
};
```

**`binding`** (in `FastPipeline` class scope):
| name | kind | symbol_id |
|---|---|---|
| `run` | definition | `<FastPipeline::run's id>` |

**`occurrence`** (in the class header):
| name | kind |
|---|---|
| `Pipeline` | type_use | (in the base-class list) |

The `override` keyword changes `cpp_attrs.is_override` to `true` and (per contract review) `cpp_attrs.is_virtual` to `true`. No occurrence emitted for `override`.

### Example 5 — Smart-pointer dereference

```cpp
std::unique_ptr<Stage> stage = std::make_unique<Stage>();
stage->execute();
```

**`occurrence`:**
| name | kind |
|---|---|
| `unique_ptr` | type_use |
| `Stage` | type_use |
| `make_unique` | call |
| `Stage` | type_use | (template arg of make_unique) |
| `stage` | read | (LHS of `->`) |
| `execute` | NOT emitted (field-row policy) |

The `->` operator on a smart pointer is treated identically to `->` on a raw pointer — `stage` emits `read`, the method name doesn't emit. Type-aware reasoning to follow `unique_ptr<Stage>` to `Stage::execute` is beyond Phase 1.
