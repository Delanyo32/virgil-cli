# Language attributes — C#

The contract for what populates `csharp_attrs` from `src/languages/csharp/`. The schema base is defined in [virgil-datalog-schema.md](virgil-datalog-schema.md); symbol ids follow [ADR-0002](adr/0002-symbol-id-scheme.md).

## Schema

```
:create csharp_attrs {
    symbol_id: String =>
    attributes: [String] default [],   # C# [Attribute] annotations on the declaration
    is_partial: Bool default false,
    is_sealed: Bool default false,
    is_virtual: Bool default false,
    is_override: Bool default false,
    is_extern: Bool default false,
    is_unsafe: Bool default false,
}
```

Three columns from the base schema (`attributes`, `is_partial`, `is_sealed`); four added here because they are common predicates in real C# audits (overridable surface area, ABI-relevant `extern`, `unsafe` review).

### Applies-to

| column | applies to |
|---|---|
| `attributes` | every symbol kind that can carry `[...]` annotations: class, struct, interface, enum, record, method, constructor, property, field, delegate |
| `is_partial` | class, struct, interface, record, method |
| `is_sealed` | class, record, method (override-sealed) |
| `is_virtual` | method, property |
| `is_override` | method, property |
| `is_extern` | method, constructor |
| `is_unsafe` | method, constructor, class, struct (any declaration that can carry the `unsafe` modifier) |

A row is emitted in `csharp_attrs` **only if at least one column has a non-default value**. Symbols whose attributes are all default are absent from the relation (queries treat absence as all-default).

## Extraction rules

C# declarations carry two kinds of metadata at the AST surface:

- **Attributes** — `[X]` annotations attached as `attribute_list` children of the declaration node.
- **Modifiers** — `modifier` children of the declaration node (`public`, `partial`, `sealed`, `virtual`, etc.).

The C# tree-sitter grammar exposes both as direct children of the declaration node, so a single walk over `node.children()` covers both.

### `attributes`

AST source: `attribute_list` children of the declaration node. Each `attribute_list` contains one or more `attribute` nodes; each `attribute` has a `name` (an `identifier_name` or `qualified_name`) and an optional `argument_list`.

Extraction:

- The stored string is the **attribute name only**, not the argument list. `[Required]` → `"Required"`. `[MaxLength(200)]` → `"MaxLength"`. `[Route("api/[controller]")]` → `"Route"`.
- The trailing `Attribute` suffix (which C# allows to be elided) is preserved if written in source. `[RequiredAttribute]` → `"RequiredAttribute"`, `[Required]` → `"Required"`. We do **not** canonicalize. Rationale: round-tripping source intent is more useful than pretending two spellings are the same; queries can match on either form.
- Qualified attribute names: `[System.ComponentModel.DataAnnotations.Required]` → `"System.ComponentModel.DataAnnotations.Required"` (full qualified text preserved). Most code uses the unqualified form via a `using` directive — both forms are kept verbatim.
- Multiple attributes from one or several `attribute_list` children all flow into the same `attributes` list, in source order.
- Target specifiers (`[assembly: ...]`, `[return: ...]`) are **skipped** — they do not attach to a declaration symbol. Assembly-level attributes are intentionally not captured by this relation.

Default (no attributes): `[]`.

### `is_partial`

AST source: a `modifier` child of the declaration whose text is exactly `partial`.

Edge cases:

- Partial classes can have **multiple declarations across files**. Each declaration gets its own `symbol` row (different `start_line`/`start_col` per ADR-0002) and each carries `is_partial = true`. Cross-file aggregation through `name` + `qualified_name` is left to the query layer.
- Partial methods declared without `partial` on the implementation side: tree-sitter's modifier list is what we read; if the modifier is absent, `is_partial = false` even if a partial declaration of the same method exists elsewhere. This is honest to the AST and matches how C# itself treats `partial`.

### `is_sealed`

AST source: a `modifier` child whose text is exactly `sealed`.

Notes:

- On a class: terminates the inheritance chain.
- On a method: only meaningful in combination with `override`; means "no further overrides". We set `is_sealed = true` whenever the `sealed` modifier is present, regardless of context.

### `is_virtual`

AST source: a `modifier` child whose text is exactly `virtual`.

Note: `abstract` methods are implicitly virtual; we do **not** set `is_virtual = true` based on `abstract`. The `is_abstract` flag on the base `symbol` row already captures that. `is_virtual` is reserved for *explicit* `virtual` modifier.

### `is_override`

AST source: a `modifier` child whose text is exactly `override`.

### `is_extern`

AST source: a `modifier` child whose text is exactly `extern`. Applies to methods (P/Invoke entry points such as `[DllImport]`-decorated methods) and to constructors in rare cases.

### `is_unsafe`

AST source: a `modifier` child whose text is exactly `unsafe`. This marks the *declaration* as unsafe; we do not recursively mark methods declared inside an `unsafe` class. Each declaration's modifier list is checked independently.

### Edge cases (cross-cutting)

- **Conditional compilation.** Tree-sitter parses the source as-is; if the file declares `#if NET8_0 ... #else ... #endif` with different attribute sets, we extract what tree-sitter sees in its (single) parse tree. We do **not** evaluate `#if` and do not union across branches. State: best-effort; matches every other language extractor in this project.
- **Generic attributes** (`[GenericAttribute<T>]`, C# 11+). The `name` portion is read up to the `<`. `"GenericAttribute"` is stored.
- **Attribute parameters that reference `nameof`/typeof**. We do not capture argument contents; only the attribute name matters here.
- **Records.** A record's primary constructor parameters can carry attributes (`public record R([Required] string Name)`). Those attribute rows attach to the *parameter* symbol if parameters are emitted as symbols. Until parameter-symbol extraction lands, those attributes are dropped.
- **Modifier order.** C# is permissive (`public static readonly` vs `static public readonly`). We scan all `modifier` children and check each text — order does not matter.

## Worked examples

All examples cite `../virgil-skills/benchmarks/csharp/dotnet-api/`. `<P>` abbreviates `src/ProjectHub.Api/`.

### Example 1 — class with multiple attributes

Source: `<P>Controllers/ProjectController.cs` lines 10–13

```csharp
[Authorize]
[ApiController]
[Route("api/[controller]")]
public class ProjectController : ControllerBase
```

Symbol id (ADR-0002): `src/ProjectHub.Api/Controllers/ProjectController.cs|13|17|ProjectController|class`.

`csharp_attrs` row:

| column | value |
|---|---|
| symbol_id | `src/ProjectHub.Api/Controllers/ProjectController.cs\|13\|17\|ProjectController\|class` |
| attributes | `["Authorize", "ApiController", "Route"]` |
| is_partial | `false` |
| is_sealed | `false` |
| is_virtual | `false` |
| is_override | `false` |
| is_extern | `false` |
| is_unsafe | `false` |

Attributes are recorded in source order. The `Route` attribute's `"api/[controller]"` argument is discarded — only the attribute *name* is stored.

### Example 2 — attribute on a class deriving from `Attribute`

Source: `<P>Filters/CacheFilter.cs` lines 8–10

```csharp
[AttributeUsage(AttributeTargets.Method | AttributeTargets.Class)]
public class CacheFilter : Attribute, IActionFilter
```

Symbol id: `src/ProjectHub.Api/Filters/CacheFilter.cs|9|17|CacheFilter|class`.

`csharp_attrs` row:

| column | value |
|---|---|
| symbol_id | `src/ProjectHub.Api/Filters/CacheFilter.cs\|9\|17\|CacheFilter\|class` |
| attributes | `["AttributeUsage"]` |
| is_partial | `false` |
| is_sealed | `false` |
| is_virtual | `false` |
| is_override | `false` |
| is_extern | `false` |
| is_unsafe | `false` |

The non-obvious part: `Attribute` (the base class on line 9) is **not** in `attributes` — it appears in the base list, which is captured by the `extends` relation, not by `csharp_attrs`. The `attributes` column captures only `[...]` annotations.

### Example 3 — property with stacked validation attributes

Source: `<P>Models/Project.cs` lines 11–13

```csharp
[Required]
[MaxLength(200)]
public string Name { get; set; }
```

Symbol id: `src/ProjectHub.Api/Models/Project.cs|13|22|Name|property`.

`csharp_attrs` row:

| column | value |
|---|---|
| symbol_id | `src/ProjectHub.Api/Models/Project.cs\|13\|22\|Name\|property` |
| attributes | `["Required", "MaxLength"]` |
| is_partial | `false` |
| is_sealed | `false` |
| is_virtual | `false` |
| is_override | `false` |
| is_extern | `false` |
| is_unsafe | `false` |

Multiple `attribute_list` siblings (one per line in source) flow into a single `attributes` array in source order. `MaxLength(200)` becomes `"MaxLength"` — the argument is dropped.

### Example 4 — method with attribute and inferred modifiers

Source: `<P>Controllers/ProjectController.cs` lines 22–23

```csharp
[HttpGet]
public async Task<IActionResult> GetAll()
```

Symbol id: `src/ProjectHub.Api/Controllers/ProjectController.cs|23|33|GetAll|method`. (Column `33` is the `GetAll` identifier's start column inside the `method_declaration`. The exact column comes from tree-sitter; the value here is illustrative.)

`csharp_attrs` row:

| column | value |
|---|---|
| symbol_id | as above |
| attributes | `["HttpGet"]` |
| is_partial | `false` |
| is_sealed | `false` |
| is_virtual | `false` |
| is_override | `false` |
| is_extern | `false` |
| is_unsafe | `false` |

The `async` modifier does **not** map into `csharp_attrs` — it sets `is_async = true` on the base `symbol` row (per the core schema). `public` similarly sets `visibility = "public"` on the `symbol` row, not in `csharp_attrs`. The split is: `csharp_attrs` carries C#-specific extras; the cross-language `symbol` columns carry the universal modifiers.

### Example 5 — class with no attributes (no row emitted)

Source: `<P>Configuration/AppSettings.cs` lines 3–4

```csharp
public class AppSettings
{
```

Symbol id: `src/ProjectHub.Api/Configuration/AppSettings.cs|3|17|AppSettings|class`.

**No `csharp_attrs` row is emitted** — all columns would be defaults. Queries that join on `csharp_attrs` must use a left-join pattern or treat absence as all-default. This keeps the relation sparse for the common case.

### Example 6 — extension class with `static` (modifier on `symbol`) and no `csharp_attrs`

Source: `<P>Extensions/QueryExtensions.cs` lines 13–14

```csharp
public static class QueryExtensions
{
```

Symbol id: `src/ProjectHub.Api/Extensions/QueryExtensions.cs|14|24|QueryExtensions|class`.

**No `csharp_attrs` row is emitted.** `static` sets `is_static = true` on the base `symbol` row, not here. This is the same pattern as Example 5 — the static-class case is just a reminder that universal modifiers never land in `csharp_attrs`.
