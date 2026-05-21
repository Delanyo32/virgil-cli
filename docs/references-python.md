# References — Python

This document is the contract for how Python identifier occurrences map to
the `references` relation. See
[`virgil-datalog-schema.md`](virgil-datalog-schema.md) for the relation
shape and [ADR-0003](adr/0003-level-3-types-and-references.md) for the
Level-3 commitment.

Every emitted row follows the schema in `docs/virgil-datalog-schema.md`:

```
references {
    referrer_id: String,
    site_file: String,
    site_start_byte: Int,
    match_index: Int =>         # 0 for the primary/only candidate; Python has no overloading, always 0
    referent_id: String?,       # null when the identifier can't be resolved to a symbol
    ref_kind: String,           # "read" | "write" | "type_use" | "import_use"
}
```

Unresolvable referents emit a single row with `referent_id = null` (per `docs/contract-review.md`, policy 1).

The `referrer_id` is the **innermost enclosing named symbol** (function,
method, or class). For module-level code with no enclosing symbol, the
referrer is the synthetic module symbol whose id is
`<path>|0|0|<module_basename>|module` (kind `module`; the extractor
emits this row up-front for every file). This keeps the column non-null.

## Lexical scope rules

Python has **only three scope levels** at the function/expression level:

1. **Local** — the body of the innermost `def` / `async def` / `lambda` /
   comprehension. There is **no block scope**: a name bound inside an `if`,
   `for`, `while`, `try`, or `with` is visible across the rest of the
   function. The function determines the scope, not the indentation block.
2. **Enclosing (non-local)** — for nested functions: each surrounding `def`
   in lexical order, walking outward until the module.
3. **Module (global)** — top-level names of the current file plus everything
   the file `import`s.

Class bodies are **not** an enclosing scope for nested functions inside the
class. A method does not see attributes of its class as bare names — it has
to write `self.x` or `cls.x` or `ClassName.x`. We model this faithfully:
the inside of a method skips the class scope when resolving free names.

A fourth conceptual scope, **builtins** (`len`, `print`, `range`, `int`,
…), is consulted last. Builtins resolve to a synthetic referent id
`<builtin>|0|0|<name>|builtin`. We do not emit `symbol` rows for builtins;
the id is used purely as a referent placeholder so downstream queries can
distinguish "resolved to builtin" from "unresolved".

### Binding rules (what counts as introducing a local)

A name is **local** to the innermost function if **any** of these appear
anywhere in the function body (Python's "find all bindings" rule):

- assignment target: `x = ...`, augmented `x += ...`, annotated `x: int = ...`
- `for x in ...`
- `with ... as x:`
- `except ... as x:`
- function/class definition `def x(...)` / `class x:`
- positional / keyword / `*args` / `**kwargs` parameter
- `import x`, `from m import x` (when nested inside a function)
- walrus `(x := ...)`
- comprehension target `[x for x in ...]` (in 3.x, comprehension targets
  are local to the comprehension expression, not the enclosing function —
  see "Comprehension scope" below)

A name is **non-local** despite a local assignment if the function body
contains a `nonlocal x` declaration; it then refers to the nearest
enclosing function's binding.

A name is **global** despite a local assignment if the function body
contains a `global x` declaration; it then refers to the module-level
binding.

### Comprehension scope

List/set/dict/generator comprehensions introduce their own implicit
function scope in Python 3. Comprehension targets and walrus assignments
inside a comprehension do **not** leak to the enclosing function. For
extraction purposes, comprehensions count as a fresh local scope nested
inside the enclosing function.

### Shadowing rules

- Later bindings inside the same scope **win** for resolution of subsequent
  reads (Python is single-assignment per name in a scope: re-binding
  overwrites). The `references` row for each occurrence resolves to the
  binding currently in scope at that occurrence; we walk top-down through
  the function and update the "current local" map as we go.
- A local binding fully shadows any enclosing or global with the same name
  **for the entire function body**, even on lines above the assignment.
  This matches Python's runtime behavior (`UnboundLocalError`).
- Imports at module scope shadow builtins of the same name: `from foo
  import int` → reads of `int` resolve to the import, not the builtin.

### Module-qualified names

`a.b.c` is parsed as nested `attribute` access. Only the **head**
identifier (`a`) gets a `references` row in Phase 1. Attribute components
(`b`, `c`) are not resolved against imported symbol tables yet (would
require import-graph traversal beyond a single file). They are still
recorded in `span` for the larger expression but not as separate
references.

Exception: `self.x` and `cls.x` are handled specifically (see `write`
below).

## `ref_kind` decision tree

For every identifier occurrence in the file, exactly one row is emitted
(or none if the identifier is a binding occurrence — see below). Binding
occurrences themselves do **not** produce `references` rows; they are
already captured as `symbol` rows via the existing extractor.

### `read`

Every `identifier` node whose role in its parent is "value being
evaluated". Patterns:

- right-hand side of any expression: `y = x + 1` (the `x` and `1` reads)
- argument in a `call`: `f(x, y)` → reads `f`, `x`, `y`
- iterable in `for x in iterable`
- subject in `if`, `while`, `return`, `assert`
- the value side of `x[k]` reads `x` and `k`
- function-call expression head: `foo()` reads `foo`; `obj.foo()` reads
  `obj` only (the `.foo` is an attribute access, not a standalone ref)
- decorator expression: `@dec` and `@dec(arg)` produce a `read` of `dec`
- default-value expressions in parameter lists: `def f(x=DEFAULT)` reads
  `DEFAULT`
- inside f-strings: `f"{name}"` reads `name`

Exceptions / non-reads:

- the `string` content of an f-string outside `{...}` is not parsed for
  identifiers
- the LHS identifier of an `as` clause in `import x as y` is an
  `import_use` (see below), not a read

### `write`

Every assignment target. Patterns:

- `x = ...` — `x` is a write
- `x += ...`, `x -= ...`, etc. — `x` is a single `write` row.
  Updated per `docs/contract-review.md`: compound assignments emit
  one `write` row, not `read + write`. Faithful read+write
  semantics is Level 4.
- annotated assign `x: T = ...` — `x` is a write; `T` is a `type_use`
- `for x in ...` — `x` is a write (loop variable binding)
- `with expr as x` — `x` is a write
- `except E as x` — `x` is a write; `E` is a `read` (it's the class being
  evaluated)
- tuple/list destructuring `a, b = pair` — `a` and `b` are each a write
- walrus `(x := expr)` — `x` is a write
- `self.x = value` — the receiver `self` is a `read`. A `write`
  row for the attribute `x` is emitted **only** when `x` has a
  known `symbol_id` in the store (per the standardized
  field-tracking policy in `docs/contract-review.md`). If no
  matching field symbol exists, no `write` row is emitted for the
  attribute. Class-level field symbols extracted by the Phase 2
  symbol pass make most `self.x` writes resolvable; pre-Phase 2,
  attribute writes against implicit fields produce no row.
- `cls.x = value` — same rule, against class scope.

A binding occurrence inside a `def` header (the parameter names
themselves) does **not** produce a `write` row, because the parameter is
itself a symbol (kind `parameter` is not currently extracted in the
existing module, but the parameter name's binding nature means we skip
it here too).

### `type_use`

Every `identifier` or `attribute` whose AST position is inside a `type`
node. Tie-in with `types-python.md`:

- parameter annotation: `def f(x: Foo)` → `Foo` is a `type_use`; its row
  resolves through the canonical-name pipeline.
- return annotation: `def f() -> Foo` → `Foo` is a `type_use`.
- generic args: `list[Foo]` → both `list` and `Foo` are `type_use`s.
- forward-reference strings: `def f(x: "Foo")` → the string contents
  `Foo` produce a `type_use` row at the string's `start_byte`.
- `cast(Foo, x)`, `isinstance(x, Foo)`, `issubclass(c, Foo)` are **not**
  type uses in this contract — the `Foo` argument is a plain `read`.
  These are runtime calls, not annotations, even though they semantically
  use the type.

`isinstance` and `issubclass` are explicit exceptions because they
participate in control flow; treating them as `read` keeps call-graph
analysis simple.

### `import_use`

Every identifier that appears inside an `import_statement` or
`import_from_statement`. Patterns:

- `import os` → `os` is an `import_use`; referent is the resolved module
  (file-path id) if internal, else `null`.
- `import foo.bar.baz` → only `foo.bar.baz` as one dotted_name → one
  `import_use` row at the start of the dotted name. We do not split into
  three rows.
- `from foo import bar, baz` → `foo` is an `import_use` for the module;
  `bar` and `baz` are each `import_use` rows for the imported names.
  Referent for `bar` is the symbol `bar` exported by the resolved
  `foo.py` if internal, else `null`.
- `from foo import bar as b` → one `import_use` row; the `referrer/site`
  is at `bar`'s position; the local name `b` does **not** get a second
  row.
- `from . import utils` → `utils` is `import_use`; referent is the
  resolved file (per `resolve_import` in `src/languages/python/queries.rs`).

The `imports` relation already records source/target file pairs;
`import_use` provides per-name resolution for cross-reference queries
("who uses this exported function?").

## `referent_id` resolution

Per identifier occurrence at byte offset `B` inside file `F`, enclosing
function `Fn` (or module `M`):

1. **Check `nonlocal` / `global` declarations** in the innermost function
   body containing `B`. If the name is declared `nonlocal x`, jump to
   step 3 (skip step 2). If declared `global x`, jump to step 4 (skip
   2 and 3).
2. **Local lookup**: walk the local-binding set of the innermost
   function. The most recent binding **before `B` in source order** is
   the referent. If there is no prior binding but the name is in the
   function's local-binding set (computed by pre-scanning all assignments
   in the body), the occurrence is a forward-use of a local that has not
   yet been bound — emit `referent_id = null` and `ref_kind = "read"`
   (this matches Python's `UnboundLocalError` semantics).
3. **Enclosing functions**: for each surrounding `def` outward, repeat
   step 2's logic. The first match wins.
4. **Module scope**: look up in the file's top-level symbols
   (`function`, `class`, `variable` rows from the existing extractor) and
   in the file's `imports` rows. The first match wins. Module-level
   symbols defined later in the file are visible to functions defined
   earlier (Python resolves names at call time), so source order at
   module level does **not** affect resolution.
5. **Builtins**: if the name is a Python builtin, referent =
   `<builtin>|0|0|<name>|builtin`.
6. **Unresolved**: `referent_id = null`. Emit the row anyway —
   downstream queries can filter on null to find dangling references.

When multiple candidates exist at the same scope level (impossible for
locals, possible at module level when the same name is both
top-level-defined and imported), the later-in-source-order binding wins.
This matches Python runtime behavior.

The resolver uses the existing `symbols_by_name` index from
`src/graph/builder.rs` for the module-scope step. The function-scope step
needs a fresh per-function local-binding map, built by a single
top-down walk of the function body.

## Worked examples

All paths below are relative to
`virgil-skills/benchmarks/python/technical-debt/`. Byte offsets in the
example tables are illustrative; the implementation must use tree-sitter's
`Range.start_byte`.

### Example 1 — `read` + `import_use` in a function call

`app/database.py:30–34`:

```python
def get_connection():
    db_path = DATABASE_URL.replace("sqlite:///", "")
    conn = sqlite3.connect(db_path, timeout=TIMEOUT)
    conn.row_factory = sqlite3.Row
    return conn
```

`referrer_id = app/database.py|30|0|get_connection|function` throughout.

| occurrence (line:col)      | ref_kind     | referent_id (resolved)                                       |
|----------------------------|--------------|--------------------------------------------------------------|
| `db_path` at 31:4          | `write`      | none (local binding; row emitted with `referent_id` pointing at the local — the local has no `symbol` row, so emit `null`) |
| `DATABASE_URL` at 31:14    | `read`       | `app/config.py|<line>|0|DATABASE_URL|variable` (via `imports`) |
| `conn` at 32:4             | `write`      | `null`                                                       |
| `sqlite3` at 32:11         | `read`       | `<builtin>|0|0|sqlite3|builtin` (stdlib module, not internal) — actually resolves through the `import sqlite3` row; referent = the import target which is external → `null` |
| `db_path` at 32:27         | `read`       | `null` (local; same as above)                                |
| `TIMEOUT` at 32:44         | `read`       | `app/config.py|<line>|0|TIMEOUT|variable`                    |
| `conn` at 33:4             | `read`       | `null`                                                       |
| `sqlite3` at 33:23         | `read`       | `null` (external module)                                     |
| `conn` at 34:11            | `read`       | `null`                                                       |

> Ambiguity resolution: locals that have no `symbol` row of their own
> (parameters, internal `x = ...` variables not extracted as symbols)
> always resolve to `referent_id = null`. The row is still emitted so
> "all reads of name `db_path` in this function" can be counted.

The `.replace`, `.connect`, `.row_factory`, `.Row` attribute parts
generate **no** rows in Phase 1 (only the head identifier of an attribute
chain is recorded).

### Example 2 — `global` declaration and module-level write

`app/workers.py:76–80`:

```python
def enqueue(self, job_type, payload, priority=0):
    """Add a job to the queue. No locking around list mutation."""
    global _job_counter
    # Race condition: non-atomic read-modify-write
    _job_counter += 1
    job_id = f"job-{_job_counter}-{generate_id()[:8]}"
```

`referrer_id = app/workers.py|76|4|enqueue|method`.

| occurrence (line:col)         | ref_kind | referent_id                                              |
|-------------------------------|----------|----------------------------------------------------------|
| `_job_counter` at 80:4        | `write`  | `app/workers.py|43|0|_job_counter|variable`              |
| `_job_counter` at 81:24       | `read`   | `app/workers.py|43|0|_job_counter|variable`              |
| `generate_id` at 81:38        | `read`   | `app/utils.py|<line>|0|generate_id|function` (via `from app.utils import ... generate_id`) |

`_job_counter += 1` produces **one** `write` row at the `_job_counter`
byte position (updated per `docs/contract-review.md`: compound
assignments are single-row `write` at Level 3). The `global`
declaration on line 78 redirects both writes and reads of
`_job_counter` to the module-scope binding defined at line 43
(`_job_counter = 0`).

Without the `global` declaration, `_job_counter += 1` would create a
**local** `_job_counter` (per Python's binding rules) and reads from line
81 would resolve to that local. The `global` declaration is the
ambiguity-resolving signal.

### Example 3 — Shadowing in nested function

`app/tasks.py:147–175`:

```python
def send_notification_batch(notifications):
    """..."""
    _stats["tasks_started"] += 1

    async def _send_one(notification):
        """Async function that does synchronous I/O."""
        time.sleep(0.05)
        conn = _get_db()
        cursor = conn.cursor()
        cursor.execute(
            f"INSERT INTO audit_log ... "
            f"VALUES ('notification', 0, 'send', {notification.get('user_id', 0)}, "
            f"'{json.dumps(notification)}')"
        )
        conn.commit()
        conn.close()
        return {"notification_id": notification.get("id", generate_id()), "status": "sent"}
```

Inside `_send_one`, the parameter `notification` shadows nothing from
the outer function (the outer parameter is `notifications`, a different
name). But `conn` is bound twice across the program — at module level
in many places, and locally here. Local wins:

`referrer_id = app/tasks.py|154|4|_send_one|function` for body
references; `referrer_id = app/tasks.py|147|0|send_notification_batch|function`
for line 152.

Key rows:

| occurrence                                | ref_kind | referent_id                                                |
|-------------------------------------------|----------|------------------------------------------------------------|
| `_stats[...]` at 152:4                    | `write`  | `app/tasks.py|<line>|0|_stats|variable` (compound `+=` → single write row per `docs/contract-review.md`) |
| `time` at 157:8                           | `read`   | `null` (external import)                                   |
| `_get_db` at 160:15                       | `read`   | `app/tasks.py|<line>|0|_get_db|function`                   |
| `conn` at 160:8                           | `write`  | `null` (local)                                             |
| `conn` at 161:17                          | `read`   | `null` (local; resolves to the write on 160)               |
| `notification` at 164:48                  | `read`   | `null` (parameter; resolves locally)                       |
| `json` at 165:14                          | `read`   | `null` (external import)                                   |

### Example 4 — `nonlocal` (synthetic, illustrating the rule)

The benchmark corpus contains no `nonlocal` usage. The rule still has to
be specified — here is a minimal synthetic example following the same
benchmark conventions, to be added if a future benchmark file uses it:

```python
def make_counter():
    count = 0
    def increment():
        nonlocal count
        count += 1
        return count
    return increment
```

Inside `increment` (`referrer_id = ...|increment|function`):

| occurrence (line:col)  | ref_kind | referent_id                              |
|------------------------|----------|------------------------------------------|
| `count` at line 5      | `write`  | resolves to enclosing `make_counter`'s `count` write at line 2 — `referent_id = null` (no symbol row for function-local variable) |
| `count` at line 6      | `read`   | same                                     |

The `nonlocal` keyword does not generate its own row; it only redirects
the resolver. The `+=` site emits a single `write` row (per
`docs/contract-review.md`, compound assignment is single-row at
Level 3). Without `nonlocal`, `count += 1` would shadow with a fresh
local — the contract still emits one row for the LHS occurrence,
with `referent_id = null`.

### Example 5 — `self.x` writes inside `__init__`, attribute resolution

`app/workers.py:69–74`:

```python
def __init__(self, max_size=1000):
    self.max_size = max_size
    self._jobs = []
    self._pending_jobs = {}
    self._completed_count = 0
    self._failed_count = 0
```

`referrer_id = app/workers.py|69|4|__init__|method`.

| occurrence                       | ref_kind | referent_id                                          |
|----------------------------------|----------|------------------------------------------------------|
| `self` at 70:8                   | `read`   | `null` (parameter)                                   |
| `max_size` at 70:24              | `read`   | `null` (parameter)                                   |

Updated per `docs/contract-review.md` (policy 5): `self.<attr> =
value` produces a `write` row for the attribute **only** when the
attribute has a known field `symbol_id`. None of `max_size`,
`_jobs`, `_pending_jobs`, `_completed_count`, `_failed_count` are
extracted as class-level field symbols in this example, so no
attribute `write` rows are emitted. The previous heuristic of
emitting `null`-referent writes for every implicit field is dropped.
Once the Phase 2 symbol pass extracts implicit fields, the same
file will produce one `write` row per attribute with a resolved
`referent_id`.

### Example 6 — `type_use` and `import_use` in a function header

`app/serializers.py:350`:

```python
def serialize_dashboard_data(stats: dict[str, Any], alerts: dict[str, Any]) -> dict[str, Any]:
```

`referrer_id = app/serializers.py|350|0|serialize_dashboard_data|function`.

| occurrence                  | ref_kind   | referent_id                                                |
|-----------------------------|------------|------------------------------------------------------------|
| `dict` at 350:36            | `type_use` | `<builtin>|0|0|dict|builtin`                               |
| `str` at 350:41             | `type_use` | `<builtin>|0|0|str|builtin`                                |
| `Any` at 350:46             | `type_use` | `null` (resolved through `from typing import Any` if present at top of file; if not present, treat as `typing.Any` builtin alias — emit `null` if no import row exists) |
| `dict` at 350:60            | `type_use` | `<builtin>|0|0|dict|builtin`                               |
| ... (same for `str`, `Any`) |            |                                                            |
| return-pos `dict` at 350:85 | `type_use` | `<builtin>|0|0|dict|builtin`                               |

Plus the module-level `from typing import Any` line (file imports) would
have emitted an `import_use` row for `Any`:

| occurrence (in import line)    | ref_kind     | referent_id |
|--------------------------------|--------------|-------------|
| `typing` (the module path)     | `import_use` | `null` (external) |
| `Any` (imported name)          | `import_use` | `null` (external) |

> Ambiguity resolution: `Any` resolves to `null` because we do not index
> the standard library. A query that wants "all uses of `typing.Any`"
> joins through the `display_name` text on `type`, not through
> `referent_id`.
