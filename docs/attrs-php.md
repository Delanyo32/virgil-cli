# Language attributes — PHP

This contract document defines the `php_attrs` sparse-extension
relation. It is the authoritative reference for the PHP extractor in
`src/languages/php/`. See [ADR-0002](adr/0002-symbol-id-scheme.md) for
symbol ids and `docs/virgil-datalog-schema.md` for the surrounding
schema.

The base schema's `php_attrs` ships with two columns; this contract
extends it to four. The schema doc in this repo will need a matching
bump alongside the extractor change.

## Schema

```
:create php_attrs {
    symbol_id:   String =>
    is_final:    Bool         default false,
    uses_traits: [String]     default [],
    attributes:  [String]     default [],
}
```

`is_abstract` is **deliberately not** in `php_attrs`. The base
`symbol` relation already has `is_abstract: Bool` (see
`docs/virgil-datalog-schema.md`), so PHP's `abstract` modifier
populates that column rather than a PHP-specific duplicate. Same
reasoning for `is_static` (already on `symbol`). Rationale: the
purpose of language-specific attribute tables is *language-specific*
attributes — overlapping with cross-language columns invites
inconsistent population by different language extractors. The
contract picks "extend `symbol`, never duplicate" as the rule.

### Column applicability

| Column | Applies to which symbol kinds |
|---|---|
| `is_final` | `class`, `method` |
| `uses_traits` | `class` only (and `trait` declarations, see notes) |
| `attributes` | `class`, `method`, `function`, `constructor`, `field` (property), `parameter` |

For every other (`symbol_id`, kind) combination, no `php_attrs` row
is emitted at all — the defaults apply implicitly when joined.
**Empty `uses_traits` or `attributes` on a class is still emitted as
a row with the empty list**, because downstream queries need a row to
exist to distinguish "PHP class, no traits" from "non-PHP symbol".

## Extraction rules

### `is_final`

AST source: the `final_modifier` child of `class_declaration` or
`method_declaration`.

- `final class Foo` → `is_final = true` on the class's symbol id.
- `final public function bar()` → `is_final = true` on the method's
  symbol id.
- `class Foo` (no `final`) → `is_final = false`.

Edge case: a `final` modifier on a class constant
(PHP 8.1+ `final const FOO = 1;`) does **not** flow into `php_attrs`
because there's no `final` semantics for non-class non-method
symbols worth surfacing as a typed column. If it ever matters,
queryers go through `symbol_attr` (the generic key-value extension).

### `uses_traits`

AST source: every `use_declaration` node appearing inside a
`declaration_list` of a `class_declaration` or `trait_declaration`.

- The value is the list of trait names imported by the class, in
  source order, with **no deduplication** (if the source repeats a
  trait, it appears twice).
- Each entry is the trait name **as written in source**, not
  canonicalized. For `use HasFactory;` the entry is the literal
  `"HasFactory"`. For `use \App\Traits\Loggable;` the entry is
  `"\\App\\Traits\\Loggable"`. (Rationale: canonicalization belongs in
  the `references` relation, where the `referent_id` resolves the
  string against the file's `use` aliases. Storing the raw string
  here keeps `php_attrs` cheap to produce and joinable against
  `references` rows whose `ref_kind = "type_use"`.)
- Multi-trait `use Foo, Bar, Baz;` (single `use_declaration` with
  three children) flattens into three list entries, in source order.
- A class with **no** trait `use` declarations has
  `uses_traits = []`.

**Trait conflict-resolution clauses** (`use Foo, Bar { Foo::a
insteadof Bar; }`): the trait *names* still appear in
`uses_traits` (`["Foo", "Bar"]`). The resolution rules inside the
braces are **not** extracted by Phase 1; they're recorded as raw text
in `symbol_attr` keyed `trait_resolution` if and when a downstream
audit needs them. Phase 1 does not emit anything in `symbol_attr`
for traits.

### `attributes`

AST source: `attribute_list` children appearing **immediately before**
a `class_declaration`, `function_definition`, `method_declaration`,
`property_declaration`, or `simple_parameter` (PHP 8 attribute
syntax: `#[AttrName(...)]`).

- Each `attribute` inside an `attribute_list` becomes one string
  entry. The string is the attribute's class name **as written**
  (the same not-canonicalized convention as `uses_traits`).
- Arguments inside the attribute's parentheses are **dropped**. The
  list captures *which* attributes are present, not their argument
  values. (Rationale: attribute argument analysis is a separate
  feature; modeling it as a sub-table is out of scope.)
- Multiple `attribute_list`s on the same symbol concatenate in
  source order. Multiple `attribute`s inside a single
  `attribute_list` (`#[A, B, C]`) concatenate left-to-right.
- A symbol with **no** attributes has `attributes = []`.

Edge case: `#[\Attribute]` (a PHP attribute applied to a class to
mark it *as* an attribute) is treated like any other — the entry is
`"\\Attribute"`.

### `is_abstract` (on `symbol`, not `php_attrs`)

For completeness — even though `php_attrs` does not own this column,
the PHP extractor is responsible for populating `symbol.is_abstract`:

- `abstract class Foo` → `symbol.is_abstract = true` for the class.
- `abstract public function bar()` → `symbol.is_abstract = true` for
  the method.
- An `interface_declaration`'s members are abstract by definition,
  but `symbol.is_abstract` is **false** for interface methods (the
  `is_abstract` column means "explicitly marked abstract in source").
  Interface-ness is captured by `symbol.kind = "interface"` and the
  containing parent_id; queries that want "all methods that must be
  implemented" should union `is_abstract = true` OR
  `parent.kind = "interface"`.

### `is_static` (on `symbol`, not `php_attrs`)

For completeness:

- `static_modifier` on a `method_declaration` or
  `property_declaration` → `symbol.is_static = true`.
- `static` keyword on a `variable_declarator` inside a function body
  (function-static local) is **not** recorded because such variables
  do not produce `symbol` rows in the current extractor.

## Worked examples

All examples are drawn from
`/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/php/laravel-store/`.

### Example 1 — class with single trait, no final, no attributes

**Source:** `app/Models/Product.php` lines 8–10

```php
class Product extends Model
{
    use HasFactory;
    ...
```

Symbol id for the class:
`app/Models/Product.php|8|0|Product|class`.

`php_attrs` row:

```
php_attrs {
    symbol_id:   "app/Models/Product.php|8|0|Product|class",
    is_final:    false,
    uses_traits: ["HasFactory"],
    attributes:  [],
}
```

The base `symbol` row carries the rest: `kind = "class"`,
`is_abstract = false`, `is_static = false`. `extends` is in the
`extends` relation, not here.

### Example 2 — class with multi-trait `use` statement

**Source:** `app/Models/User.php` line 12 (class body starts at line 10)

```php
class User extends Authenticatable
{
    use HasApiTokens, HasFactory, Notifiable;
    ...
```

A single `use_declaration` node lists three trait names. The
extractor flattens them in source order. The entries are the raw
source spellings — `HasApiTokens` is **not** canonicalized to
`\Laravel\Sanctum\HasApiTokens` even though the file has a
`use Laravel\Sanctum\HasApiTokens;` at line 8.

Symbol id: `app/Models/User.php|10|0|User|class`.

```
php_attrs {
    symbol_id:   "app/Models/User.php|10|0|User|class",
    is_final:    false,
    uses_traits: ["HasApiTokens", "HasFactory", "Notifiable"],
    attributes:  [],
}
```

### Example 3 — base class composed entirely of trait `use`s (no other body)

**Source:** `app/Http/Controllers/Controller.php` lines 10–13

```php
class Controller extends BaseController
{
    use AuthorizesRequests, DispatchesJobs, ValidatesRequests;
}
```

Symbol id:
`app/Http/Controllers/Controller.php|10|0|Controller|class`.

```
php_attrs {
    symbol_id:   "app/Http/Controllers/Controller.php|10|0|Controller|class",
    is_final:    false,
    uses_traits: ["AuthorizesRequests", "DispatchesJobs", "ValidatesRequests"],
    attributes:  [],
}
```

This example is the simplest non-obvious case: the class has no
methods, no properties, and no constants — only the trait `use`. The
extractor still emits the `php_attrs` row. A query like "which
classes mix in `AuthorizesRequests`?" needs this row to exist.

### Example 4 — class with no traits and no attributes (empty-list row still emitted)

**Source:** `app/Services/PaymentService.php` lines 7–8

```php
class PaymentService
{
    // Hardcoded API key -- should be in env
```

Symbol id:
`app/Services/PaymentService.php|7|0|PaymentService|class`.

```
php_attrs {
    symbol_id:   "app/Services/PaymentService.php|7|0|PaymentService|class",
    is_final:    false,
    uses_traits: [],
    attributes:  [],
}
```

This is the contract's explicit "emit even if all defaults" decision
for classes. A future query like "all PHP classes in the workspace"
joins through `php_attrs` and would miss `PaymentService` if the row
were elided.

### Example 5 — `final` method (synthetic, demonstrating non-default `is_final` on a method)

The `laravel-store` benchmark contains no `final` modifiers. The
contract still must specify behavior. Worked example written as if
`app/Services/ShippingService.php` line 29 were:

```php
final private function getDomesticCost(float $subtotal): float
```

Symbol id:
`app/Services/ShippingService.php|29|4|getDomesticCost|method`.

```
php_attrs {
    symbol_id:   "app/Services/ShippingService.php|29|4|getDomesticCost|method",
    is_final:    true,
    uses_traits: [],
    attributes:  [],
}
```

The method symbol's containing-class row is unaffected — `is_final`
is per-symbol, not inherited downward. `uses_traits` is `[]` on the
method row because traits are class-level only; method-level
`uses_traits` is meaningless but the column applies uniformly (the
default empty list keeps the schema simple).

### Example 6 — PHP 8 `#[Attribute]` syntax (non-obvious AST construct)

The `laravel-store` benchmark contains no PHP 8 attribute syntax.
The contract must specify behavior. Worked example written as if
`app/Http/Middleware/ThrottleRequests.php` line 18 were preceded by:

```php
#[\Symfony\Component\HttpKernel\Attribute\AsController]
#[Cached(ttl: 60)]
public function handle(Request $request, Closure $next, $maxAttempts = 60, $decayMinutes = 1)
```

(Two `attribute_list` siblings before the `method_declaration`, the
first with one attribute and the second with one attribute that
takes a named argument.)

Symbol id:
`app/Http/Middleware/ThrottleRequests.php|18|4|handle|method`.

```
php_attrs {
    symbol_id:   "app/Http/Middleware/ThrottleRequests.php|18|4|handle|method",
    is_final:    false,
    uses_traits: [],
    attributes:  ["\\Symfony\\Component\\HttpKernel\\Attribute\\AsController", "Cached"],
}
```

Notes on the non-obvious bits:

- `\Symfony\Component\HttpKernel\Attribute\AsController` is recorded
  with the leading `\` preserved — the same convention as
  `uses_traits`. JSON-encoded list entries store a single `\` per
  segment; the doubled `\\` in this doc is markdown escaping only.
- `Cached(ttl: 60)` is recorded as `"Cached"`. The `ttl: 60`
  argument is dropped. To recover argument values, a downstream pass
  would need to query the raw AST through `symbol_attr` — out of
  scope here.
- Two `attribute_list` siblings flatten into one ordered list, in
  source order: `[outer-first, outer-second]`.
- If both attribute lines had been merged as `#[AsController, Cached(ttl: 60)]`
  (one `attribute_list` with two children), the output `attributes`
  list would be identical. The grouping syntax is **not** observable
  from `php_attrs`.
