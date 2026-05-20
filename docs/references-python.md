# References — Python

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes
**fact emission** only: how the Python extractor emits `occurrence` /
`scope` / `binding` rows. Resolution (`occurrence` → `referent_id`) lives
in [`docs/resolution.md`](resolution.md) as Cozoscript rules and is
language-agnostic — this file does not describe it and worked examples
do not list expected `references` rows.

Symbol ids follow [ADR-0002](adr/0002-symbol-id-scheme.md):
`path|start_line|start_col|name|kind`. Relation shapes live in
[`docs/virgil-datalog-schema.md`](virgil-datalog-schema.md).

## Scope tree

Python has only three meaningful lexical scope levels:

| Opens a scope | Emit `scope.kind` | Notes |
|---|---|---|
| File (top of module) | `"module"` | `parent_id = null`. Always exactly one per file. |
| `class Foo:` body | `"class"` | Parent is the enclosing scope (usually `module`). |
| `def f(...)` / `async def f(...)` body | `"function"` | Parameters bind in this scope. |
| `lambda ...` body | `"function"` | Lambdas open a function scope identical in shape to `def`. |
| `[ ... for x in ... ]` / `{ ... }` / `( ... )` (list/set/dict/generator comprehensions) | `"function"` | Python 3+: comprehensions are their own implicit function scope. The comprehension target (`x`) binds inside this scope and does NOT leak to the enclosing function. |

There is **no block scope**. `if`, `for`, `while`, `try`, `with`,
`match`, `else` and their bodies do **not** open scopes. A name bound
inside any of these is visible across the rest of the enclosing function
or module — the function/module is the scope, not the indentation block.

Class bodies are emitted as `"class"` scopes but they are **not** an
enclosing scope for nested functions. The resolver in `resolution.md`
handles this by treating class scopes as transparent during name
lookup from inside a method (it's a resolver-side concern; the extractor
still emits the `"class"` scope row faithfully).

`parent_id` is the innermost enclosing scope's id. `id` is
`<file_path>|<start_byte>|<kind>` per the schema.

## Bindings

For every binding pattern below, emit one `binding` row with
`scope_id = <innermost enclosing scope>`, `name = <bound name>`,
`start_byte = <tree-sitter start_byte of the binding occurrence>`,
`symbol_id = <id of the bound symbol, or null if external/unresolvable>`.

### `definition`

Definition sites that introduce a name in their enclosing scope:

- `def f(...)` / `async def f(...)` — binds `f` in the enclosing scope.
- `class C:` — binds `C` in the enclosing scope.
- Module-level assignment `X = ...` (including `X: T = ...`) at module
  scope — binds `X`.
- Function-local assignment `x = ...` where the name is not declared
  `global` / `nonlocal` in the same function — binds `x` in the
  function scope.
- `for x in ...` — binds `x`.
- `with expr as x:` — binds `x`.
- `except E as x:` — binds `x`.
- Tuple/list destructuring `a, b = pair` — one row per target name.
- Walrus `(x := expr)` — binds `x` in the **enclosing function** scope
  (the comprehension-walrus exception); inside an ordinary expression,
  binds in the enclosing function.
- Comprehension target `[expr for x in iter]` — binds `x` in the
  comprehension's own scope.
- `global x` / `nonlocal x` declarations inside a function body — emit
  one `binding` row with `binding_kind = "definition"` and
  `symbol_id = null` at the declaration site. The presence of this
  binding is what the resolver needs to know the name escapes the
  function; the resolver walks outward to find the target.

`symbol_id` for `definition` bindings matches the `symbol` row's id when
the symbol pass emits one (functions, classes, module-level variables,
parameters). For function-local variables that the symbol pass does not
extract, emit `symbol_id = null`.

### `parameter`

Every positional, keyword, `*args`, and `**kwargs` parameter in a `def`
/ `async def` / `lambda` header. `scope_id` is the function/lambda's own
scope. `symbol_id` matches the parameter's `symbol` row when emitted by
the symbol pass; otherwise `null`.

`self` and `cls` are emitted as ordinary `parameter` bindings — no
special-casing in the extractor.

### `import`

`import foo` and dotted forms `import foo.bar.baz` — emit one binding
in the **module** scope with `name = "foo"` (the head identifier — the
local name introduced by the import). `symbol_id = null` when the target
is external (stdlib, third-party, anything outside the indexed
workspace); otherwise the id of the imported module/file symbol.

### `import_alias`

`import foo as bar` — emit one binding at module scope with
`name = "bar"`, `binding_kind = "import_alias"`,
`symbol_id = <id of foo>` (or `null` if external).

`from foo import bar as baz` — emit one binding at module scope with
`name = "baz"`, `binding_kind = "import_alias"`,
`symbol_id = <id of bar in foo.py>` (or `null` if external).

Imports inside a function body bind in the **function** scope, not the
module scope. Same rules otherwise.

### `wildcard_import`

`from foo import *` — emit one binding at module scope with
`name = "*"`, `binding_kind = "wildcard_import"`, `symbol_id = null`.
The resolver expands wildcards at materialise time using the `imports`
graph (see `docs/resolution.md`).

## Occurrence emission

For every identifier in the file, emit at most one `occurrence` row.
Binding occurrences (the LHS of `x = ...`, parameter names in a `def`
header, etc.) emit a `write` occurrence per the rules below — they do
not get suppressed.

For every occurrence: `enclosing_symbol_id` is the innermost symbol
containing the occurrence (`null` for module-level expressions outside
any function/class). `enclosing_scope_id` is the innermost `scope` row.
`id` is `<path>|<start_byte>|<name>|<occurrence_kind>` per the schema.

### `call`

The identifier in callee position of every call expression.

- `foo(...)` — `call` of `foo`.
- `obj.foo(...)` — emit `read` of `obj` (the head of the attribute
  chain). The `.foo` attribute component does NOT emit its own
  occurrence. The resolver may emit a `references` row for the method
  call when the class of `obj` has a known field/method symbol; the
  extractor itself does not.
- `ClassName(...)` constructor call — `call` of `ClassName`.

### `read`

Every `identifier` in value position that isn't already covered by
`call`, `write`, `type_use`, or `import_use`:

- RHS of expressions: `y = x + 1` → `read` of `x`.
- Argument in a call: `f(a, b)` → `read` of `a` and `b` (and `call` of
  `f`).
- Iterable in `for x in iterable` → `read` of `iterable`.
- Subject of `if`, `while`, `return`, `assert`, `yield`.
- Default-value expressions in parameter lists: `def f(x=DEFAULT)` →
  `read` of `DEFAULT` (binding for `x` is `parameter`).
- Inside f-strings: `f"{name}"` → `read` of `name`. F-string literal
  text outside `{...}` is not parsed.
- Decorator expression: `@dec` and `@dec(arg)` both emit a `read` of
  `dec` (and `read` of `arg`). The decorator is just a name evaluated
  at definition time.
- `except E as x` — `read` of `E` (the exception class). The `x` is a
  `write` binding occurrence.

Attribute chain rule: for `a.b.c`, only the head `a` produces an
occurrence (`read`). The `.b` and `.c` segments produce no rows. The
resolver may emit attribute-aware `references` rows when class-field
symbols exist; the extractor stays head-only.

### `write`

Every assignment LHS, compound-assignment LHS, and binding occurrence
that the binding rules above produce:

- `x = ...` → `write` of `x`.
- `x += ...`, `x -= ...`, etc. — single `write` row, no read row (per
  ADR-0003: compound assignments are write-only at Level 3).
- Annotated `x: T = ...` → `write` of `x`; `T` is a `type_use`.
- `for x in ...` → `write` of `x`.
- `with expr as x:` → `write` of `x`.
- `except E as x:` → `write` of `x` (and `read` of `E`).
- Tuple/list destructuring `a, b = pair` → one `write` per target.
- Walrus `(x := expr)` → `write` of `x`.
- `self.x = value` — emit `read` of `self`. The `.x` attribute does
  **not** emit its own occurrence. The resolver may emit a `write`
  reference for the field when the class has a known field symbol —
  that's a resolver concern, not the extractor's. Same rule for
  `cls.x = value`.

Parameter names in a `def` / `lambda` header do not emit `write`
occurrences — they are covered by `parameter` bindings. Default-value
expressions in the same header still emit their normal `read`
occurrences.

### `type_use`

Identifiers in PEP 484 annotation position only:

- Parameter annotation: `def f(x: Foo)` → `type_use` of `Foo`.
- Return annotation: `def f() -> Foo` → `type_use` of `Foo`.
- Generic arguments: `list[Foo]` → `type_use` of both `list` and `Foo`.
- Annotated assignment: `x: T = ...` → `type_use` of `T`.
- Forward-reference strings: `def f(x: "Foo")` → `type_use` of `Foo`
  emitted at the string's start_byte.

`isinstance(x, Foo)`, `issubclass(c, Foo)`, `cast(Foo, x)`,
`typing.get_args(...)`, etc. are **not** type uses — `Foo` is a plain
`read`. They are runtime calls, not annotations, even if they
semantically inspect a type.

Duck-typed code (no annotation) produces no `type_use` occurrences. We
emit `type_use` exclusively from explicit annotations; we do not infer
types from usage.

These occurrences overlap exactly with the `type` rows that
`types-python.md` emits — each `type_use` corresponds to one identifier
inside one `type` row.

### `import_use`

Identifiers inside `import_statement` or `import_from_statement`:

- `import os` → `import_use` of `os`.
- `import foo.bar.baz` → one `import_use` for the dotted name as a
  whole, at the start_byte of `foo`. Do not split into three rows.
- `from foo import bar, baz` → `import_use` of `foo` (module side) and
  one `import_use` each for `bar` and `baz` (the imported names).
- `from foo import bar as b` — one `import_use` at `bar`'s position; the
  local-name `b` does not get a second occurrence (it is captured by the
  `import_alias` binding).
- `from . import utils` → `import_use` of `utils`.
- `from foo import *` — no `import_use` for `*` itself; the wildcard
  binding covers it.

## What this contract does NOT cover

- **Resolution / `referent_id`.** Lives in `docs/resolution.md`. The
  resolver consumes the rows specified here.
- **`references` rows.** Worked examples below show only emitted
  `scope` / `binding` / `occurrence` rows — the inputs to resolution.
- **Runtime-only constructs.** `getattr(obj, name)`, `setattr(...)`,
  `eval(...)`, `exec(...)`, `__import__(...)`, `globals()[...]` etc.
  produce ordinary `call` occurrences for the builtin and `read`
  occurrences for their arguments, but no special bindings or
  references — the targets are not statically determinable.
- **Method dispatch.** Attribute parts of chains (`obj.method()`) emit
  no occurrence for `method`. See the resolver spec for how the
  `references` view fills these in when class-field symbols exist.
- **Class-body bindings as method scopes.** Methods defined inside a
  class still emit a `class` scope row; the resolver decides that name
  lookup from inside the method skips the class.

## Worked examples

All paths are relative to
`virgil-skills/benchmarks/python/technical-debt/`. Byte offsets and line
numbers are illustrative; the implementation uses tree-sitter's
`Range.start_byte`. Symbol ids in `symbol_id` columns follow ADR-0002.
For brevity, occurrence `id` and `scope.id` strings are abbreviated to
`<occ:line:col>` and `<scope:kind@line>`; the schema dictates the real
form.

In each table:
- `scope` shows the rows the extractor emits for the snippet (file
  scope is assumed already emitted).
- `binding` shows new bindings introduced by the snippet.
- `occurrence` shows every occurrence inside the snippet.
- Existing module-scope bindings (top-level imports, module-level
  functions) referenced by the snippet are noted in prose, not
  re-listed.

### Example 1 — `global` redirect

`app/config.py:133–161` (function `load_config`):

```python
def load_config():
    """..."""
    global DATABASE_URL, DATABASE_PATH, SECRET_KEY, API_KEY
    global MAX_RETRIES, TIMEOUT, CACHE_TTL, PAGE_SIZE, RATE_LIMIT

    DATABASE_URL = os.environ.get("DATABASE_URL", DATABASE_URL)
    DATABASE_PATH = os.environ.get("DATABASE_PATH", DATABASE_PATH)
    # ... (similar lines for SECRET_KEY, API_KEY, MAX_RETRIES, ...)
    return SETTINGS
```

Pre-existing module-scope bindings (emitted elsewhere): `os` (import),
`DATABASE_URL`, `DATABASE_PATH`, `SECRET_KEY`, `API_KEY`,
`MAX_RETRIES`, `TIMEOUT`, `CACHE_TTL`, `PAGE_SIZE`, `RATE_LIMIT`,
`SETTINGS`, `BASE_URL`, `FEATURE_FLAGS` — all `definition` bindings in
the module scope.

`scope` (new):
| id | parent_id | kind | start_byte |
|---|---|---|---|
| `<scope:function@133>` | `<scope:module>` | `function` | start of `load_config` body |

`binding` (new, in `<scope:function@133>`):
| name | binding_kind | symbol_id | notes |
|---|---|---|---|
| `DATABASE_URL` | `definition` | `null` | from `global DATABASE_URL` declaration |
| `DATABASE_PATH` | `definition` | `null` | from `global` declaration |
| `SECRET_KEY` | `definition` | `null` | |
| `API_KEY` | `definition` | `null` | |
| `MAX_RETRIES` | `definition` | `null` | |
| `TIMEOUT` | `definition` | `null` | |
| `CACHE_TTL` | `definition` | `null` | |
| `PAGE_SIZE` | `definition` | `null` | |
| `RATE_LIMIT` | `definition` | `null` | |

The `global` declarations emit `definition` bindings inside the function
scope with `symbol_id = null`. This is the trick that makes resolution
behave correctly: when the resolver walks outward from a `DATABASE_URL`
occurrence inside `load_config`, it hits this innermost binding first;
because `symbol_id` is null, the resolver continues walking outward to
the module scope and finds the real `DATABASE_URL` definition. See
`docs/resolution.md` for the rule that handles this.

`occurrence` (selected; one row per `DATABASE_URL` group shown,
others follow the same pattern):

| line:col | name | kind | enclosing_symbol_id | enclosing_scope_id |
|---|---|---|---|---|
| 140:4 | `DATABASE_URL` | `write` | `load_config|function` | `<scope:function@133>` |
| 140:19 | `os` | `read` | `load_config|function` | `<scope:function@133>` |
| 140:46 | `DATABASE_URL` | `read` | `load_config|function` | `<scope:function@133>` |
| 141:4 | `DATABASE_PATH` | `write` | `load_config|function` | `<scope:function@133>` |
| 141:19 | `os` | `read` | `load_config|function` | `<scope:function@133>` |
| 141:48 | `DATABASE_PATH` | `read` | `load_config|function` | `<scope:function@133>` |
| ... | ... | ... | ... | ... |
| 161:11 | `SETTINGS` | `read` | `load_config|function` | `<scope:function@133>` |

`.environ` and `.get` attribute segments emit no occurrences.

### Example 2 — `nonlocal` redirect

`app/workers.py` does not use `nonlocal`; this synthetic example follows
the same conventions for illustration:

```python
def make_counter():
    count = 0
    def increment():
        nonlocal count
        count += 1
        return count
    return increment
```

`scope`:
| id | parent_id | kind |
|---|---|---|
| `<scope:function@1>` | `<scope:module>` | `function` (body of `make_counter`) |
| `<scope:function@3>` | `<scope:function@1>` | `function` (body of `increment`) |

`binding`:
| scope | name | binding_kind | symbol_id |
|---|---|---|---|
| `<scope:function@1>` | `count` | `definition` | `null` (function-local var, no symbol row) |
| `<scope:function@1>` | `increment` | `definition` | `increment|function` |
| `<scope:function@3>` | `count` | `definition` | `null` (from `nonlocal count`) |

`occurrence`:
| line:col | name | kind | enclosing_symbol | enclosing_scope |
|---|---|---|---|---|
| 2:4 | `count` | `write` | `make_counter` | `<scope:function@1>` |
| 5:8 | `count` | `write` | `increment` | `<scope:function@3>` (compound `+=`, single write) |
| 6:15 | `count` | `read` | `increment` | `<scope:function@3>` |
| 7:11 | `increment` | `read` | `make_counter` | `<scope:function@1>` |

The `nonlocal count` declaration emits its `definition` binding with
null `symbol_id` (same trick as `global`). The resolver sees the
binding, finds null, walks outward, and resolves to
`make_counter`'s `count`.

### Example 3 — Aliased import

`app/api.py:30` (module-scope import):

```python
from app.auth import verify_token, CORS_HEADERS, login as auth_login
```

`scope`: no new scope (module-scope statement).

`binding` (all in `<scope:module>` of `app/api.py`):
| name | binding_kind | symbol_id |
|---|---|---|
| `verify_token` | `import` | `app/auth.py|<line>|0|verify_token|function` |
| `CORS_HEADERS` | `import` | `app/auth.py|<line>|0|CORS_HEADERS|variable` |
| `auth_login` | `import_alias` | `app/auth.py|<line>|0|login|function` |

`occurrence`:
| line:col | name | kind | enclosing_symbol | enclosing_scope |
|---|---|---|---|---|
| 30:5 | `app.auth` | `import_use` | `null` | `<scope:module>` (dotted name, single occurrence at head) |
| 30:21 | `verify_token` | `import_use` | `null` | `<scope:module>` |
| 30:35 | `CORS_HEADERS` | `import_use` | `null` | `<scope:module>` |
| 30:49 | `login` | `import_use` | `null` | `<scope:module>` |

The local name `auth_login` does **not** get its own `import_use`
occurrence — it is captured by the `import_alias` binding.

Similar pattern: `app/auth.py:341` shows a function-local aliased
import:

```python
from app.utils import verify_password as _verify_pw
```

— same shape, but the bindings land in the enclosing function scope, not
the module scope.

### Example 4 — Wildcard import

`app/__init__.py:6–7`:

```python
from app.config import *
from app.utils import *
```

`binding` (both in `<scope:module>` of `app/__init__.py`):
| name | binding_kind | symbol_id |
|---|---|---|
| `*` | `wildcard_import` | `null` |
| `*` | `wildcard_import` | `null` |

`occurrence`:
| line:col | name | kind | enclosing_symbol | enclosing_scope |
|---|---|---|---|---|
| 6:5 | `app.config` | `import_use` | `null` | `<scope:module>` |
| 7:5 | `app.utils` | `import_use` | `null` | `<scope:module>` |

No `import_use` row is emitted for the `*` character itself. The
resolver expands `*` at materialise time by looking up exported symbols
in `app/config.py` and `app/utils.py` via the `imports` graph (see
`docs/resolution.md`'s `wildcard_target` rule).

### Example 5 — Comprehension scope

`app/services.py:255–256`:

```python
low_stock = [d for d in data if d.get("quantity", 0) <= LOW_STOCK_THRESHOLD]
critical = [d for d in data if d.get("quantity", 0) <= CRITICAL_STOCK_THRESHOLD]
```

These lines sit inside a function (`generate_report` or similar — the
exact enclosing function is omitted for clarity). Call its scope
`<scope:function@N>`.

`scope` (new):
| id | parent_id | kind |
|---|---|---|
| `<scope:function@255>` | `<scope:function@N>` | `function` (the list comprehension on line 255) |
| `<scope:function@256>` | `<scope:function@N>` | `function` (the list comprehension on line 256) |

Each comprehension opens its own function-kind scope. The comprehension
target `d` is bound inside that scope and does not leak into
`<scope:function@N>`.

`binding`:
| scope | name | binding_kind | symbol_id |
|---|---|---|---|
| `<scope:function@N>` | `low_stock` | `definition` | `null` |
| `<scope:function@N>` | `critical` | `definition` | `null` |
| `<scope:function@255>` | `d` | `definition` | `null` (comprehension target) |
| `<scope:function@256>` | `d` | `definition` | `null` |

`occurrence` (line 255 only; line 256 is identical with
`CRITICAL_STOCK_THRESHOLD`):
| line:col | name | kind | enclosing_scope |
|---|---|---|---|
| 255:8 | `low_stock` | `write` | `<scope:function@N>` |
| 255:21 | `d` | `read` | `<scope:function@255>` |
| 255:32 | `d` | `write` | `<scope:function@255>` (comprehension target) |
| 255:37 | `data` | `read` | `<scope:function@255>` |
| 255:45 | `d` | `read` | `<scope:function@255>` (`d.get(...)` head; `.get` attribute emits nothing) |
| 255:73 | `LOW_STOCK_THRESHOLD` | `read` | `<scope:function@255>` |

The two `d` reads on line 255 resolve to the comprehension-local
binding, not to anything in `<scope:function@N>`. The two
comprehensions on lines 255 and 256 each have their own independent
`d` binding — they don't shadow each other because they are in sibling
scopes.

### Example 6 — Decorator as a `read`

`app/errors.py:144–155`:

```python
def convert_database_error(func):
    """Decorator that converts sqlite3 errors to DatabaseError
    but loses the original exception context (no 'from' clause)."""
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        try:
            return func(*args, **kwargs)
        except Exception as e:
            logger.error(f"Database operation failed: {e}")
            raise DatabaseError("Database operation failed")
    return wrapper
```

Pre-existing module-scope bindings (referenced): `functools` (import),
`DatabaseError` (definition or import), `Exception` (no binding — it's a
Python builtin; the resolver may attribute it to a synthetic builtin
referent), `logger` (module-level variable).

`scope`:
| id | parent_id | kind |
|---|---|---|
| `<scope:function@144>` | `<scope:module>` | `function` (body of `convert_database_error`) |
| `<scope:function@148>` | `<scope:function@144>` | `function` (body of `wrapper`) |

`binding`:
| scope | name | binding_kind | symbol_id |
|---|---|---|---|
| `<scope:module>` | `convert_database_error` | `definition` | `app/errors.py|144|0|convert_database_error|function` |
| `<scope:function@144>` | `func` | `parameter` | (param symbol id or `null`) |
| `<scope:function@144>` | `wrapper` | `definition` | `app/errors.py|148|4|wrapper|function` |
| `<scope:function@148>` | `args` | `parameter` | `null` |
| `<scope:function@148>` | `kwargs` | `parameter` | `null` |
| `<scope:function@148>` | `e` | `definition` | `null` (from `except ... as e`) |

`occurrence` (selected):
| line:col | name | kind | enclosing_scope | notes |
|---|---|---|---|---|
| 147:5 | `functools` | `read` | `<scope:function@144>` | decorator expression; `.wraps` attribute emits no occurrence |
| 147:20 | `func` | `read` | `<scope:function@144>` | argument to `functools.wraps(...)` |
| 150:19 | `func` | `call` | `<scope:function@148>` | `func(*args, **kwargs)` callee |
| 150:24 | `args` | `read` | `<scope:function@148>` | |
| 150:33 | `kwargs` | `read` | `<scope:function@148>` | |
| 151:15 | `Exception` | `read` | `<scope:function@148>` | `except E as x` — `E` is a read |
| 151:28 | `e` | `write` | `<scope:function@148>` | `as e` is a write |
| 152:12 | `logger` | `read` | `<scope:function@148>` | `.error` attribute emits no occurrence |
| 152:37 | `e` | `read` | `<scope:function@148>` | inside f-string `{e}` |
| 153:18 | `DatabaseError` | `call` | `<scope:function@148>` | constructor call |
| 154:11 | `wrapper` | `read` | `<scope:function@144>` | |

The key point: `@functools.wraps(func)` produces a plain `read`
occurrence of `functools` (the attribute chain head). The decorator is
just an expression evaluated at definition time — no special
`occurrence_kind`. The `.wraps` attribute does not get its own
occurrence; the call to `functools.wraps(...)` likewise produces no
`call` occurrence because the callee is an attribute expression, not a
bare identifier (the head `functools` is a `read`, not a `call`).
