# Contract document template

Every language in `src/languages/<lang>/` has three contract documents under `docs/`:

- `docs/types-<lang>.md` — how that language's type expressions map to the `type` relation.
- `docs/references-<lang>.md` — how identifier occurrences map to the `references` relation and how `referent_id` is resolved.
- `docs/attrs-<lang>.md` — what populates the per-language `<lang>_attrs` table.

These docs are the **contract** a subagent works against when implementing the extractor for a phase. The schema being targeted lives in `docs/virgil-datalog-schema.md`. The id scheme lives in [ADR-0002](adr/0002-symbol-id-scheme.md). The Level-3 commitment lives in [ADR-0003](adr/0003-level-3-types-and-references.md).

Every doc must end with at least 5 worked examples drawn from `../virgil-skills/benchmarks/<lang>/`. Worked examples are the unambiguous done-criterion — a subagent is finished when the extractor produces the exact rows the doc commits to for those examples.

---

## `types-<lang>.md` skeleton

```md
# Types — <Language>

## Tree-sitter node kinds

List every tree-sitter node kind that can appear as a type expression in this language. For each:

- node kind name (e.g. `generic_type`, `reference_type`)
- what it represents in source
- the schema `kind` variant it maps to: one of `primitive`, `named`, `generic`, `union`, `intersection`, `function`, `tuple`, `array`

If a single node kind splits across multiple schema kinds depending on context, say so explicitly.

## `display_name` construction

How the textual `display_name` is built from the AST. State the exact rules: how whitespace is normalized, how generic arguments are rendered, how lifetimes/qualifiers are included or stripped.

`display_name` must round-trip the source's intent — `Vec<i32>` and `Vec< i32 >` produce the same `display_name`.

## `canonical_name` resolution

Per [ADR-0003](adr/0003-level-3-types-and-references.md), every `type` row gets a `canonical_name` when resolvable.

Spell out:
- Scope walk order: what's the lookup precedence (local imports, parent module, prelude, etc.)?
- What counts as "unresolved" (parse failure, external crate not indexed, generic type parameter, etc.) — these rows get `canonical_name = null`.
- How aliases are resolved (`type Foo = Vec<u8>;` — does `Foo` canonicalize to `Vec<u8>` or stay `Foo`?). State the choice.
- How generic parameters render in `canonical_name` (fully-qualified or local-name?).

## Identity

Per [ADR-0003](adr/0003-level-3-types-and-references.md), `type.id = blake3(language | file_id | display_name)`. State any language-specific normalization applied to `display_name` before hashing.

## Worked examples

At least 5 examples drawn from `../virgil-skills/benchmarks/<lang>/`. For each:

1. The source snippet with file path + line range.
2. The full `type` row that should be emitted (every column).
3. Any `parameter` / `returns_type` / `throws` rows that reference it.

Pick examples that exercise *different* `kind` variants — at minimum one each of `named`, `generic`, and one more variant the language uses heavily.
```

---

## `references-<lang>.md` skeleton

```md
# References — <Language>

## Lexical scope rules

Describe the scoping model:
- What scopes exist (block, function, class/impl, module, file).
- How lookup walks outward.
- Shadowing rules (later binding wins, earlier wins, error?).
- Module-qualified names: how `a::b::c` is resolved against imports.

## `ref_kind` decision tree

For each `ref_kind`, list the AST patterns that produce it.

### `read`
Every AST pattern where an identifier is *evaluated*. State exceptions (e.g. macro arguments, attribute paths).

### `write`
Every AST pattern where an identifier is *assigned to* or *mutated*. Include compound assignment (`+=`), increment/decrement, method calls that mutate by language convention.

### `type_use`
Where an identifier appears in a type position (parameter type annotation, return type, cast target, generic argument). Tie to the rows already emitted in `types-<lang>.md`.

### `import_use`
Identifiers that appear inside an `import`/`use`/`require` statement. Tie to the existing `raw_import` / `imports` rows.

## `referent_id` resolution

Algorithm to map an identifier occurrence → the `referent_id` of the symbol it names.

State precisely:
- Lookup precedence (locals → enclosing function → enclosing class → module → imports → prelude).
- Behavior when multiple candidates exist (record all matches? pick most-specific? skip?).
- Behavior when no candidate exists (record `referent_id = null` or skip the row?).

State whether the resolver uses the `symbols_by_name` index that already exists in `src/graph/builder.rs` or a fresh per-file scope tree.

## Worked examples

At least 5 examples from `../virgil-skills/benchmarks/<lang>/`. For each:

1. The source snippet (full function or block) with file path + line range.
2. Every `references` row that should be emitted (referrer_id, referent_id, ref_kind, site_file, site_start_byte).
3. Cases that look ambiguous and how the resolver decides.

Include at least one example with shadowing and at least one with a write to a non-local.
```

---

## `attrs-<lang>.md` skeleton

```md
# Language attributes — <Language>

## Schema

```
:create <lang>_attrs {
    symbol_id: String =>
    <field>: <type> default <default>,
    ...
}
```

State every column with type, default, and what kind of symbol it applies to (function only? all symbols? classes only?).

## Extraction rules

For each column:
- AST source: which tree-sitter node or modifier produces a non-default value.
- Default behavior when the source is absent.
- Edge cases (e.g. conditional compilation: pick the first cfg, or union of all? — state explicitly).

## Worked examples

At least 3 examples from `../virgil-skills/benchmarks/<lang>/`. For each:

1. The source snippet with file path + line range.
2. The full `<lang>_attrs` row that should be emitted.
3. At least one example where the value comes from a non-obvious AST construct.
```

---

## Cross-cutting expectations

- All `start_byte` / `start_line` / `start_col` values in worked examples are the tree-sitter `Range` of the relevant node, not adjusted for trivia.
- `symbol_id` strings in examples follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.
- When the contract doc and the implementation disagree, the contract doc is authoritative until updated. Disagreements get raised as PR review feedback, not silent extractor drift.
