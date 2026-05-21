# Language attributes — C

This contract describes the `c_attrs` relation: per-symbol attributes
that are meaningful in C but not in every other language we index.
Hot attributes get typed columns here; rare attributes fall back to
the generic `symbol_attr` escape hatch.

## Schema

```
:create c_attrs {
    symbol_id: String =>
    is_file_static: Bool default false,
    is_extern: Bool default false,
    is_inline: Bool default false,
    is_const: Bool default false,
    is_volatile: Bool default false,
    is_restrict: Bool default false,
    gcc_attributes: [String] default [],
}
```

Updated per `docs/contract-review.md` (policy 4): `*_attrs` columns
must not duplicate `symbol` columns. The previous `is_static` column
collided with `symbol.is_static`; it is renamed `is_file_static` to
record C's file-scope linkage modifier without shadowing the
cross-language `is_static` flag.

This extends the schema in `docs/virgil-datalog-schema.md`, which
declared `c_attrs` with only `is_static`/`is_extern`/`is_inline`. The
four additional columns (`is_const`, `is_volatile`, `is_restrict`,
`gcc_attributes`) are added here because they are cheap to extract and
load-bearing for embedded-C audits.

### Column applicability

| column           | applies to                                              | default |
|------------------|---------------------------------------------------------|---------|
| `is_file_static` | functions, file-scope variables                         | `false` |
| `is_extern`      | functions, file-scope variables                         | `false` |
| `is_inline`      | functions only                                          | `false` |
| `is_const`       | variables, function parameters, typedefs                | `false` |
| `is_volatile`    | variables, function parameters, typedefs                | `false` |
| `is_restrict`    | pointer-typed variables/parameters                      | `false` |
| `gcc_attributes` | functions, variables, struct/union/enum, typedefs       | `[]`    |

A row is emitted for EVERY C symbol — even if all columns are default
— so that downstream queries can join `c_attrs` without an outer-join
dance. Defaults are written explicitly when the source has no
qualifier.

### Symbol kinds that get a row

The `c_attrs` row is keyed by the `symbol.id` of any symbol whose
`language = "c"`. That covers (per the existing C extractor):

- functions (`kind = "function"`, including prototypes in headers)
- variables (`kind = "variable"`)
- struct/union/enum tags (`kind = "struct" | "union" | "enum"`)
- typedefs (`kind = "typedef"`)
- macros (`kind = "macro"`)

Macros never carry any of these qualifiers — their rows always hold
the default values. They're emitted for join-uniformity, not because
the source ever populates them.

## Extraction rules

### `is_file_static`

AST source: a `storage_class_specifier` node whose text is `"static"`
appears as a direct child of the symbol's declaration node
(`function_definition`, `declaration`).

Edge case: `static` inside a function body (block-scope) marks a local
as having *static storage duration*, NOT file-local linkage. The
extractor sets `is_file_static = true` for these too — the column
records every C `static` keyword occurrence, regardless of whether
its effect is file-local linkage (file scope) or static storage
duration (block scope). Downstream queries that care about linkage
cross-check `symbol.parent_id` (a non-null parent means block-scope).

The renamed column avoids collision with `symbol.is_static`, which
is a cross-language flag carried by every symbol row (per
`docs/contract-review.md`, policy 4).

Default: `false`.

### `is_extern`

AST source: `storage_class_specifier` with text `"extern"` as a direct
child.

Edge case: a function prototype WITHOUT an `extern` keyword still has
external linkage by C's default rules. The extractor does NOT infer
`is_extern = true` from absence of `static` — `is_extern` reflects the
presence of the keyword only. Use `is_file_static = false AND is_extern = false`
to query "default external linkage".

Default: `false`.

### `is_inline`

AST source: a `storage_class_specifier` with text `"inline"` (the
tree-sitter-c grammar treats `inline` as a storage class even though
the standard categorises it as a function specifier).

Default: `false`. Only meaningful for `kind = "function"`; on
non-function symbols this stays `false`.

### `is_const`

AST source: a `type_qualifier` node with text `"const"` appearing in
the type prefix of the declaration.

Edge case: `const` can sit on the pointee OR the pointer
(`const int *p` vs `int * const p`). `is_const` reflects ONLY the
top-level qualifier on the symbol itself (the second form). The
qualifier on the pointee is encoded in the `type` row's
`display_name` (see `docs/types-c.md`), not here.

Default: `false`.

### `is_volatile`

AST source: a `type_qualifier` node with text `"volatile"` at the
top level. Same rules as `is_const` regarding pointer-vs-pointee.

Default: `false`.

### `is_restrict`

AST source: a `type_qualifier` node with text `"restrict"` (or
`"__restrict"`, `"__restrict__"` — these vendor variants normalise to
`true` as well).

Default: `false`. Only set on pointer-typed symbols; non-pointer
symbols stay `false`.

### `gcc_attributes`

AST source: every `attribute_specifier` node attached to the symbol's
declaration. Tree-sitter-c exposes these as
`__attribute__((name))` / `__attribute__((name(args)))`. We record
just the leading identifier — the argument list, if any, is dropped.
Multiple attributes accumulate in the list, in syntactic order.

Examples of what gets captured (one attribute spec, multiple
attributes):

| source                                          | `gcc_attributes` |
|-------------------------------------------------|------------------|
| `__attribute__((unused))`                       | `["unused"]`     |
| `__attribute__((warn_unused_result))`           | `["warn_unused_result"]` |
| `__attribute__((nonnull(1, 2)))`                | `["nonnull"]`    |
| `__attribute__((noreturn)) __attribute__((cold))` | `["noreturn", "cold"]` |
| `__attribute__((aligned(8), packed))`           | `["aligned", "packed"]` |

C23 `[[attribute]]`-style attributes are NOT captured by this column
in this revision; if/when tree-sitter-c exposes them as a separate
node kind they will follow the same extraction rule (drop arguments,
keep leading identifier). Until then they fall through to the
`symbol_attr` escape hatch with key `"c23_attr"`.

### Default policy on conflicts

If the same qualifier appears twice (`const const int x;` — legal
under C99's "duplicate qualifiers compress" rule), the column is set
to `true` exactly once. No multiset semantics.

## Worked examples

All examples sourced from
`../virgil-skills/benchmarks/c/embedded-sensors/`. The `symbol_id`
columns use the ADR-0002 format
`path|start_line|start_col|name|kind`. Spans are tree-sitter `Range`
of the symbol's declaration node.

### Example 1 — `static` file-scope variable

Source: `src/init.c:23`

```c
static int g_initialized = 0;
```

Symbol id: `src/init.c|23|11|g_initialized|variable`

The `start_col = 11` corresponds to the column of the *name*
`g_initialized` in the declaration, per the existing extractor's
behaviour (the symbol's name-node position).

`c_attrs` row:

| column           | value          |
|------------------|----------------|
| `symbol_id`      | `src/init.c\|23\|11\|g_initialized\|variable` |
| `is_file_static` | `true`         |
| `is_extern`      | `false`        |
| `is_inline`      | `false`        |
| `is_const`       | `false`        |
| `is_volatile`    | `false`        |
| `is_restrict`    | `false`        |
| `gcc_attributes` | `[]`           |

This is the canonical "file-local linkage" case. The same file has six
more `static` globals on lines 24-29; each produces an analogous row.

### Example 2 — variable with multiple type qualifiers (non-obvious)

Source: `include/types.h:59`

```c
typedef volatile uint32_t reg32_t;
```

The TYPEDEF symbol `reg32_t` is what gets a `c_attrs` row. Even though
`volatile` applies to the *type* that `reg32_t` aliases, our rule
above says `is_volatile` reflects a top-level qualifier on the
symbol's declaration — and the `volatile` keyword DOES sit at the
top level of this `type_definition` node.

Symbol id: `include/types.h|59|26|reg32_t|typedef`

`c_attrs` row:

| column           | value          |
|------------------|----------------|
| `symbol_id`      | `include/types.h\|59\|26\|reg32_t\|typedef`  |
| `is_file_static` | `false`        |
| `is_extern`      | `false`        |
| `is_inline`      | `false`        |
| `is_const`       | `false`        |
| `is_volatile`    | `true`         |
| `is_restrict`    | `false`        |
| `gcc_attributes` | `[]`           |

Non-obvious extraction: the `volatile` token in
`typedef volatile uint32_t reg32_t;` is parsed by tree-sitter-c as a
`type_qualifier` child of the `type_definition` node, sibling to the
`type_identifier`s `uint32_t` and `reg32_t`. The extractor sees that
sibling and flips `is_volatile`. The `display_name` in the `type`
table (`"volatile uint32_t"`) carries the same information at a
different layer.

The same source file (`include/types.h:60-61`) has two more
`volatile` typedefs (`reg16_t`, `reg8_t`); each gets an analogous
row.

### Example 3 — `static` storage-duration array

Source: `src/init.c:29`

```c
static char g_version[] = "0.8.3-dev";
```

Symbol id: `src/init.c|29|12|g_version|variable`

`c_attrs` row:

| column           | value          |
|------------------|----------------|
| `symbol_id`      | `src/init.c\|29\|12\|g_version\|variable`    |
| `is_file_static` | `true`         |
| `is_extern`      | `false`        |
| `is_inline`      | `false`        |
| `is_const`       | `false`        |
| `is_volatile`    | `false`        |
| `is_restrict`    | `false`        |
| `gcc_attributes` | `[]`           |

The fact that the variable has an array type (`char[]`) is recorded in
the `type` row, not here. `c_attrs` describes the SYMBOL's qualifiers,
not the shape of its type.

### Example 4 — externally-linked function (no qualifiers, baseline row)

Source: `src/utils/ringbuf.c:13-29`

```c
int ringbuf_init(ringbuf_t *rb, size_t capacity) {
    if (rb == NULL || capacity == 0) {
        return -1;
    }
    ...
}
```

Symbol id: `src/utils/ringbuf.c|13|4|ringbuf_init|function`

`c_attrs` row (the all-default baseline):

| column           | value          |
|------------------|----------------|
| `symbol_id`      | `src/utils/ringbuf.c\|13\|4\|ringbuf_init\|function`  |
| `is_file_static` | `false`        |
| `is_extern`      | `false`        |
| `is_inline`      | `false`        |
| `is_const`       | `false`        |
| `is_volatile`    | `false`        |
| `is_restrict`    | `false`        |
| `gcc_attributes` | `[]`           |

Even though the function has external linkage by default, `is_extern`
stays `false` — the column reflects the keyword's presence, not the
inferred linkage class. Query for "external-linkage functions" is
`is_file_static = false` (the `is_extern = true` form is rare in
practice and signals a forward declaration in a header).

The row is emitted unconditionally so that downstream joins can
filter without worrying about missing keys.

### Example 5 — `static` storage with initialized string (subtle)

Source: `src/config.c:18`

```c
static char g_config_path[256] = "/etc/sensorhub/config.ini";
```

Symbol id: `src/config.c|18|12|g_config_path|variable`

`c_attrs` row:

| column           | value          |
|------------------|----------------|
| `symbol_id`      | `src/config.c\|18\|12\|g_config_path\|variable`  |
| `is_file_static` | `true`         |
| `is_extern`      | `false`        |
| `is_inline`      | `false`        |
| `is_const`       | `false`        |
| `is_volatile`    | `false`        |
| `is_restrict`    | `false`        |
| `gcc_attributes` | `[]`           |

`g_config_path` is `static char[256]`, not `const`. The string literal
on the RHS is `const` at the source level but the destination array
is mutable. Audits that look for "writable hardcoded paths" key on
`is_file_static = true AND is_const = false AND` the variable's `type`
having `display_name` starts-with `array<char`.

### Example 6 — synthetic `inline` + `__attribute__` (illustrative)

The benchmark corpus has no `inline` functions and no
`__attribute__((...))` annotations. To pin the contract for those
cases, here is a synthesised example that the extractor must handle
identically when a future workspace includes it:

```c
static inline __attribute__((always_inline))
uint32_t crc32_step(uint32_t crc, uint8_t byte) {
    return (crc >> 8) ^ table[(crc ^ byte) & 0xff];
}
```

Symbol id (hypothetical path/line): `src/utils/crc.c|10|17|crc32_step|function`

`c_attrs` row:

| column           | value                  |
|------------------|------------------------|
| `symbol_id`      | `src/utils/crc.c\|10\|17\|crc32_step\|function` |
| `is_file_static` | `true`                 |
| `is_extern`      | `false`                |
| `is_inline`      | `true`                 |
| `is_const`       | `false`                |
| `is_volatile`    | `false`                |
| `is_restrict`    | `false`                |
| `gcc_attributes` | `["always_inline"]`    |

This row is illustrative, not produced by the current benchmark. When
the benchmark adds `inline`/`__attribute__` cases, the implementation
is expected to produce a row of this exact shape.
