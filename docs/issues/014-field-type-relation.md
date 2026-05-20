# Populate `field_type` relation across all 9 languages

**Label:** enhancement
**Type:** AFK

## What to build

The `field_type {symbol_id => type_id}` relation (added in the contract reconciliation pass) links a field/property/struct-member symbol to its declared type. Populate it for every language that has typed fields:

- Rust struct fields
- Go struct fields
- Java/C# class fields
- TypeScript class fields and interface properties (none for `.js`)
- PHP typed properties (PHP 7+); untyped properties emit no row
- C/C++ struct members

Untyped fields (JS class fields, dynamic PHP properties) emit no row. Field symbols themselves are produced by the symbol extraction pass; this slice connects them to their `type_id`s using the resolution machinery from issue #13.

Parallelizable per language.

## Acceptance criteria

- [ ] Every typed field symbol has exactly one `field_type` row
- [ ] Untyped fields produce no `field_type` row (verify explicitly for Python class attributes without PEP 526 annotations, dynamic PHP properties, JS class fields)
- [ ] `type_id` references rows already in the `type` relation (populated by #13)
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/field-types.cozoql` validate expected rows
- [ ] `cargo test` passes

## Blocked by

- #13
