# References — PHP

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes **fact emission** only. The PHP extractor emits `scope` / `binding` / `occurrence` rows; the Cozoscript resolver in [`docs/resolution.md`](resolution.md) consumes them. Resolution is not described here.

Symbol IDs follow [ADR-0002](adr/0002-symbol-id-scheme.md). PHP grammar is `LANGUAGE_PHP` (handles `<?php` tags) per CLAUDE.md.

## Scope tree

PHP has a four-level scope hierarchy:

| Source construct | `scope.kind` | Notes |
|---|---|---|
| The file itself | `"file"` | `parent_id = null`. Holds file-level `namespace` and `use` directives. |
| `namespace Foo\Bar;` or `namespace Foo { ... }` | `"namespace"` | Parent is file scope. Files without a `namespace` directive get a synthetic namespace row with `name = ""`. |
| `class`, `interface`, `trait`, `enum` body | `"class"` | Parent is the enclosing namespace. |
| `function` / method body / closure body | `"function"` | Parameters bind here. |

PHP has **no block scope** inside functions. The extractor does NOT emit `"block"` scopes. Arrow functions (`fn($x) => …`) open a new `function` scope.

## Bindings

### `definition`

One row per:
- `function_definition` (top-level function)
- `class_declaration`, `interface_declaration`, `trait_declaration`, `enum_declaration`
- Methods, properties, class constants inside a class body
- Top-level `const` declarations
- Local variable first-assignment inside a function (PHP creates the variable at first assignment)

`use Trait;` inside a class body emits a `binding{name: <trait short name>, kind: "import"}` in the class scope.

### `parameter`

Function / method / closure / arrow-function parameters. `$this` is implicit (resolver special case). Closure `use ($var)` captures emit `binding` of `binding_kind: "parameter"` in the closure scope, with `symbol_id` pointing at the captured variable in the enclosing scope.

### `import`

`use Namespace\Class;` — emit `binding{name: <last segment>, kind: "import", symbol_id: <Namespace\Class's id when resolvable>}` in the file or namespace scope. Grouped imports `use Namespace\{ClassA, ClassB};` emit one binding per class. `use function Foo\bar;` and `use const Foo\BAR;` use the same shape.

`use Trait;` *inside a class body* is also emitted as `import` in the class scope.

### `import_alias`

`use Namespace\Class as Alias;` — emit `binding{name: "Alias", kind: "import_alias", symbol_id: <Namespace\Class's id>}`.

### `wildcard_import`

PHP has no glob `use`. No `wildcard_import` rows are emitted from `use` directives.

## Occurrence emission

### `call`

```php
foo($x);                       // call: "foo"
$this->method($x);             // read: "$this"; "method" not emitted (field-row)
self::staticMethod($x);        // type_use: "self"
static::lateBoundMethod();     // type_use: "static"
parent::ctor();                // type_use: "parent"
SomeClass::create($x);         // type_use: "SomeClass"
```

`new SomeClass($x)` emits `call` of `SomeClass` AND `type_use` of `SomeClass`. Dynamic dispatch `$obj->{$name}()` emits no `call`.

### `read`

Every variable/identifier in value position. `occurrence.name` for variables includes the `$` prefix (e.g. `"$counter"`).

### `write`

Every assignment LHS that's a plain variable. Compound `+=`, `.=`, etc. → single `write` per ADR-0003. Property writes `$obj->prop = $x` emit `read` of `$obj`; property name not emitted (field-row).

`global $x;` declarations bind a name in the function scope pointing at the global. Destructuring `list(…) = $arr` / `[$a, $b] = $arr` emits one `write` per destructured target.

### `type_use`

Every type-position identifier:
- Parameter type hints, return types, property type declarations
- Class name in `new`, `instanceof`, `catch`
- `self`, `static`, `parent` in type position
- `implements` / `extends` lists
- PHP 8 attributes `#[Attribute]`

### `import_use`

Path components inside `use Namespace\Class;` emit `import_use` occurrences per segment.

## What this contract does NOT cover

- Resolution (in [`docs/resolution.md`](resolution.md))
- Magic methods (`__get`, `__set`, `__call`)
- Dynamic property access (`$obj->{$name}`)
- Eloquent magic properties — produce no occurrence for the magic name
- `eval`, `assert`, dynamic call mechanisms

## Worked examples

All examples drawn from `../virgil-skills/benchmarks/php/laravel-store/`.

### Example 1 — `use Trait;` inside a class

```php
namespace App\Models;

use Illuminate\Database\Eloquent\Model;
use HasFactory;

class Product extends Model {
    use HasFactory;
}
```

**`binding`:**
| scope_id | name | kind |
|---|---|---|
| (file) | `Model` | import |
| (file) | `HasFactory` | import |
| (namespace `App\Models`) | `Product` | definition |
| (class `Product`) | `HasFactory` | import |

Two distinct `HasFactory` bindings — file-level (namespace alias) and class-level (trait inclusion).

### Example 2 — Eloquent magic property

```php
$user = new User();
echo $user->name;
```

**`occurrence`:**
| name | kind |
|---|---|
| `User` | type_use, call |
| `$user` | write |
| `$user` | read | (LHS of `->`) |

The magic property `name` produces **no occurrence** (field-row policy + Eloquent magic). Resolver finds no binding for `name` in `User`'s class scope and emits no `references` row.

### Example 3 — Closure with `use ($var)` capture

```php
public function makeAdder($base) {
    return function ($x) use ($base) {
        return $x + $base;
    };
}
```

**`scope`:** outer function `makeAdder`, inner closure (both `function` kind, closure's parent is `makeAdder`).

**`binding`** (closure scope):
| name | kind | symbol_id |
|---|---|---|
| `$x` | parameter | `<closure's $x>` |
| `$base` | parameter | `<$base from makeAdder>` (captured) |

The `use ($base)` reifies the captured variable as a parameter-kind binding in the closure scope.

### Example 4 — Namespace use with alias

```php
use App\Repositories\UserRepository as UserRepo;

class UserService {
    public function __construct(private UserRepo $repo) {}
}
```

**`binding`** (file scope):
| name | kind | symbol_id |
|---|---|---|
| `UserRepo` | import_alias | `<UserRepository's id>` |
| `UserService` | definition | `<UserService's id>` |

**`occurrence`** (constructor):
| name | kind |
|---|---|
| `UserRepo` | type_use |
| `$repo` | parameter (also emits a binding) |

### Example 5 — `parent::method()` call

```php
class FastPipeline extends Pipeline {
    public function run() {
        parent::run();
        $this->processInternal();
    }
}
```

**`occurrence`** (inside `FastPipeline::run`):
| name | kind |
|---|---|
| `parent` | type_use |
| `$this` | read |

Method names `run` and `processInternal` are NOT emitted (field-row policy). The resolver, given the `extends` row from issue #13 + the `parent` keyword's semantics, can compute that `parent::run()` references `Pipeline::run` at resolution time.
