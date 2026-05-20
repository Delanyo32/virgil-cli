# Per-language `*_attrs` tables across 9 languages (parity)

**Label:** enhancement
**Type:** AFK

## What to build

Create and populate the 9 per-language attribute relations defined in `docs/attrs-<lang>.md`:

- `rust_attrs` — `is_unsafe`, `is_const`, `derives`, etc.
- `python_attrs` — `decorators`, `is_generator`, `is_coroutine`, `docstring_style`
- `typescript_attrs` — `is_readonly`, `is_optional`, `type_parameters`
- `cpp_attrs` — `is_virtual`, `is_const`, `is_noexcept`, `is_template`, `is_constexpr`, `is_override`, `is_final`
- `csharp_attrs` — `attributes`, `is_partial`, `is_sealed`, etc.
- `go_attrs` — `is_exported`, `has_receiver`, `build_tags`
- `php_attrs` — `is_final`, `uses_traits`, `attributes`
- `c_attrs` — `is_file_static`, `is_extern`, `is_inline`, `is_const`, `is_volatile`, `is_restrict`, `gcc_attributes`
- `java_attrs` — `annotations`, `is_final`, `is_synchronized`, `throws_clause`

Per contract review policy 4: no `*_attrs` column duplicates a `symbol` column. Exception: Java's `throws_clause` (raw textual) is allowed alongside the resolved `throws` relation.

Dispatch one subagent per language with the attrs contract doc + benchmark corpus.

## Acceptance criteria

- [ ] All 9 `*_attrs` relations declared in the schema (additive — no `SCHEMA_VERSION` bump if structured per ADR-0003 cache policy; otherwise per the per-phase bump rule)
- [ ] Per-language extractors populate the columns per their contract doc
- [ ] No `*_attrs` column shadows `is_async`/`is_static`/`is_abstract`/`is_mutable` on `symbol`
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/attrs.cozoql` validate expected rows
- [ ] `cargo test` passes

## Blocked by

- #12
