# Types — C

This contract describes how every C type expression encountered by the
tree-sitter-c grammar maps to a row in the `type` relation defined in
`docs/virgil-datalog-schema.md`. C has no generics, no templates, no
lifetimes — but it does have pointer, array, and function-pointer types
that require structural decomposition under Level 3 (ADR-0003).

Source-of-truth tree-sitter grammar version: the one already wired into
`src/languages/c_lang/`. Headers (`.h`) are deliberately parsed as C, so
this contract also applies to `.h` files.

## Tree-sitter node kinds

Every node kind below can appear as part of a type expression in C. The
right column states the schema `kind` value the extractor must emit.

| Node kind                | What it represents                              | Schema `kind` |
|--------------------------|-------------------------------------------------|---------------|
| `primitive_type`         | `void`, `int`, `char`, `float`, `double`, `_Bool` (the grammar's built-in keyword set) | `primitive` |
| `sized_type_specifier`   | `unsigned int`, `long long`, `signed char`, `short`, etc. (multi-keyword integer/char specifiers) | `primitive` |
| `type_identifier`        | A name introduced by `typedef` or a bare struct/union/enum tag used as a type (e.g. `sensor_id_t`, `size_t`, `FILE`) | `named` |
| `struct_specifier`       | `struct S` or `struct S { ... }` used in a type position | `named` |
| `union_specifier`        | `union U` or `union U { ... }` used in a type position | `named` |
| `enum_specifier`         | `enum E` or `enum E { ... }` used in a type position | `named` |
| `pointer_declarator`     | The `*` (or `**`, `***`, …) attached to a declarator — encodes pointer-to-T at parse time | `generic` (see decision below) |
| `array_declarator`       | `T name[N]` / `T name[]` — array-of-T | `array` |
| `function_declarator`    | `R name(P1, P2, …)` — when synthesised as a type (function pointer or function-prototype type) | `function` |
| `abstract_pointer_declarator` | Pointer in an abstract type (e.g. inside `sizeof(int *)` or a cast) | `generic` |
| `abstract_array_declarator`   | Array in an abstract type                                                | `array` |
| `abstract_function_declarator` | Function-prototype shape in an abstract type                            | `function` |

### Splits across schema kinds

- `type_identifier` always maps to `named`. We do not try to inline the
  referent (e.g. `sensor_id_t` does not get rewritten to `uint16_t`); the
  rewrite happens at `canonical_name` resolution, not at `kind` selection.
- `struct_specifier` / `union_specifier` / `enum_specifier` map to `named`
  even when they carry an anonymous inline body (e.g. `struct { int x; }`);
  in that case `display_name` is the literal source slice
  (`"struct { int x; }"`), `canonical_name` is `null`, and the type is
  per-file by construction.
- `pointer_declarator` ALWAYS becomes a single `generic` row regardless of
  pointer depth: `int **` produces two nested `generic` rows
  (`ptr<ptr<int>>`), one per `*`. The schema does not get a fresh `pointer`
  kind — see decision below.

### Decision: pointers are `generic`, not a new kind

ADR-0003 fixes the eight `kind` variants (`primitive`, `named`, `generic`,
`union`, `intersection`, `function`, `tuple`, `array`). Pointers naturally
fit `generic` with one type argument (the pointee), so we reuse that
variant rather than adding a `pointer` kind. The constructor name is
encoded purely in `display_name` (`ptr<T>`) and `canonical_name`
(`ptr<canonical(T)>`).

Rationale: keeps the schema closed (no per-language kind drift),
mirrors how Rust extractors will encode `Box<T>` / `&T`, and lets
downstream Cozoscript queries find "any pointer-ish thing" via
`display_name` starts-with `ptr<`. The alternative — a fresh `pointer`
kind — was rejected because it leaks C-specific representation into a
schema shared by nine languages.

## `display_name` construction

`display_name` is the textual rendering of the type. Construction is
purely structural — no name resolution happens here.

Rules:

1. Whitespace is normalized to a single space between tokens. Leading,
   trailing, and runs-of-multiple spaces collapse. `unsigned    int`
   becomes `"unsigned int"`.
2. `const` and `volatile` qualifiers ARE preserved (`const char *` and
   `char *` are different types and must hash to different ids). They
   appear in the order they syntactically appear in the source, then a
   single space, then the unqualified type token. Example:
   `volatile uint32_t` → `"volatile uint32_t"`.
3. `restrict` is preserved with the same rules as `const`.
4. Pointers render as `ptr<INNER>`. Multi-level pointers nest:
   `int **` → `"ptr<ptr<int>>"`. A qualified pointer like `const int *`
   renders as `"ptr<const int>"`; a `const`-pointer `int * const`
   renders as `"const ptr<int>"` (the qualifier sits on the pointer
   itself, not the pointee).
5. Arrays render as `array<INNER, N>` when the size literal is a plain
   integer constant, and `array<INNER>` when the size is missing
   (`T[]`) or non-literal (`T[N+1]`, `T[expr]`). Multi-dimensional
   arrays nest: `int m[3][4]` → `"array<array<int, 4>, 3>"` (outer
   dimension first, matching declaration order).
6. Function-prototype types render as `fn(P1, P2, …) -> R` where each
   `Pn` is the recursive `display_name` of the parameter type (no
   parameter names) and `R` is the recursive `display_name` of the
   return type. Variadic prototypes render the trailing `...` literally:
   `fn(const char *, ...) -> int`. A bare `void` parameter list renders
   as `fn() -> R` (the `void` is dropped — it's grammar, not a type).
7. Struct/union/enum tags render as the literal source token including
   the keyword: `struct sensor_reading_t` → `"struct sensor_reading_t"`,
   `enum status_t` → `"enum status_t"`. A `type_identifier` referring
   to a typedef of a tagged struct renders just as the typedef name
   (`"sensor_reading_t"`); the tag prefix is dropped at this layer.
8. Anonymous inline structs/unions/enums render as the literal source
   slice (whitespace-normalised), e.g. `"struct { int x; int y; }"`.

`display_name` must round-trip the source's intent: `Vec<i32>` vs
`Vec< i32 >` is irrelevant for C, but `unsigned int` vs `unsigned   int`
must collapse to the same string, and `const char*` vs `const char *`
must collapse to the same string.

## `canonical_name` resolution

Per ADR-0003, every resolvable `type` row gets a `canonical_name`. C's
scope walk for type names is:

1. **Local scope** — `typedef`s and tag declarations that appear in the
   current translation unit before the use site. Tree-sitter does not
   give us preprocessor expansion, so "current translation unit" means
   the parsed `.c` or `.h` file plus the file-level symbols it directly
   includes. Walk symbols in declaration order; later declarations
   shadow earlier ones (illegal in standard C, but we don't enforce it).
2. **Included headers** — every file reachable via the `imports`
   relation from the current file (recursive transitive closure, but
   capped at depth 5 to mirror the call-graph cap). Headers are searched
   in include order; first match wins.
3. **Project root** — any header in the workspace whose `path`
   basename matches a `type_identifier`'s `name`. This is a heuristic
   fallback for cases where `#include` paths are inexact.
4. **Unresolved** — emit `canonical_name = null`. We do NOT walk system
   headers (`<stdint.h>`, `<stddef.h>`, etc.); types like `uint16_t`,
   `size_t`, `int16_t`, `FILE`, `NULL` end up unresolved by design.

### Alias resolution

C `typedef`s are NOT flattened. `typedef uint16_t sensor_id_t;` produces:

- a `symbol` row for `sensor_id_t` with `kind = "typedef"`
- when `sensor_id_t` is used as a type, the `type` row has
  `display_name = "sensor_id_t"`, `canonical_name =` the symbol id of
  the typedef declaration (per ADR-0002 format).

The transitive walk (`sensor_id_t` → `uint16_t` → `unsigned short`) is
NOT performed by the extractor; it's a query-time decision. Rationale:
flattening loses the typedef's identity, which downstream audits (e.g.
"all uses of `sensor_id_t`") need to recover.

### Tag types

`struct sensor_reading_t` (with the keyword) and `sensor_reading_t`
(typedef'd alias) have different `display_name`s and therefore different
`type.id`s, even though they refer to the same underlying record. Both
resolve to the same `canonical_name` when the typedef is visible — the
canonical points at the typedef's symbol id, not the struct tag's.

When only the bare struct tag is declared (no typedef), `canonical_name`
points at the `struct_specifier`'s own symbol id.

### Built-in primitives

Primitive types (`int`, `char`, `float`, `double`, `void`, `_Bool`, plus
sized variants like `unsigned int`, `long long`) have
`canonical_name = display_name` — they are their own canonical form.

Sized-type ordering is canonicalised: the canonical form lists
`signed`/`unsigned` first, then `short`/`long`/`long long`, then the
base type. So `int unsigned long` (legal but unusual) and
`unsigned long int` both canonicalise to `"unsigned long int"`.

## Identity

Per ADR-0003:

```
type.id = blake3(language | file_id | display_name)
```

For C:
- `language = "c"`
- `file_id` = the file's `id` per the `file` relation (the path itself,
  per ADR-0002).
- `display_name` = the string produced by the rules above. The hash
  input is the post-normalisation string (whitespace already collapsed).

The `|` separator is a literal `0x7C` byte. No length prefixing — the
inputs are not user-controlled in a way that admits prefix collisions.

Pointer/array/function type rows are emitted per `display_name`, which
means `int *` in `foo.c` and `int *` in `bar.c` get different `type.id`s
even though the canonical form is identical. Cross-file aggregation
joins through `canonical_name`, exactly as ADR-0003 prescribes.

## Field types — `field_type` relation

Per the schema, every C struct/union member declaration emits a
`field_type {symbol_id, type_id}` row linking the member symbol to
its `type` row. Members of anonymous structs/unions whose containing
type has no name produce no row (no field symbol to key against).
Function parameters and locals use `parameter` / `references` wiring
instead.

## Worked examples

Every example below is sourced from
`../virgil-skills/benchmarks/c/embedded-sensors/`. Line numbers are the
tree-sitter `Range` of the *type expression node*, not the enclosing
declaration. The `type.id` column is shown as `blake3(...)` because the
hash value is not knowable without running blake3; the inputs are
spelled out so the implementation is mechanically verifiable.

### Example 1 — primitive (`primitive` kind)

Source: `include/types.h:11`

```c
typedef uint16_t sensor_id_t;
```

The `uint16_t` token (a `type_identifier`, technically — it's a stdint
typedef) appears as the underlying type of the `sensor_id_t` typedef.

Row emitted:

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/types.h" \| "uint16_t")` |
| `kind`          | `named` |
| `language`      | `"c"` |
| `display_name`  | `"uint16_t"` |
| `canonical_name`| `null` (stdint header not indexed) |

The typedef symbol `sensor_id_t` is recorded in `symbol`, not `type`;
the `type` row here describes the *underlying* expression on the RHS.

### Example 2 — struct tag as type (`named` kind)

Source: `include/types.h:32-40`

```c
typedef struct {
    sensor_id_t   id;
    sensor_type_t type;
    float         value;
    float         raw_value;
    uint32_t      timestamp_ms;
    uint8_t       quality;
    status_t      status;
} sensor_reading_t;
```

The anonymous `struct { ... }` produces ONE `type` row (the anonymous
struct itself); each field declaration produces additional `type` rows
for its type expression (covered separately by the field extractor).

Row for the anonymous struct:

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/types.h" \| "struct { sensor_id_t id; sensor_type_t type; float value; float raw_value; uint32_t timestamp_ms; uint8_t quality; status_t status; }")` |
| `kind`          | `named` |
| `language`      | `"c"` |
| `display_name`  | `"struct { sensor_id_t id; sensor_type_t type; float value; float raw_value; uint32_t timestamp_ms; uint8_t quality; status_t status; }"` |
| `canonical_name`| symbol id of the `sensor_reading_t` typedef (the typedef gives the anonymous struct its public name) |

When `sensor_reading_t` is later used as a type (e.g. in
`include/sensorhub.h:24`), it produces a separate `type` row with
`display_name = "sensor_reading_t"`, `kind = named`, and
`canonical_name =` the same typedef symbol id.

### Example 3 — pointer (`generic` kind)

Source: `include/sensorhub.h:24`

```c
int sensorhub_read_sensor(sensor_id_t id, sensor_reading_t *reading);
```

The parameter type `sensor_reading_t *` decomposes into two nested
type rows:

Inner row (the pointee `sensor_reading_t`):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/sensorhub.h" \| "sensor_reading_t")` |
| `kind`          | `named` |
| `display_name`  | `"sensor_reading_t"` |
| `canonical_name`| symbol id of the typedef at `include/types.h:32-40` |

Outer row (the pointer):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/sensorhub.h" \| "ptr<sensor_reading_t>")` |
| `kind`          | `generic` |
| `display_name`  | `"ptr<sensor_reading_t>"` |
| `canonical_name`| `"ptr<" + canonical(inner) + ">"` if the inner is resolved, else `null` |

The corresponding `parameter` row references the outer (pointer) row's
id as `type_id`. Inner rows are emitted only because they are
structurally part of the outer type — they are not referenced from the
`parameter` relation.

### Example 4 — array (`array` kind)

Source: `src/init.c:25`

```c
static sensor_reading_t g_readings[MAX_SENSORS];
```

The array size `MAX_SENSORS` is a macro identifier, not an integer
literal, so the size is omitted from `display_name`.

Inner row (`sensor_reading_t`): as in Example 3.

Outer row (the array):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "src/init.c" \| "array<sensor_reading_t>")` |
| `kind`          | `array` |
| `display_name`  | `"array<sensor_reading_t>"` |
| `canonical_name`| `"array<" + canonical(inner) + ">"` |

If the same file had `int buf[16];`, the row would have
`display_name = "array<int, 16>"` because `16` is a plain integer
literal.

### Example 5 — function-pointer typedef (`function` kind)

Source: `include/types.h:56`

```c
typedef void (*sensor_callback_t)(sensor_id_t id, const sensor_reading_t *reading);
```

This typedef introduces `sensor_callback_t` as an alias for a pointer
to a function. The function type and the pointer wrapper produce two
nested rows.

Function-type row (the inner `fn(...)`):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/types.h" \| "fn(sensor_id_t, ptr<const sensor_reading_t>) -> void")` |
| `kind`          | `function` |
| `display_name`  | `"fn(sensor_id_t, ptr<const sensor_reading_t>) -> void"` |
| `canonical_name`| canonical form built by substituting each parameter/return's `canonical_name` (or `null` if any component is unresolved) |

Pointer row (the outer `ptr<fn(...)>`):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/types.h" \| "ptr<fn(sensor_id_t, ptr<const sensor_reading_t>) -> void>")` |
| `kind`          | `generic` |
| `display_name`  | `"ptr<fn(sensor_id_t, ptr<const sensor_reading_t>) -> void>"` |
| `canonical_name`| `"ptr<" + canonical(inner) + ">"` |

When `sensor_callback_t` is later used as a type (in the
`g_callbacks[MAX_SENSORS]` declaration at `src/init.c:27`), it produces
a separate `type` row with `display_name = "sensor_callback_t"`,
`kind = named`, `canonical_name =` the typedef's symbol id.

### Example 6 — sized primitive plus qualifier (`primitive` kind)

Source: `include/types.h:59`

```c
typedef volatile uint32_t reg32_t;
```

The underlying type `volatile uint32_t` is a `sized_type_specifier`-like
construct (the `volatile` qualifier wraps the `type_identifier`
`uint32_t`).

Row emitted (the type the typedef aliases):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/types.h" \| "volatile uint32_t")` |
| `kind`          | `named` |
| `display_name`  | `"volatile uint32_t"` |
| `canonical_name`| `null` (uint32_t lives in `<stdint.h>`, not indexed) |

The `volatile` is preserved because dropping it would silently equate
register-mapped types with their plain counterparts — a meaningful
distinction in embedded code.

### Example 7 — pointer to const (`generic` kind, with qualified pointee)

Source: `include/sensorhub.h:25`

```c
int sensorhub_configure_sensor(sensor_id_t id, const sensor_config_t *cfg);
```

The parameter type `const sensor_config_t *` decomposes as:

Inner row (`const sensor_config_t`):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/sensorhub.h" \| "const sensor_config_t")` |
| `kind`          | `named` |
| `display_name`  | `"const sensor_config_t"` |
| `canonical_name`| `"const " + canonical("sensor_config_t")` |

Outer row (`ptr<const sensor_config_t>`):

| column          | value |
|-----------------|-------|
| `id`            | `blake3("c" \| "include/sensorhub.h" \| "ptr<const sensor_config_t>")` |
| `kind`          | `generic` |
| `display_name`  | `"ptr<const sensor_config_t>"` |
| `canonical_name`| `"ptr<" + canonical(inner) + ">"` |

This is distinct from a `sensor_config_t * const cfg` (const pointer
to mutable struct), which would render as
`"const ptr<sensor_config_t>"`. The position of `const` matters.
