# Types — PHP

This contract document defines how PHP type expressions map to the
`type` relation in the Virgil-CLI schema. It is the authoritative
reference for the PHP extractor that lives in
`src/languages/php/`. See [ADR-0002](adr/0002-symbol-id-scheme.md) for
the symbol id format and [ADR-0003](adr/0003-level-3-types-and-references.md)
for the Level-3 commitment.

The PHP grammar used is `tree-sitter-php::LANGUAGE_PHP` (it accepts
`<?php` tags and HTML fragments) — **not** `LANGUAGE_PHP_ONLY`.

## Tree-sitter node kinds

Every node kind that can appear in a type position. PHP type
declarations may attach to:

- a parameter (`simple_parameter`, `property_promotion_parameter`, `variadic_parameter`)
- a return type (the `return_type` field on `function_definition` / `method_declaration` / `arrow_function` / `anonymous_function`)
- a property type (the `type` field on `property_declaration`)
- a constant type (PHP 8.3+ `class_constant_declaration` typed const)

| Node kind | Source example | Schema `kind` |
|---|---|---|
| `primitive_type` | `int`, `string`, `bool`, `float`, `array`, `iterable`, `callable`, `void`, `mixed`, `never`, `null`, `false`, `true`, `object` | `primitive` |
| `named_type` | `User`, `\App\Models\User`, `self`, `static`, `parent` | `named` |
| `optional_type` | `?User`, `?string` | `union` (rewritten as `T \| null`) |
| `union_type` | `int\|string`, `User\|null` | `union` |
| `intersection_type` | `Countable&Stringable` | `intersection` |
| `disjunctive_normal_form_type` | `(A&B)\|C` (PHP 8.2+) | `union` (containing one or more intersection children) |
| `cast_type` | `(int)`, `(array)`, `(string)` inside `cast_expression` | `primitive` |

Notes:

- **`primitive_type` vs `named_type` split.** The grammar uses the
  `primitive_type` node for the built-in keywords listed above
  (including `array`, `mixed`, `void`, `never`, `null`, `false`, `true`,
  `object`). Anything that lexes as an identifier (`User`,
  `\App\Models\User`, `self`, `static`, `parent`) is a `named_type`.
  The schema `kind` follows the node kind one-to-one — no reclassification
  based on what the name resolves to.
- **`array<T>`-style generics do not exist** at the language level in
  PHP. They only appear inside PHPDoc comments and are out of scope for
  this contract (Phase 1 does not parse PHPDoc types). PHP has no
  `generic` `kind` rows.
- **`tuple` and `array` `kind`s are unused** by the PHP extractor. PHP
  has no tuple syntax; `array` appears only as a `primitive_type` token
  and is recorded with `kind = "primitive"`, never with `kind = "array"`.
  The `array` schema variant is reserved for languages whose grammar
  distinguishes element-typed array syntax (TypeScript `T[]`, Rust `[T; N]`).
- **`function` `kind` is unused.** PHP's `callable` is a primitive, not
  a structural function type. `Closure` is a `named_type` like any
  other class. Languages with `(A, B) => C` syntax populate `kind =
  "function"`; PHP does not.
- **Untyped parameters / returns.** PHP allows omitting the type
  declaration entirely (legacy code, dynamic signatures). When a
  parameter or return has no type node, **no `type` row is emitted**,
  and the corresponding `parameter.type_id` / `returns_type` row's
  `type_id` is `null`. The extractor does not synthesize a `mixed`
  placeholder. (Rationale: a synthetic `mixed` row would inflate
  per-file dedup and lie to queries that filter `kind = "primitive"`.)

## `display_name` construction

`display_name` is the source text of the type node with whitespace
normalized. Exact rules:

1. Take the tree-sitter `utf8_text` of the type node.
2. Collapse any run of ASCII whitespace (` \t\r\n`) to a single space.
3. Strip whitespace immediately inside `(`, `)`, around `|`, around
   `&`, and around `\` namespace separators.
4. Preserve the leading `\` on fully-qualified names. `\App\Models\User`
   and `App\Models\User` have different `display_name`s — this is a
   real semantic distinction in PHP (the first is anchored at the root
   namespace, the second is resolved against the current namespace).
5. `?T` is **not** rewritten in `display_name`. `display_name = "?User"`,
   while `kind = "union"` and `canonical_name` (if resolvable) is
   `"User|null"`. This preserves source intent so queries can find
   `?T`-style nullables specifically.
6. Union and intersection types preserve the operand order from source.
   `int|string` and `string|int` produce different `display_name`s.

Round-trip guarantee: `User|null` and `User | null` produce the same
`display_name`. `?User` and `?User` do too. `?User` and `User|null`
do **not** — they're spelled differently in source.

## `canonical_name` resolution

`canonical_name` is the fully-qualified, normalized form, set to
`null` when resolution fails.

Scope walk for a bare `named_type` reference (e.g. `User`) — in order:

1. **Same-file `use` aliases** — `use App\Models\User as U;` makes the
   local name `U` canonicalize to `App\Models\User`. A bare `User`
   that has a matching `use App\Models\User;` resolves to the same.
2. **Same-namespace lookup** — if the file declares
   `namespace App\Services;` and references `Cart`, the resolver
   tries `App\Services\Cart`. If no such symbol is indexed in the
   current cold/incremental build, it falls through.
3. **Root namespace fallback** — for primitive-adjacent names that
   PHP itself ships (`Closure`, `Generator`, `Iterator`, `Throwable`,
   `Exception`, `Stringable`, `Countable`, `ArrayAccess`,
   `IteratorAggregate`, etc.), canonicalize to the leading-backslash
   form (`\Closure`, `\Throwable`, …). This list is a fixed allow-list
   in the extractor — anything outside it is **not** auto-promoted to
   the root namespace.
4. **Unresolved** — if none of the above match, `canonical_name = null`.

Additional rules:

- **Already-qualified names** (`\App\Models\User`) canonicalize to
  themselves with the leading `\` preserved.
- **`self` / `static` / `parent`** canonicalize to the fully-qualified
  name of the enclosing class (for `self` and `static`) or its parent
  class (for `parent`). If the enclosing class has no resolvable
  parent (no `extends` clause, or the parent isn't indexed),
  `parent` resolves to `null`.
- **Type aliases** — PHP has no `type` keyword. Eloquent / userland
  `class_alias()` calls are not followed. `class_alias()` resolution
  is explicitly out of scope.
- **Union / intersection canonicalization.** When a parent
  `union_type` / `intersection_type` row is emitted, each operand
  becomes a separate `type` row referenced by `display_name` inside
  the parent's `display_name`. The parent's `canonical_name` is the
  pipe-joined (or `&`-joined) sequence of operand canonicals, in source
  order, with `null` when any operand fails to resolve. (Rationale:
  partial canonicalization would silently lose information; queries
  that need to inspect operands should join through the per-operand
  rows.)
- **`?T` canonicalization.** `display_name = "?User"`, `kind = "union"`,
  `canonical_name = "<canonical(User)>|null"` when `User` resolves,
  otherwise `null`.
- **Primitive `canonical_name`.** Built-in `primitive_type` rows have
  `canonical_name` equal to the lowercase keyword (`int`, `string`,
  `void`, …). They never resolve to `null`.

## Identity

Per ADR-0003: `type.id = blake3(language | file_id | display_name)`.
PHP-specific normalization applied **before** hashing:

- `language` is the literal string `"php"`.
- `file_id` is the workspace-relative path (no synthetic id; matches
  ADR-0002's choice to use the path as `file.id`).
- `display_name` is the whitespace-normalized form described above —
  the same string stored on the `type` row.

Consequence: `User|null` and `User | null` collide in the same file
(same id). `?User` and `User|null` do **not** collide even though
they're semantically equivalent — this is deliberate so queries can
distinguish syntactic forms.

## Field types — `field_type` relation

Per the schema, every PHP typed property declaration
(`private string $name;`, `public ?User $owner;`) emits a
`field_type {symbol_id, type_id}` row linking the property symbol
to its `type` row. Untyped properties (no type annotation, common
in pre-7.4 code or dynamic Eloquent magic properties) emit no
`field_type` row. Local variables and function parameters use
`parameter` / `references` wiring instead.

## Worked examples

All examples are drawn from
`/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/php/laravel-store/`.

### Example 1 — primitive parameter type

**Source:** `app/Services/CartService.php` line 14

```php
public function getCart(int $userId): Cart
```

`int` is a `primitive_type`. `Cart` is a `named_type` resolved against
the same file's `use App\Models\Cart;` (line 5).

Symbol id for the enclosing method (ADR-0002):
`app/Services/CartService.php|14|4|getCart|method`.

Rows for the `int` parameter type:

```
type {
    id:             blake3("php" | "app/Services/CartService.php" | "int"),
    kind:           "primitive",
    language:       "php",
    display_name:   "int",
    canonical_name: "int",
}

parameter {
    function_id:  "app/Services/CartService.php|14|4|getCart|method",
    index:        0,
    name:         "userId",
    type_id:      <id of the int row above>,
    is_optional:  false,
    has_default:  false,
}
```

Rows for the `Cart` return type:

```
type {
    id:             blake3("php" | "app/Services/CartService.php" | "Cart"),
    kind:           "named",
    language:       "php",
    display_name:   "Cart",
    canonical_name: "App\\Models\\Cart",
}

returns_type {
    function_id: "app/Services/CartService.php|14|4|getCart|method",
    type_id:     <id of the Cart row above>,
}
```

### Example 2 — nullable named return (`?Type`)

**Source:** `app/Repositories/UserRepository.php` lines 13–16

```php
public function findById(int $id): ?User
{
    return User::find($id);
}
```

`?User` is an `optional_type` node. The extractor emits **two** `type`
rows: the union parent and the inner `User` named operand. The bare
`null` does not get its own `type` row — it's encoded in the union's
`display_name` and `canonical_name`.

Symbol id: `app/Repositories/UserRepository.php|13|4|findById|method`.

```
type { id: T1, kind: "union", language: "php",
       display_name: "?User",
       canonical_name: "App\\Models\\User|null" }

type { id: T2, kind: "named", language: "php",
       display_name: "User",
       canonical_name: "App\\Models\\User" }

returns_type { function_id: <method id>, type_id: T1 }
```

where `T1 = blake3("php" | file | "?User")` and
`T2 = blake3("php" | file | "User")`.

Note that `T2` is recorded as a separate row even though no
`returns_type` / `parameter` row references it directly. This is the
contract: every operand of a union/intersection gets its own row, so
downstream queries can join through `canonical_name` to count `User`
references across files.

### Example 3 — qualified return type with leading backslash

**Source:** `app/Repositories/UserRepository.php` line 49

```php
public function getAllPaginated(int $perPage = 30): \Illuminate\Contracts\Pagination\LengthAwarePaginator
```

The return type is a `named_type` written with a leading `\`. It is
**not** further qualified — the leading `\` already anchors it at the
root namespace.

```
type {
    id:             blake3("php" | "app/Repositories/UserRepository.php"
                            | "\\Illuminate\\Contracts\\Pagination\\LengthAwarePaginator"),
    kind:           "named",
    language:       "php",
    display_name:   "\\Illuminate\\Contracts\\Pagination\\LengthAwarePaginator",
    canonical_name: "\\Illuminate\\Contracts\\Pagination\\LengthAwarePaginator",
}
```

The `int` parameter at index 0 gets the same `primitive` row as in
example 1 (deduped per file by `display_name`). The `has_default`
column on its `parameter` row is `true` because of `= 30`.

### Example 4 — untyped parameter (legacy PHP, no type declaration)

**Source:** `app/Http/Middleware/ThrottleRequests.php` line 18

```php
public function handle(Request $request, Closure $next, $maxAttempts = 60, $decayMinutes = 1)
```

Parameters 2 and 3 (`$maxAttempts`, `$decayMinutes`) have **no type
node**. The extractor emits **no** `type` row for them. Their
`parameter` rows have `type_id = null`:

```
parameter { function_id: <handle id>, index: 0, name: "request",
            type_id: <Request row>, is_optional: false, has_default: false }
parameter { function_id: <handle id>, index: 1, name: "next",
            type_id: <Closure row>, is_optional: false, has_default: false }
parameter { function_id: <handle id>, index: 2, name: "maxAttempts",
            type_id: null, is_optional: false, has_default: true }
parameter { function_id: <handle id>, index: 3, name: "decayMinutes",
            type_id: null, is_optional: false, has_default: true }
```

`Closure` is recorded with `canonical_name = "\\Closure"` (it's in
the PHP built-in allow-list described under `canonical_name`
resolution). `Request` resolves to `Illuminate\Http\Request` via the
file's `use Illuminate\Http\Request;` at line 6.

There is no `returns_type` row for `handle` at all — the source has
no return type declaration. (Contract: untyped returns produce no
row, not a `null`-typed row, because `returns_type.type_id` is
non-nullable in the schema.)

### Example 5 — self-class typed property + class constants in same file

**Source:** `app/Services/PaymentService.php` lines 10–11

```php
private string $stripeKey = 'sk_live_placeholder_do_not_use_in_prod';
private string $stripePublicKey = 'pk_live_placeholder_do_not_use_in_prod';
```

Both `property_declaration`s carry a `type` field of `string`
(`primitive_type`). Each property is a separate `symbol` row of kind
`field` (the existing extractor calls it `Property`; this contract
preserves that mapping). The single `string` `type` row is reused
(deduplicated per file by `display_name`).

```
type {
    id:             blake3("php" | "app/Services/PaymentService.php" | "string"),
    kind:           "primitive",
    language:       "php",
    display_name:   "string",
    canonical_name: "string",
}
```

A `parameter`-shaped row is not emitted for properties — properties
aren't function parameters. Instead, the **property symbol** carries
its type via a generic mechanism not in the core schema today; for
Phase 1 the PHP extractor records the property type by attaching the
property symbol id as a separate `parameter`-equivalent only if the
schema is extended. **Contract:** for Phase 1, property types are
emitted **only** as standalone `type` rows (the `type.id` is
referenced from no other relation). This is acknowledged as
incomplete — the follow-up schema bump that adds
`property_type { symbol_id, type_id }` is tracked separately and is
**not** part of the PHP extractor's done-criterion. Queries that want
"types appearing in this file" still see the `string` row; queries
that want "what type is `$stripeKey`?" cannot answer that today.

### Example 6 — union return type (PHP 8+)

**Source:** synthesised from the benchmark style (no union return
type appears literally in `laravel-store`; the contract still must
specify behavior). Worked example written as if added at
`app/Services/PaymentService.php` line 116 in place of `getStatus`:

```php
public function getStatus(string $transactionId): string|null
```

(The current benchmark returns `: string`; this example documents
the behavior the extractor must produce when a union-typed return
appears.)

Rows:

```
type { id: U1, kind: "union", language: "php",
       display_name: "string|null",
       canonical_name: "string|null" }

type { id: U2, kind: "primitive", language: "php",
       display_name: "string", canonical_name: "string" }

type { id: U3, kind: "primitive", language: "php",
       display_name: "null", canonical_name: "null" }

returns_type { function_id: <getStatus id>, type_id: U1 }
```

`null` here is a `primitive_type` node, not a `named_type`. It's part
of the PHP built-in primitive list at the top of this document.
