# Language attributes — TypeScript / JavaScript

This contract covers `.ts`, `.tsx`, `.js`, `.jsx`. One `typescript_attrs` table receives rows for symbols extracted from any of the four. The `language` column on the parent `symbol` row distinguishes TS from JS; the attrs table is intentionally unified.

## Schema

```
:create typescript_attrs {
    symbol_id: String =>
    is_readonly: Bool default false,
    is_optional: Bool default false,
    type_parameters: [String] default [],
}
```

Defined in `docs/virgil-datalog-schema.md` (Pattern 1, sparse extension table).

### Column applicability

| column            | applies to                                                          |
|-------------------|---------------------------------------------------------------------|
| `is_readonly`     | `field`, `parameter`, `variable`, `type_alias`. False otherwise.    |
| `is_optional`     | `field`, `parameter`. False otherwise.                              |
| `type_parameters` | `function`, `method`, `class`, `interface`, `type_alias`. Empty otherwise. |

A row is emitted for **every** TS/JS symbol — even when all values are defaults — so that joins against `typescript_attrs` cannot silently miss symbols. Rationale: a left join with COALESCE works either way, but emitting the row makes the per-language symbol set queryable as `*typescript_attrs{symbol_id}` directly.

### Symbol-kind reminder

The existing extractor produces `SymbolKind` values mapped to `symbol.kind` strings: `function`, `class`, `method`, `interface`, `type_alias`, `enum`, `variable`, `arrow_function`. We treat `arrow_function` the same as `function` for attrs purposes (a `const foo = <T,>(x: T) => …` arrow can carry type parameters). Parameters and fields are not currently emitted as `symbol` rows by the TS extractor — when they are added (a prerequisite for fully exercising `is_optional`), this contract is what their rows must conform to.

## Extraction rules

### `is_readonly`

`true` when any of:

- **Field on an interface or class:** the property signature/declaration has the `readonly` modifier (tree-sitter `accessibility_modifier` / `readonly` token child). E.g. `interface User { readonly id: number; }`.
- **Parameter:** the parameter is declared with `readonly` in a parameter-property (TypeScript constructor parameter shorthand): `constructor(readonly id: number) {}`.
- **Type alias / variable:** the *type* annotation is a `readonly_type` node (`readonly number[]`, `readonly [string, number]`). The modifier on a type expression (not on the symbol itself) sets the symbol's `is_readonly` to `true`. Rationale: `readonly` on a type annotation is a property of the binding, not of the type's identity (`number[]` and `readonly number[]` share a `type` row but the binding differs).
- **`const` declarations:** `const x = 1` does **not** set `is_readonly`. Rationale: `const` prevents reassignment of the binding but is not the `readonly` keyword. We keep the column tied to the explicit modifier; consumers can filter on `kind = "variable"` AND parent declaration being `lexical_declaration` with `const` separately if needed. Decision-rationale: overloading `is_readonly` with `const` semantics conflates "you can't reassign the binding" with "you can't mutate the value", which are distinct in TS.

`false` otherwise.

**JavaScript:** JS has no `readonly` keyword in source-level syntax (it's a TS-only modifier). `is_readonly = false` for every `.js`/`.jsx` symbol.

### `is_optional`

`true` when any of:

- **Parameter:** the parameter name has a `?` suffix: `function f(x?: number)`. Tree-sitter renders this with an `optional_parameter` or an `?` token child on the parameter node, depending on grammar version. Either form sets the flag.
- **Field:** the property signature has `?` after the name: `{ avatar?: string }`. Tree-sitter `property_signature` with an `?` token child.
- **Parameter with default value:** `function f(x = 5)` — we treat this as **not** `is_optional`. The default makes the parameter optional from a caller's perspective, but `is_optional` is reserved for the `?` syntax. Rationale: `has_default` is already a separate column on the `parameter` relation; double-flagging would muddle the distinction.

`false` otherwise.

**JavaScript:** JS has no `?` optional-parameter syntax. `is_optional = false` for every `.js`/`.jsx` symbol. JS parameters with default values populate `parameter.has_default = true` instead (handled in the `parameter` relation, not here).

### `type_parameters`

The list of declared type-parameter names in source order. Sourced from a `type_parameters` AST node:

- `function foo<T>(x: T)` → `["T"]`.
- `function foo<T, U extends number>(x: T, y: U)` → `["T", "U"]`. The `extends` constraint is **not** captured in `type_parameters` — only the parameter name. Rationale: constraints are a separate concept; if we ever need them, a `typescript_type_param` relation can be added. The current schema commits to names only.
- `class Container<T> { … }` → `["T"]`.
- `interface Boxed<T, U = string> { … }` → `["T", "U"]`. The default `= string` is dropped, same rationale as `extends` constraints.
- `type Pair<A, B> = [A, B]` → `["A", "B"]`.
- Method: `class C { foo<T>(x: T) {} }` — the method symbol's `type_parameters` is `["T"]`. Class type parameters do **not** flow into the method's `type_parameters`.

Empty list (`[]`) when no `type_parameters` node is present.

**Edge case — generic arrow assigned to const:**

```ts
export const useDebounce = <T,>(value: T, delay: number): T => { … };
```

(This shape is not used in the benchmarks but is legal TS.) The symbol kind is `arrow_function`; `type_parameters = ["T"]`. The `<T,>` syntax (trailing comma) is the only way to disambiguate generic arrows from JSX in `.tsx` files — the trailing comma is not part of the captured name, just `"T"`.

**JavaScript:** JS has no type parameter syntax. `type_parameters = []` for every `.js`/`.jsx` symbol.

### Edge cases (all kinds)

- **Conditional compilation:** TS/JS has none. No `cfg`-style ambiguity.
- **Decorators:** TypeScript decorators (`@foo`) do not affect any column in this table. (A separate `typescript_decorators` relation could be added later; out of scope for Level 3.)
- **`declare` / `ambient` declarations:** `declare const X: number` produces a symbol with `is_readonly = false`, `is_optional = false`, `type_parameters = []`. The `declare` modifier does not flow into any current attr column.
- **`abstract` classes / methods:** the `abstract` modifier flows into `symbol.is_abstract` (already in the core schema), not into `typescript_attrs`.
- **`export default`:** does not affect attrs.
- **JSDoc annotations in `.js`:** out of scope — we do not parse `/** @readonly */` or `/** @param {string=} */` to populate attrs. Defaults stand.

## Worked examples

Every example is sourced from the benchmark corpora. Symbol IDs follow ADR-0002 (`path|start_line|start_col|name|kind`).

### Example 1 — Generic function, no other modifiers

**Source** — `src/hooks/useDebounce.ts:5`:

```ts
export function useDebounce<T>(value: T, delay: number): T {
```

Symbol id: `src/hooks/useDebounce.ts|5|0|useDebounce|function`.

`typescript_attrs` row:

| symbol_id                                            | is_readonly | is_optional | type_parameters |
|------------------------------------------------------|-------------|-------------|-----------------|
| `src/hooks/useDebounce.ts\|5\|0\|useDebounce\|function` | false       | false       | `["T"]`         |

The `<T>` generic produces `type_parameters = ["T"]`. No `readonly`, no `?`.

### Example 2 — Generic interface

**Source** — `src/types/common.ts:3`:

```ts
export interface PaginatedResponse<T> {
  data: T[];
  total: number;
  page: number;
  limit: number;
  totalPages: number;
}
```

Symbol id for the interface: `src/types/common.ts|3|0|PaginatedResponse|interface`.

`typescript_attrs` row:

| symbol_id                                                     | is_readonly | is_optional | type_parameters |
|---------------------------------------------------------------|-------------|-------------|-----------------|
| `src/types/common.ts\|3\|0\|PaginatedResponse\|interface`     | false       | false       | `["T"]`         |

If/when field symbols are emitted, each property (`data`, `total`, `page`, `limit`, `totalPages`) gets its own row with all defaults (`is_optional = false`, `is_readonly = false`, `type_parameters = []`).

### Example 3 — Optional fields on an interface

**Source** — `src/types/user.ts:4-16`:

```ts
export interface User {
  id: number;
  email: string;
  name: string;
  role: string;
  status: string;
  avatar?: string;
  createdAt: string;
  updatedAt: string;
  lastLogin?: string;
  department: string;
  preferences: Record<string, any>;
}
```

The interface itself: `src/types/user.ts|4|0|User|interface` → row with all defaults (no `<T>`, no modifiers): `is_readonly=false, is_optional=false, type_parameters=[]`.

When field symbols are emitted (this becomes a hard requirement once the schema/extractor add field rows), the `avatar` and `lastLogin` fields get `is_optional = true`:

| symbol_id (sketch)                                            | is_readonly | is_optional | type_parameters |
|---------------------------------------------------------------|-------------|-------------|-----------------|
| `src/types/user.ts\|10\|2\|avatar\|field`                     | false       | true        | `[]`            |
| `src/types/user.ts\|13\|2\|lastLogin\|field`                  | false       | true        | `[]`            |

Every other field row: all defaults.

### Example 4 — JavaScript symbol (all defaults)

**Source** — `src/middleware/auth.js:9`:

```js
function authenticate(req, res, next) {
```

Symbol id: `src/middleware/auth.js|9|0|authenticate|function`.

`typescript_attrs` row:

| symbol_id                                                | is_readonly | is_optional | type_parameters |
|----------------------------------------------------------|-------------|-------------|-----------------|
| `src/middleware/auth.js\|9\|0\|authenticate\|function`   | false       | false       | `[]`            |

JS has no type parameters, no `readonly`, no `?`. The row is still emitted (per "applicability" rule above) so left joins are unnecessary.

### Example 5 — Non-obvious: `readonly` on a type alias's RHS

**Source** — synthesised against the contract; the benchmarks do not contain a `readonly_type` example. The resolver MUST emit the following:

```ts
// File: example.ts
type ReadonlyIds = readonly number[];
```

Symbol id: `example.ts|1|0|ReadonlyIds|type_alias`.

`typescript_attrs` row:

| symbol_id                                       | is_readonly | is_optional | type_parameters |
|-------------------------------------------------|-------------|-------------|-----------------|
| `example.ts\|1\|0\|ReadonlyIds\|type_alias`     | true        | false       | `[]`            |

Rationale: the right-hand side is a `readonly_type` node; per the extraction rule for `is_readonly` on a type alias, the symbol's `is_readonly = true`. The `type` row emitted for the RHS has `display_name = "number[]"` (the `readonly` modifier is stripped during display-name normalisation; see `types-typescript.md` node-kind table). The two facts are consistent: the type identity does not vary by `readonly`-ness, but the binding's readonly-ness is recorded on the symbol.

### Example 6 — Method-level generics distinct from class-level

**Source** — synthesised against the contract; benchmarks have generic interfaces but no generic methods.

```ts
class Container<T> {
  map<U>(fn: (x: T) => U): Container<U> { … }
}
```

Two symbols, two rows:

| symbol_id                                  | is_readonly | is_optional | type_parameters |
|--------------------------------------------|-------------|-------------|-----------------|
| `<path>\|<line>\|0\|Container\|class`      | false       | false       | `["T"]`         |
| `<path>\|<line2>\|2\|map\|method`          | false       | false       | `["U"]`         |

The method `map`'s `type_parameters` is `["U"]` — the class's `T` is **not** inherited into the method's row. Rationale stated in the extraction rule: class type parameters are accessible *inside* method bodies (for type resolution; this is a `references-typescript.md` concern) but the `typescript_attrs.type_parameters` column records only what the method *declares* itself.
