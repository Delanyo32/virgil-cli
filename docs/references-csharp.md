# References — C#

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes **fact emission** only. The C# extractor emits `scope` / `binding` / `occurrence` rows; the Cozoscript resolver in [`docs/resolution.md`](resolution.md) consumes them to materialise `references` rows. Resolution is not described here.

Symbol IDs follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

## Scope tree

C# has a five-level scope hierarchy:

| Source construct | `scope.kind` | Notes |
|---|---|---|
| The file itself | `"file"` | `parent_id = null`. Holds file-scoped `using` directives and (C# 10+) global usings. |
| `namespace foo { ... }` | `"namespace"` | Parent is the enclosing namespace / file. File-scoped namespace (`namespace foo;`) opens a namespace scope covering the rest of the file. |
| `class` / `struct` / `interface` / `record` body | `"class"` | Parent is the enclosing namespace / class. |
| Method / constructor / property accessor / local function / lambda / anonymous method body | `"function"` | Parameters bind here. |
| `{ ... }` block inside a method | `"block"` | `if`, `for`, `foreach`, `while`, `using`, `try`, `catch`, `finally` bodies all open block scopes. |

Top-level statements (C# 9+) live in the synthetic `Main` method's `function` scope.

## Bindings

### `definition`

One row per:
- `class`, `struct`, `interface`, `record`, `enum`, `delegate` declarations
- Method, constructor, destructor, property, indexer, event, field declarations
- Local variable declarations (`var x = …` / `T x = …` inside a block)
- Local functions (C# 7+)
- `for (int i = 0; ...; ...)` loop variable
- `foreach (var x in …)` iteration variable
- `using (var s = …)` resource variable
- `out var n` and pattern variables (`is T t`) bind in the enclosing block scope

### `parameter`

Method, constructor, indexer, delegate, lambda parameters. `out`, `ref`, `in`, `params`, and `this` (extension method receiver) all bind as `parameter`. Discard `_` does NOT bind.

### `import`

C# has no per-name `import`. `using` directives are always namespace-level wildcards or aliases.

### `import_alias`

`using IO = System.IO;` — binds the alias name to the target namespace. `symbol_id` points at the target's synthetic namespace id (or type symbol id for `using T = Namespace.Type`).

### `wildcard_import`

Three sources, all emitting one row per directive in the file or namespace scope:
- `using Some.Namespace;` — `symbol_id` = the target namespace's id.
- `using static Some.Type;` — `symbol_id` = the type symbol id. The resolver expands to all static members.
- `global using ...` (C# 10+) — same shape, but emitted in every file's file scope.

## Occurrence emission

### `call`

Every `invocation_expression` whose function is an identifier or member access:

```csharp
Foo(x);              // call: "Foo"
obj.Bar(x);          // read: "obj"; "Bar" not emitted (field-row policy)
T.StaticMethod(x);   // type_use: "T"; "StaticMethod" not emitted
this.Helper();       // read: "this"; "Helper" not emitted
base.Foo();          // read: "base"; "Foo" not emitted
```

Constructor calls (`new Foo(x)`) emit `call` of `Foo` AND `type_use` of `Foo`.

### `read`

Every identifier in value position. Includes `this`, `base`, pattern variables in use position, range-for iteration variables, LHS of `.` / `?.`.

**Excluded:** `nameof(X)` is intentionally NOT emitted as an occurrence (per contract review).

### `write`

Every assignment LHS that's a plain identifier. Compound `+=` etc. → single `write` per ADR-0003. `out` and `ref` parameter passing emit a `write` for the argument identifier at the call site:

```csharp
int.TryParse(s, out var n);   // emits write of "n"
DoSomething(ref counter);     // emits write of "counter"
```

Property writes (`obj.Prop = …`) emit `read` of `obj` only. The resolver discovers the property symbol via the class's binding; if it exists, the resolver emits a `references{ref_kind: "write"}` against the property symbol (field-row policy).

Auto-property semantics: the property itself is the symbol — reads and writes are recorded against the property symbol by the resolver, not by emitting synthetic `get_X` / `set_X` occurrences.

### `type_use`

Every type-position identifier. Includes:
- Variable / field / parameter / return type annotations
- Generic type arguments (`List<int>`)
- Inheritance / interface lists
- Cast targets (`(T)x`, `x as T`)
- `typeof(T)` operand
- Attribute usage (`[ApiController]` → `type_use` of `ApiController`)
- Constraints (`where T : SomeClass`)

`var` emits NO `type_use`. Nullable annotations (`string?`) do not change occurrence emission.

### `import_use`

Identifiers inside `using Some.Namespace;` emit `import_use` occurrences for each path segment.

## What this contract does NOT cover

- Resolution (in [`docs/resolution.md`](resolution.md))
- Method overload dispatch (resolver `match_index`)
- Auto-property accessor synthesis
- Type inference for `var` / `dynamic`
- `nameof(X)` (deliberately excluded)
- Reflection / dynamic dispatch

## Worked examples

All examples drawn from `../virgil-skills/benchmarks/csharp/dotnet-api/`.

### Example 1 — `[ApiController]` attribute usage

```csharp
[ApiController]
[Route("api/[controller]")]
public class UsersController : ControllerBase { ... }
```

**`occurrence`:**
| name | kind |
|---|---|
| `ApiController` | type_use |
| `Route` | type_use |
| `ControllerBase` | type_use |

### Example 2 — Aliased `using`

```csharp
using IO = System.IO;

class FileReader {
    private IO.Stream stream;
}
```

**`binding`** (in file scope):
| name | kind | symbol_id |
|---|---|---|
| `IO` | import_alias | `<System.IO namespace synthetic id>` |

**`occurrence`** (in the field declaration):
| name | kind |
|---|---|
| `IO` | type_use |
| `Stream` | type_use |

Namespace member references through an alias DO emit `type_use` for both components. (Class-member references through an instance follow the field-row policy.)

### Example 3 — `out` parameter call site

```csharp
public bool TryGetUser(string id, out User user) {
    if (int.TryParse(id, out var n)) {
        user = LookupById(n);
        return true;
    }
    user = null;
    return false;
}
```

**`binding`** (in method body):
| name | kind | start_byte |
|---|---|---|
| `n` | definition | `<byte of "var n">` |

**`occurrence`** (selected):
| name | kind |
|---|---|
| `int` | type_use |
| `id` | read |
| `n` | write | (the `out var n` declares + writes) |
| `user` | write |
| `LookupById` | call |
| `n` | read |

The `out var n` pattern both *binds* and *writes*: emit the `binding` row AND a `write` occurrence at the same site.

### Example 4 — `nameof(SomeMember)` (no occurrence emitted)

```csharp
public class Service {
    public string MemberName => nameof(Service);
}
```

**`occurrence`** inside `nameof(...)`: none.

Per the contract review, `nameof` excludes its operand from `references` to keep dead-code analysis honest.

### Example 5 — Local function inside a method

```csharp
public int Process(IEnumerable<int> items) {
    int sum = 0;
    void Add(int x) { sum += x; }
    foreach (var item in items) Add(item);
    return sum;
}
```

**`scope`** for `Add`: `function` scope, parent = `Process`'s function scope.

**`binding`** (in `Process`'s function scope):
| name | kind |
|---|---|
| `items` | parameter |
| `sum` | definition |
| `Add` | definition |
| `item` | definition | (the foreach variable) |

**`occurrence`** (selected):
| name | kind | enclosing_symbol_id |
|---|---|---|
| `sum` | write | `<Add's sym>` (compound `+=`) |
| `x` | read | `<Add's sym>` |
| `Add` | call | `<Process's sym>` |
| `item` | read | `<Process's sym>` |
| `items` | read | `<Process's sym>` |

The local function captures `sum` from its enclosing scope via standard scope walking — no special closure-capture binding needed.
