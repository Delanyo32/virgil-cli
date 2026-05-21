# References — C

This contract describes how every identifier occurrence in a C source
file maps to a row in the `references` relation (per
`docs/virgil-datalog-schema.md`) and how `referent_id` is resolved
(Level 3 per ADR-0003).

`references` is keyed by `(referrer_id, site_file, site_start_byte, match_index)` with `referent_id` and `ref_kind` in the value position. `match_index = 0` for the primary/only candidate; C has no overload resolution, so every C row uses `match_index = 0` in practice. Unresolvable referents emit a single row with `referent_id = null` (per `docs/contract-review.md`, policy 1).

## Lexical scope rules

C has four kinds of scope. Lookup walks outward through them in this
order; the first hit wins.

1. **Block scope** — anything declared between `{` and `}` (function
   bodies, `for`/`while`/`if` blocks, compound statements). Nested
   blocks extend the enclosing block's scope; later declarations in the
   same block shadow earlier ones IF the names differ in declaration
   position (a standard-illegal redeclaration is treated as the later
   binding winning, with no diagnostic — we are not a type-checker).
2. **Function scope** — parameters of the enclosing
   `function_definition`. Labels (targets of `goto`) live here too; the
   extractor does not emit references for labels.
3. **File scope** — top-level declarations in the current translation
   unit. Includes `typedef`s, `struct`/`union`/`enum` tags,
   file-scope variables, file-scope functions, and `#define` macros.
   A `static` modifier limits *linkage*, not *scope*, so a `static`
   file-scope symbol is still visible to every site in the same file.
4. **External linkage** — symbols declared with `extern`, or default-
   linkage symbols (non-`static` file-scope functions and variables)
   in any other translation unit the current file `#include`s,
   transitively. We resolve through the `imports` relation (see
   "`referent_id` resolution" below).

C has no module-qualified names (`a::b::c`) — references are always
bare identifiers, field accesses, or member-pointer accesses. The
extractor treats `s.field` and `p->field` specially (see `read`/`write`
below).

### Shadowing

A block-scope binding shadows an outer-scope binding with the same
name. The extractor records the reference against the innermost binding
visible at the use site's `start_byte`. Shadowing across linkage
boundaries (e.g. a local variable named `g_initialized` inside
`sensorhub_init`) is resolved purely by lexical containment — no
attempt is made to warn or skip the row.

### `struct` member tags

Struct/union/enum tag names live in a SEPARATE namespace from ordinary
identifiers in standard C. The extractor maintains a single name index
keyed by `(name, kind)`. A reference to `struct ringbuf_t` resolves
against the `struct_specifier` whose name is `ringbuf_t`; a reference
to the typedef `ringbuf_t` (no `struct` keyword) resolves against the
`type_definition`. If both exist, the keyword on the reference
disambiguates.

## `ref_kind` decision tree

Every `references` row gets exactly one `ref_kind`. The decision is
based on the AST pattern of the use site.

### `read`

An identifier is *evaluated* (its value is used). Emit `read` for every
occurrence of an identifier in these positions:

- RHS of any assignment: `x = y;` emits `read` for `y` (and `write`
  for `x`; see below).
- Operand of any binary/unary expression: `y + 1`, `!flag`, `*p`.
- Argument to a function call: `foo(x, y)` emits `read` for `x` and
  `y` (and depending on the resolver state, a `read` for `foo` itself
  — see "function names" below).
- Inside a return statement: `return result;` → `read` for `result`.
- Inside any condition: `if (g_initialized)` → `read` for
  `g_initialized`.
- Pointer dereferencing reads: `*p` emits `read` for `p`. The
  dereference *expression* itself is not a separate row — only the
  base identifier.
- Field reads: `s.x` and `p->x` emit:
  - `read` for the base (`s` or `p`).
  - `read` for the field (`x`) **only** when the field has a known
    `symbol_id` in the store (per `docs/contract-review.md`,
    policy 5). C struct/union members extracted as `field` symbols
    qualify; fields whose containing type is anonymous or not
    extracted produce no field-level row.
- Indexed reads: `arr[i]` emits `read` for both `arr` and `i`.
- The condition and step of `for`/`while`/`do`/`if`/`switch`.
- Inside `sizeof(x)` when `x` is an expression (not a type).
- Function names in call positions: `foo()` emits a `read` for `foo`
  with `ref_kind = "read"`. This is in ADDITION to the `calls` row
  the existing call-graph extractor produces — the two relations serve
  different queries. (The `calls` extractor is unchanged; this contract
  only governs `references`.)

#### Exceptions — do NOT emit a `read`

- Identifiers inside `#define` bodies (`preproc_def`,
  `preproc_function_def`) are not analysed. The macro body is treated
  as opaque text; identifiers used at the macro's call site are
  resolved at the call site (without macro expansion).
- Identifiers inside `#if`/`#ifdef`/`#ifndef`/`#elif` directives are
  not emitted as references.
- Identifiers used as struct/union/enum tag *names* in a declaration
  position (`struct ringbuf_t { ... }`) — that's the binding site, not
  a use.
- The declarator name of any declaration (`int x;` does not emit a
  `read` for `x` — it's a binding).

### `write`

An identifier is *assigned to* or its storage is *mutated*. Emit
`write` for:

- LHS of any assignment operator: `x = y;`, `x += 1;`, `x %= n;`.
  Compound assignment (`+=`, `-=`, etc.) emits a single `write`
  row at the LHS site, no separate `read`. Updated per
  `docs/contract-review.md`: faithful read+write semantics is
  Level 4; Level 3 records only the dominant kind. The RHS reads
  are emitted separately as normal.
- Increment/decrement: `i++`, `--n`, `++p`, `n--`. These produce one
  `write` (no separate `read` — same single-write rule).
- Assignment through a pointer dereference: `*p = x;` emits `write`
  for `p` (the pointer's pointee is being written, so the pointer is
  the symbol affected). The dereferenced value site does NOT also get
  a `read` for `p` — it gets exactly one `write`.
- Field writes: `s.x = 1;` emits `write` for `s`. A `write` row for
  the field `x` is emitted **only** when `x` has a known
  `symbol_id` (per policy 5 above). `p->x = 1;` emits `write` for
  `p` (the pointer is being used to mutate state) and a `write` row
  for `x` under the same field-symbol condition.
- Address-of in a context that allows mutation: `&x` ALONE emits
  `read` (taking the address doesn't mutate), but `scanf("%d", &x);`
  is treated structurally as a function call — `&x` produces a `read`
  for `x`. We do NOT pattern-match function names to infer
  pass-by-reference writes.
- Indexed writes: `arr[i] = v;` emits `write` for `arr`, `read` for
  `i`, `read` for `v`.

### `type_use`

An identifier appears in a position that names a type. Emit `type_use`
for:

- The type specifier of any declaration: in `int x;` emit `type_use`
  for `int` (well, `int` is a keyword — see below); in
  `sensor_id_t id;` emit `type_use` for `sensor_id_t`.
- The type in a parameter declaration: `int foo(sensor_id_t id)` →
  `type_use` for `sensor_id_t`.
- The return type of a function: `sensor_reading_t *foo()` →
  `type_use` for `sensor_reading_t`.
- The target of a cast: `(int16_t)x` → `type_use` for `int16_t`.
- The argument to `sizeof(T)` when `T` is a type, not an expression.
- The struct/union/enum keyword tag in a type position:
  `struct ringbuf_t *rb` emits `type_use` for `ringbuf_t` (one row;
  the keyword does not produce its own ref).
- Inside `typedef T new_name;` emit `type_use` for `T`.

Built-in type keywords (`int`, `char`, `float`, `void`, `_Bool`,
`signed`, `unsigned`, `short`, `long`, `volatile`, `const`,
`restrict`) are NOT emitted as `references` rows — they don't name
user-defined symbols. The corresponding `type` row in the `type`
relation still carries them in `display_name`.

`type_use` rows tie to the `type` rows emitted per `docs/types-c.md`:
when an extractor emits a `type` row for `sensor_reading_t`, the same
identifier occurrence also produces a `references` row with
`ref_kind = "type_use"` and `referent_id =` the symbol id of the
typedef declaration (the same id stored in `type.canonical_name`).

### `import_use`

An identifier appears inside a `preproc_include` directive. C is
unusual here — `#include` references a *file path*, not a symbol. We
emit:

- One `import_use` row per `preproc_include` whose `path` resolves to
  a known file in the workspace (via `resolve_import` in
  `src/languages/c_lang/queries.rs`).
- `referrer_id` is the symbol id of the *file itself*, i.e. the
  synthetic symbol for the `.c`/`.h` declaring the include.
- `referent_id` is the file id of the resolved target. C does not
  expose individual imported symbols at the directive level — every
  external linkage symbol from the included header is brought in
  wholesale, and per-identifier resolution happens at the use site
  via the cross-file walk (see below).
- `site_file` = current file; `site_start_byte` = the `preproc_include`
  node's start byte.

If the include cannot be resolved (system header, missing file), the
row is still emitted with `referent_id = null` (updated per
`docs/contract-review.md` policy 1: the schema now allows nullable
`referent_id` in value position). Audits that want only resolved
includes filter `referent_id IS NOT NULL`.

## `referent_id` resolution

Given an identifier occurrence at `(file, byte_offset, name)`, the
resolver walks scopes in this order and stops at the first match.

1. **Innermost block** containing `byte_offset` — search local
   declarations in declaration order, latest first.
2. **Enclosing function** — function parameters, then any block scopes
   strictly containing the use site (already covered by step 1 if the
   walker is structured recursively).
3. **File scope** of the current file — all top-level symbols emitted
   by `src/languages/c_lang/queries.rs` for this file (functions,
   variables, typedefs, structs/unions/enums, macros).
4. **Transitively included files** — every file reachable from the
   current file through the `imports` relation (BFS, depth cap 5).
   Within each included file, search only the file-scope symbols whose
   linkage is external (i.e. NOT `static`). The walk visits files in
   the order include directives appear in the current file.
5. **Unresolved** — emit the row with `referent_id = null`. We DO emit
   rows for unresolved references because Cozoscript queries need to
   distinguish "we saw `printf` here but couldn't bind it" from "this
   identifier never appeared".

### Multiple candidates

If two file-scope candidates have the same `name` and `kind`
(legitimate when a `static` function in `foo.c` and a different
`static` function in `bar.c` share a name), the resolver picks the one
in the file closest to the use site:

1. Same file wins.
2. Otherwise, the included file whose path matches first in the
   topological order of `imports` wins.
3. If still ambiguous, the row gets `referent_id = null` (no silent
   wrong-arrow). Disambiguation by header path is documented as a
   future improvement; the contract today is "ambiguous → unresolved".

If two candidates differ in `kind` (e.g. a struct tag `ringbuf_t` and
a typedef `ringbuf_t`), the use-site context disambiguates: a `struct`
keyword adjacent to the use selects the struct tag; a bare use selects
the typedef. This is what the C namespace rule already requires.

### Resolver state

The resolver builds a per-file scope tree on demand (parsing the
file's tree-sitter `Tree` once, materialising block scopes from
`compound_statement` nodes). It REUSES the existing `symbols_by_name`
index in `src/graph/builder.rs` for steps 3 and 4 — that index already
keys file-scope symbols by name, which matches what the C scope walk
needs. The per-file scope tree handles only block- and function-scope
bindings; the global index handles file-scope and included-file
bindings.

## Worked examples

All snippets quoted from
`../virgil-skills/benchmarks/c/embedded-sensors/`. Each example lists
the `references` rows that should be emitted; the symbol ids in
`referrer_id`/`referent_id` use the ADR-0002 format
`path|start_line|start_col|name|kind`. Where the row's referrer is
the enclosing function, we use that function's symbol id; where the
referrer is a file-scope identifier in an initializer, we use the
declaring symbol's id.

### Example 1 — `read` and `write` to file-scope globals

Source: `src/init.c:42-50`

```c
    if (g_initialized) {
        return STATUS_OK;
    }

    /* Clear global state */
    memset(g_readings, 0, sizeof(g_readings));
    memset(g_configs, 0, sizeof(g_configs));
    memset(g_callbacks, 0, sizeof(g_callbacks));
    g_error_count = 0;
```

Referrer for every row below: the enclosing `sensorhub_init` symbol
(`src/init.c|34|4|sensorhub_init|function`).

| ref_kind | name           | referent_id                                            | site_file     | site_start_byte |
|----------|----------------|--------------------------------------------------------|---------------|-----------------|
| `read`   | `g_initialized` | `src/init.c\|23\|11\|g_initialized\|variable`        | `src/init.c`  | (byte of `g_initialized` on line 42) |
| `read`   | `STATUS_OK`     | `include/types.h\|14\|4\|STATUS_OK\|variable` (enumerator constant) | `src/init.c` | (byte on line 43) |
| `read`   | `memset`        | `null` (libc, not indexed)                             | `src/init.c`  | (byte on line 47) |
| `read`   | `g_readings`    | `src/init.c\|25\|24\|g_readings\|variable`           | `src/init.c`  | (byte on line 47) |
| `read`   | `g_configs`     | `src/init.c\|26\|24\|g_configs\|variable`            | `src/init.c`  | (byte on line 48) |
| `read`   | `g_callbacks`   | `src/init.c\|27\|25\|g_callbacks\|variable`          | `src/init.c`  | (byte on line 49) |
| `write`  | `g_error_count` | `src/init.c\|24\|11\|g_error_count\|variable`        | `src/init.c`  | (byte on line 50) |

Notes:
- `sizeof(g_readings)` etc. produce one `read` per identifier inside
  the `sizeof` expression (since these are EXPRESSIONS, not types).
  Three additional `read`s are emitted for `g_readings`, `g_configs`,
  `g_callbacks` inside the three `sizeof` calls. Omitted from the
  table for brevity but required by the contract.
- `memset` resolves to `null` because `<string.h>` is not in the
  workspace. The row is still emitted.

### Example 2 — pointer dereference (read) and pointer-written-through

Source: `src/utils/ringbuf.c:67-69`

```c
    *byte = rb->buffer[rb->tail];
    rb->tail = (rb->tail + 1) % rb->capacity;
    return 0;
```

Referrer: `src/utils/ringbuf.c|58|4|ringbuf_get|function`.

Line 67 (`*byte = rb->buffer[rb->tail];`):

| ref_kind | name      | referent_id                                                              | notes |
|----------|-----------|--------------------------------------------------------------------------|-------|
| `write`  | `byte`    | `src/utils/ringbuf.c\|58\|33\|byte\|parameter`                          | LHS is `*byte` — pointer is written through |
| `read`   | `rb`      | `src/utils/ringbuf.c\|58\|22\|rb\|parameter`                            | base of `rb->buffer` |
| `read`   | `buffer`  | `include/utils/ringbuf.h\|13\|13\|buffer\|field`                        | field via `->`; field referent resolves through `rb`'s type `ringbuf_t *` |
| `read`   | `rb`      | `src/utils/ringbuf.c\|58\|22\|rb\|parameter`                            | base of `rb->tail` (index expression) |
| `read`   | `tail`    | `include/utils/ringbuf.h\|16\|20\|tail\|field`                          | field via `->` |

Line 68 (`rb->tail = (rb->tail + 1) % rb->capacity;`):

| ref_kind | name       | referent_id                                                  | notes |
|----------|------------|--------------------------------------------------------------|-------|
| `write`  | `rb`       | `src/utils/ringbuf.c\|58\|22\|rb\|parameter`                | LHS is `rb->tail = ...` — pointer used to mutate state |
| `write`  | `tail`     | `include/utils/ringbuf.h\|16\|20\|tail\|field`              | field being written |
| `read`   | `rb`       | …                                                            | RHS `rb->tail` |
| `read`   | `tail`     | …                                                            | RHS field |
| `read`   | `rb`       | …                                                            | RHS `rb->capacity` |
| `read`   | `capacity` | `include/utils/ringbuf.h\|14\|13\|capacity\|field`          | RHS field |

The "pointer field write also emits write on the base pointer" rule is
the most subtle decision in this contract — it lets taint analysis
follow the pointer back to its source. Audits that only care about
field-level writes filter on `ref_kind = "write"` AND
`referent_id.kind = "field"`.

### Example 3 — shadowing inside a nested block

Source: `src/config.c:42-57`

```c
                if (strcmp(key, "sample_rate") == 0) {
                    int rate = atoi(value);
                    if (rate > 0) {
                        if (rate <= 10000) {
                            if (rate >= 1) {
                                /* Apply sample rate to all sensors */
                                sensor_config_t cfg;
                                cfg.sample_rate_hz = rate;
                                cfg.averaging_count = 4;
                                cfg.min_threshold = -1000.0f;
                                cfg.max_threshold = 1000.0f;
                                cfg.enabled = 1;
                                int i;
                                for (i = 0; i < SENSOR_MAX; i++) {
                                    sensorhub_configure_sensor(i, &cfg);
                                }
                            }
                        }
                    }
```

Two block-scope declarations: `int rate` at line 43 and `int i` at line
54. Both shadow any same-named file-scope binding (there are none in
this file, but the resolver's behaviour is the same).

Sample rows (referrer = `src/config.c|21|4|config_load_from_file|function`):

| line | ref_kind | name                  | referent                                                                                |
|------|----------|-----------------------|-----------------------------------------------------------------------------------------|
| 43   | `read`   | `atoi`                | `null` (libc)                                                                           |
| 43   | `read`   | `value`               | `src/config.c\|25\|9\|value\|variable` (block-scope from outer scope, line 25)         |
| 44   | `read`   | `rate`                | `src/config.c\|43\|24\|rate\|variable` (the just-declared local, NOT any outer)        |
| 49   | `write`  | `cfg`                 | `src/config.c\|48\|32\|cfg\|variable`                                                  |
| 49   | `write`  | `sample_rate_hz`      | `include/types.h\|43\|13\|sample_rate_hz\|field`                                       |
| 49   | `read`   | `rate`                | `src/config.c\|43\|24\|rate\|variable`                                                 |
| 56   | `read`   | `sensorhub_configure_sensor` | `include/sensorhub.h\|25\|4\|sensorhub_configure_sensor\|function` (resolved via `#include "sensorhub.h"`) |
| 56   | `read`   | `i`                   | `src/config.c\|54\|36\|i\|variable`                                                    |
| 56   | `read`   | `cfg`                 | `src/config.c\|48\|32\|cfg\|variable` (the `&cfg` produces a `read` only)              |

If `config_load_from_file` had an outer parameter also named `rate`,
the line-44 row would still bind to the block-scope `rate` declared on
line 43 — innermost wins.

### Example 4 — `type_use` plus cross-file resolution

Source: `include/sensorhub.h:24-25`

```c
int sensorhub_read_sensor(sensor_id_t id, sensor_reading_t *reading);
int sensorhub_configure_sensor(sensor_id_t id, const sensor_config_t *cfg);
```

Each typedef name in a parameter type produces a `type_use` row. The
referrer is the function being declared (the prototype symbol).

Line 24, referrer = `include/sensorhub.h|24|4|sensorhub_read_sensor|function`:

| ref_kind   | name               | referent_id                                                  |
|------------|--------------------|--------------------------------------------------------------|
| `type_use` | `sensor_id_t`      | `include/types.h\|11\|17\|sensor_id_t\|typedef`             |
| `type_use` | `sensor_reading_t` | `include/types.h\|40\|2\|sensor_reading_t\|typedef`         |

The `int` return type emits no row (built-in keyword). Resolution
walks file scope (`include/sensorhub.h` itself has no matching
declarations) → included files (`#include "types.h"` at line 15) →
finds both typedefs.

Line 25, referrer = `include/sensorhub.h|25|4|sensorhub_configure_sensor|function`:

| ref_kind   | name               | referent_id                                                  |
|------------|--------------------|--------------------------------------------------------------|
| `type_use` | `sensor_id_t`      | `include/types.h\|11\|17\|sensor_id_t\|typedef`             |
| `type_use` | `sensor_config_t`  | `include/types.h\|48\|2\|sensor_config_t\|typedef`          |

The `const` qualifier and the pointer `*` are part of the `type` row
(`display_name = "ptr<const sensor_config_t>"`); only the
user-defined name (`sensor_config_t`) produces a `references` row.

### Example 5 — `import_use` and a `read` on an extern-linkage function

Source: `src/init.c:8-16`

```c
#include "sensorhub.h"
#include "config.h"
#include "types.h"
#include "drivers/gpio.h"
#include "drivers/spi.h"
#include "drivers/i2c.h"
#include "drivers/uart.h"
#include "protocol/modbus.h"
#include "protocol/mqtt.h"
```

Each line emits one `import_use` row.

| ref_kind     | referrer (the file)                                | referent (resolved file)             | site_start_byte |
|--------------|----------------------------------------------------|--------------------------------------|-----------------|
| `import_use` | `src/init.c\|1\|0\|init.c\|file`                  | `include/sensorhub.h`                | byte of line 8  |
| `import_use` | `src/init.c\|1\|0\|init.c\|file`                  | `include/config.h`                   | byte of line 9  |
| `import_use` | …                                                  | `include/types.h`                    | byte of line 10 |
| `import_use` | …                                                  | `include/drivers/gpio.h`             | byte of line 11 |
| `import_use` | …                                                  | `include/drivers/spi.h`              | byte of line 12 |
| `import_use` | …                                                  | `include/drivers/i2c.h`              | byte of line 13 |
| `import_use` | …                                                  | `include/drivers/uart.h`             | byte of line 14 |
| `import_use` | …                                                  | `include/protocol/modbus.h`          | byte of line 15 |
| `import_use` | …                                                  | `include/protocol/mqtt.h`            | byte of line 16 |

`<stdio.h>`, `<stdlib.h>`, `<string.h>` (lines 18-20) produce
`import_use` rows with `referent_id = null` (updated per
`docs/contract-review.md` policy 1: unresolved includes are emitted
as null rows, not skipped).

Now a `read` row that exercises cross-file resolution. Line 54 of the
same file:

```c
        result = gpio_init(10 + i, GPIO_MODE_OUTPUT, GPIO_PULL_NONE);
```

Referrer: `src/init.c|34|4|sensorhub_init|function`.

| ref_kind | name                | referent_id                                                  | resolution path |
|----------|---------------------|--------------------------------------------------------------|-----------------|
| `write`  | `result`            | `src/init.c\|35\|8\|result\|variable`                       | block-scope |
| `read`   | `gpio_init`         | `include/drivers/gpio.h\|<line>\|<col>\|gpio_init\|function`| cross-file via `imports` to `include/drivers/gpio.h` |
| `read`   | `i`                 | `src/init.c\|36\|8\|i\|variable`                            | block-scope |
| `read`   | `GPIO_MODE_OUTPUT`  | `include/drivers/gpio.h\|<line>\|<col>\|GPIO_MODE_OUTPUT\|macro` | cross-file (enum or macro depending on header content) |
| `read`   | `GPIO_PULL_NONE`    | `include/drivers/gpio.h\|<line>\|<col>\|GPIO_PULL_NONE\|macro` | cross-file |

`gpio_init` etc. produce both a `read` row (this contract) AND a
`calls` row (existing call-graph extractor). The two relations are
independent — queries that want call edges use `calls`; queries that
want every identifier occurrence use `references`.

### Example 6 — write to a non-local (file-scope global)

Source: `src/init.c:194-196`

```c
void sensorhub_clear_errors(void) {
    g_error_count = 0;
}
```

Referrer: `src/init.c|194|0|sensorhub_clear_errors|function`.

| ref_kind | name            | referent_id                                            |
|----------|-----------------|--------------------------------------------------------|
| `write`  | `g_error_count` | `src/init.c\|24\|11\|g_error_count\|variable`         |

The file-scope `static int g_error_count = 0;` at line 24 has
file-local linkage. The `sensorhub_clear_errors` function is in the
same file, so the lookup finds it at step 3 (file scope) of the
resolver walk. The `static` modifier does not affect resolution within
the file — only across files.

### Ambiguity note — `volatile` field reads

Source: `src/utils/ringbuf.c:106-109`

```c
    if (rb->head >= rb->tail) {
        return rb->head - rb->tail;
    }
```

The `head` and `tail` fields are declared `volatile` in the struct.
The `volatile` qualifier surfaces in the `type` row's `display_name`
(`"volatile size_t"`) but does NOT change `ref_kind`: every access
here is a `read`. A `volatile` *write* would still be `write` —
qualifiers don't change reference kinds.
