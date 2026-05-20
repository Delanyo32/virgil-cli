# References — C

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes **fact emission** only. The C extractor emits `scope` / `binding` / `occurrence` rows; the Cozoscript resolver in [`docs/resolution.md`](resolution.md) turns those facts into `references` rows. Resolution is not described here.

Symbol IDs follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`. `start_byte` / `end_byte` / `start_col` are the tree-sitter `Range` of the relevant node.

## Scope tree

C has three lexical scope levels visible in source. The extractor emits one `scope` row per region:

| Source construct | `scope.kind` | Notes |
|---|---|---|
| The file itself (translation unit) | `"file"` | `parent_id = null`. Holds all top-level declarations. |
| Function body | `"function"` | Parent is the file scope. Parameters bind here. |
| `{ ... }` block inside a function | `"block"` | Parent is the enclosing function or block scope. `if`/`for`/`while`/`switch` bodies open a block scope. The `for(init; ...; ...)` clause opens a block scope that wraps the body. |

C has no module / namespace / class concept. Preprocessor `#ifdef` does not introduce scope — the extractor sees whatever tree-sitter parses (no conditional-compilation evaluation).

`static` at file scope **restricts linkage** but does not open a new scope; the binding still lives at `"file"` kind. The resolver treats `static`-file-scope bindings as not exported across files; that semantics is the resolver's concern.

## Bindings

### `definition`

Emit one row per top-level definition / declaration that introduces a name:
- `function_definition` and forward `function_declarator` declarations
- `declaration` with `init_declarator` at file scope (`int x;`, `int x = 5;`)
- `struct_specifier` / `union_specifier` / `enum_specifier` with a tag name
- `type_definition` (typedef)
- Enumerator constants inside an `enum_specifier`

Block-scope local variables (`int local;` inside a function) also emit a `definition` binding in their enclosing block scope. Symbol id matches the `symbol` row (per Issue #11).

### `parameter`

Emit one row per function parameter in the function scope. The receiver of a function-pointer typedef is **not** a parameter (it's a definition).

### `import`

C has no `import` keyword. The closest construct is `#include`. State explicitly: the extractor does **not** emit per-declaration `import` bindings for header contents — it treats each `#include` as a `wildcard_import` (see below).

### `import_alias`

C has no aliased imports. No rows of this kind are emitted.

### `wildcard_import`

Emit one row per `#include` directive at the file scope:

```
binding {
    scope_id: <file scope>,
    name: "*",
    binding_kind: "wildcard_import",
    symbol_id: null,
}
```

The `start_byte` of the binding row is the `#include` directive's byte offset (used by the resolver to order multiple includes). The matching `imports{importer_file_id, imported_id}` row is emitted by the existing import-extraction pipeline; the resolver joins through it to find exported names in the included header.

**Phase-1 narrowing:** `#define` macros are **not** emitted as bindings. Preprocessor-defined symbols are a follow-up (Phase 4 or later).

## Occurrence emission

### `call`

Every `call_expression` whose function is a plain identifier:

```c
foo(x);          // -> occurrence{name: "foo", kind: "call"}
ptr_to_fn(x);    // -> occurrence{name: "ptr_to_fn", kind: "call"}
```

Indirect calls through expressions (`(funcs[i])(x)`) emit no `call` occurrence — the receiver is an expression, not a name. The argument expressions emit their own `read` occurrences.

### `read`

Every `identifier` node in value position. Includes:
- the operand of `&` (`&x` emits `read` of `x`)
- the operand of `*` dereference (`*p` emits `read` of `p`)
- LHS of `->` (`p->field` emits `read` of `p` only — `field` is NOT emitted per the field-row policy)
- LHS of `.` member access (`s.field` emits `read` of `s` only)
- argument identifiers in function calls
- subscript expression base (`a[i]` emits `read` of `a` and `read` of `i`)

### `write`

Every assignment LHS where the LHS is a plain identifier:

```c
x = 5;       // -> occurrence{name: "x", kind: "write"}
x += 1;      // -> occurrence{name: "x", kind: "write"} (compound: single write, no read)
x++; ++x;    // -> occurrence{name: "x", kind: "write"}
```

Pointer-target writes (`*p = 5`) emit `read` of `p`, not `write` of `p` — the pointer itself isn't being written, the pointee is. Field writes (`s.field = 5`, `p->field = 5`) emit `read` of `s` / `p`; `field` is not emitted (field-row policy).

### `type_use`

Every type-position identifier. Includes:
- the type spec in a declaration (`MyStruct x;` emits `type_use` of `MyStruct`)
- struct/union/enum tag references (`struct Foo y;` emits `type_use` of `Foo`)
- typedef names in casts (`(MyType)x` emits `type_use` of `MyType`)
- `sizeof(T)` for type operand

Each `type_use` occurrence should align with a `type` row emitted by `types-c.md`.

### `import_use`

The path string inside `#include "foo.h"` is **not** emitted as an `occurrence` — there's no identifier to resolve, just a filename. The header's resolved file path lives in `imports`, and the wildcard binding in `binding` is what the resolver uses.

## What this contract does NOT cover

- Resolution algorithm (in [`docs/resolution.md`](resolution.md))
- Preprocessor expansion (`#define` macros not emitted)
- Conditional compilation (no `#ifdef` evaluation)
- Header guard semantics
- Field-precision tracking on struct/union access (field-row policy from contract review)

## Worked examples

All examples drawn from `../virgil-skills/benchmarks/c/embedded-sensors/`. Quoted line ranges are inclusive.

### Example 1 — `extern` declaration referencing a symbol defined elsewhere

`sensors.c`, ~lines 10–12 (representative — pick an actual `extern` from the corpus):

```c
extern int sensor_count;
void init_sensors(void) {
    sensor_count = 0;
}
```

**`scope`:**
| id | parent_id | kind | start_byte |
|---|---|---|---|
| `sensors.c\|0\|file` | null | file | 0 |
| `sensors.c\|<func start_byte>\|function` | `sensors.c\|0\|file` | function | … |

**`binding`:**
| scope_id | name | kind | symbol_id |
|---|---|---|---|
| `sensors.c\|0\|file` | `sensor_count` | definition | `sensors.c\|10\|0\|sensor_count\|variable` |
| `sensors.c\|0\|file` | `init_sensors` | definition | `sensors.c\|11\|0\|init_sensors\|function` |

**`occurrence`** (inside `init_sensors`):
| name | kind | enclosing_symbol_id |
|---|---|---|
| `sensor_count` | write | `sensors.c\|11\|0\|init_sensors\|function` |

The resolver walks from the function scope outward, finds the `extern` declaration in the file scope, and resolves `referent_id` to `sensor_count`'s symbol id.

### Example 2 — Pointer write `*ptr = value`

```c
void set_value(int *ptr, int value) {
    *ptr = value;
}
```

**`binding`** (in function scope):
| name | kind | symbol_id |
|---|---|---|
| `ptr` | parameter | `<param sym for ptr>` |
| `value` | parameter | `<param sym for value>` |

**`occurrence`:**
| name | kind | notes |
|---|---|---|
| `ptr` | read | base of `*ptr` |
| `value` | read | RHS of assignment |

No `write` occurrence for `ptr` — the pointer itself isn't being written, the memory it points at is. No occurrence for the implicit "thing pointed to" — there's no name.

### Example 3 — Struct field write `s.field = 5`

```c
struct Config { int retries; };
void configure(struct Config *cfg) {
    cfg->retries = 5;
}
```

**`occurrence`** (inside `configure`):
| name | kind |
|---|---|
| `cfg` | read |

The field name `retries` is **not** emitted. The resolver doesn't have a `binding` for it in any scope visible here (field bindings inside `struct Config` exist only if Issue #11+ emits them); if absent, no `references` row for the field. This is the field-row policy applied to writes.

### Example 4 — Function-pointer call

```c
typedef int (*Handler)(int);
void dispatch(Handler h, int x) {
    h(x);
}
```

**`binding`** (in function scope):
| name | kind | symbol_id |
|---|---|---|
| `h` | parameter | `<param sym>` |
| `x` | parameter | `<param sym>` |

**`occurrence`:**
| name | kind |
|---|---|
| `h` | call |
| `x` | read |

The `call` occurrence uses `h` as the callee. The resolver walks the function scope, finds the parameter binding for `h`, and resolves to the parameter's symbol. (`h` is a `Handler` pointer, but the resolver doesn't follow the typedef.)

### Example 5 — `static` file-scope variable

```c
static int counter = 0;
int next_id(void) { return ++counter; }
```

**`binding`** (in file scope):
| name | kind | symbol_id |
|---|---|---|
| `counter` | definition | `<file>\|1\|0\|counter\|variable` |
| `next_id` | definition | `<file>\|2\|0\|next_id\|function` |

**`occurrence`** (inside `next_id`):
| name | kind |
|---|---|
| `counter` | write | (pre-increment counts as write per the compound rule) |

The `static` keyword changes the *symbol*'s linkage (recorded in `c_attrs.is_file_static` per the attrs contract) but does **not** change how the `binding` row is emitted. The resolver, walking from `next_id`'s function scope to the file scope, finds the definition. Cross-file lookups skip file-static bindings (resolver's job, not this contract's).
