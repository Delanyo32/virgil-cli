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

**Updated per [ADR-0005](adr/0005-datalog-resolution.md):** references contracts describe **fact emission** (`occurrence` / `scope` / `binding` rows), not resolution. Resolution lives in `docs/resolution.md` as Cozoscript rules that apply uniformly across all languages.

```md
# References — <Language>

## Scope tree

Describe the language's lexical scopes that map to `scope` rows:
- Which AST nodes open a new scope (function body, block, class body, namespace, file).
- The `kind` value to emit for each: `"file"` / `"module"` / `"namespace"` / `"class"` / `"function"` / `"block"`.
- Parent linkage: the `parent_id` of each emitted scope is the innermost enclosing scope.
- Edge cases — single-statement blocks, comprehensions, lambdas, arrow functions: state whether each opens its own scope.

## Bindings

For each `binding_kind`, list the AST patterns the extractor must recognize:

### `definition`
Definition sites that introduce a name in their enclosing scope (functions, classes, methods, top-level variables, etc.). The `symbol_id` is the same id that the `symbol` extractor emits for the definition.

### `parameter`
Function/method parameters. `symbol_id` matches the parameter's `symbol` row (per Issue #11).

### `import`
Plain imports that introduce the imported name into the importing file's module scope. `symbol_id` is `null` when the target is external (unindexed crate, stdlib, etc.); otherwise it's the imported symbol's id in the target file.

### `import_alias`
Aliased imports (`import { foo as bar }`, `use foo::baz as qux`). `name` is the alias (`bar` / `qux`). `symbol_id` is the underlying symbol's id (transitive re-exports already chased during import resolution).

### `wildcard_import`
`use foo::*`, `from foo import *`, etc. Emit one row with `name = "*"` per wildcard. `symbol_id = null` (the resolver expands at materialise time using the `imports` graph).

## Occurrence emission

For each `occurrence_kind`, list the AST patterns the extractor emits:

### `call`
Every call expression — the identifier in callee position.

### `read`
Every identifier in value position. State exceptions explicitly (e.g. macro arguments, attribute paths, format-string interpolation).

### `write`
Every assignment LHS, compound assignment, increment/decrement. State which compound forms collapse to a single `write` per ADR-0003 (default: yes — single write row, no read row).

### `type_use`
Identifiers in a type position. These should overlap exactly with the `type` rows your `types-<lang>.md` produces — each `type` mention is also a `type_use` occurrence.

### `import_use`
Identifiers within an import declaration. State whether these are the module/package name, the imported names, or both.

For every occurrence: `enclosing_symbol_id` is the innermost symbol containing the occurrence (`null` for file-level expressions). `enclosing_scope_id` is the innermost `scope` row.

## What this contract does NOT cover

- **Resolution algorithm.** That's `docs/resolution.md`, applied uniformly. Per-language `references` contracts don't describe how `occurrence` → `referent_id` happens.
- **`references` rows.** Worked examples below show the *inputs* to resolution (occurrences, scopes, bindings), not the resolver's output rows.

## Worked examples

At least 5 examples from `../virgil-skills/benchmarks/<lang>/`. For each:

1. The source snippet (full function or block) with file path + line range.
2. Every `scope` row the extractor must emit for the snippet.
3. Every `binding` row.
4. Every `occurrence` row.

Pick examples that exercise: shadowing, an aliased import, a wildcard import (or its absence), a `self`/`this` field access, and one call where the callee isn't statically resolvable.

Do NOT enumerate expected `references` rows — those are the resolver's output and live in `docs/resolution.md`'s test suite.
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
