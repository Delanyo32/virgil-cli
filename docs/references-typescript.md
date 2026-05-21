# References — TypeScript / JavaScript

This contract covers `.ts`, `.tsx`, `.js`, `.jsx`. They share a tree-sitter grammar family and one extractor.

Per [ADR-0005](adr/0005-datalog-resolution.md), this document describes **fact emission**: the `occurrence`, `scope`, and `binding` rows the TS/JS extractor must produce. Resolution (turning each `occurrence` into a `references` row) is the resolver's job — see `docs/resolution.md`. Worked examples below enumerate the *inputs* to resolution, not the resolver's output.

JS-vs-TS divergence is called out inline. The short version:
- JS files emit **no** `type_use` occurrences and **no** type-related bindings (no `type`, `interface`, `type_parameter` constructs).
- Everything else (scopes, value-position occurrences, value bindings, imports) is identical.

Symbol IDs throughout follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`. Scope IDs follow the schema convention `path|start_byte|kind`.

## Scope tree

Every file produces one `scope` row of `kind = "file"` with `parent_id = null`. From there, additional scopes nest as follows.

| AST node kind                     | `scope.kind`   | Notes |
|-----------------------------------|----------------|-------|
| `program` (root)                  | `file`         | One per file. `parent_id = null`. |
| `internal_module` / `module`      | `namespace`    | TS-only (`namespace Foo { … }` / `module 'foo' { … }`). JS files emit none. |
| `class_declaration` body, `class_expression` body, `abstract_class_declaration` body | `class` | Opens for the class body braces. Field declarations and method definitions live here. |
| `function_declaration` body, `generator_function_declaration` body | `function` | Function body braces. Parameters bind here. |
| `function_expression` body, `generator_function` body | `function` | Anonymous function expressions. |
| `arrow_function` body             | `function`     | Arrow functions create function scopes. The body may be an expression (no braces) — the scope still exists, covering the expression's byte range. |
| `method_definition` body          | `function`     | Methods. Parameters bind here. |
| `statement_block` (anywhere except as a function body) | `block`        | Bare blocks, `if`/`else`/`for`/`while`/`switch` bodies, `try`/`catch`/`finally`. Block scopes hold `let` and `const` bindings — `var` skips through to the enclosing function scope. |
| `for_statement` / `for_in_statement` / `for_of_statement` header | `block`     | The header introduces a block scope that wraps the body. `for (let i = 0; …)` binds `i` here, not in the body. The body is a nested `block` only when it is itself a `statement_block`. |
| `catch_clause`                    | `block`        | `catch (err) { … }` binds `err` in this scope. |
| `switch_body`                     | `block`        | All `case` clauses share a single block scope. |

### Parent linkage

`parent_id` is the innermost enclosing scope. The file scope's `parent_id` is `null`. A `class` scope's parent is its declaring module/file/function scope (whatever lexically contains the class declaration). A method `function` scope's parent is the `class` scope (not the file). A function-expression assigned to a variable has the variable's enclosing scope as its parent.

### Edge cases

- **Single-expression arrow bodies** (`x => x + 1`): emit a `function` scope covering the entire arrow's byte range (including the parameter list). The parameter `x` binds at the start of this scope.
- **Destructuring patterns** in parameters or `const`/`let`/`var` declarations do not open a new scope; each name extracted from the pattern becomes a binding in the surrounding scope.
- **JSX expression containers** (`{expr}` in JSX) do **not** open a scope. They are pure expression contexts.
- **Object literals** do not open a scope (property keys are not bindings).
- **`with` statements**: deprecated and rare. We emit the inner `statement_block` as a normal `block` scope and do not model `with`'s dynamic binding. References inside resolve via normal scope walking; the `with`'d object's properties are not modelled.
- **`var` declarations** anywhere in a function: emit `binding` at the enclosing function/file scope, *not* the surrounding block. The `scope` tree is unchanged — only the binding's `scope_id` differs from where the declaration syntactically sits.

## Bindings

Every name introduction emits one `binding` row. The `start_byte` field is the byte offset of the *name token* (used by the resolver to order shadowing).

### `definition`

Definition sites that introduce a name in their enclosing scope. Emitted for:

- `function_declaration` and `generator_function_declaration` — `name` is the function identifier; `symbol_id` matches the `symbol` row's id.
- `class_declaration` and `abstract_class_declaration` — class identifier. A class declaration also creates a *type-level* binding (the class name is usable in type positions); we emit a **single** `definition` binding — the resolver handles dual value/type lookup by ignoring `occurrence_kind` distinctions for class names.
- `interface_declaration` (TS only).
- `type_alias_declaration` (TS only).
- `enum_declaration` (TS only).
- `variable_declarator` with a `const` / `let` / `var` parent — the LHS name, or each name extracted from a destructuring pattern. `var` declarations bind to the enclosing function scope; `let` / `const` bind to the innermost block.
- `method_definition` — method name binds in the enclosing `class` scope.
- `public_field_definition` / `field_definition` (`class C { x = 1; }`) — field name binds in the enclosing `class` scope. JS class fields behave identically.
- Named function/class expressions (`const f = function inner() { … }`, `const C = class Inner { … }`) — `inner` / `Inner` bind only within the function/class body scope itself, not the outer scope.

### `parameter`

Function / arrow / method parameters. Emitted for the parameter scope (the function body's `function` scope).

- Simple parameters: `function f(a, b)` — two `parameter` bindings (`a`, `b`).
- Destructuring patterns: `function f({ x, y: z }, [a, b])` — bindings for `x`, `z`, `a`, `b`. The rename in `{ y: z }` introduces `z` only; `y` is the property key and is **not** emitted as an occurrence or binding (see Occurrence emission below).
- Rest parameters: `function f(...rest)` — one `parameter` binding for `rest`.
- Default values: `function f(x = computeDefault())` — one `parameter` binding for `x`; the `computeDefault` expression emits occurrences as normal value-position code.
- Arrow function parameters: identical treatment.
- Type-annotated parameters (TS): the annotation produces `type_use` *occurrences* (see below) — the parameter itself is still one `binding` row.

`symbol_id` matches the parameter's `symbol` row (per Issue #11). If parameters are not emitted as standalone symbols (current TS extractor behavior), `symbol_id` is `null` and the resolver falls back to scope-walk semantics.

### `import`

Plain ES-module imports that introduce the imported name into the file scope.

- `import { foo } from './x'` — one `import` binding (`name = "foo"`). `symbol_id` is the resolved target symbol's id when `resolve_import('./x')` succeeds and `foo` exists in that file's exports; otherwise `null` (external/unindexed/unknown).
- `import { foo, bar } from './x'` — two rows.
- `import 'side-effect'` — no binding (no name introduced).
- CommonJS `const x = require('./y')` — the `x` LHS is a normal `variable_declarator`, so the binding kind is `definition`, *not* `import`. The `imports` graph captures the file relationship separately. Destructured `require` (`const { x } = require('./y')`) emits one `definition` per destructured name.
- Dynamic `const m = await import('./z')` — same: `m` is a `definition` binding.

### `import_alias`

Aliased imports — the alias is what binds locally; the original name is recorded in the binding's `symbol_id`.

- `import { foo as bar } from './x'` — one `import_alias` binding with `name = "bar"`, `symbol_id` = the `foo` symbol's id in `./x` (or `null` if unresolved).
- `import * as ns from './x'` — one `import_alias` binding with `name = "ns"`. This is a **namespace alias**, not a glob — it binds the name `ns` locally; member access `ns.foo` resolves at occurrence time, not binding time. See "Namespace imports vs wildcard imports" below.
- Default imports — `import x from './x'` binds `x` as a `import_alias` for the default export. `name = "x"`, `symbol_id` = the default-exported symbol's id in `./x` when resolvable (look up the symbol named `"default"` or follow the `export default` declaration).
- `import x, { y } from './x'` — two rows: `import_alias` for `x` (default), `import` for `y`.
- `export { foo as bar } from './x'` (re-export with rename) — the re-exporting file gains an `import_alias` binding for `bar` in its file scope, with `symbol_id` pointing at `foo` in `./x` (transitive re-exports resolved during import resolution per `docs/resolution.md`'s assumption).

### `wildcard_import`

ES-module wildcard re-exports.

- `export * from './x'` — emit one `wildcard_import` binding in the re-exporting file's file scope with `name = "*"` and `symbol_id = null`. The resolver expands at materialise time by joining against the `imports` graph and `./x`'s exported symbols.

**Note: `import * as ns from './x'` is NOT a wildcard_import.** It is a namespace alias (`import_alias`). The distinction:

- `import *` (TS/JS) — binds a *single* name (`ns`) holding a namespace object; member access goes through `ns.member`. No wildcard expansion.
- `export *` — re-exports every name from the source module into the current module's surface; downstream importers see those names as if declared here. This *is* a wildcard expansion at resolution time.

This differs from Rust's `use foo::*`, which brings every exported name into the local scope as a bare identifier. TS/JS has no equivalent inside a single file: there is no way to bring every name from a module into the local scope as bare identifiers. The closest analog is `export * from`, but it only operates at the module's export surface, not inside function bodies.

## Occurrence emission

Every identifier occurrence emits exactly one `occurrence` row. The `id` is `path|start_byte|name|occurrence_kind`. `enclosing_symbol_id` is the innermost named symbol containing the occurrence (anonymous function expressions and arrow functions do **not** count as symbols — the occurrence attributes to the next named ancestor). `enclosing_scope_id` is the innermost `scope` row.

For module-top-level occurrences (no enclosing named symbol), `enclosing_symbol_id` is `null`. The resolver skips emitting `references` rows for those per ADR-0002's name-required referrer convention, but the `occurrence` row itself is still emitted (it may be needed by other consumers).

### `call`

The identifier in callee position of a `call_expression` or `new_expression`:

- `foo()` — `foo` is a `call`.
- `obj.foo()` — `obj` is a `read`; `foo` (the method name) emits **no occurrence** (see "Property access" below).
- `obj[name]()` — `obj` is `read`, `name` is `read`. No `call` occurrence (the callee is computed).
- `new Cls(args)` — `Cls` is a `call`. Arguments emit occurrences as usual.
- Tag template literals (`` tag`hello ${x}` ``) — `tag` is a `call`.
- Optional chaining `foo?.()`, `obj?.foo()` — same rules as the non-optional forms.

### `read`

Identifiers in value position that are not calls, writes, or imports:

- Variable references: `const y = x + 1` — `x` is `read`.
- Function arguments: `foo(a, b)` — `a` and `b` are `read`.
- Property-access object: `obj.x` — `obj` is `read`.
- Computed access: `obj[key]` — both `obj` and `key` are `read`.
- Template-literal expressions: `` `hello ${name}` `` — `name` is `read`.
- Conditional / logical / arithmetic / comparison operands.
- JSX (TSX/JSX): `<Foo bar={baz} />` — `Foo` is `read`, `baz` is `read`. The attribute key `bar` emits **no occurrence** (it is a JSX attribute name, not an identifier reference).
- `typeof x` in a value position — `x` is `read`.
- `instanceof Cls` — `Cls` is `read`.
- `class C extends Base {}` — `Base` is `read` (the `extends` clause is a value expression in JS semantics).
- Spread arguments: `foo(...args)` — `args` is `read`.

### `write`

The identifier is the target of an assignment or mutation:

- `x = expr` — `x` is `write`. (`obj.x = …` emits `obj` as `read` only; the property name `x` emits no occurrence.)
- Compound assignment (`x += 1`, `x -= 1`, `x ||= …`, `x &&= …`, `x ??= …`, `x <<= …`, `x >>= …`, `x >>>= …`, `x &= …`, `x |= …`, `x ^= …`, `x **= …`, `x *= …`, `x /= …`, `x %= …`) — emit a single `write` occurrence for `x`. Per ADR-0003, the read is collapsed into the write — no separate `read` row.
- Increment / decrement (`x++`, `++x`, `x--`, `--x`) — `x` is `write`.
- Destructuring assignment without declaration: `({ a, b: c } = obj)` — `a` and `c` are `write`; `obj` is `read`. The property name `b` emits no occurrence.
- Array destructuring assignment: `[x, y] = arr` — `x` and `y` are `write`; `arr` is `read`.
- `delete obj.x` — `obj` is `read`. The property `x` emits no occurrence.

Note: destructuring used in a *declaration* (`const { a } = obj`) is a binding site, not a write — emit the `definition` binding for `a`; do not emit an occurrence for `a`. `obj` is `read`.

### `type_use`

**TS only.** Every identifier appearing in a type position emits a `type_use` occurrence. These overlap exactly with the `type` rows emitted per `types-typescript.md` — each named-type mention is also a `type_use` occurrence.

Type positions:
- Parameter type annotations: `function f(x: Foo)` — `Foo` is `type_use`.
- Return type annotations: `function f(): Foo` — `Foo` is `type_use`.
- Variable / field type annotations: `const x: Foo`.
- Generic type arguments: `Array<Foo>` — `Array` is `type_use`, `Foo` is `type_use`.
- `extends` / `implements` clauses on classes and interfaces: `class C extends Base implements I` — `Base` and `I` are `type_use` (note: a class's `extends` clause is a value expression — `Base` is *also* a `read`; we emit both rows because the same identifier serves two roles).
- Type assertions: `x as Foo`, `<Foo>x` — `Foo` is `type_use`.
- Type-alias and interface bodies — `type Foo = Bar` makes `Bar` a `type_use`.
- `typeof X` in a type position — `X` is `type_use`, even though `X` is a value name in source.
- `keyof X` — `X` is `type_use`.
- Nested type identifiers (`ns.Foo` in a type position) — emit one `type_use` for `ns`. We do **not** emit a separate occurrence for `Foo` (field-row policy: property-position names are not emitted as occurrences). The resolver handles `ns.Foo` resolution through the `imports` graph.

**JavaScript files emit zero `type_use` occurrences.** They have no type positions.

### `import_use`

Identifiers within an `import_statement` or an `export_statement` whose `source` is a module specifier:

- `import { foo } from './x'` — `foo` is `import_use`.
- `import { foo as bar } from './x'` — `foo` is `import_use`; `bar` emits no occurrence (it is a binding site, captured by the `import_alias` binding).
- `import x from './x'` — `x` emits no occurrence (binding site).
- `import * as ns from './x'` — `ns` emits no occurrence (binding site).
- `export { foo } from './x'` — `foo` is `import_use`.
- `export { foo as bar } from './x'` — `foo` is `import_use`; `bar` is a binding site.
- `export * from './x'` — no identifiers, no occurrence.
- The module specifier string (`'./x'`) is captured by the `imports` graph; it emits no occurrence (it is not an identifier).

### Property access — field-row policy

For `obj.member`, emit `obj` (as `read`, `write`, or `call` depending on context) but emit **no occurrence** for `member`. Rationale: without type information we cannot bind `member` to a specific symbol; emitting unbindable occurrences would flood the resolver with `null` rows. Class field declarations *do* emit `definition` bindings, so when the resolver gains type info (a follow-up), `obj.member` becomes resolvable through a type-aware rule without changing the extractor.

The same rule applies to:
- Property keys in object literals (`{ key: value }`) — `key` emits no occurrence; `value` emits one if it is an identifier.
- Shorthand object property `{ foo }` — `foo` *does* emit a `read` occurrence (it is the value identifier; the property name happens to share the same byte range).
- JSX attribute names — no occurrence.
- Object pattern keys in destructuring — `const { key: local } = obj` emits no occurrence for `key`; `local` is a binding site; `obj` is `read`. Shorthand `const { foo } = obj` emits a `definition` binding for `foo`, no occurrence.

### `this`, `super`, and `new.target`

- `this` — emit a `read` occurrence with `name = "this"`. The resolver matches it against an injected `parameter` binding at the enclosing class/function scope (the extractor inserts a synthetic `parameter` binding for `this` in method bodies; see "Synthetic `this` binding" below).
- `super` — emit a `read` occurrence with `name = "super"` in method bodies. The resolver treats it like `this` (synthetic binding inserted at class-method scope).
- `new.target` — meta-property. Emit no occurrence. Edge-case dynamic form not load-bearing for our queries.

### Synthetic `this` binding

For every `method_definition` inside a `class_declaration` / `class_expression`, emit a synthetic `parameter` binding `(name = "this", scope_id = <method body's function scope>, symbol_id = <enclosing class's symbol id>)`. This lets the resolver bind `this` occurrences to the class without special-casing class scopes. Arrow functions inside methods do NOT receive a synthetic binding — `this` lexically resolves to the enclosing method's `this` via normal scope walking.

For `super`, emit a similar synthetic `parameter` binding at the method scope pointing at the class's `extends`-clause base when available; `symbol_id = null` when the base class is not resolvable.

## What this contract does NOT cover

- **Resolution algorithm.** How an `occurrence` becomes a `references` row lives in `docs/resolution.md`, applied uniformly across all languages.
- **Property-name-to-class-member binding.** Without type information we cannot resolve `obj.x` to a specific field. The extractor emits `definition` bindings for class fields and methods in the class scope; type-aware resolution is a follow-up that joins `obj`'s resolved type to the class scope. Today's resolver does not perform this join.
- **Method dispatch.** `obj.method()` — the property name `method` emits no occurrence; the resolver produces no row for it. The `obj` identifier resolves normally.
- **`var` hoisting subtleties.** Temporal-dead-zone semantics for `let`/`const` are not modelled; the extractor's `binding.start_byte` lets the resolver pick the shadowing binding by source order, but it does not flag uses before the binding's `start_byte` as errors.
- **Re-export chasing beyond one hop.** `export { foo } from './a'` where `./a` itself does `export { foo } from './b'`. The contract assumes the importer resolves to the final defining symbol when the import chain is resolvable; transitive chasing happens during import resolution (separate concern), not at occurrence-emission time.

## Worked examples

Every example is drawn from one of the two benchmark corpora. Symbol IDs follow ADR-0002. Byte offsets shown as `<byte>` are placeholders read from tree-sitter `Range`s at extraction time. Scope IDs use `path|start_byte|kind`.

### Example 1 — Block-scope shadowing (`let` in nested block shadows outer)

**Source** — `src/utils/formatters.ts:107-130` (excerpt; from `nextjs-dashboard`):

```ts
export function formatTableData(rows: any[], columns: string[]): any[] {
  const formatted: any[] = [];

  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    const formattedRow: any = {};

    for (let j = 0; j < columns.length; j++) {
      const col = columns[j];
      const value = row[col];
      // …
    }
  }
}
```

**`scope` rows:**

| id (path\|start_byte\|kind)                          | parent_id                                            | kind     |
|------------------------------------------------------|------------------------------------------------------|----------|
| `src/utils/formatters.ts\|0\|file`                   | `null`                                               | `file`   |
| `src/utils/formatters.ts\|<fn_body_byte>\|function`  | `src/utils/formatters.ts\|0\|file`                   | `function` |
| `src/utils/formatters.ts\|<outer_for_byte>\|block`   | `src/utils/formatters.ts\|<fn_body_byte>\|function`  | `block`  |
| `src/utils/formatters.ts\|<inner_for_byte>\|block`   | `src/utils/formatters.ts\|<outer_for_byte>\|block`   | `block`  |

(The outer `for` header opens the block scope that holds `i`; the body's `statement_block` does not open a separate scope — its bindings sit in the same scope as `i`.)

**`binding` rows:**

| scope_id                                                  | name        | binding_kind | symbol_id |
|-----------------------------------------------------------|-------------|--------------|-----------|
| `src/utils/formatters.ts\|0\|file`                        | `formatTableData` | `definition` | `src/utils/formatters.ts\|107\|0\|formatTableData\|function` |
| `src/utils/formatters.ts\|<fn_body_byte>\|function`       | `rows`      | `parameter`  | `null` (params not emitted as symbols) |
| `src/utils/formatters.ts\|<fn_body_byte>\|function`       | `columns`   | `parameter`  | `null` |
| `src/utils/formatters.ts\|<fn_body_byte>\|function`       | `formatted` | `definition` | `src/utils/formatters.ts\|109\|2\|formatted\|variable` |
| `src/utils/formatters.ts\|<outer_for_byte>\|block`        | `i`         | `definition` | `null` (block-scoped local, not a top-level symbol) |
| `src/utils/formatters.ts\|<outer_for_byte>\|block`        | `row`       | `definition` | `null` |
| `src/utils/formatters.ts\|<outer_for_byte>\|block`        | `formattedRow` | `definition` | `null` |
| `src/utils/formatters.ts\|<inner_for_byte>\|block`        | `j`         | `definition` | `null` |
| `src/utils/formatters.ts\|<inner_for_byte>\|block`        | `col`       | `definition` | `null` |
| `src/utils/formatters.ts\|<inner_for_byte>\|block`        | `value`     | `definition` | `null` |

**`occurrence` rows (selected, body only):**

| name      | kind     | enclosing_symbol_id                                   | enclosing_scope_id                                       |
|-----------|----------|-------------------------------------------------------|----------------------------------------------------------|
| `rows`    | `read`   | `src/utils/formatters.ts\|107\|0\|formatTableData\|function` | `src/utils/formatters.ts\|<outer_for_byte>\|block` (in `rows.length`) |
| `i`       | `read`   | (same)                                                | `src/utils/formatters.ts\|<outer_for_byte>\|block` |
| `i`       | `write`  | (same)                                                | `src/utils/formatters.ts\|<outer_for_byte>\|block` (the `i++`) |
| `rows`    | `read`   | (same)                                                | (same)                                                   |
| `columns` | `read`   | (same)                                                | `src/utils/formatters.ts\|<inner_for_byte>\|block`       |
| `j`       | `read`   | (same)                                                | `src/utils/formatters.ts\|<inner_for_byte>\|block`       |
| `j`       | `write`  | (same)                                                | `src/utils/formatters.ts\|<inner_for_byte>\|block` (`j++`) |
| `row`     | `read`   | (same)                                                | `src/utils/formatters.ts\|<inner_for_byte>\|block` (`row[col]`) |
| `col`     | `read`   | (same)                                                | (same)                                                   |

Also emitted: `type_use` occurrences for `any` (in the `any[]` and `: any` annotations) at the function-body scope. Since this is a TS file, the type-annotation tokens emit `type_use` rows; if this were a `.js` file, none of those would be emitted.

### Example 2 — Aliased import + namespace import

**Source** — `src/pages/api/reports/export.ts:1-8` (from `nextjs-dashboard`):

```ts
import type { NextApiRequest, NextApiResponse } from 'next';
import * as fs from 'fs';
import * as path from 'path';
```

**`scope` rows:** only the `file` scope is relevant here.

| id                                                  | parent_id | kind   |
|-----------------------------------------------------|-----------|--------|
| `src/pages/api/reports/export.ts\|0\|file`          | `null`    | `file` |

**`binding` rows:**

| scope_id                                            | name             | binding_kind    | symbol_id |
|-----------------------------------------------------|------------------|-----------------|-----------|
| `src/pages/api/reports/export.ts\|0\|file`          | `NextApiRequest` | `import`        | `null` (external module `'next'`) |
| `src/pages/api/reports/export.ts\|0\|file`          | `NextApiResponse`| `import`        | `null` |
| `src/pages/api/reports/export.ts\|0\|file`          | `fs`             | `import_alias`  | `null` (external module `'fs'`) |
| `src/pages/api/reports/export.ts\|0\|file`          | `path`           | `import_alias`  | `null` |

The `import * as fs from 'fs'` is a **namespace alias**, not a wildcard. It binds the single name `fs`; later occurrences of `fs.readFile`, `fs.existsSync`, etc. emit `read` occurrences for `fs` only — the property name (`readFile`) emits no occurrence per field-row policy.

**`occurrence` rows (selected):**

| name              | kind         | enclosing_scope_id                                  |
|-------------------|--------------|-----------------------------------------------------|
| `NextApiRequest`  | `import_use` | `src/pages/api/reports/export.ts\|0\|file`          |
| `NextApiResponse` | `import_use` | (same)                                              |

The binding names `fs` and `path` themselves emit **no** `import_use` occurrence — they are binding sites, recorded as `import_alias` bindings. Later in the file, when `fs.writeFileSync(…)` appears (line 92 et al.), `fs` emits a `read` occurrence.

A JavaScript file with `const fs = require('fs')` would instead emit a `definition` binding for `fs` (not `import_alias`), because CommonJS `require` is a regular variable declaration syntactically. The `imports` graph still records the file relationship.

### Example 3 — Anonymous arrow inside a method-like function (enclosing_symbol_id attribution)

**Source** — `src/lib/cache.ts:88-94` (from `nextjs-dashboard`):

```ts
export function getCacheStats(): { size: number; totalHits: number } {
  let totalHits = 0;
  cache.forEach((entry) => {
    totalHits += entry.hits;
  });
  return { size: cache.size, totalHits };
}
```

**`scope` rows:**

| id                                                  | parent_id                                          | kind       |
|-----------------------------------------------------|----------------------------------------------------|------------|
| `src/lib/cache.ts\|0\|file`                         | `null`                                             | `file`     |
| `src/lib/cache.ts\|<gcs_body_byte>\|function`       | `src/lib/cache.ts\|0\|file`                        | `function` |
| `src/lib/cache.ts\|<arrow_byte>\|function`          | `src/lib/cache.ts\|<gcs_body_byte>\|function`     | `function` |

The arrow function `(entry) => { totalHits += entry.hits; }` opens a `function` scope. It is **not** a symbol — the arrow is anonymous. Occurrences inside attribute to the next named ancestor.

**`binding` rows (function body and arrow):**

| scope_id                                            | name        | binding_kind | symbol_id |
|-----------------------------------------------------|-------------|--------------|-----------|
| `src/lib/cache.ts\|0\|file`                         | `getCacheStats` | `definition` | `src/lib/cache.ts\|88\|0\|getCacheStats\|function` |
| `src/lib/cache.ts\|<gcs_body_byte>\|function`       | `totalHits` | `definition` | `null` (local `let`, not a top-level symbol) |
| `src/lib/cache.ts\|<arrow_byte>\|function`          | `entry`     | `parameter`  | `null` |

**`occurrence` rows:**

| name        | kind     | enclosing_symbol_id                              | enclosing_scope_id                                  |
|-------------|----------|--------------------------------------------------|-----------------------------------------------------|
| `cache`     | `read`   | `src/lib/cache.ts\|88\|0\|getCacheStats\|function` | `src/lib/cache.ts\|<gcs_body_byte>\|function` |
| `totalHits` | `write`  | `src/lib/cache.ts\|88\|0\|getCacheStats\|function` | `src/lib/cache.ts\|<arrow_byte>\|function` |
| `entry`     | `read`   | `src/lib/cache.ts\|88\|0\|getCacheStats\|function` | `src/lib/cache.ts\|<arrow_byte>\|function` |
| `cache`     | `read`   | `src/lib/cache.ts\|88\|0\|getCacheStats\|function` | `src/lib/cache.ts\|<gcs_body_byte>\|function` (the `cache.size`) |
| `totalHits` | `read`   | `src/lib/cache.ts\|88\|0\|getCacheStats\|function` | `src/lib/cache.ts\|<gcs_body_byte>\|function` (the shorthand `totalHits` in the returned object literal) |

Note `totalHits += entry.hits`: the `+=` collapses to a single `write` occurrence for `totalHits` (no separate `read`). `entry.hits` emits one `read` for `entry`; the property name `hits` emits no occurrence.

The `enclosing_symbol_id` for every row is `getCacheStats` — the arrow is anonymous and not its own symbol. The `enclosing_scope_id` correctly identifies the inner arrow scope for occurrences inside the callback, which lets the resolver walk outward to find `totalHits` bound in the outer function scope.

`type_use` occurrences are also emitted for `number` (twice, in the return type annotation). JS would emit none.

### Example 4 — `this` inside a method-like function (JS, no class)

**Source** — `src/models/AuditLog.js:56-66` (from `express-api`):

```js
auditLogSchema.statics.create = function (data) {
  const log = new this({
    ...data,
    timestamp: data.timestamp || new Date(),
  });
  log.save().catch(function (err) {
    console.error('Failed to save audit log:', err.message);
  });
  return log;
};
```

This is an anonymous `function (data) { … }` assigned to a property. It is not a class method, so the extractor does **not** insert a synthetic `this` binding (that synthetic-binding rule is scoped to `method_definition` inside `class_*` nodes). Here, `this` occurs but binds dynamically at call time — the extractor emits the occurrence; the resolver produces `referent_id = null`.

**`scope` rows:**

| id                                                  | parent_id                                          | kind       |
|-----------------------------------------------------|----------------------------------------------------|------------|
| `src/models/AuditLog.js\|0\|file`                   | `null`                                             | `file`     |
| `src/models/AuditLog.js\|<outer_fn_byte>\|function` | `src/models/AuditLog.js\|0\|file`                  | `function` |
| `src/models/AuditLog.js\|<catch_fn_byte>\|function` | `src/models/AuditLog.js\|<outer_fn_byte>\|function`| `function` |

**`binding` rows:**

| scope_id                                            | name   | binding_kind | symbol_id |
|-----------------------------------------------------|--------|--------------|-----------|
| `src/models/AuditLog.js\|0\|file`                   | `mongoose` | `definition` | `src/models/AuditLog.js\|6\|0\|mongoose\|variable` |
| `src/models/AuditLog.js\|0\|file`                   | `auditLogSchema` | `definition` | `src/models/AuditLog.js\|8\|0\|auditLogSchema\|variable` |
| `src/models/AuditLog.js\|<outer_fn_byte>\|function` | `data` | `parameter`  | `null` |
| `src/models/AuditLog.js\|<outer_fn_byte>\|function` | `log`  | `definition` | `null` |
| `src/models/AuditLog.js\|<catch_fn_byte>\|function` | `err`  | `parameter`  | `null` |

**`occurrence` rows (body only):**

| name      | kind   | enclosing_symbol_id | enclosing_scope_id                                    |
|-----------|--------|---------------------|-------------------------------------------------------|
| `this`    | `read` | `null` (the anonymous fn is not a symbol; no named ancestor inside the assignment expression) | `src/models/AuditLog.js\|<outer_fn_byte>\|function` |
| `data`    | `read` | (same)              | (same)                                                |
| `data`    | `read` | (same)              | (same — the second `data.timestamp` reference)        |
| `Date`    | `call` | (same)              | (same)                                                |
| `log`     | `read` | (same)              | (same)                                                |
| `console` | `read` | (same)              | `src/models/AuditLog.js\|<catch_fn_byte>\|function`   |
| `err`     | `read` | (same)              | `src/models/AuditLog.js\|<catch_fn_byte>\|function`   |
| `log`     | `read` | (same)              | (same as outer — `return log`)                        |

The right-hand-side of `auditLogSchema.statics.create = function (data) { … }` is an anonymous function expression. The LHS `auditLogSchema.statics.create` emits one `read` for `auditLogSchema` (the property names `statics` and `create` emit no occurrences per field-row policy). The `=` is an assignment, but the LHS is a member-expression, so we emit no `write` row for any identifier — only `read` of `auditLogSchema`.

Since this is JS, no `type_use` occurrences are emitted.

Inside the body: `new this({...})` emits `this` as `read` and not `call` — the `new` operator's callee position when applied to `this` still emits `read` for `this` (we reserve `call` for plain identifier callees and `new Cls(...)` where `Cls` is an identifier; `this` is a keyword-expression treated as a plain identifier-shaped occurrence with `kind = read`). The resolver gets a `read` with `name = "this"` and produces `referent_id = null` (no synthetic binding here — this is a function expression, not a `method_definition`).

### Example 5 — Plain ES-module import + call across files (express-api)

**Source** — `src/app.js:21-38` (from `express-api`):

```js
const { requestLogger } = require('./middleware/logger');

function createApp() {
  const app = express();

  // Security headers
  app.use(helmet());

  // CORS
  app.use(cors());

  // Body parsing
  app.use(bodyParser.json({ limit: '10mb' }));
  app.use(bodyParser.urlencoded({ extended: true }));

  // Logging
  app.use(morgan('combined'));
  app.use(requestLogger);
```

CommonJS destructuring `const { requestLogger } = require('./middleware/logger')` is a variable declaration. The extractor emits a `definition` binding for `requestLogger` at file scope (not an `import` binding — that is reserved for ES `import` statements). The `imports` graph captures the file-to-file relationship separately.

**`scope` rows:**

| id                                  | parent_id                       | kind       |
|-------------------------------------|---------------------------------|------------|
| `src/app.js\|0\|file`               | `null`                          | `file`     |
| `src/app.js\|<createApp_byte>\|function` | `src/app.js\|0\|file`     | `function` |

**`binding` rows (selected):**

| scope_id                          | name             | binding_kind | symbol_id                                            |
|-----------------------------------|------------------|--------------|------------------------------------------------------|
| `src/app.js\|0\|file`             | `express`        | `definition` | `src/app.js\|6\|6\|express\|variable`               |
| `src/app.js\|0\|file`             | `helmet`         | `definition` | `src/app.js\|10\|6\|helmet\|variable`               |
| `src/app.js\|0\|file`             | `cors`           | `definition` | `src/app.js\|8\|6\|cors\|variable`                  |
| `src/app.js\|0\|file`             | `morgan`         | `definition` | `src/app.js\|9\|6\|morgan\|variable`                |
| `src/app.js\|0\|file`             | `bodyParser`     | `definition` | `src/app.js\|7\|6\|bodyParser\|variable`            |
| `src/app.js\|0\|file`             | `requestLogger`  | `definition` | `null` (destructured-require, not extracted as a symbol by the current pipeline) |
| `src/app.js\|0\|file`             | `createApp`      | `definition` | `src/app.js\|23\|0\|createApp\|function`            |
| `src/app.js\|<createApp_byte>\|function` | `app`     | `definition` | `null` (local `const`) |

**`occurrence` rows (body of `createApp`, selected):**

| name            | kind   | enclosing_symbol_id                          | enclosing_scope_id                              |
|-----------------|--------|----------------------------------------------|-------------------------------------------------|
| `express`       | `call` | `src/app.js\|23\|0\|createApp\|function`     | `src/app.js\|<createApp_byte>\|function`        |
| `app`           | `read` | (same)                                       | (same)                                          |
| `helmet`        | `call` | (same)                                       | (same)                                          |
| `app`           | `read` | (same)                                       | (same)                                          |
| `cors`          | `call` | (same)                                       | (same)                                          |
| `app`           | `read` | (same)                                       | (same)                                          |
| `bodyParser`    | `read` | (same)                                       | (same) (the `bodyParser.json(...)` site) |
| `morgan`        | `call` | (same)                                       | (same)                                          |
| `requestLogger` | `read` | (same)                                       | (same) (`app.use(requestLogger)`) |

`app.use(...)` emits `app` as `read` (not `call`); the property name `use` emits no occurrence per field-row policy. `helmet()`, `cors()`, `express()`, `morgan(…)` are all bare-identifier callees and emit `call` occurrences for the callee identifier.

For the resolver: `requestLogger`'s `read` occurrence walks the scope tree, finds the file-scope `definition` binding (with `symbol_id = null`), and produces a `references` row with `referent_id = null`. Improving destructured-require resolution to point at the target file's symbol is a follow-up; the contract today emits the binding and lets the resolver produce a null target.

### Example 6 — ES-module re-export wildcard (`export * from`)

**Source** — synthesised against the contract (the benchmarks do not contain an `export *`; the resolver MUST handle this form when present):

```ts
// File: src/types/index.ts
export * from './user';
export * from './api';
```

**`scope` rows:** only the file scope.

**`binding` rows:**

| scope_id                          | name | binding_kind      | symbol_id |
|-----------------------------------|------|-------------------|-----------|
| `src/types/index.ts\|0\|file`     | `*`  | `wildcard_import` | `null`    |
| `src/types/index.ts\|0\|file`     | `*`  | `wildcard_import` | `null`    |

Each `export * from` emits one row with `name = "*"`, `symbol_id = null`. The `imports` graph records the source file for each (`./user`, `./api`). The resolver expands at materialise time by joining against each source file's exported symbols (per the `wildcard_target` rule in `docs/resolution.md`).

**No `occurrence` rows** are emitted by either `export *` statement — they have no identifiers (the string `'./user'` is the module specifier, not an identifier).

Contrast with `import * as ns from './user'` (Example 2): that emits an `import_alias` binding for `ns`, not a `wildcard_import` binding. The `import *` form binds a *namespace object* under a single name; the `export *` form re-exports every name from the source module.
