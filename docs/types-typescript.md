# Types — TypeScript / JavaScript

This contract covers `.ts`, `.tsx`, `.js`, `.jsx`. They share a tree-sitter grammar family and a single extractor lives under `src/languages/typescript/`. Where TS and JS diverge (chiefly: JS has no type annotations), the rules below state both behaviors explicitly.

`language` column on every emitted `type` row is `"typescript"` for `.ts`/`.tsx` and `"javascript"` for `.js`/`.jsx`. The grammar variant (`typescript` vs `tsx`) does not affect schema output.

## Tree-sitter node kinds

The TypeScript grammar exposes the following nodes as type expressions (they appear as the value of a `type_annotation`, the value of a `type_alias_declaration`, a generic argument, a heritage clause, etc.). Each row maps a node kind to its schema `type.kind` variant.

| tree-sitter node           | source example                  | schema `kind`  | notes |
|----------------------------|---------------------------------|----------------|-------|
| `predefined_type`          | `string`, `number`, `boolean`, `any`, `unknown`, `void`, `never`, `null`, `undefined`, `symbol`, `bigint`, `object` | `primitive` | One reserved keyword per token. `null` and `undefined` as type literals are also `predefined_type`. |
| `type_identifier`          | `User`, `Date`                  | `named`        | Bare named reference. |
| `nested_type_identifier`   | `React.FC`, `Express.Request`   | `named`        | Qualified name — `display_name` keeps the dots. |
| `generic_type`             | `Record<string, any>`, `Promise<T>`, `Array<User>` | `generic` | Always wraps a `type_identifier` / `nested_type_identifier` plus a `type_arguments` child. |
| `union_type`               | `string \| number`              | `union`        | n-ary; tree-sitter nests left-associatively but we flatten — see "`display_name` construction" below. |
| `intersection_type`        | `A & B`                         | `intersection` | Flattened the same way as `union_type`. |
| `function_type`            | `(req: Request) => void`        | `function`     | Includes arrow-style function types and `new (...) => T` constructor types. |
| `constructor_type`         | `new () => Foo`                 | `function`     | Folded into `function`. Schema has no `constructor_type` variant; the `new` keyword is preserved in `display_name`. |
| `tuple_type`               | `[string, number]`              | `tuple`        | Includes labeled tuple elements (`[first: string, second: number]`) and rest elements (`[string, ...number[]]`). |
| `array_type`               | `number[]`                      | `array`        | The `T[]` shorthand only. `Array<T>` parses as `generic_type` and is emitted as `generic` — we do **not** normalise the two to the same row. Rationale: the `display_name` differs, the `type.id` differs, and we want queries to be able to count `[]` vs `Array<>` usage as distinct facts. |
| `literal_type`             | `'admin'`, `42`, `true`         | `named`        | Treated as a `named` type whose `display_name` is the source text (including quotes for string literals). Rationale: querying `type.display_name = "'admin'"` is the cheapest way to surface stringly-typed literal-union usage flagged in `nextjs-dashboard`. |
| `object_type` / `type_literal` | `{ id: number; name: string }` | `named`    | Anonymous object types: `display_name` is the whitespace-normalised source text (see below). `canonical_name = null`. Rationale: there is no `kind = "object"` variant in the schema and structurally-typed object types do not map cleanly to `tuple` or `named`; we treat them as opaque `named` with no canonical resolution. |
| `parenthesized_type`       | `(string \| number)`            | (transparent)  | Strip the parens; emit one row for the inner type. The inner type's `kind` wins. |
| `type_predicate`           | `x is string`                   | `function`     | Appears only as the return type of a function. We emit it as `function` with the literal source as `display_name`. |
| `index_type_query`         | `typeof user`                   | `named`        | `display_name` keeps the `typeof` prefix. `canonical_name = null` (resolving requires type-checking, not parsing). |
| `lookup_type`              | `User["role"]`                  | `named`        | Indexed-access types. `display_name` is the source text. `canonical_name = null`. |
| `conditional_type`         | `T extends U ? X : Y`           | `named`        | Recorded as opaque `named`. `canonical_name = null`. Rationale: conditional types are not a first-class schema kind; flattening them would lose information. |
| `mapped_type_clause` (inside `object_type`) | `{ [K in keyof T]: ... }` | `named` | The enclosing `object_type` row is emitted; the mapped clause does not get its own row. |
| `readonly_type`            | `readonly number[]`             | (transparent + attr) | Strip the `readonly` modifier; emit the inner type. The `readonly` flag flows into `typescript_attrs.is_readonly` for the *enclosing symbol* (parameter, field) — not the type row. |
| `template_literal_type`    | `` `prefix_${string}` ``        | `named`        | `display_name` is the source text. `canonical_name = null`. |

Node kinds that look like types but get **no** `type` row:

- `type_parameter` (the `T` in `function foo<T>(x: T)`) — these are type-parameter declarations, not type usages. They populate `typescript_attrs.type_parameters` and otherwise do not appear in the `type` relation. A usage of `T` *as* a type annotation parses as `type_identifier` and gets its own row with `canonical_name = null` (see scope walk).
- `extends_clause`, `implements_clause` — emitted as `extends` / `implements` edges by the existing extractor, not as `type` rows. The type-identifier inside an `extends_clause` *does* get a `type` row (so `type_use` references resolve) — but the heritage relationship itself is a graph edge, not a type fact.

### JavaScript divergence

`.js` and `.jsx` files have no type annotations. Concretely:

- `parameter.type_id = null` for every parameter.
- No `returns_type` row is emitted.
- No `throws` row is emitted (JS lacks `throws` declarations entirely; TS does too in practice, but JSDoc `@throws` is out of scope for Level 3).
- The `type` relation receives **zero rows** sourced from `.js`/`.jsx` files. JSDoc type comments are not parsed in this phase. Rationale: contract-doc-grade fidelity for JSDoc requires a separate parser pass and is explicitly out of scope per ADR-0003's Level-3 commitment.
- `typescript_attrs` rows for JS symbols use defaults — see `attrs-typescript.md`.

## `display_name` construction

The `display_name` is the human-readable rendering of the type expression. Rules:

1. **Whitespace normalisation:** collapse any run of whitespace inside the type to a single space, then trim leading/trailing whitespace. `Vec< i32 >` and `Vec<i32>` and `Vec<\n  i32\n>` all produce `Vec<i32>`. TS does not use `Vec` (Rust example for parity) — concretely: `Record< string , any >` → `Record<string, any>`. We keep the single space after a comma in argument lists.
2. **Comma separator inside generic / tuple / function-param lists:** always `", "` (comma + single space), regardless of source.
3. **Union / intersection flattening:** the tree-sitter AST nests `A | B | C` as `union_type(union_type(A, B), C)`. We flatten to a single n-ary list and render with `" | "` between elements, in source order. Same rule for `&`.
4. **Generic arguments:** `Identifier<Arg1, Arg2>` — angle brackets adjacent to the identifier, no spaces inside the brackets.
5. **Array shorthand:** `T[]` — no space between `T` and `[]`. If `T` is a union or function type, parens are re-introduced (`(string | number)[]`) so `display_name` is unambiguous when read back. This is *added* parens — original source may omit them, the canonicaliser adds them when the inner type's precedence demands it. Specifically: `union_type`, `intersection_type`, `function_type`, `conditional_type` inside an `array_type` get parenthesised.
6. **Function types:** `(name1: T1, name2: T2) => R`. Parameter names are preserved if present in the source; if the source uses an unnamed parameter style like `(string, number) => void` (not legal TS but legal in some grammar dialects), we render it verbatim.
7. **Tuple types:** `[T1, T2]`. Labels preserved: `[first: string, second: number]`. Rest element: `[string, ...number[]]`.
8. **`readonly` modifier:** stripped from `display_name` even though it appears in source — the modifier is captured in `typescript_attrs` instead. Rationale: a `readonly number[]` parameter and a `number[]` parameter target the same `type` row; the readonly-ness is a property of the symbol-bearing position, not the type itself.
9. **Parens:** strip the outer parens of `parenthesized_type`; rule (5)'s inserted parens are the only ones that may appear.

`display_name` is plain text — no markdown, no escape sequences beyond what was in the source string-literal types.

## `canonical_name` resolution

Per ADR-0003, every `type` row gets a `canonical_name` when resolvable; otherwise `null`. Resolution is per-file and uses the same scope walk as `references-typescript.md`.

**Scope walk order** (for a bare `type_identifier`):

1. **Local type parameters in scope.** A `type_identifier` inside a function body whose name matches an enclosing `type_parameter` declaration (function generics, method generics, class generics) is treated as a type parameter. → `canonical_name = null` (type parameters do not get canonicalised because they are positional, not nominal).
2. **File-local declarations.** `interface`, `class`, `enum`, `type_alias_declaration` declared in the same file. → `canonical_name = "<file_path>::<declared_name>"`.
3. **Imports.** Walk the `imports` rows already emitted for this file. If a binding's `local_name` matches the identifier:
   - For internal imports (resolved to a workspace file via `resolve_import`): `canonical_name = "<resolved_path>::<imported_name>"`. If `imported_name == "default"`, use `"<resolved_path>::default"`.
   - For external imports (bare specifier like `"react"`): `canonical_name = "<module_specifier>::<imported_name>"`. We never try to walk into `node_modules`.
   - For namespace imports (`import * as ns`): a `nested_type_identifier` like `ns.Foo` canonicalises to `"<resolved_path>::Foo"` (internal) or `"<module>::Foo"` (external). A bare `ns` alone has no useful canonical form → `null`.
4. **Predefined types.** `string`, `number`, etc. → `canonical_name = "typescript::primitive::<name>"`. These are emitted with `kind = "primitive"` (see node-kind table).
5. **Global ambient types.** `Promise`, `Array`, `Record`, `Map`, `Set`, `Date`, `Error`, `RegExp`, `Object`, `Function`, `JSON`, `Math`, `console`, `Window`, `Document`, `Element`, `HTMLElement`, `Node` — these and their friends live in `lib.es*.d.ts` / `lib.dom.d.ts` which we do not index. We canonicalise an allow-listed subset to `"typescript::global::<name>"`. The allow-list lives next to the resolver in code; the doc commits to **at minimum** the names just listed.
6. **Otherwise:** `canonical_name = null`.

**Aliases.** `type Foo = Vec<u8>;` — `Foo` declared in the file canonicalises to `"<file_path>::Foo"`. A usage of `Foo` elsewhere in the same file *also* canonicalises to `"<file_path>::Foo"`. We do **not** dereference the alias to `"<file_path>::Vec<u8>"`. Rationale: the alias is the named concept the developer wrote, and downstream "find usages of `Foo`" queries should work on the alias name, not the expansion.

**Generic types.** `Promise<User>`:
- The outer `generic_type` row's `display_name` is `Promise<User>`. Its `canonical_name` is `canonical_name(Promise)` (e.g. `"typescript::global::Promise"`) — the **outer** type's canonical name ignores the generic arguments. Rationale: a query asking "all uses of `Promise`" should match `Promise<User>`, `Promise<void>`, etc., without needing to parse `display_name`.
- The inner `User` (the type argument) gets its **own** `type` row, with its own `display_name = "User"` and its own `canonical_name`.

**Compound types.** `union_type`, `intersection_type`, `function_type`, `tuple_type`, `array_type` rows: `canonical_name = null` unconditionally. Their constituent types get their own rows. Rationale: there is no useful nominal canonical form for "the type `A | B`"; queries that need to find unions can filter on `kind = "union"`.

**Anonymous object types** (`type_literal`): `canonical_name = null`.

**JavaScript files:** since no `type` rows are emitted, `canonical_name` resolution does not run. (The same scope walk *does* run for `references-typescript.md`, but that's for value-position identifiers.)

## Identity

Per ADR-0003: `type.id = blake3(language | file_id | display_name)`.

Inputs joined by `|` (literal pipe), in that order, with `language` ∈ {`"typescript"`, `"javascript"`}, `file_id` the file path (per ADR-0002), and `display_name` the normalised string from the construction rules above. The normalisation is the same one applied to the emitted column, so two textually-different source occurrences that normalise to the same `display_name` produce the same `id` and dedup naturally within a file.

The hash is computed over the bytes of the joined string. No language-specific extra normalisation beyond the rules in "`display_name` construction" above.

## Field types — `field_type` relation

Per the schema, every TypeScript class field or interface property with a typed declarator emits a `field_type {symbol_id, type_id}` row linking the field symbol to its `type` row. JavaScript files emit no `field_type` rows (no type annotations). Local variables and function parameters are not fields and use `parameter` / `references` wiring instead.

## Worked examples

Every example is sourced from `../virgil-skills/benchmarks/{typescript/nextjs-dashboard,javascript/express-api}/`. Symbol IDs follow ADR-0002: `path|start_line|start_col|name|kind`. `start_byte` values use the tree-sitter `Range` of the relevant node, not the symbol. For brevity, the file-path prefix is the path relative to the benchmark workspace root.

### Example 1 — `primitive`, `named` (declaration + parameter)

**Source** — `src/utils/calculations.ts:4`:

```ts
export function calculatePercentage(value: number, total: number): number {
```

The function symbol id is `src/utils/calculations.ts|4|0|calculatePercentage|function`.

`type` rows emitted (deduped within the file):

| id (sketch)                              | kind        | language     | display_name | canonical_name                       |
|------------------------------------------|-------------|--------------|--------------|--------------------------------------|
| `blake3("typescript\|src/utils/calculations.ts\|number")` | `primitive` | `typescript` | `number`     | `typescript::primitive::number`      |

(One row, not three — `number` appears three times in the signature and dedups to one row by `id`.)

`parameter` rows:

| function_id (sketch shown as symbol_id) | index | name    | type_id (= id of `number` row above) | is_optional | has_default |
|-----------------------------------------|-------|---------|--------------------------------------|-------------|-------------|
| `…\|calculatePercentage\|function`      | 0     | `value` | `<number-id>`                        | false       | false       |
| `…\|calculatePercentage\|function`      | 1     | `total` | `<number-id>`                        | false       | false       |

`returns_type` row:

| function_id                          | type_id        |
|--------------------------------------|----------------|
| `…\|calculatePercentage\|function`   | `<number-id>`  |

### Example 2 — `generic` plus nested `primitive` argument

**Source** — `src/types/user.ts:15`:

```ts
  preferences: Record<string, any>;
```

This appears inside the `User` interface (line 4). The field declaration is a `property_signature`; for Level 3 we treat interface fields the same as parameters w.r.t. type rows — each annotated position contributes a row.

`type` rows:

| id (sketch) | kind        | language     | display_name              | canonical_name                       |
|-------------|-------------|--------------|---------------------------|--------------------------------------|
| `<R-id>`    | `generic`   | `typescript` | `Record<string, any>`     | `typescript::global::Record`         |
| `<s-id>`    | `primitive` | `typescript` | `string`                  | `typescript::primitive::string`      |
| `<a-id>`    | `primitive` | `typescript` | `any`                     | `typescript::primitive::any`         |

Three rows. The outer `Record<string, any>` row canonicalises to `Record` only — its generic arguments are *separate* rows. A query "find all uses of `Record`" matches `<R-id>` directly without parsing `display_name`.

The `preferences` field on `User` is a `symbol` row with `kind = "field"` and `parent_id = "src/types/user.ts|4|0|User|interface"`. Whether `field` symbols get `parameter`-style rows or a separate `field_type` relation is decided at the schema level — this contract commits only to emitting the three `type` rows above and the existing `references` `type_use` row pointing at `<R-id>` (see `references-typescript.md`).

### Example 3 — `union` (literal-typed)

**Source** — `src/types/common.ts:57`:

```ts
export type SortDirection = 'asc' | 'desc';
```

Symbol id: `src/types/common.ts|57|0|SortDirection|type_alias`.

`type` rows from the *right-hand side* of the alias:

| id (sketch) | kind    | language     | display_name      | canonical_name |
|-------------|---------|--------------|-------------------|----------------|
| `<u-id>`    | `union` | `typescript` | `'asc' \| 'desc'` | `null`         |
| `<a-id>`    | `named` | `typescript` | `'asc'`           | `null`         |
| `<d-id>`    | `named` | `typescript` | `'desc'`          | `null`         |

The union has two flattened elements (rule 3 of `display_name` construction). String-literal types are `named` with quotes preserved in `display_name` (node-kind table rationale). All three `canonical_name = null`: literal types have no useful canonical form, and the union is a compound type.

There is no `parameter` / `returns_type` row here — `SortDirection` is itself a type-alias declaration, not a function signature.

The *left-hand side* — the name `SortDirection` — is the symbol, not a type usage. A subsequent file that imports and uses `SortDirection` *will* emit a `type` row with `display_name = "SortDirection"` and `canonical_name = "src/types/common.ts::SortDirection"`.

### Example 4 — `intersection` + reuse via `extends`

**Source** — `src/types/user.ts:18`:

```ts
export interface UserProfile extends User {
```

The `extends` clause is **not** a `type` row — it's an `extends` graph edge. However, the type-identifier `User` inside the heritage clause **does** produce a `type` row + a `type_use` reference (see `references-typescript.md`).

`type` rows (only the one from the heritage clause):

| id (sketch) | kind    | language     | display_name | canonical_name                |
|-------------|---------|--------------|--------------|-------------------------------|
| `<U-id>`    | `named` | `typescript` | `User`       | `src/types/user.ts::User`     |

Now a separate intersection example. **Source** — synthesised position (hypothetical line `src/types/user.ts:50` for illustration, but to keep this contract strictly benchmark-sourced, we use the real intersection-shaped construct from `src/lib/api.ts` if present; otherwise we commit to the *behaviour*):

For a real intersection in the benchmark — none of `nextjs-dashboard/src/**` uses the `&` type operator. We commit to the following behaviour for the next benchmark file that does:

Given `type Combined = User & UserProfile;`, the right-hand side produces:

| id (sketch) | kind           | language     | display_name          | canonical_name |
|-------------|----------------|--------------|-----------------------|----------------|
| `<i-id>`    | `intersection` | `typescript` | `User & UserProfile`  | `null`         |
| `<U-id>`    | `named`        | `typescript` | `User`                | `src/types/user.ts::User`        |
| `<UP-id>`   | `named`        | `typescript` | `UserProfile`         | `src/types/user.ts::UserProfile` |

Three rows. The intersection canonicalises to `null` (compound).

### Example 5 — `function` type + `array` type

**Source** — `src/utils/calculations.ts:26-30`:

```ts
export function calculateAverage(values: number[]): number {
  if (values.length === 0) return 0;
  const sum = values.reduce((acc, val) => acc + val, 0);
  return sum / values.length;
}
```

Signature-level rows:

| id (sketch) | kind        | language     | display_name | canonical_name                       |
|-------------|-------------|--------------|--------------|--------------------------------------|
| `<n-id>`    | `primitive` | `typescript` | `number`     | `typescript::primitive::number`      |
| `<arr-id>`  | `array`     | `typescript` | `number[]`   | `null`                               |

`parameter` row:

| function_id                       | index | name     | type_id     | is_optional | has_default |
|-----------------------------------|-------|----------|-------------|-------------|-------------|
| `…\|calculateAverage\|function`   | 0     | `values` | `<arr-id>`  | false       | false       |

`returns_type` row: `(function_id, <n-id>)`.

The arrow function `(acc, val) => acc + val` *inside* the body has no annotated parameters and no annotated return type (the parameters' types are inferred from `Array.prototype.reduce`'s overload). Per Level-3 commitment we do **not** infer types — these parameters emit `parameter` rows with `type_id = null`, exactly like JavaScript parameters. No `function`-kind `type` row is emitted for the arrow function itself because there is no `function_type` AST node here (it's a `function_expression`, not a type).

A `function`-kind type *is* emitted when the source has an explicit annotation like `const cb: (x: number) => string = …`. The function-type row for `(x: number) => string`:

| id (sketch) | kind       | language     | display_name           | canonical_name |
|-------------|------------|--------------|------------------------|----------------|
| `<f-id>`    | `function` | `typescript` | `(x: number) => string`| `null`         |

### Example 6 — `tuple`

**Source** — `src/hooks/useDebounce.ts:6`:

```ts
  const [debouncedValue, setDebouncedValue] = useState<T>(value);
```

This is a *value-position* tuple destructuring; the tuple comes from the return type of `useState<T>` which is `[T, Dispatch<SetStateAction<T>>]`. Since we do not infer types, we emit **no** tuple `type` row here.

For an explicit tuple-type annotation we commit to:

Given (synthesised but representative) `function range(): [number, number] { return [0, 1]; }`, the return type produces:

| id (sketch) | kind        | language     | display_name       | canonical_name                       |
|-------------|-------------|--------------|--------------------|--------------------------------------|
| `<t-id>`    | `tuple`     | `typescript` | `[number, number]` | `null`                               |
| `<n-id>`    | `primitive` | `typescript` | `number`           | `typescript::primitive::number`      |

Two rows. The tuple element rows dedup (`number` appears twice → one row).

### Example 7 — JavaScript file: zero type rows

**Source** — `src/middleware/auth.js:9-28`:

```js
function authenticate(req, res, next) {
  const authHeader = req.headers.authorization;
  // …
}
```

Symbol id: `src/middleware/auth.js|9|0|authenticate|function`.

`type` rows: **zero**.

`parameter` rows:

| function_id                                                 | index | name        | type_id | is_optional | has_default |
|-------------------------------------------------------------|-------|-------------|---------|-------------|-------------|
| `src/middleware/auth.js\|9\|0\|authenticate\|function`      | 0     | `req`       | `null`  | false       | false       |
| `src/middleware/auth.js\|9\|0\|authenticate\|function`      | 1     | `res`       | `null`  | false       | false       |
| `src/middleware/auth.js\|9\|0\|authenticate\|function`      | 2     | `next`      | `null`  | false       | false       |

No `returns_type` row (no annotation, and we do not infer).

This matches the rule stated at the top of "JavaScript divergence": every parameter in a `.js`/`.jsx` file gets `type_id = null`, no `returns_type` row, no `type` rows from this file.
