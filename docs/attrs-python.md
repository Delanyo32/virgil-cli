# Language attributes — Python

This document is the contract for what populates the `python_attrs`
extension relation. See [`virgil-datalog-schema.md`](virgil-datalog-schema.md)
(§ "Language-specific extensions") for the pattern.

## Schema

```
:create python_attrs {
    symbol_id: String =>
    decorators: [String] default [],
    is_generator: Bool default false,
    is_coroutine: Bool default false,
    docstring_style: String?,
}
```

Rows are emitted **only for symbols of kind `function`, `method`, or
`class`**. Variables, parameters, fields, and constants do not get
`python_attrs` rows. Methods of all kinds (instance, `@classmethod`,
`@staticmethod`, `@property`) are included.

Per-column applicability:

| column           | applies to symbol kinds   | default          |
|------------------|---------------------------|------------------|
| `decorators`     | function, method, class   | `[]`             |
| `is_generator`   | function, method          | `false`          |
| `is_coroutine`   | function, method          | `false`          |
| `docstring_style`| function, method, class   | `null`           |

For classes, `is_generator` and `is_coroutine` are always `false` (class
definitions cannot themselves be coroutines or generators in Python).
The schema still requires the row to carry the columns; the defaults
suffice.

## Extraction rules

### `decorators: [String]`

Source AST: `decorated_definition` node. Each child `decorator` node
contains a decorator expression. The list is collected in **source order**
(top-to-bottom, which is the order they apply outermost-last).

For each `decorator`:

1. Extract the expression after the `@`.
2. Render it as source text, normalised: strip whitespace down to single
   spaces and drop any trailing newline. Keep the call syntax: if the
   decorator is `@deprecated("Use v2 instead")`, the string is
   `deprecated("Use v2 instead")` (with the argument list verbatim).
3. Drop the leading `@`. Each entry is the bare expression.

Edge cases:

- An undecorated function/class produces `decorators = []`. The row is
  still emitted (with the default empty list) so all functions have a
  `python_attrs` row.
- Multiple decorators: list contains one entry per `@` line, in source
  order. `@a` then `@b` produces `["a", "b"]`.
- `@module.dec` keeps the dotted form: `["module.dec"]`.
- `@dec(arg1, arg2)` keeps the full call: `["dec(arg1, arg2)"]`.

### `is_generator: Bool`

Source AST: scan the function body (the `block` child of
`function_definition` or `async_function_definition`) for **any**
`yield` or `yield_from` node at any depth that is **not** nested inside
a deeper `function_definition` or `lambda`. The presence of even one
`yield` makes the function a generator.

Edge case: `async def` + `yield` produces an **async generator**. In
that case we set **both** `is_generator = true` and `is_coroutine = true`.
Downstream queries can distinguish:

| pattern                              | `is_generator` | `is_coroutine` |
|--------------------------------------|----------------|----------------|
| `def f(): return x`                  | false          | false          |
| `def f(): yield x`                   | true           | false          |
| `async def f(): return x`            | false          | true           |
| `async def f(): yield x`             | true           | true           |

### `is_coroutine: Bool`

Source AST: the function's `def` node is `async_function_definition`
(or equivalently the `function_definition` has an `async` modifier
child, depending on grammar version). Tree-sitter's Python grammar uses
`function_definition` with a leading `async` child token.

`await` expressions inside the body are **not** required for the
attribute to be true; only the `async def` declaration matters. A
function declared `async` but containing no `await` is still a
coroutine.

### `docstring_style: String?`

Source AST: the function/class/module's first child statement, if it is
an `expression_statement` wrapping a `string` node (handled by
`is_docstring_position` in the existing extractor).

If no docstring exists: `docstring_style = null`.

If a docstring exists, classify it by scanning its raw text for marker
patterns. The first match wins; if none match, `docstring_style = null`.

| style    | markers (any one of these substrings, case-insensitive, anchored at line start after whitespace) |
|----------|--------------------------------------------------------------------------------------------------|
| `google` | `Args:` / `Arguments:` / `Returns:` / `Yields:` / `Raises:` / `Attributes:` / `Note:` (followed by a newline and then indented content) |
| `numpy`  | `Parameters\n----------` / `Returns\n-------` (a section header on one line, a row of `-` matching its length on the next) |
| `sphinx` | `:param ` / `:returns:` / `:return:` / `:rtype:` / `:raises ` / `:type ` (PEP 287 / reST field lists) |

Priority order when more than one matches: `numpy` > `sphinx` > `google`.
Numpy's underline pattern is the most distinctive; sphinx's `:field:`
syntax is unambiguous; Google-style markers (`Returns:`) can otherwise
appear in plain prose.

Edge cases:

- A docstring containing only a one-line summary with no markers:
  `docstring_style = null`. Common pattern in this codebase.
- Module-level docstrings are recorded against the synthetic module
  symbol (`<path>|0|0|<basename>|module`); the same classification rule
  applies. Module rows are still optional — only emit a `python_attrs`
  row for the module if a docstring is present (`decorators` etc. are
  all defaults).

## Worked examples

All paths below are relative to
`virgil-skills/benchmarks/python/technical-debt/`.

### Example 1 — `@deprecated("...")` decorator with one argument

`app/api.py:545–546`:

```python
@deprecated("Use GET /api/v2/inventory instead")
def get_inventory_legacy(headers, params):
    """GET /api/inventory/legacy — deprecated inventory endpoint.
    Still in use by 3 mobile clients that haven't upgraded.
    """
```

`symbol_id = app/api.py|546|0|get_inventory_legacy|function`.

`python_attrs` row:

| column            | value                                                |
|-------------------|------------------------------------------------------|
| `symbol_id`       | `app/api.py|546|0|get_inventory_legacy|function`     |
| `decorators`      | `["deprecated(\"Use GET /api/v2/inventory instead\")"]` |
| `is_generator`    | `false`                                              |
| `is_coroutine`    | `false`                                              |
| `docstring_style` | `null` (plain prose, no markers)                     |

Note: `start_line` for the symbol is the `def` line (546), not the
`@decorator` line. This follows ADR-0002 — the symbol id anchors on the
`function_definition` start, not the wrapping `decorated_definition`.
The decorator row contents are still gathered from the parent
`decorated_definition`.

### Example 2 — Generator function, no decorators

`app/utils.py:631–633`:

```python
def chunks(lst, n):
    for i in range(0, len(lst), n):
        yield lst[i:i + n]
```

`symbol_id = app/utils.py|631|0|chunks|function`.

`python_attrs` row:

| column            | value                                  |
|-------------------|----------------------------------------|
| `symbol_id`       | `app/utils.py|631|0|chunks|function`   |
| `decorators`      | `[]`                                   |
| `is_generator`    | `true` (contains `yield`)              |
| `is_coroutine`    | `false`                                |
| `docstring_style` | `null` (no docstring)                  |

The `yield lst[i:i + n]` on line 633 is at depth 2 (inside `for` block
inside the function body). It is **not** nested inside any inner `def`
or `lambda`, so it counts.

### Example 3 — Async function (coroutine, not generator)

`app/workers.py:515–527`:

```python
async def _async_fetch(url):
    """Async function that uses synchronous I/O — blocks the event loop."""
    # Should use aiohttp or asyncio-compatible HTTP client
    import urllib.request
    # time.sleep blocks the event loop — should use asyncio.sleep
    time.sleep(0.1)
    try:
        req = urllib.request.Request(url)
        response = urllib.request.urlopen(req, timeout=10)
        return json.loads(response.read().decode())
    except Exception:
        return None
```

`symbol_id = app/workers.py|515|0|_async_fetch|function`.

`python_attrs` row:

| column            | value                                          |
|-------------------|------------------------------------------------|
| `symbol_id`       | `app/workers.py|515|0|_async_fetch|function`   |
| `decorators`      | `[]`                                           |
| `is_generator`    | `false`                                        |
| `is_coroutine`    | `true` (declared with `async def`)             |
| `docstring_style` | `null` (one-line summary, no markers)          |

Note the body contains `return None` and no `await` — `is_coroutine` is
driven by the `async` modifier on the declaration, not by the presence
of `await`.

### Example 4 — Stacked decorators (`@property` and `@classmethod` patterns)

`app/models.py:1679–1691`:

```python
class ProductDetails:
    """Product with properties that hide expensive DB calls behind
    attribute-access syntax. ..."""

    def __init__(self, product_id):
        self._product_id = product_id

    @property
    def total_sold(self):
        """Looks like a simple attribute but executes a DB query on every access.
        Accessing product.total_sold in a loop causes N queries."""
        conn = get_connection()
        ...
        return row["total"] if row else 0
```

`symbol_id = app/models.py|1680|4|total_sold|method`.

`python_attrs` row:

| column            | value                                          |
|-------------------|------------------------------------------------|
| `symbol_id`       | `app/models.py|1680|4|total_sold|method`       |
| `decorators`      | `["property"]`                                 |
| `is_generator`    | `false`                                        |
| `is_coroutine`    | `false`                                        |
| `docstring_style` | `null` (plain prose, no markers)               |

### Example 5 — `@abstractmethod` on a method

`app/models.py:59–61`:

```python
class BaseEntity(ABC):
    _instances = {}

    def __init__(self, id=None):
        ...

    @abstractmethod
    def validate(self):
        pass
```

`symbol_id = app/models.py|60|4|validate|method`.

`python_attrs` row:

| column            | value                                          |
|-------------------|------------------------------------------------|
| `symbol_id`       | `app/models.py|60|4|validate|method`           |
| `decorators`      | `["abstractmethod"]`                           |
| `is_generator`    | `false`                                        |
| `is_coroutine`    | `false`                                        |
| `docstring_style` | `null`                                         |

### Example 6 — Sphinx-style docstring

`app/utils.py:189–208`:

```python
def calculate_discount(price, quantity, customer_type="standard"):
    """Calculate volume discount for a line item.

    Applies tiered discounts based on the customer's loyalty tier
    and the promotional calendar. Discounts are stacked with any
    active coupon codes.

    :param base_price: The original unit price before discounts.
    :param qty: The number of units being purchased.
    :param tier: Customer loyalty tier ('bronze', 'silver', 'gold', 'platinum').
    :param coupon_code: Optional coupon code to apply on top of volume discount.
    :returns: The final discounted unit price.
    :rtype: Decimal

    .. note::
        ...
    """
```

`symbol_id = app/utils.py|189|0|calculate_discount|function`.

`python_attrs` row:

| column            | value                                                 |
|-------------------|-------------------------------------------------------|
| `symbol_id`       | `app/utils.py|189|0|calculate_discount|function`      |
| `decorators`      | `[]`                                                  |
| `is_generator`    | `false`                                               |
| `is_coroutine`    | `false`                                               |
| `docstring_style` | `"sphinx"` (`:param ...`, `:returns:`, `:rtype:` markers) |

> Ambiguity resolution: the docstring marker scan must check the entire
> docstring body, not just the first few lines, because the summary
> paragraph appears first and contains no markers.

### Example 7 — Class with no decorators, with a docstring

`app/workers.py:61–67`:

```python
class JobQueue:
    """A job queue backed by a plain list instead of queue.Queue.

    Uses a list for the internal buffer, which is not thread-safe for
    concurrent append/pop operations. Should use queue.Queue but "it
    worked in testing" so it was never changed.
    """
```

`symbol_id = app/workers.py|61|0|JobQueue|class`.

`python_attrs` row:

| column            | value                                  |
|-------------------|----------------------------------------|
| `symbol_id`       | `app/workers.py|61|0|JobQueue|class`   |
| `decorators`      | `[]`                                   |
| `is_generator`    | `false` (always false for classes)     |
| `is_coroutine`    | `false` (always false for classes)     |
| `docstring_style` | `null` (no marker patterns)            |

The docstring exists but contains only prose — no `Args:`, no
`:param:`, no underlined section headers. Classification is `null`,
not `"unknown"` or `"plain"`. Downstream queries distinguishing
"has-docstring" from "has-typed-docstring" go through the existing
`comment` relation (with `kind = "doc"`) for the former and through
`python_attrs.docstring_style IS NOT NULL` for the latter.
