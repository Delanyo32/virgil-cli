# References — C#

The contract for how identifier occurrences in C# map to the `references` relation defined in [virgil-datalog-schema.md](virgil-datalog-schema.md). Resolution depth is Level 3 per [ADR-0003](adr/0003-level-3-types-and-references.md); symbol ids follow [ADR-0002](adr/0002-symbol-id-scheme.md).

`references` is keyed by `(referrer_id, site_file, site_start_byte, match_index)` with `referent_id` and `ref_kind` in the value position. `match_index = 0` for the primary/only candidate; overload resolution emits additional rows at `match_index = 1, 2, ...` sharing the same site. Unresolvable referents emit a single row with `referent_id = null` (per `docs/contract-review.md`, policy 1).

## Lexical scope rules

C# is lexically scoped with namespace-rooted name lookup. Scopes, innermost to outermost:

1. **Block scope.** Each `{ }` block introduces a scope. Locals declared inside are visible to the end of the block.
2. **Method (member) scope.** Parameters, type parameters, `this`/`base`.
3. **Type scope.** Members of the enclosing `class`/`struct`/`record`/`interface` — fields, properties, methods, nested types. Inherited members are also in scope (we resolve them as well, via the type's base list).
4. **Outer type scope.** For nested types, the enclosing type's members.
5. **Namespace scope.** Other types declared in any `namespace` block containing the reference site (across files in the workspace).
6. **`using` directives.** File-level imports, including alias `using X = Y;` and `using static T;`.
7. **Global namespace.** Top-level types declared without a namespace.

### Shadowing

- A local variable shadows a field, property, or outer local of the same name within its block. The C# compiler emits warning CS0136 in some cases but we follow the language rule: **innermost binding wins**.
- A method parameter shadows a field of the same name. (Standard `_camelCase` field convention exists precisely because of this.)
- `this.X` and `base.X` force type-scope lookup, bypassing block/local lookup entirely.

### Module-qualified names

`a.b.c` is parsed as a `member_access_expression`. We resolve it left-to-right:

1. Resolve the leftmost token `a` through the lookup walk (locals first, then type members, then namespace, then usings, then global).
2. Once `a` resolves to a symbol whose type or namespace is known, lookup of `b` happens *inside* that symbol's scope (member-of-type or namespace-of-namespace).
3. Continue until the last segment.

If at any step the receiver's type is unknown (e.g., `a` is a parameter of unindexed type), we record the first segment's reference and emit `null` referent_id for subsequent segments.

## `ref_kind` decision tree

### `read`

An identifier in an expression context where its current value is consumed.

AST patterns:

- `identifier_name` inside an `argument`, `binary_expression`, `assignment_expression` *RHS*, `return_statement`, `if_statement` condition, `for_statement` condition, `while_statement` condition, `switch_statement` operand, `interpolation`, indexer subscript, ternary condition/branches.
- `this_expression` and `base_expression` are `read` of the enclosing instance.
- The receiver in a `member_access_expression` (the `a` in `a.b`) is `read`. The member side `b` is **also** `read` unless the whole expression is the LHS of an assignment — see `write`.
- The callee in an `invocation_expression` (the `f` in `f(x)`) is `read`.
- Pattern-match bindings (`if (x is Project p)`) — `x` is `read`; `p` is a new local binding (no reference row, it's a declaration).
- Inside `nameof(X)` — **not emitted**. `nameof` is compile-time string extraction; we record neither read nor write. (Justification: querying `nameof` occurrences as references confuses dead-code analysis.)

Exceptions:

- An identifier inside a comment, XML doc, attribute target specifier (`[assembly: X]`), or preprocessor directive is never emitted.
- An identifier appearing only as a *type* (parameter type, return type, generic arg) is `type_use`, not `read`. See below.

### `write`

An identifier whose stored value is being mutated.

AST patterns:

- LHS of `assignment_expression` with operator `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `??=`, `>>>=`. The LHS identifier is `write` — one row only. Updated per `docs/contract-review.md` (policy 3): compound assignment emits a single `write` row at Level 3; faithful read+write semantics is Level 4.
- Operand of `prefix_unary_expression` or `postfix_unary_expression` with `++` or `--`. Emit one `write` row.
- The receiver of `out` and `ref` argument passing: `int.TryParse(s, out var x)` — `x` is `write` (the call writes into it). For `ref T x`, the call may read *and* write — emit both rows. (Heuristic: `ref` → emit both; `out` → emit only `write`; `in` → emit only `read`.)
- Object initializer fields: `new Project { Name = "x" }` — `Name` is `write` (member assignment).
- Property auto-accessor invocations: writing `obj.Foo = 5` is `write` on the property symbol `Foo`. **The contract does not emit a separate `read`/`write` on synthetic accessor methods `get_Foo`/`set_Foo`** — we record references against the property symbol itself. Rationale: auto-properties have no user-written accessor body to land in, and downstream queries asking "who writes Foo?" should not have to know whether Foo is auto or explicit.
- For explicit accessor bodies (`get { ... } set { ... }`), references to the backing field inside the body are emitted normally; the property-level `write` row is still emitted at the *assignment site*, not at the accessor body.

### `type_use`

An identifier in a *type position* — meaning the identifier resolves through type-lookup, not value-lookup. These tie to the rows from `types-csharp.md`.

AST patterns:

- Inside `parameter` type, method `returns` type, `field_declaration` type, `property_declaration` type, `variable_declaration` type, `cast_expression` target, `object_creation_expression` constructed type, `array_creation_expression` element type, `typeof_expression` operand, `is_expression`/`as_expression` RHS, `default_expression` operand, generic argument lists, base lists, type parameter constraints.
- Each unique type *occurrence* emits one `type_use` row pointing at the type's declaring symbol (the `class`/`struct`/`interface`/`enum`/`record`/`delegate` symbol id). The `type` row itself is *separate* — `references` carries the link to the declaration; `type` carries the structural decomposition.
- Generic arguments emit additional `type_use` rows recursively. `List<Project>` produces two `type_use` rows: one against `List`'s declaring symbol (in `System.Collections.Generic`, unresolved if not indexed → `referent_id = null`), one against `Project`'s declaring symbol.

### `import_use`

Identifiers inside a `using_directive` — the namespace path.

AST patterns:

- For `using A.B.C;` we emit *one* `import_use` row keyed on the full namespace path. The `referent_id` is the namespace symbol if the workspace contains a `namespace A.B.C { }` declaration; otherwise `null` (external).
- For `using static T;` we emit `import_use` against `T`'s declaring type symbol.
- For `using Alias = X.Y.Z;` we emit `import_use` against `X.Y.Z`'s declaring symbol (the alias `Alias` is a new file-local name, not a reference).
- We do **not** emit one row per dotted segment. `using A.B.C;` is one `import_use`, not three. This matches the existing `imports` relation's granularity (one row per `using_directive`).

## `referent_id` resolution

Algorithm to map an identifier occurrence → the symbol id of the entity it names. We do **not** use the existing `symbols_by_name` global index for this — that index is unscoped and would return cross-namespace ghosts. Instead, the C# resolver builds a per-file scope tree at extraction time:

```
FileScope
 ├── using directives (file-level)
 ├── namespace block(s)
 │   └── type declarations
 │       ├── members (fields/properties/methods)
 │       └── nested types
 │           └── ...
 └── (block scopes built lazily inside method bodies)
```

Lookup precedence (first match wins):

1. **Locals in the enclosing block, walking outward** to method/lambda boundary.
2. **Method parameters** (including `this` for instance methods, type parameters for generic methods).
3. **Enclosing-type members.** Walk through nested types outward. For each, include inherited members from base classes/interfaces declared in the workspace (single hop only — we do not transitively walk the inheritance chain across `out-of-workspace` bases).
4. **Enclosing-namespace types.** Walk namespace blocks outward.
5. **`using Alias = ...` aliases** in the file (file-level, never block-level — C# does not allow block-level aliases in this grammar).
6. **`using` namespace imports** in the file, in source order.
7. **`using static T;`** members in the file, in source order.
8. **Global namespace top-level types.**

### Multiple candidates

If two `using` directives bring in identically-named symbols (e.g. `using A;` and `using B;` both expose `Foo`), C# itself produces an error. Our resolver records the **first match in source order** as `match_index = 0` and does not emit duplicates.

For C# method overloads (multiple methods with the same name on the same type), the resolver emits one row per candidate that survives name-only lookup: `match_index = 0` for the primary/first candidate, `1, 2, ...` for additional overloads — sharing `(referrer_id, site_file, site_start_byte)` per the schema key. Downstream queries can prune by argument count or type when signature data is available.

### No candidate

If no candidate exists, emit the `references` row with `referent_id = null`. We never skip rows — downstream queries need to count unresolved-rate to assess workspace completeness.

### Inheritance scope

For type-member lookup, we include members of declared bases (`class Foo : Bar` brings `Bar`'s public/protected members into scope inside `Foo`'s methods). We do **not** resolve members from `Bar` if `Bar` is not declared in the workspace — those become `referent_id = null`.

### Site columns

For every row:

- `site_file` = the file containing the occurrence (same as `referrer_id`'s file).
- `site_start_byte` = the tree-sitter `Range.start_byte` of the *identifier* node, not the enclosing expression. For a `member_access_expression` `a.b`, the row about `b` has `site_start_byte` pointing at `b`'s first byte.

### Referrer

`referrer_id` is the **innermost enclosing symbol** that has a `symbol` row — usually the method/constructor/property containing the occurrence. References inside field initializers (e.g. `public int Priority { get; set; } = 3;` if the initializer mentions a name) attribute to the containing field/property. References inside a `class` declaration's base list attribute to the class itself.

## Worked examples

All examples cite `../virgil-skills/benchmarks/csharp/dotnet-api/`. Ids show the [ADR-0002](adr/0002-symbol-id-scheme.md) format `path|start_line|start_col|name|kind` directly (truncated/aliased where it improves readability). `<P>` abbreviates `src/ProjectHub.Api/`.

### Example 1 — `read` and method-call resolution

Source: `<P>Controllers/ProjectController.cs` lines 22–27

```csharp
[HttpGet]
public async Task<IActionResult> GetAll()
{
    var projects = await _projectService.GetAllProjectsAsync();
    return Ok(projects);
}
```

Method symbol: `<P>Controllers/ProjectController.cs|23|8|GetAll|method` (call this `GetAll`).

Rows emitted (referrer_id = `GetAll` for all):

| referent_id | ref_kind | site_start_byte | notes |
|---|---|---|---|
| `<P>Controllers/ProjectController.cs\|15\|33\|_projectService\|variable` | `read` | byte of `_projectService` on line 25 | local field of the class |
| `null` (external method on `IProjectService`) | `read` | byte of `GetAllProjectsAsync` on line 25 | `IProjectService` is declared but `GetAllProjectsAsync` is on an interface we resolve; if `IProjectService` is in workspace → referent is that method symbol, otherwise null |
| `<P>Controllers/ProjectController.cs\|23\|8\|GetAll\|method` (self) is **not** referenced — `projects` is a fresh local | — | — | — |
| local `projects` | `read` | byte of `projects` on line 26 | local-to-local, referrer-id = method, referent-id = `projects`'s symbol if locals get rows; if not, emit `read` with `referent_id = null` and a note (see "Local-variable referents" below) |
| `null` (`Ok` is `ControllerBase.Ok`) | `read` | byte of `Ok` on line 26 | inherited; `ControllerBase` is out-of-workspace → null |

**Local-variable referents.** Whether locals (`var projects = ...`) get their own `symbol` rows is governed by the symbol extractor (currently it does not — see `src/languages/csharp/queries.rs`). The references extractor must therefore handle two cases:

- If a local has a symbol row: emit `referent_id` pointing at it.
- If not: emit `referent_id = null` for reads/writes of that local. Mark the row as "unresolvable-local" via the existing `referent_id IS NULL` predicate.

This contract assumes the symbol extractor is extended to emit `local_variable_declaration` as `symbol` rows of `kind = "variable"`. Until that lands, all local-local reads/writes have `referent_id = null`.

### Example 2 — `write` through assignment to a field via `this`

Source: `<P>Controllers/ProjectController.cs` lines 17–20

```csharp
public ProjectController(IProjectService projectService)
{
    _projectService = projectService;
}
```

Constructor symbol: `<P>Controllers/ProjectController.cs|17|8|ProjectController|method` (constructors are extracted as `Method` per `determine_csharp_kind`).

Rows:

| referent_id | ref_kind | notes |
|---|---|---|
| `<P>Controllers/ProjectController.cs\|15\|33\|_projectService\|variable` | `write` | LHS of `=` |
| parameter `projectService` symbol id | `read` | RHS — assuming parameters are emitted as symbols of kind `parameter`; otherwise null |

Note: no separate `read` of `_projectService` because the operator is plain `=`, not compound. If the assignment were `_projectService ??= projectService`, we would emit `read` *and* `write` for `_projectService`.

### Example 3 — `write` via `out` parameter

Source: `<P>Middleware/AuthMiddleware.cs` lines 48–55

```csharp
tokenHandler.ValidateToken(token, new TokenValidationParameters
{
    ValidateIssuerSigningKey = true,
    IssuerSigningKey = new SymmetricSecurityKey(key),
    ValidateIssuer = false,
    ValidateAudience = false,
    ClockSkew = TimeSpan.Zero
}, out SecurityToken validatedToken);
```

Method: `<P>Middleware/AuthMiddleware.cs|42|22|AttachUserToContext|method`.

Selected rows:

| referent_id | ref_kind | site_start_byte | notes |
|---|---|---|---|
| `tokenHandler` (local) | `read` | byte of `tokenHandler` | receiver |
| `null` (`ValidateToken` is external) | `read` | byte of `ValidateToken` | callee |
| `token` (parameter) | `read` | byte of `token` | first arg |
| `validatedToken` (local, declared inline) | `write` | byte of `validatedToken` | `out`-parameter declaration **and** write of the resulting binding |
| `TokenValidationParameters` (type) | `type_use` | byte of `TokenValidationParameters` | constructor-call type |
| `ValidateIssuerSigningKey` etc. | `write` | byte of each | object-initializer member assignments |

The inline `out SecurityToken validatedToken` introduces `validatedToken` as a local. It is both a declaration *and* a write site. We emit a single `write` row (the declaration is captured elsewhere as a symbol row, if symbol extraction is extended; the references row carries the assignment semantics).

### Example 4 — `type_use` from a base class and `read` of `base`

Source: `<P>Controllers/ProjectController.cs` lines 13–14, 26

```csharp
public class ProjectController : ControllerBase
{
    ...
    return Ok(projects);
```

Class symbol: `<P>Controllers/ProjectController.cs|13|17|ProjectController|class`.

Rows (referrer for the base list = the class itself):

| referent_id | ref_kind | site_start_byte | notes |
|---|---|---|---|
| `null` (`ControllerBase` out-of-workspace) | `type_use` | byte of `ControllerBase` on line 13 | base class reference |

Inside `GetAll` (line 26):

| referent_id | ref_kind | notes |
|---|---|---|
| `null` (`Ok` is inherited from `ControllerBase`, out-of-workspace) | `read` | inherited method call |

Had `ControllerBase` been declared in the workspace, both rows would carry its symbol id and `Ok`'s method symbol id respectively. **No `read` of `this` is emitted for the implicit-receiver call** (`Ok(...)` has no `this.` written). When the source explicitly writes `this.Ok(...)`, we emit `read` for `this` *and* `read` for `Ok`.

### Example 5 — shadowing inside a method

Source (illustrative pattern visible in `<P>Repositories/ProjectRepository.cs` lines 50–65):

```csharp
public async Task<IEnumerable<Project>> GetProjectsWithMembersAsync()
{
    var projects = await _context.Projects
        .Where(p => !p.IsDeleted)
        .ToListAsync();

    foreach (var project in projects)
    {
        project.Members = await _context.ProjectMembers
            .Where(m => m.ProjectId == project.Id && m.IsActive)
            .Include(m => m.User)
            .ToListAsync();
    }

    return projects;
}
```

Method: `<P>Repositories/ProjectRepository.cs|50|45|GetProjectsWithMembersAsync|method`.

Selected reference rows (referrer = method):

| referent_id | ref_kind | notes |
|---|---|---|
| local `projects` | `read` (line 56, in `foreach ... in projects`) | the local declared on line 52 |
| local `project` (loop variable) | `read` (line 59 receiver) | `foreach` loop variable — innermost scope |
| local `project` | `write` (line 59 LHS `project.Members = ...`) | assignment to a member of the loop var: this is a `write` on the **`Members` property symbol**, plus a `read` of `project` |
| `null` (`Members` is on `Project`, in workspace) → resolves to `<P>Models/Project.cs\|37\|41\|Members\|property` | `write` (line 59) | property write |
| `m` (lambda parameter) | `read` (multiple in `.Where(m => ...)`) | lambda parameter scope |

If an outer method had declared `string project = ...;` and the `foreach (var project in ...)` reused the name, the inner `project` shadows the outer for the duration of the loop body. References inside the body resolve to the inner binding. This is the standard innermost-binding-wins rule.

### Example 6 — `write` to non-local (static field through indexer)

Source: `<P>Middleware/AuthMiddleware.cs` line 58

```csharp
context.Items["UserId"] = int.Parse(jwtToken.Claims.First(x => x.Type == "nameid").Value);
```

Method: `<P>Middleware/AuthMiddleware.cs|42|22|AttachUserToContext|method`.

Selected rows:

| referent_id | ref_kind | notes |
|---|---|---|
| parameter `context` | `read` | receiver |
| `null` (`Items` external) | `read` | property read on `context` (indexer LHS still reads the receiver chain) |
| (no row for the *indexer* itself as a write) | — | indexer assignments do not produce a `write` against an identifier — the indexed expression has no name. The string key `"UserId"` is a literal and produces no row. |
| `null` (`int.Parse` external) | `read` | static method call on `int` |
| local `jwtToken` | `read` | RHS receiver |
| `null` (`Claims`/`First`/`Value`/`Type` external) | `read` | chained member accesses |
| `x` (lambda parameter) | `read` (twice — `x.Type`) | lambda binding |

The takeaway for the indexer assignment: when the LHS is `expr[key] = value`, we treat the LHS as a *read* of the indexed object and a *read* of the indexer key, and emit **no synthetic write row** because there is no named target. (If the indexed expression is itself a member access, e.g. `this.dict[k] = v`, we still emit only `read`s of `dict`.) Rationale: the C# compiler lowers indexer assignment to a method call on `set_Item`; without resolving that, we cannot honestly report which named member is written.

### Example 7 — `import_use`

Source: `<P>Controllers/ProjectController.cs` lines 1–6

```csharp
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using ProjectHub.Api.DTOs;
using ProjectHub.Api.Services;
using System.Security.Claims;
using System.Threading.Tasks;
```

Six `import_use` rows. Referrer = the file's namespace symbol if one exists, else the file itself. Per the schema, the `references` relation has no `file` referrer, so we use the *enclosing namespace symbol*: `<P>Controllers/ProjectController.cs|8|10|ProjectHub.Api.Controllers|namespace`.

| referent_id | ref_kind | site_start_byte | notes |
|---|---|---|---|
| `null` (Microsoft external) | `import_use` | byte of `Microsoft` on line 1 | external namespace |
| `null` (Microsoft external) | `import_use` | line 2 | |
| `<P>DTOs/...|...|...|ProjectHub.Api.DTOs|namespace` (if any DTO file declares this namespace) | `import_use` | line 3 | in-workspace |
| `<P>Services/...|...|...|ProjectHub.Api.Services|namespace` | `import_use` | line 4 | in-workspace |
| `null` | `import_use` | line 5 | |
| `null` | `import_use` | line 6 | |

In-workspace namespaces canonicalize to the first-declared namespace symbol encountered while scanning files (tie-break: lexicographic file path). External namespaces leave `referent_id = null`. The existing `imports` relation also gets a row per `using_directive` — `references` and `imports` are complementary, not redundant: `imports` records the file-level import edge, `references` records the identifier occurrence.
