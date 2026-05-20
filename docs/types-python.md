# Types — Python

This document is the contract for how Python type expressions map to the `type`
relation in the virgil-cli Datalog schema. See
[`virgil-datalog-schema.md`](virgil-datalog-schema.md) for the relation shape
and [ADR-0003](adr/0003-level-3-types-and-references.md) for the Level-3
commitment.

Python types come from PEP 484/585/604 annotations (`x: int`, `def f() -> str`,
`y: list[int]`, `z: int | None`). They appear on:

- function parameters (`parameter.type_id`)
- function return values (`returns_type.type_id`)
- variable annotations (`x: T = ...`) — **not modelled in Phase 1**; only
  parameter and return annotations produce `type` rows.
- raised exceptions (`raise X` / `raises X` in docstrings) — **not modelled**;
  the `throws` relation stays empty for Python.

Duck-typed assignments (`x = []`, `x = some_call()`) do **not** generate
`type` rows. Only **explicit** PEP 484 annotations populate the relation.
Inferred types are out of scope for the extractor.

## Tree-sitter node kinds

The `tree-sitter-python` grammar exposes a single named node `type` that wraps
every type expression. Its first child determines the schema `kind`:

| tree-sitter node (inside `type`) | source example                | schema `kind` |
|----------------------------------|-------------------------------|---------------|
| `identifier`                     | `int`, `str`, `Product`, `T`  | `primitive` or `named` (see below) |
| `none`                           | `None` (in a type position)   | `primitive`   |
| `generic_type`                   | `list[int]`, `Optional[X]`    | `generic` (with subkind override for `Optional`/`Union`/`Callable`) |
| `subscript`                      | `dict[str, int]` when the grammar emits `subscript` instead of `generic_type` (older grammars) | `generic` |
| `binary_operator` with `\|`      | `int \| None`, `X \| Y \| Z`  | `union`       |
| `tuple`                          | bare `(int, str)` inside `Tuple[(int, str)]` (rare; usually nested inside `generic_type`) | `tuple` |
| `string`                         | `"User"` (forward reference)  | `named` (the string contents are the display name; quotes stripped) |
| `attribute`                      | `typing.Optional`, `mod.T`    | `named`       |
| `member_type` / `attribute` chain inside generic args | `dict[str, models.Product]` | recursed into; each arg gets its own row |

### `primitive` vs `named` for bare identifiers

A bare `identifier` inside a `type` node maps to:

- `kind = "primitive"` if the name is one of:
  `int`, `float`, `bool`, `str`, `bytes`, `bytearray`, `complex`, `None`,
  `object`, `Any`, `NoneType`.
- `kind = "named"` otherwise (`Product`, `User`, `T`, `Optional` used bare).

`Any` is treated as `primitive` deliberately — it is a sentinel from the
`typing` module, but querying for "untyped" parameters is the dominant use
case and unifying it with `primitive` keeps that query simple.

### `generic_type` subkind override

A `generic_type` node has shape `<base> [ <args> ]`. The base is an
`identifier` (or `attribute`); the args are a comma-separated list of `type`
nodes inside a `type_parameter` wrapper.

The base determines whether we override `kind = "generic"`:

| base name (after stripping `typing.` prefix) | `kind`         |
|---------------------------------------------|----------------|
| `Optional`                                  | `union`        |
| `Union`                                     | `union`        |
| `Callable`                                  | `function`     |
| `Tuple` / `tuple` with an explicit arg list | `tuple`        |
| `Literal`                                   | `named`        |
| `Type` / `type`                             | `generic`      |
| everything else (`List`, `list`, `Dict`, `dict`, `Set`, `set`, `Iterable`, `Mapping`, …) | `generic` |

Rationale: `Optional[X]` is `Union[X, None]` semantically — it should match
the same `union` queries as `X | None`. `Callable[[A, B], R]` describes a
function signature and gets its own `kind`. `Literal["a", "b"]` collapses to
`named` because its arguments are not types but value tokens.

### `binary_operator` with `|`

PEP 604 syntax (`int | None`, `A | B | C`) parses as a left-associated
chain of `binary_operator` nodes. We flatten the chain into a single `union`
type row whose `display_name` is `"A | B | C"` (no parens, single space
around `|`).

## `display_name` construction

Built by walking the AST and emitting the exact source text after
normalization:

1. Strip all whitespace between tokens, then re-insert one space:
   - after `,`
   - on either side of `|`
   - **never** inside `[ ... ]` or `( ... )` (so `list[int]`, not `list[ int ]`).
2. Strip surrounding quotes from string forward references: `"User"` →
   `User`.
3. Drop the `typing.` prefix from well-known names so `typing.Optional[int]`
   and `Optional[int]` produce the same `display_name`: `Optional[int]`.
   This applies only to names directly importable from `typing` —
   `mymodule.Optional` keeps the prefix.
4. Render generic args in source order. Do not reorder `Union[X, Y]` vs
   `Union[Y, X]` — they get different ids.
5. Trailing commas inside generic args (`Tuple[int, str,]`) are dropped.

Examples:

| source                            | `display_name`            |
|-----------------------------------|---------------------------|
| `int`                             | `int`                     |
| `Optional[ int ]`                 | `Optional[int]`           |
| `int \| None`                     | `int \| None`             |
| `list[ int ]`                     | `list[int]`               |
| `dict[str ,int]`                  | `dict[str, int]`          |
| `typing.Callable[[int, str], bool]` | `Callable[[int, str], bool]` |
| `"User"`                          | `User`                    |
| `Literal["a", "b"]`               | `Literal["a", "b"]`       |

## `canonical_name` resolution

Canonicalization fills `type.canonical_name` when the type is resolvable to a
named, importable entity. Algorithm:

1. **Primitive types** — `int`, `str`, `bool`, …: `canonical_name = "<builtin>.<name>"`.
   For example `int` → `<builtin>.int`. `None`/`NoneType` → `<builtin>.None`.
   `Any` → `<typing>.Any`.

2. **Imported names** — if the identifier matches a local name brought in by
   `from <mod> import <name>` or `import <mod>`, resolve via the existing
   `imports` rows for the file. `canonical_name` is `<resolved_module>.<name>`
   where `<resolved_module>` is the dotted module path. Relative imports are
   resolved by `resolve_import` in `src/languages/python/queries.rs`.

3. **Same-file names** — if the identifier matches a top-level
   `class`/`function`/variable defined in the same file, `canonical_name` is
   `<module_path>.<name>` (the module path is the file path with `/` →
   `.` and `.py` stripped, e.g. `app/models.py` → `app.models`).

4. **`typing.` names** — `Optional`, `Union`, `Callable`, `List`, `Dict`,
   `Tuple`, `Literal`, `Any`, `Iterable`, `Mapping`, `Sequence`, etc.,
   resolve to `<typing>.<name>` even if not explicitly imported in the
   file. This matches what runtime Python does via the `typing` module.

5. **Generic / Optional / Union / Callable / Tuple wrappers** — the wrapper
   row's `canonical_name` is the wrapper's own canonical (e.g.
   `<typing>.Optional`). Each argument is a **separate `type` row** with its
   own canonical resolution; the wrapper row references them only by
   `display_name` text.

6. **Type aliases** — Python's `Foo = list[int]` and PEP 695
   `type Foo = list[int]` are **not** transparently dereferenced. A use of
   `Foo` resolves to `<module>.Foo`; we do not chase the alias to
   `list[int]`. Aliases stay as `named` types pointing at their definition
   site.

7. **Type parameters** (`T`, `K`, `V` declared via `TypeVar` or PEP 695
   `def f[T](x: T)`) — `canonical_name = null`. They cannot be canonicalized
   across files.

8. **Forward references** (`"User"` strings) — resolved exactly as if the
   string contents were the bare identifier (rules 2–4).

9. **Unresolved names** — anything not matched by the above (external library
   name we have not indexed, typo, dynamic import): `canonical_name = null`.

## Identity

Per ADR-0003: `type.id = blake3("python" | "|" | file_id | "|" | display_name)`.

`file_id` is the file path (ADR-0002 — `file.id` is the path itself).
The hash inputs are pipe-separated. Identical `display_name`s in different
files produce different `type.id`s, which is intentional: cross-file
joining goes through `canonical_name`.

`display_name` is normalized per the rules above **before** hashing.

## Field types — `field_type` relation

Per the schema, every Python class-level attribute with a PEP-526
annotation (`class Foo: x: int`, including dataclass fields and
typed `__init__` self-assignments where the field is also declared
at class scope) emits a `field_type {symbol_id, type_id}` row
linking the field symbol to its `type` row. Untyped attributes
(`self.x = 0` with no class-level annotation) emit no row — the
field has no `type` to point at. Local variables and function
parameters use `parameter` / `references` wiring instead.

## Worked examples

All paths below are relative to
`virgil-skills/benchmarks/python/technical-debt/`.

### Example 1 — `primitive` and `named`, return annotation

`app/views.py:462`:

```python
def render_filtered_products(session, category: str, sort_by: str = "name") -> dict:
```

`type` rows (file_id = `app/views.py`):

| id (blake3 abbrev) | kind        | language | display_name | canonical_name      |
|--------------------|-------------|----------|--------------|---------------------|
| `t:str@views`      | `primitive` | python   | `str`        | `<builtin>.str`     |
| `t:dict@views`     | `named`     | python   | `dict`       | `<builtin>.dict`    |

`parameter` rows:

| function_id (ADR-0002)                            | index | name       | type_id        | is_optional | has_default |
|---------------------------------------------------|-------|------------|----------------|-------------|-------------|
| `app/views.py\|462\|0\|render_filtered_products\|function` | 0 | `session`  | `null`         | false       | false       |
| same                                              | 1     | `category` | `t:str@views`  | false       | false       |
| same                                              | 2     | `sort_by`  | `t:str@views`  | false       | true        |

`returns_type` row: `function_id → t:dict@views`.

Note `session` has `type_id = null` because it is unannotated. `dict` is
classified `named` (not `primitive`) because it admits generic parameters;
the primitive set is restricted to scalar value types.

### Example 2 — `generic`, two type arguments

`app/serializers.py:350`:

```python
def serialize_dashboard_data(stats: dict[str, Any], alerts: dict[str, Any]) -> dict[str, Any]:
```

`type` rows (file_id = `app/serializers.py`):

| id            | kind        | display_name      | canonical_name      |
|---------------|-------------|-------------------|---------------------|
| `t:str@ser`   | `primitive` | `str`             | `<builtin>.str`     |
| `t:Any@ser`   | `primitive` | `Any`             | `<typing>.Any`      |
| `t:dictSA@ser`| `generic`   | `dict[str, Any]`  | `<builtin>.dict`    |

Three rows total; the two `dict[str, Any]` occurrences (params + return)
collapse to one row because dedup is per `(language, file_id, display_name)`.

`parameter` rows: indices 0 and 1 both reference `t:dictSA@ser`.
`returns_type` row: → `t:dictSA@ser`.

The wrapper's `canonical_name` is the base type (`<builtin>.dict`), not the
parameterised form — args live in `display_name` and as their own rows.

### Example 3 — `union` via `Optional`

`app/models.py:16` imports `from typing import Optional`. Hypothetical use
at `app/models.py:53` would be:

```python
def __init__(self, id: Optional[int] = None):
```

`type` rows (file_id = `app/models.py`):

| id              | kind        | display_name    | canonical_name      |
|-----------------|-------------|-----------------|---------------------|
| `t:int@models`  | `primitive` | `int`           | `<builtin>.int`     |
| `t:Optint@models` | `union`   | `Optional[int]` | `<typing>.Optional` |

The wrapper kind is `union` (not `generic`) because the base is `Optional`.
The `None` arg is implicit — we do not synthesize a `<builtin>.None` row
just because `Optional` semantically expands to `Union[X, None]`. We model
what the source wrote.

`parameter` row for `id`: `type_id = t:Optint@models`, `is_optional = true`
(because the type is `Optional[...]`), `has_default = true`.

> Ambiguity resolution: `is_optional = true` is set when **either** the
> annotation is `Optional[...]` / `... | None` **or** the default value is
> `None`. A non-`None` default (`x: int = 0`) leaves `is_optional = false`
> with `has_default = true`.

### Example 4 — `function` via `Callable`

If `app/utils.py:610` were annotated:

```python
def timer(func: Callable[[int, str], bool]) -> Callable[..., Any]:
```

`type` rows (file_id = `app/utils.py`):

| id             | kind        | display_name                       | canonical_name      |
|----------------|-------------|------------------------------------|---------------------|
| `t:int@utils`  | `primitive` | `int`                              | `<builtin>.int`     |
| `t:str@utils`  | `primitive` | `str`                              | `<builtin>.str`     |
| `t:bool@utils` | `primitive` | `bool`                             | `<builtin>.bool`    |
| `t:Any@utils`  | `primitive` | `Any`                              | `<typing>.Any`      |
| `t:cb1@utils`  | `function`  | `Callable[[int, str], bool]`       | `<typing>.Callable` |
| `t:cb2@utils`  | `function`  | `Callable[..., Any]`               | `<typing>.Callable` |

`Callable[..., R]` (the `...` literal as args) is allowed: `display_name`
preserves the `...`. Arg rows are not generated for the `...` — only for
explicit arg type expressions.

### Example 5 — PEP 604 union with `None`

If `app/services.py:592` were re-annotated for accuracy:

```python
def get_order_status_label(order_id: int) -> str | None:
```

`type` rows (file_id = `app/services.py`):

| id                | kind        | display_name | canonical_name        |
|-------------------|-------------|--------------|-----------------------|
| `t:int@svc`       | `primitive` | `int`        | `<builtin>.int`       |
| `t:str@svc`       | `primitive` | `str`        | `<builtin>.str`       |
| `t:None@svc`      | `primitive` | `None`       | `<builtin>.None`      |
| `t:strNone@svc`   | `union`     | `str \| None` | `null`               |

For PEP 604 unions, `canonical_name` is `null` (there is no `typing.X`
backing name — it is built-in syntax). The wrapper's args (`str`, `None`)
each get their own row with their own canonical.

> Distinction: `Optional[str]` resolves the wrapper to `<typing>.Optional`;
> `str | None` resolves the wrapper to `null`. Queries that want "nullable"
> types should match on `kind = "union"` plus arg structure, not on
> `canonical_name`.

### Example 6 — `Any`, all-`Any` signature

`app/utils.py:701`:

```python
def transform_record(record: Any, mapping: Any) -> Any:
```

`type` rows (file_id = `app/utils.py`):

| id            | kind        | display_name | canonical_name  |
|---------------|-------------|--------------|-----------------|
| `t:Any@utils` | `primitive` | `Any`        | `<typing>.Any`  |

One row, three references (two parameters + one return). The dedup-per-file
rule means we emit a single row even for an `Any`-only signature.

### Example 7 — unannotated function (negative case)

`app/utils.py:631`:

```python
def chunks(lst, n):
    for i in range(0, len(lst), n):
        yield lst[i:i + n]
```

No `type` rows are emitted for this function. Two `parameter` rows are
emitted with `type_id = null` (one for `lst`, one for `n`). No
`returns_type` row is emitted at all (we do not synthesize an implicit
`None` return for unannotated functions).

> Ambiguity resolution: missing return annotation → no `returns_type` row.
> Annotation of `-> None` → one `returns_type` row pointing at a
> `primitive` `None` type. These two cases must remain distinguishable.
