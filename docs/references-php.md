# References — PHP

This contract document defines how PHP identifier occurrences map to
the `references` relation. It is the authoritative reference for the
PHP extractor in `src/languages/php/`. See
[ADR-0002](adr/0002-symbol-id-scheme.md) for symbol ids and
[ADR-0003](adr/0003-level-3-types-and-references.md) for the Level-3
commitment.

The PHP grammar used is `tree-sitter-php::LANGUAGE_PHP`.

Per `docs/virgil-datalog-schema.md`, `references` is keyed by `(referrer_id, site_file, site_start_byte, match_index)` with `referent_id` and `ref_kind` in the value position. `match_index = 0` for the primary/only candidate; PHP has no static overload resolution, so every PHP row uses `match_index = 0` in practice. Unresolvable referents emit a single row with `referent_id = null` (per `docs/contract-review.md`, policy 1: not skipped, not a sentinel string).

## Lexical scope rules

PHP's scoping is **not** lexical-block-scoped. The model is:

- **File scope** — declares functions, classes, interfaces, traits,
  enums, namespaces, file-level `use` aliases, and (rarely) top-level
  function definitions. `use` declarations are file-scoped, not
  namespace-scoped.
- **Function-local scope** — every `function_definition`,
  `method_declaration`, `arrow_function`, and `anonymous_function`
  introduces a fresh local-variable scope. **Variables defined in an
  outer function do not leak into a nested function** (this is the
  central PHP rule that makes resolution simpler than in Python or
  JavaScript).
- **Class member scope** — `$this`, `self::`, `static::`, and
  `parent::` are bound to the enclosing class. They are *not* a
  free-variable mechanism — they're members of a fixed class.
- **No block scope** — `if`, `for`, `foreach`, `while` blocks do
  **not** introduce a new variable scope. A variable assigned inside
  an `if` is visible in the rest of the enclosing function.

### Closure capture

Closures (`function (...) use ($x, $y) { ... }`) and
short closures / arrow functions (`fn ($p) => $expr`) handle
captures differently:

- **`anonymous_function` with `use` clause** — the `use` clause
  enumerates which outer variables are captured. **Only those listed
  in `use` are visible** inside the closure body. The captured
  variables become locals of the closure (by-value unless prefixed
  with `&`). Variables not listed are unresolved inside the closure.
- **`arrow_function` (`fn () => ...`)** — captures all outer locals
  by-value automatically. No `use` clause exists.

### `global` keyword

`global $foo;` inside a function imports the global-scope variable
`$foo` into the function's local scope. Without it, `$foo` inside a
function refers to a fresh function-local variable, **not** a
shadowed global. The PHP extractor treats `global` declarations as
explicit imports of a top-level symbol into the local scope.

### Shadowing

- Parameter names override any later same-named local assignment in
  that function — but PHP itself raises no error on reassignment, so
  the same name is just rebound. The extractor treats a parameter and
  later assignments as **the same symbol** (same `referrer_id` /
  `referent_id`). Local variables in PHP have no separate "binding
  site" symbol; the parameter row in `parameter` is the canonical
  binding.
- A `use ($x)` capture inside a closure creates a **new** local
  binding inside the closure body that shadows any same-named outer
  variable. References to `$x` inside the closure body resolve to the
  closure's local; references in the surrounding function continue to
  resolve to the outer binding.

### Module-qualified names

PHP uses backslash separators (`App\Models\User`). Resolution against
file-level `use` aliases is identical to type resolution (see
`types-php.md` — same algorithm):

1. Same-file `use` alias table (including `as` renames and grouped
   `use App\Models\{User, Post}`).
2. Same-namespace lookup against indexed symbols.
3. Root-namespace allow-list for built-in PHP classes.
4. Otherwise unresolved.

References that start with a leading `\` (`\App\Models\User`) skip
steps 1–3 and go straight to "indexed symbol with that qualified
name, or unresolved".

## `ref_kind` decision tree

Every identifier occurrence the extractor emits maps to exactly one
of the four `ref_kind` values defined in the schema.

### `read`

Emitted for AST patterns where an identifier is **evaluated**:

- `variable_name` (`$x`) appearing in any expression position that is
  not the LHS of an assignment.
- `member_access_expression` reading a property: `$this->price`,
  `$user->email`. The `$this` (or other base) is one `read` row; the
  member name is a separate `read` row keyed to the property symbol
  if resolvable.
- `scoped_call_expression` and `scoped_property_access_expression`
  reading a static member: `Cart::firstOrCreate(...)` — `Cart` is a
  `read` of the class symbol; `firstOrCreate` is a `read` of the
  method symbol if it can be resolved on `Cart`.
- `function_call_expression` calling a free function or imported
  function: `now()`, `view(...)`, `compact(...)`. The function name
  is a `read` of the function symbol if resolvable. The accompanying
  `calls` row (the existing pre-Level-3 graph) is **also** emitted —
  `references` is additive, not a replacement.
- `class_constant_access_expression` reading a class constant:
  `self::STANDARD_SHIPPING`. The class qualifier (`self`) and the
  constant name (`STANDARD_SHIPPING`) each produce a `read` row.
- `enum_case` access: `Status::Active` follows the same pattern as a
  class constant.
- Identifier appearing inside a `new` expression's class slot:
  `new InventoryService()` — `InventoryService` is a `read`. The
  constructor call itself is also recorded via the existing `calls`
  edge.
- `instanceof_expression` right-hand side: `$x instanceof User` —
  `User` is a `read`.

**Exclusions** — these tree-sitter nodes are *not* emitted as `read`
rows:

- String literals that *happen* to look like class names (`'App\\Models\\User'`
  passed to `class_exists` or similar). The extractor does not
  string-match.
- `heredoc` / `nowdoc` content is not scanned for identifiers.
- Attribute argument values (PHP 8 `#[Attr("foo")]`) — the attribute
  *class* itself is a `type_use` (see below); the arguments are not
  parsed for identifiers.
- PHPDoc comments are not parsed.

### `write`

Emitted for AST patterns where a binding is **created or mutated**:

- `assignment_expression` LHS: `$x = ...`, `$this->price = ...`,
  `self::$count = ...`. The LHS variable / property / static is a
  `write`. For property and static-property LHS, a `write` row for
  the property/static name is emitted **only** when that field has a
  known `symbol_id` in the store (per `docs/contract-review.md`,
  policy 5). Eloquent magic properties and other implicit fields
  produce no field-level `write` row.
- `augmented_assignment_expression` LHS (`+=`, `-=`, `.=`, `??=`):
  one `write` row, no separate `read`. Updated per
  `docs/contract-review.md`: compound assignment is single-row
  `write` at Level 3; faithful read+write semantics is Level 4.
- `update_expression` (`$x++`, `++$x`, `$x--`, `--$x`): single
  `write` row, same as compound assignment.
- `foreach_statement` value variable: `foreach ($items as $item)` —
  `$item` is a `write`. If the key form is used
  (`foreach ($items as $k => $v)`), both `$k` and `$v` are `write`s.
- `by_ref` parameter on entry to a function (`function f(&$x)`): the
  parameter is treated as a `write` site on the **caller-side**
  binding. The extractor records this only if call-site argument
  analysis is implemented; for Phase 1 this is **explicitly out of
  scope** and produces no `write` row from the call site.
- `unset($x)` — `unset` arguments are recorded as `write` rows (the
  variable is being mutated to an unbound state).
- `list_assignment` / `array_destructuring`: `[$a, $b] = $pair;` —
  each identifier on the LHS is a `write`.

Note: PHP method calls that *conventionally* mutate (`$arr->push(...)`,
`$cart->save()`) are **not** treated as `write`s on the receiver. PHP
has no convention enforced by the language. The receiver is a `read`,
the method name is a `read`. Mutation analysis would require
per-class semantic knowledge that the extractor does not have.

### `type_use`

Emitted for every identifier that appears in a **type position**, in
lockstep with the `type` rows from `types-php.md`. Specifically:

- Parameter type declaration: `function f(User $u)` — `User` is a
  `type_use`.
- Return type declaration: `: ?User` — `User` is a `type_use`. (The
  `?` itself produces no row; only the inner named/primitive operands
  do.)
- Property type declaration (PHP 7.4+): `private string $key;` —
  `string` is a `type_use`. (`string` is a primitive, so its
  `referent_id` is `null` — primitives have no symbol.)
- `instanceof_expression` RHS: `$x instanceof User` — `User` is a
  `type_use` **and** a `read`; **two rows** are emitted, distinguishing
  the syntactic role. (Rationale: queries that look for "where is this
  type mentioned?" should see both.)
- `catch_clause` exception type: `catch (\Exception $e)` — `\Exception`
  is a `type_use`.
- Union / intersection operands: each named operand in
  `User|Admin|null` produces its own `type_use` row.
- Class name in `new ClassName(...)`: this is a `read`, **not** a
  `type_use`. The convention: `type_use` is reserved for nodes
  syntactically in a type position (parameter type, return type, etc.).
  `new` is a constructor call.
- Static-method call qualifier (`Cart::find(...)`): this is a `read`
  of the class symbol, **not** a `type_use`. Same reasoning.

For each `type_use` row, the corresponding row in the `type`
relation (defined by `types-php.md`) has a matching `display_name` /
`canonical_name` — `references` and `type` are independent tables and
the join goes through the resolved class symbol id, not through any
type id.

### `import_use`

Emitted for every name appearing inside a `namespace_use_declaration`
or in `require` / `require_once` / `include` / `include_once`
arguments **when the argument resolves to a known file**:

- `use App\Models\User;` — `App\Models\User` is one `import_use` row,
  with `referent_id` = the class symbol's id when indexed, else `null`.
- `use App\Models\{User, Post};` — two `import_use` rows, one per
  brace-group entry.
- `use App\Models\User as U;` — one `import_use` row; the alias `U`
  does **not** get its own row (it's a local rename, tracked in the
  file's alias table for resolution but not emitted as a reference).
- `require __DIR__ . '/helpers.php';` — produces an `import_use` row
  only if the resolved file path matches a known workspace file (see
  `resolve_import` in `src/languages/php/queries.rs`); otherwise no
  row.

Note: the existing `imports` relation (file → imported symbol) is
also emitted by the pre-Level-3 pipeline. `import_use` rows in
`references` are additive — they record the *site* (`site_file`,
`site_start_byte`), which `imports` does not.

## `referent_id` resolution

For each occurrence, the resolver computes `referent_id` by walking
scopes in this order:

1. **Closure `use` captures**, if inside a `anonymous_function` and
   the name matches a captured variable.
2. **Enclosing function parameters**, if the name matches a parameter
   of the immediately-enclosing `function_definition` /
   `method_declaration` / `arrow_function` / `anonymous_function`.
3. **Enclosing function local variables**, identified by the first
   `write` site in the same function body that matches the name.
4. **`global $x;` table** for the enclosing function — if the name
   was declared `global`, resolve to a file-scope or workspace-scope
   symbol with the same name.
5. **Enclosing class members** when the access expression has `$this->`,
   `self::`, `static::`, or `parent::` as its base. The resolver
   computes the class's qualified name and looks up the property /
   method / constant on that class. `static::` is resolved to the
   enclosing class statically (no late-binding analysis); `parent::`
   resolves against the class's `extends` clause.
6. **File-level `use` aliases** — for bare unqualified identifiers in
   read/type positions, the alias table is consulted before
   same-namespace lookup.
7. **Same-namespace lookup** — concatenate the file's
   `namespace` declaration with the identifier and look up the
   resulting qualified name in the `symbols_by_name` index.
8. **Root-namespace allow-list** for PHP built-in classes (`\Closure`,
   `\Throwable`, `\Exception`, etc.) — see `types-php.md`.
9. **Unresolved** — `referent_id = null`. The row is still emitted.

### Multiple candidates

If multiple symbols match (e.g. two methods with the same name on
different classes when the receiver isn't class-typed), the resolver
picks **none** — `referent_id = null`. PHP's dynamic dispatch makes
guessing actively harmful, and the schema doesn't support multi-target
edges from a single occurrence. This contract chooses "skip rather
than guess" to keep downstream queries trustworthy.

### Resolver implementation

The resolver uses the existing `symbols_by_name` index in
`src/graph/builder.rs` (one global multimap from short name to
symbol ids), filtered post-lookup by the qualified-name string built
during scope walking. It does **not** build a per-file scope tree —
PHP's flat function-local scoping makes a tree unnecessary; an
ordered-scan over function-local writes is enough.

### When to emit the row

A `references` row is emitted for **every** occurrence in the four
`ref_kind` categories, including those where `referent_id = null`.
Rationale: skipping unresolved occurrences would make
"references-by-site" queries silently lossy. Downstream queries that
need only resolved rows filter `referent_id != null`.

## Worked examples

All examples are drawn from
`/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/php/laravel-store/`.

For brevity, `referrer_id` in each row is the id of the **enclosing
symbol** (function or method) per ADR-0002. `site_file` is the
workspace-relative path. `site_start_byte` is the tree-sitter byte
offset of the referencing identifier node.

### Example 1 — `$this->field` read inside a method

**Source:** `app/Models/Product.php` lines 53–61

```php
public function getDisplayPrice()
{
    $basePrice = $this->sale_price > 0 ? $this->sale_price : $this->price;

    // Magic number: 0.08 tax rate hardcoded
    $withTax = $basePrice * 1.08;

    return round($withTax, 2);
}
```

Symbol ids in scope:
- Enclosing method: `app/Models/Product.php|53|4|getDisplayPrice|method`
- Property `sale_price`: not present as a `symbol` row in this file
  (Eloquent magic — the property is implied by the `$fillable` array
  at lines 12–24, not declared). The extractor does **not** synthesize
  a symbol for it. References therefore resolve to `referent_id = null`.
- Property `price`: same — implied by `$fillable`, no symbol row.

Rows emitted (showing the field access reads only; the `$this`
itself is recorded once per access expression as a self-read on the
class). Per `docs/contract-review.md` policy 5, a field-level `read`
row is only emitted when the field has a known `symbol_id`. Since
the Eloquent magic properties `sale_price` and `price` have no
class-level declaration and are not extracted as field symbols, no
field-level rows are produced — the only rows for these accesses
are the `read` rows for the receiver `$this`:

```
references { referrer_id: <getDisplayPrice id>,
             referent_id: "app/Models/Product.php|<line>|0|Product|class",
             ref_kind: "read",
             site_file: "app/Models/Product.php",
             site_start_byte: <byte of `$this` at line 55, first occurrence> }

# (additional `$this` reads at each `$this->...` access — one per access expression)
```

Plus the local variable `$basePrice`: one `write` row (line 55, LHS
of assignment) and one `read` row (line 58, the `$basePrice * 1.08`).
Both have `referent_id = null` because PHP locals don't get `symbol`
rows.

Plus `round` at line 60: one `read` row with `referent_id = null`
(it's a PHP built-in not indexed in this workspace) plus a `calls`
edge per the existing pipeline.

**Ambiguity note.** This example is deliberately the worst-case for
PHP: Eloquent's dynamic properties mean `$this->sale_price` cannot be
statically resolved. The contract requires emitting the row anyway
with `referent_id = null`. Queries that need "all dynamic property
reads" filter `ref_kind = "read"` AND a member-access AST predicate
(which this extractor does **not** expose as a column; the contract
acknowledges this is partial). Recording the bytes is enough for
later passes to enrich.

### Example 2 — `self::` static constant access

**Source:** `app/Services/ShippingService.php` lines 29–40

```php
private function getDomesticCost(float $subtotal): float
{
    if ($subtotal < 50) {
        return self::STANDARD_SHIPPING;
    } elseif ($subtotal < 100) {
        return self::REDUCED_SHIPPING;
    } elseif ($subtotal < self::FREE_SHIPPING_THRESHOLD) {
        return self::DISCOUNTED_SHIPPING;
    } else {
        return 0.0;
    }
}
```

Symbol ids:
- Enclosing method:
  `app/Services/ShippingService.php|29|4|getDomesticCost|method`
- Constants on `ShippingService` (line 8–12 of the same file):
  e.g. `app/Services/ShippingService.php|9|4|STANDARD_SHIPPING|constant`,
  similarly for `REDUCED_SHIPPING`, `DISCOUNTED_SHIPPING`,
  `FREE_SHIPPING_THRESHOLD`.

Each `self::CONST` produces **two** `references` rows: one `read` for
the `self` qualifier (referent = the enclosing class symbol) and one
`read` for the constant name (referent = the constant symbol).

```
# self::STANDARD_SHIPPING at line 32
references { referrer_id: <getDomesticCost id>,
             referent_id: "app/Services/ShippingService.php|5|0|ShippingService|class",
             ref_kind: "read",
             site_file: "app/Services/ShippingService.php",
             site_start_byte: <byte of `self` at line 32> }

references { referrer_id: <getDomesticCost id>,
             referent_id: "app/Services/ShippingService.php|9|4|STANDARD_SHIPPING|constant",
             ref_kind: "read",
             site_file: "app/Services/ShippingService.php",
             site_start_byte: <byte of `STANDARD_SHIPPING` at line 32> }
```

(One pair per access: lines 32, 34, 35, 36 — eight rows total in this
method.)

Parameter `$subtotal` reads at lines 31, 33, 35: three `read` rows
with `referent_id = null` (PHP locals).

**Resolver decision:** `self` resolves to the enclosing class
(`ShippingService`) by lexical containment, not by lookup. The
extractor records the **defining class's** symbol id, not the
runtime late-bound class. If the source used `static::` instead, the
resolver would still record `ShippingService` — this contract
**explicitly does not** model late static binding.

### Example 3 — closure with `use` capture, shadowing outer variable

**Source:** `app/Services/SearchService.php` lines 14–47 (closure at
lines 18–21)

```php
public function searchProducts(string $query, array $filters = []): \Illuminate\Database\Eloquent\Collection
{
    $builder = Product::where('active', true);

    $builder->where(function ($q) use ($query) {
        $q->where('name', 'LIKE', "%{$query}%")
            ->orWhere('description', 'LIKE', "%{$query}%");
    });
    ...
}
```

Symbol ids:
- Enclosing method:
  `app/Services/SearchService.php|14|4|searchProducts|method`
- The closure itself is **not** emitted as a separate `symbol` row by
  the current extractor (`SymbolKind::Closure` is not in the PHP
  symbol-query list). Per this contract, anonymous functions are
  **not** independent referrer scopes for `references`: rows inside
  the closure body are attributed to the **enclosing named symbol**
  (`searchProducts`). This is a deliberate simplification; the
  follow-up extension that emits closure symbols is **not** part of
  the Phase 1 done-criterion.

Rows for the closure body:

```
# $q->where(...) at line 19 — $q is the closure parameter
references { referrer_id: <searchProducts id>,
             referent_id: null,   # closure parameter, no symbol row
             ref_kind: "read",
             site_file: "app/Services/SearchService.php",
             site_start_byte: <byte of `$q` at line 19> }

# $query inside the closure body at line 19 — resolves to the use() capture
references { referrer_id: <searchProducts id>,
             referent_id: null,   # outer parameter, no symbol row for locals
             ref_kind: "read",
             site_file: "app/Services/SearchService.php",
             site_start_byte: <byte of `$query` at line 19> }
```

**Resolution detail.** The `$query` at line 19 is resolved by
checking step 1 of the resolver (closure `use` captures) — `$query`
appears in the `use ($query)` clause at line 18, so it binds. Without
that clause, the closure body would treat `$query` as undefined and
the extractor would still emit a `read` row with `referent_id = null`.

**Ambiguity note.** `referent_id = null` for parameters loses the
distinction between "captured from outer scope" and "not defined
anywhere". The contract acknowledges this — distinguishing the two
requires a `local_symbol` schema extension that is not in Phase 1.
The site bytes are recorded so a follow-up pass can disambiguate.

### Example 4 — `global` keyword writing to a non-local

PHP global usage rarely appears in modern Laravel code; the
benchmark does not contain one. The contract specifies the behavior
anyway. Worked example written as if this snippet existed at
`app/Helpers/CurrencyHelper.php` line 65:

```php
function bumpCounter() {
    global $callCount;
    $callCount = ($callCount ?? 0) + 1;
}
```

Symbol ids:
- Enclosing function:
  `app/Helpers/CurrencyHelper.php|65|0|bumpCounter|function`
- `$callCount` is **not** a `symbol` row anywhere — PHP globals are
  not function/class/method definitions. Resolution falls through
  steps 1–8 and lands on step 9 (unresolved). The `global`
  declaration is recorded by the extractor as a side-table entry for
  this function's scope (so subsequent occurrences inside the
  function resolve consistently to the same `referent_id = null`),
  but the row's `referent_id` remains `null`.

Rows:

```
# $callCount inside the body, read on RHS (after `($callCount ??`)
references { referrer_id: <bumpCounter id>,
             referent_id: null,
             ref_kind: "read",
             site_file: "app/Helpers/CurrencyHelper.php",
             site_start_byte: <byte of `$callCount` in `($callCount ?? 0)`> }

# $callCount on LHS of assignment
references { referrer_id: <bumpCounter id>,
             referent_id: null,
             ref_kind: "write",
             site_file: "app/Helpers/CurrencyHelper.php",
             site_start_byte: <byte of `$callCount` at LHS> }
```

The `global $callCount;` statement itself produces **no** `references`
row in Phase 1. (Rationale: there's no symbol to point at; the
declaration only changes the resolver's behavior for subsequent
occurrences.)

### Example 5 — `type_use` rows tracking parameter / return / catch types

**Source:** `app/Http/Controllers/OrderController.php` lines 24–32

```php
public function __construct(
    CartService $cartService,
    PaymentService $paymentService,
    EmailService $emailService
) {
    $this->cartService = $cartService;
    $this->paymentService = $paymentService;
    $this->emailService = $emailService;
}
```

Symbol ids:
- `__construct`:
  `app/Http/Controllers/OrderController.php|24|4|__construct|constructor`
- `CartService` class (defined in another file): resolved via the
  file's `use App\Services\CartService;` at line 9 →
  `app/Services/CartService.php|9|0|CartService|class`. Same pattern
  for `PaymentService` and `EmailService`.

Rows:

```
# CartService parameter type at line 25
references { referrer_id: <__construct id>,
             referent_id: "app/Services/CartService.php|9|0|CartService|class",
             ref_kind: "type_use",
             site_file: "app/Http/Controllers/OrderController.php",
             site_start_byte: <byte of `CartService` at line 25> }

# PaymentService parameter type at line 26
references { referrer_id: <__construct id>,
             referent_id: "app/Services/PaymentService.php|7|0|PaymentService|class",
             ref_kind: "type_use",
             site_file: "app/Http/Controllers/OrderController.php",
             site_start_byte: <byte of `PaymentService` at line 26> }

# EmailService parameter type at line 27
references { referrer_id: <__construct id>,
             referent_id: "app/Services/EmailService.php|<line>|0|EmailService|class",
             ref_kind: "type_use",
             site_file: "app/Http/Controllers/OrderController.php",
             site_start_byte: <byte of `EmailService` at line 27> }
```

Inside the body, each `$this->cartService = $cartService;` assignment
produces:
- a `write` row on `$this->cartService` (referent = property symbol
  on `OrderController` at line 20, resolved via step 5 of the
  resolver);
- a `read` row on `$cartService` (referent = `null`, PHP local).

Plus the matching `import_use` rows on the file-level `use`
declarations themselves (see example 6 below).

### Example 6 — `import_use` rows for `namespace_use_declaration`

**Source:** `app/Http/Controllers/OrderController.php` lines 5–16

```php
use App\Models\Order;
use App\Models\OrderItem;
use App\Models\Product;
use App\Models\Coupon;
use App\Services\CartService;
use App\Services\PaymentService;
use App\Services\EmailService;
use App\Services\InventoryService;
use App\Services\ShippingService;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Log;
```

The file-level `use` declarations all attribute to a single referrer:
the **file-level pseudo-symbol** for `app/Http/Controllers/OrderController.php`.
This contract uses the file path itself as `referrer_id` for
file-scope imports — there is no symbol row to point at. (Rationale:
the schema's `referrer_id` is a free `String`; ADR-0002 names allow
file paths as ids by construction.)

For the first one (`use App\Models\Order;` at line 5):

```
references { referrer_id: "app/Http/Controllers/OrderController.php",
             referent_id: "app/Models/Order.php|8|0|Order|class",
             ref_kind: "import_use",
             site_file: "app/Http/Controllers/OrderController.php",
             site_start_byte: <byte of `App\\Models\\Order` at line 5> }
```

`Illuminate\Http\Request` (line 14): the framework's class is not in
the workspace index, so `referent_id = null`. The row is still
emitted, identical to the above pattern with the null `referent_id`.

**Resolution detail.** `import_use` rows are emitted *additionally*
to the existing `imports` table — they live in `references` so that
"every site where a name appears" queries return uniform rows. The
`imports` table answers "what does this file depend on?", which is
still the authoritative answer for cross-file dependency analysis.
