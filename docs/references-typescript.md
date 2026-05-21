# References — TypeScript / JavaScript

This contract covers `.ts`, `.tsx`, `.js`, `.jsx`. They share a tree-sitter grammar family and one extractor.

The `references` relation is keyed by `(referrer_id, site_file, site_start_byte, match_index)` with `referent_id` and `ref_kind` in the value position (per the updated schema in `docs/virgil-datalog-schema.md`). The `referrer_id` is the **enclosing symbol** containing the identifier occurrence — the function, method, or top-level declaration that "owns" the use site. `referent_id` is the symbol the identifier names, or `null` when unresolved. `match_index = 0` for the primary/only candidate; TS/JS does not have C++-style overloading, so every TS/JS row uses `match_index = 0` in practice.

## Lexical scope rules

TypeScript/JavaScript scoping is lexical and follows the ECMAScript 2015+ block-scoping rules. The resolver walks scopes from the innermost outward.

### Scopes that exist

In order from innermost to outermost:

1. **Block scope.** Any `{ … }` introduces a new scope for `let` and `const` (but **not** `var`). `for`/`while`/`if`/`switch` bodies, arrow-function bodies, function bodies, try/catch bodies all qualify. A `catch (err) { … }` introduces a scope with `err` bound.
2. **Function scope.** The parameter list plus the function body. Parameters are bound in this scope; `var` declarations inside the body are bound here, not in any inner block.
3. **Class scope.** `class C { … }` — methods, fields, and `this` resolve here. Class declarations themselves create a binding in the enclosing scope.
4. **Module / file scope.** Each file is a module (we treat all files in the workspace as modules, even pure-script `.js` files; non-module JS is rare and the difference doesn't matter for our reference resolution). Top-level `function`, `class`, `const`, `let`, `var`, `interface`, `type`, `enum`, and imported names bind here.
5. **Imports.** Imported bindings live in the module scope but their `referent_id` points to a symbol in another file (when resolvable).
6. **Global / ambient.** `console`, `process`, `window`, `document`, `globalThis`, `Math`, `JSON`, etc. Always `referent_id = null` — we never index ambient declarations.

### Lookup order

For an identifier occurrence inside the body of a function or method, walk:

1. Block scopes outward to the function/method body scope.
2. The function/method's parameter scope.
3. If inside a class method: class scope (resolves `this.x` field access, `super`, and bare names of class methods/fields *only when prefixed with `this`*; bare `x` inside a method does **not** resolve to class field `x` — that requires `this.x` in JS/TS).
4. Module scope (top-level declarations + imports).
5. Allow-listed globals → `referent_id = null` but the row is still emitted with `ref_kind = "read"` or `"write"`.

### Shadowing

ECMAScript shadowing: the **innermost binding wins**. A parameter named `data` shadows a module-level `const data`. A `let data` inside a block shadows a parameter of the same name for the rest of that block. `var` declarations *do not* respect block scope — a `var x` inside an `if` is hoisted to the enclosing function and may shadow nothing (or shadow a parameter).

Per ECMAScript: redeclaring a `let`/`const` in the same scope is a syntax error and tree-sitter will produce an ERROR node. We do not try to resolve references inside parse-error regions; rows are skipped.

### Module-qualified names

`ns.foo` where `ns` came from `import * as ns from "./bar"`:

- The `ns` identifier (left of the dot) resolves to the namespace import binding. We emit `ref_kind = "read"` for the `ns` occurrence.
- The `foo` property does **not** get its own `references` row in this phase. Rationale: resolving the member access requires walking into the imported file's exports, which is a separate cross-file resolution step. The `import_use` reference already records that the importing file uses `ns`, and the `imports` row records the module relationship.

Exception: when `ns.foo` appears in a **type position** (e.g. `const x: ns.Foo`), the `nested_type_identifier` node `ns.Foo` is treated as a single name for resolution, and the resolver tries to canonicalise it to `<resolved_path>::Foo` (see `types-typescript.md`). One `type_use` reference row is emitted, with `referent_id` = symbol id of `Foo` in the imported file when known, else `null`.

## `ref_kind` decision tree

Every emitted row has exactly one `ref_kind`. The cases below are checked in order; the first match wins.

### `import_use`

The identifier appears inside an `import_statement` or an `export_statement` whose `source` is a module specifier (i.e., a re-export). This covers:

- `import { foo } from "…"` — `foo` is `import_use`. If the binding is renamed `{ foo as myFoo }`, both `foo` and `myFoo` are `import_use`.
- `import Default from "…"` — `Default` is `import_use`.
- `import * as ns from "…"` — `ns` is `import_use`.
- `export { foo } from "…"` — `foo` is `import_use` (re-export).
- `import "./polyfill"` — no identifier, no row.
- `const x = require("…")` — the `require` identifier itself is **not** `import_use`; the call is captured in the `imports` relation already. The bound name `x` gets bound to the module via the existing `imports` row; the `x` occurrence on the left-hand side is `write` per the rules below.
- `const x = import("…")` (dynamic): identical treatment to `require`. The `import` keyword is not an identifier.

Each `import_use` row's `referent_id` is the symbol id of the imported declaration in the *target* file, when `resolve_import` resolves the specifier *and* the target file's symbol extraction has produced a matching symbol. Otherwise `null`. External (bare) imports always have `referent_id = null` because the target is outside the indexed workspace.

The `site_start_byte` is the byte offset of the identifier occurrence, not the import statement.

### `type_use`

The identifier appears in a type position. Concretely: anywhere a `type` row would be emitted per `types-typescript.md`. Includes:

- Parameter type annotations.
- Return type annotations.
- Variable / field type annotations.
- Generic type arguments.
- `extends` / `implements` clause type identifiers.
- Type assertions: `x as Foo`, `<Foo>x`.
- Type-alias right-hand side.
- `typeof X` in a type position (the `X` is `type_use`, even though it's a value name in source).

`referent_id` resolution uses the scope walk above. For `type_identifier`s the lookup checks type-level bindings (interfaces, classes, type aliases, enums) and value-level bindings (classes — `class C` introduces both a value binding and a type binding). Generic `type_parameter` names resolve to the enclosing parameter list's declaration; `referent_id` = symbol id of the `type_parameter` if we emit those as symbols, else `null`. **Decision: we do not emit `type_parameter`s as `symbol` rows** in this phase (they are language attributes — see `attrs-typescript.md`). So a usage of `T` inside a generic function resolves to `referent_id = null`.

JavaScript files emit **zero** `type_use` rows. They have no type positions.

### `write`

The identifier is the target of an assignment or a mutation:

- `x = expr` — the `x` on the left is `write`. (Property accesses `obj.x = …` do not produce a `write` row for `x`; the *property* is not in our symbol table. The `obj` part is `read`.)
- Compound assignment: `x += 1`, `x -= 1`, `x *= 1`, `x /= 1`, `x %= 1`, `x **= 1`, `x &&= …`, `x ||= …`, `x ??= …`, `x <<= …`, `x >>= …`, `x >>>= …`, `x &= …`, `x |= …`, `x ^= …` — `x` is `write` (and also a logical `read`, but per the "first match wins" rule we emit `write` only). **Rationale:** compound assignments mutate-and-read; we record the mutation. Queries that need to surface "reads that are also writes" can filter on the operator pattern in source.
- Increment/decrement: `x++`, `++x`, `x--`, `--x` — `x` is `write`.
- Destructuring assignment target: `[a, b] = …`, `({ x } = …)` — each bound name is `write`. **Caveat:** the existing symbol extractor *skips* destructuring on the left-hand side of `const`/`let` (see test `destructured_variables_skipped`). For references, destructured targets that are *not* declarations (i.e., `({ a } = obj)` without a `const`/`let`) **are** `write` references to existing bindings. Destructured **declarations** (`const { a } = obj`) declare new symbols and the `a` occurrence is the declaration itself, not a reference — no row emitted for `a` here. The `obj` on the right is `read`.
- `delete obj.x` — no row for `x` (not a symbol). `obj` is `read`.
- Function parameter default: the parameter itself is a declaration (no row). A `=` in `function f(x = computeDefault())` is *not* an assignment to a pre-existing binding.

### `read`

Default for any identifier in a value position that didn't match `write`, `import_use`, or `type_use`. Includes:

- Function call: `foo()` — `foo` is `read`. Each argument identifier is `read`.
- Property access object: `obj.x` — `obj` is `read`.
- Computed access: `obj[key]` — both `obj` and `key` are `read`.
- Template-literal expressions: `` `hello ${name}` `` — `name` is `read`.
- Conditional / logical / arithmetic / comparison operands.
- JSX expression children (TSX/JSX): `<Foo bar={baz} />` — `Foo` is `read`, `baz` is `read`. The attribute key `bar` is not an identifier reference (it's a property name).
- `typeof x` in a **value** position — `x` is `read`. (`typeof X` in a type position is `type_use` per the `type_use` rule above.)
- `instanceof Cls` — `Cls` is `read`.
- `new Cls(args)` — `Cls` is `read`.
- `class C extends Base {}` — `Base` is `read` (it's a value expression in JS semantics, even though we *also* emit an `extends` graph edge).
- `super` and `this` — **no row emitted**. Rationale: `this` resolves contextually (lexical for arrow functions, dynamic otherwise) and `super` is a syntactic form, not a name. Querying `this` usage is better served by a dedicated relation if ever needed.

### Identifiers that get **no** row

- Identifiers that are *declarations* (the `name` field of a `function_declaration`, `class_declaration`, `interface_declaration`, `variable_declarator` LHS, etc.). These produce `symbol` rows, not `references` rows.
- Property keys in object literals (`{ key: value }`) — `key` is a property name, not an identifier reference.
- Method names in classes — declarations, not references.
- Labels in labeled statements (`outer: for (…)`) — not symbol references.
- Parameters in their own declaration position.

## `referent_id` resolution

Algorithm (per identifier occurrence that does qualify for a row):

1. Determine the enclosing symbol → `referrer_id`. Walk up the AST from the identifier until hitting a `function_declaration`, `method_definition`, `class_declaration`, `interface_declaration`, `type_alias_declaration`, `enum_declaration`, or a `variable_declarator` whose value is a function/arrow/class. Use that symbol's ADR-0002 id. If no symbol-bearing ancestor exists, the identifier is at module top level — `referrer_id` is a synthetic module-level symbol id: `<path>|0|0|<module>|module`. *Decision:* we emit module-scope synthetic ids rather than dropping the row, because top-level uses (e.g., `app.use(helmet());` in `src/app.js`) are load-bearing for "who calls helmet" queries.
2. Build a per-file scope tree from the AST. This is **not** the existing `symbols_by_name` index — that index is module-flat and would conflate inner shadowed bindings with the outer one. The scope tree is constructed fresh per file during reference extraction.
3. Walk the scope tree outward from the occurrence's enclosing scope. The first scope that binds the identifier name wins; record its symbol id. Bindings include: parameters, locally-declared `const`/`let`/`var`, functions and classes declared in scope, imports at module scope.
4. If no scope binds the name, check the per-file `imports` rows: if any `local_name` matches, use that as the binding and resolve `referent_id` via the imports' `module_specifier` + `imported_name` → the resolved-file symbol (when `resolve_import` succeeds and that file's `symbols_by_name` has an entry for `imported_name`). Otherwise `referent_id = null`.
5. If still nothing matches, `referent_id = null` and the row is still emitted (we want unresolved-reference visibility for diagnostics — queries that don't care can filter `referent_id != null`).

### Behavior when multiple candidates exist

Single-scope: per ECMAScript, two `let` declarations with the same name in the same scope is a syntax error. Two `var` declarations with the same name are merged into one binding. **We pick the first declaration position as the canonical symbol** and resolve every occurrence to it. If both a `var` and a parameter share a name, the parameter wins (parameter scope is inner to the function body).

Imports vs locals: a local declaration shadows an import. The scope walk handles this naturally — local scope is checked before the module-scope import bindings.

Class scope: a method named `foo` and a top-level `function foo` — inside the class, `this.foo()` resolves to the method (`referent_id = method symbol id`). A bare `foo()` inside the same method resolves to the **top-level function**, not the method. Rationale: bare names in JS/TS methods do not see class members; this is the language's actual rule.

### Behavior when no candidate exists

Emit the row with `referent_id = null`. Always emit — never skip.

### Resolver implementation note

Per ADR-0003, every language module owns its scope rules — we do **not** share resolver code across languages. The TS/JS resolver builds a per-file scope tree (not the global `symbols_by_name` index) so it correctly handles shadowing. The `symbols_by_name` index is consulted only at step 4, for resolving an imported name's target symbol in another file.

## Worked examples

Every example is sourced from the benchmark corpora. Symbol IDs follow ADR-0002. `site_start_byte` values are placeholders shown as `<byte>`; the implementation reads them from the tree-sitter `Range` of the identifier node.

### Example 1 — `read` + `import_use` + closure-captured `write` to a non-local

**Source** — `src/middleware/logger.js:7-32`:

```js
const _requestLog = [];

function requestLogger(req, res, next) {
  const start = Date.now();

  res.on('finish', function () {
    const duration = Date.now() - start;
    const entry = {
      method: req.method,
      path: req.path,
      status: res.statusCode,
      duration,
      ip: req.ip,
      userAgent: req.headers['user-agent'],
      timestamp: new Date().toISOString(),
    };

    _requestLog.push(entry);

    if (res.statusCode >= 400) {
      console.error(`[${entry.timestamp}] ${req.method} ${req.path} - ${res.statusCode} (${duration}ms)`);
    }
  });

  next();
}
```

`referrer_id` for occurrences inside `requestLogger` (and its nested anonymous `res.on` callback): `src/middleware/logger.js|9|0|requestLogger|function`. The anonymous callback at line 12 has no name and is not declared as a symbol — we attribute its body's references to the enclosing named function. Rationale: per ADR-0002 the symbol id needs a `name`, and we have established (matching the existing extractor) that bare function-expression arguments are not symbols.

`references` rows (selected; not exhaustive):

| referrer_id                                              | referent_id                                                    | ref_kind     | site_file                  | site_start_byte |
|----------------------------------------------------------|----------------------------------------------------------------|--------------|----------------------------|-----------------|
| `…\|9\|0\|requestLogger\|function`                       | `null` (global)                                                | `read`       | `src/middleware/logger.js` | `<byte of Date>` |
| `…\|9\|0\|requestLogger\|function`                       | `…\|9\|22\|res\|parameter` *(or `null`; see note)*             | `read`       | `src/middleware/logger.js` | `<byte of res>`  |
| `…\|9\|0\|requestLogger\|function`                       | `null` (global `Date`)                                         | `read`       | `src/middleware/logger.js` | `<byte of Date>` |
| `…\|9\|0\|requestLogger\|function`                       | `src/middleware/logger.js\|11\|2\|start\|variable`             | `read`       | `src/middleware/logger.js` | `<byte of start at line 13>` |
| `…\|9\|0\|requestLogger\|function`                       | `src/middleware/logger.js\|7\|0\|_requestLog\|variable`        | `read`       | `src/middleware/logger.js` | `<byte of _requestLog>` |
| `…\|9\|0\|requestLogger\|function`                       | `null` (global `console`)                                      | `read`       | `src/middleware/logger.js` | `<byte of console>` |
| `…\|9\|0\|requestLogger\|function`                       | `null` (the `next` parameter resolves; see below)              | `read`       | `src/middleware/logger.js` | `<byte of next>` |

**Closure-captured write.** Line 56 (`clearRequestLog`):

```js
function clearRequestLog() {
  _requestLog.length = 0;
}
```

`_requestLog.length = 0` is a property write, not a write to `_requestLog` itself. We emit a `read` for `_requestLog` (no symbol for `.length`). To meet the contract requirement of "one write to a closure-captured variable", consider line 24 inside the `res.on('finish', …)` callback: `_requestLog.push(entry);` — again a method call, `_requestLog` is `read`.

A true closure-captured **write** appears in `src/types/common.ts:46-50` not at all (no example); the canonical TS benchmark example is `src/lib/cache.ts:88-94`:

```ts
export function getCacheStats(): { size: number; totalHits: number } {
  let totalHits = 0;
  cache.forEach((entry) => {
    totalHits += entry.hits;
  });
  return { size: cache.size, totalHits };
}
```

The arrow callback `(entry) => { totalHits += entry.hits; }` writes to `totalHits` — which is declared in the *enclosing* `getCacheStats` function scope, captured by the closure. Both the arrow body's identifier `totalHits` (a `+=` compound assignment) and the outer `let totalHits = 0` declaration are visible.

`references` rows for the `totalHits += entry.hits` site:

| referrer_id                                                  | referent_id                                                  | ref_kind | site_file              | site_start_byte |
|--------------------------------------------------------------|--------------------------------------------------------------|----------|------------------------|-----------------|
| `src/lib/cache.ts\|88\|0\|getCacheStats\|function`           | `src/lib/cache.ts\|89\|2\|totalHits\|variable`               | `write`  | `src/lib/cache.ts`     | `<byte of totalHits at line 91>` |
| `src/lib/cache.ts\|88\|0\|getCacheStats\|function`           | `null` (`entry` is the arrow's parameter, not a top-level symbol) | `read`   | `src/lib/cache.ts`     | `<byte of entry>` |

Two notes:
- The arrow function on line 90 is not a declared symbol (it's an inline expression), so `referrer_id` is the enclosing named function `getCacheStats`. This is consistent with the "anonymous-callbacks-belong-to-enclosing-named-symbol" rule above.
- `entry` resolves to `null` because the arrow's parameters are not extracted as symbols by the existing TS extractor (parameters are part of the `parameter` relation, keyed to their function; they are not standalone `symbol` rows). The reference is still emitted with `referent_id = null` so downstream queries can see the use.

### Example 2 — Parameter shadowing a top-level constant (shadowing #1)

**Source** — `src/lib/auth.ts:60-64`:

```ts
export function hasRole(requiredRole: string): boolean {
  const session = getSessionInfo();
  if (!session) return false;
  return session.role === requiredRole || session.role === 'admin';
}
```

There is no name collision in this exact snippet; let's switch to one that *does* shadow. **Source** — `src/lib/cache.ts:43-55`:

```ts
export function cacheGetOrSet<T>(
  key: string,
  fetcher: () => Promise<T>,
  ttlMs?: number
): Promise<T> {
  const cached = cacheGet<T>(key);
  if (cached !== null) return Promise.resolve(cached);

  return fetcher().then((data) => {
    cacheSet(key, data, ttlMs);
    return data;
  });
}
```

The parameter `key` shadows nothing here (no top-level `key`), so this isn't quite shadowing either. The cleanest benchmark-real shadowing example is at the inner-function level in `src/utils/formatters.ts:107-176` (`formatTableData`): the parameter `value` (no — there is no top-level `value`). The benchmark does not contain a real parameter-shadows-module-binding case in this small corpus.

**Decision:** we commit to the following resolver behavior for shadowing, and accept that the smallest worked example must be synthesised against the contract — the resolver must still pass it.

Synthesised but contract-binding case (the resolver MUST produce these rows when fed this exact source):

```ts
// File: src/types/user.ts (imaginary trailing addition)
const session = "module-level";

export function hasRole(session: string): boolean {  // parameter shadows
  return session === "admin";  // reads parameter, NOT the const
}
```

`references` rows:

| referrer_id                                                | referent_id                                                  | ref_kind |
|------------------------------------------------------------|--------------------------------------------------------------|----------|
| `src/types/user.ts\|<line>\|0\|hasRole\|function`          | parameter declaration: shown as `null` (params are not symbols) | `read`   |

The `session === "admin"` `read` of `session` resolves to the **parameter**, not the module-level `const session`. Because parameters are not in the `symbol` table (they are in the `parameter` table keyed to their function), `referent_id = null`. The row's existence and `ref_kind = read` is the load-bearing fact; the explicit binding can be recovered by joining `parameter.function_id = referrer_id` and matching `parameter.name = "session"`.

**Real benchmark example of class-member-vs-parameter shadowing:** none of the small benchmark corpora contains a class with a method whose parameter shadows a field. We commit to: bare `x` inside a method resolves to the **innermost** local binding (parameter or local `let`), never to a class field `x` — only `this.x` would. This matches the language rule.

### Example 3 — Block-scoped `let` shadowing a parameter (shadowing #2)

**Source** — `src/utils/formatters.ts:107-130` (excerpt):

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
```

Inside the inner `for` loop, `value` is a block-scoped `const`. There is no outer `value` to shadow here, but `i` and `j` cleanly demonstrate block scoping: `j` lives only inside the inner `for`. A use of `j` outside that block would be `referent_id = null` (no binding in scope, no import match).

Selected `references` rows for the body of `formatTableData`:

| referrer_id                                                          | referent_id                                                                       | ref_kind |
|----------------------------------------------------------------------|-----------------------------------------------------------------------------------|----------|
| `src/utils/formatters.ts\|107\|0\|formatTableData\|function`         | `null` (param `rows` — not a symbol)                                              | `read`   |
| `src/utils/formatters.ts\|107\|0\|formatTableData\|function`         | `src/utils/formatters.ts\|109\|2\|formatted\|variable`                            | `read`   |
| `src/utils/formatters.ts\|107\|0\|formatTableData\|function`         | `null` (loop-var `i` is block-scoped; not a top-level symbol)                     | `write`  |
| `src/utils/formatters.ts\|107\|0\|formatTableData\|function`         | `null` (`i` read inside the loop body)                                            | `read`   |
| `src/utils/formatters.ts\|107\|0\|formatTableData\|function`         | `null` (`j`, `col`, `value`, `row` — all block-scoped, no symbol entry)           | various  |

The `formatted.push(formattedRow);` at line 173: `formatted` is the function-body `const`, **not** a parameter — but again it's not a top-level symbol, so `referent_id = null`. The row exists with `ref_kind = read`.

**This is the load-bearing constraint:** the row count and `ref_kind` for every identifier occurrence must be correct, even when `referent_id` is `null`. Downstream "find writes to a captured variable" queries reconstruct the binding by joining `parameter` and `variable` symbol rows on `(file, name, line range)`.

### Example 4 — `import_use` (CommonJS `require`)

**Source** — `src/app.js:6-21`:

```js
const express = require('express');
const bodyParser = require('body-parser');
const cors = require('cors');
const morgan = require('morgan');
const helmet = require('helmet');

const postRoutes = require('./routes/posts');
const userRoutes = require('./routes/users');
const commentRoutes = require('./routes/comments');
const authRoutes = require('./routes/auth');
const mediaRoutes = require('./routes/media');
const tagRoutes = require('./routes/tags');
const categoryRoutes = require('./routes/categories');

const errorHandler = require('./middleware/errorHandler');
const { requestLogger } = require('./middleware/logger');
```

Each `require(...)` call is already captured in `imports` (kind = `"require"`). For `references`, the **bound LHS name** (`express`, `bodyParser`, `postRoutes`, …) is a *declaration*, not a reference — no row.

When later code uses one of these — e.g. line 24 `const app = express();` — the occurrence of `express` produces a `read` row, **not** an `import_use`. Rationale: `import_use` is reserved for occurrences *inside* `import`/`export` syntax. A use elsewhere is a normal `read`. The fact that the bound symbol came from an import is recoverable via the `imports` row.

Selected rows for `createApp` (line 23):

| referrer_id                                       | referent_id                                                    | ref_kind   | site_file     |
|---------------------------------------------------|----------------------------------------------------------------|------------|---------------|
| `src/app.js\|23\|0\|createApp\|function`          | `src/app.js\|6\|0\|express\|variable`                          | `read`     | `src/app.js`  |
| `src/app.js\|23\|0\|createApp\|function`          | `null` (`app` is block-scoped function-local, no symbol)       | `read`     | `src/app.js`  |
| `src/app.js\|23\|0\|createApp\|function`          | `src/app.js\|10\|0\|helmet\|variable`                          | `read`     | `src/app.js`  |
| `src/app.js\|23\|0\|createApp\|function`          | `src/app.js\|12\|0\|postRoutes\|variable`                      | `read`     | `src/app.js`  |
| `src/app.js\|23\|0\|createApp\|function`          | `src/app.js\|20\|0\|errorHandler\|variable`                    | `read`     | `src/app.js`  |
| `src/app.js\|23\|0\|createApp\|function`          | `src/middleware/logger.js\|<line>\|<col>\|requestLogger\|function` *if resolvable; otherwise `null`* | `read` | `src/app.js`  |

The last row is the most interesting: `requestLogger` came in via `const { requestLogger } = require('./middleware/logger');`. The `imports` row for this destructured require has `imported_name = "*"` and `local_name = "*"` per the current extractor's behavior (require destructuring is not deconstructed). Thus the resolver cannot directly map `requestLogger` → a specific symbol in the target file; it falls back to `null` for `referent_id`. **Documented limitation:** improving require-destructuring resolution is a follow-up; the contract today is `referent_id = null` for names bound via destructured `require`.

ES-module style is different. `src/types/user.ts` `import { User } from "./user"` produces an `imports` row with `imported_name = "User"` and `local_name = "User"`. Subsequent uses of `User` (as a type) emit a `type_use` row whose `referent_id` is the `User` interface in `src/types/user.ts`.

### Example 5 — `type_use` + `read` + `write` mix

**Source** — `src/lib/cache.ts:12-23`:

```ts
export function cacheGet<T>(key: string): T | null {
  const entry = cache.get(key);
  if (!entry) return null;

  if (Date.now() > entry.expiresAt) {
    cache.delete(key);
    return null;
  }

  entry.hits++;
  return entry.data;
}
```

`referrer_id`: `src/lib/cache.ts|12|0|cacheGet|function`.

Selected rows (the type-use rows correspond to the signature; the value-position rows are inside the body):

| referrer_id                                | referent_id                                                | ref_kind   | site_file          |
|--------------------------------------------|------------------------------------------------------------|------------|--------------------|
| `…\|cacheGet\|function`                    | `null` (type parameter `T`)                                | `type_use` | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (primitive `string` — ambient)                      | `type_use` | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (type parameter `T`, return position)               | `type_use` | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (literal `null` type)                               | `type_use` | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `src/lib/cache.ts\|10\|0\|cache\|variable`                 | `read`     | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (param `key`)                                       | `read`     | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (local `entry`, block-scoped)                       | `read`     | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (global `Date`)                                     | `read`     | `src/lib/cache.ts` |
| `…\|cacheGet\|function`                    | `null` (local `entry`, in `entry.hits++`)                  | `read`     | `src/lib/cache.ts` |

The `entry.hits++` line: `entry` is `read` (we don't have property-level symbols, so `.hits` is not a row). The `++` is a mutation of `entry.hits`, but in our symbol table the closest writable target is `entry` itself — and `entry` is `const`-bound, so the write is to its *property*. **Decision:** we do **not** emit a `write` row for `entry` here. The `++` operator on a member expression produces a `read` row for the object identifier. Rationale: the schema doesn't model property writes (no `field` symbol for `.hits`); a `write` row for `entry` would be misleading because `entry` itself is `const`.

The `cache.delete(key)` line: `cache` is `read`, `key` is `read` (param use). No `write` for `cache`.

### Example 6 — `write` via compound assignment, true variable

**Source** — `src/hooks/useFetch.ts:38`:

```ts
  const refetch = () => setRefetchTrigger((prev) => prev + 1);
```

`setRefetchTrigger` is the React state setter destructured from `useState`. The arrow function increments via `prev + 1`. The state setter call itself is a `read` of `setRefetchTrigger`. Inside, the arrow's `prev` is a parameter, not a symbol → `null`. There is **no** `write` row here — `setRefetchTrigger(prev => prev + 1)` is a function call, not an assignment to a bound name.

For a true compound-write benchmark example, see `src/lib/cache.ts:88-94` (Example 1 above): `totalHits += entry.hits` produces a `write` row on `totalHits`, which **is** a `let`-declared variable in the enclosing function — but again, it's not a top-level `symbol` row. The `write` is recorded with `referent_id = null`.

To see a real `write` resolving to a non-null symbol, a top-level mutable binding is needed. The benchmarks avoid module-level `let`. The contract still binds: if such a binding exists, the resolver MUST produce `referent_id = <module-level symbol id>` for compound assignments to it.
