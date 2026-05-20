# Types — C#

The contract for how C# type expressions in `src/languages/csharp/` map to the `type` relation defined in [virgil-datalog-schema.md](virgil-datalog-schema.md). Identity follows [ADR-0003](adr/0003-level-3-types-and-references.md); symbol ids referenced from worked examples follow [ADR-0002](adr/0002-symbol-id-scheme.md).

## Tree-sitter node kinds

Every tree-sitter node kind that can occupy a *type position* in C# (parameter type, return type, field type, property type, generic argument, cast target, `typeof`, `is`/`as` operand). Mapping is to the schema `kind` discriminant.

| node kind | source | schema `kind` |
|---|---|---|
| `predefined_type` | `int`, `bool`, `string`, `void`, `object`, `double`, `float`, `long`, `short`, `byte`, `char`, `decimal`, `sbyte`, `ushort`, `uint`, `ulong`, `nint`, `nuint`, `dynamic` | `primitive` |
| `identifier_name` (in type position) | unqualified user-defined type: `User`, `Project`, `T` | `named` |
| `qualified_name` (in type position) | namespace-qualified type: `System.String`, `ProjectHub.Api.Models.Project` | `named` |
| `generic_name` | `List<Project>`, `Task<IActionResult>`, `IEnumerable<TaskItem>` | `generic` |
| `array_type` | `int[]`, `string[][]`, `Project[]` | `array` |
| `nullable_type` | `int?`, `DateTime?`, `Project?` (NRT or value-type nullable — syntactically identical) | see "Nullable handling" below |
| `tuple_type` | `(int, string)`, `(int Id, string Name)` | `tuple` |
| `pointer_type` | `int*`, `byte**` (only valid inside `unsafe`) | `named` (display preserves `*`) |
| `function_pointer_type` | `delegate*<int, void>` | `function` |
| `ref_type` | `ref int`, `ref readonly int` (parameter/return modifier) | wrapping node — see "Ref / out / in" below |
| `implicit_type` (`var`) | `var x = ...` | **never emitted** — see "`var` handling" below |

If a single node kind splits across multiple schema kinds depending on context, the only such case is `nullable_type` (handled below). Every other mapping is one-to-one.

### Nullable handling

`nullable_type` wraps an inner type with a trailing `?`. The schema does **not** distinguish value-type nullables (`int?` = `Nullable<int>`) from nullable-reference-type annotations (`string?`), because the tree-sitter grammar does not either. We resolve this as follows:

- `nullable_type` produces **a single `type` row** with the same `kind` as its inner expression (`primitive` for `int?`, `named` for `string?`, `generic` for `List<int>?`, etc.).
- The trailing `?` is preserved in `display_name` (e.g. `int?`, `List<Project>?`).
- For `canonical_name`, value-type nullables resolve through `System.Nullable<T>` (so `int?` canonicalizes to `System.Nullable<System.Int32>`). NRT annotations on reference types resolve to the underlying type unchanged (so `string?` canonicalizes to `System.String`).
- We do **not** emit a synthetic `null` union member; the nullability is encoded in the textual `display_name` only.

The distinction between value-nullable and NRT requires resolving the inner type before deciding. Until the canonical resolver knows whether `T` is a value type, both cases produce identical rows except `canonical_name`. The decision: emit the row with `canonical_name = null` (unresolved) rather than guess.

### Ref / out / in

`ref_type`, `out` modifier on a parameter, and `in` modifier are **not** emitted as separate `type` rows. They are properties of the *parameter*, not of the type. The wrapped type underneath is what reaches the `type` relation. The `out`/`ref`/`in` flag is captured on the parameter row's value columns when those exist (presently the schema only has `is_optional` and `has_default`, so the modifier is dropped for now — a follow-up item).

### `var` handling

`implicit_type` (`var`) is **never** emitted as a `type` row. It is a binding-time inference, not a type expression. The declared variable's static type is whatever its initializer evaluates to — we do not attempt to infer it. Variables declared with `var` get a `parameter`/symbol row with `type_id = null`.

## `display_name` construction

The `display_name` is built by re-serializing the AST node, *not* by slicing the source byte range. This normalizes whitespace.

Rules:

1. **Primitives.** Use the canonical keyword text as written: `int`, `bool`, `string`, `void`, `object`, `double`, `float`, `long`, `short`, `byte`, `char`, `decimal`, `sbyte`, `ushort`, `uint`, `ulong`, `nint`, `nuint`, `dynamic`. Aliases like `String` (which is actually a `qualified_name` or `identifier_name`, not a `predefined_type`) are kept as written and do not collapse to `string`.
2. **Named types.** Write the identifier text. For `qualified_name`, join components with `.` (no surrounding spaces).
3. **Generic types.** `Name<Arg1, Arg2>` with one space after each comma, no spaces around `<` or `>`. Recursive: `List<Dictionary<string, int>>`.
4. **Array types.** `T[]`. For jagged arrays, `T[][]`. For rank-2, `T[,]`. Brackets immediately follow the element type, no space.
5. **Nullable types.** Inner display followed by `?`, no space.
6. **Tuple types.** `(T1, T2)` with one space after each comma, no spaces inside parens. Element names (when present, e.g. `(int Id, string Name)`) are preserved.
7. **Pointer types.** Inner display followed by `*`, no space. Stacked: `int**`.
8. **Function pointer types.** `delegate*<T1, T2, R>` where the last type argument is the return type, matching source order.

Whitespace normalization: every internal whitespace run between tokens collapses to a single space (or zero space where the rules above say so). Newlines and tabs inside a type expression are removed.

`display_name` round-trips intent: `List<int>` and `List< int >` both produce `display_name = "List<int>"`.

## `canonical_name` resolution

Per [ADR-0003](adr/0003-level-3-types-and-references.md), every `type` row is given a `canonical_name` when resolvable. C# uses fully-qualified namespace-rooted names (`System.Collections.Generic.List<System.Int32>`), matching CLR metadata conventions.

### Scope walk for unqualified names

For an unqualified `identifier_name` or the head of a `generic_name`:

1. **Enclosing type scope.** If the file declares `class Foo { class Inner { } }`, references to `Inner` inside `Foo` resolve to `Foo.Inner`. Walk outward through nested types.
2. **Enclosing namespace scope.** Walk the namespace block(s) the reference lives inside, innermost first. `namespace A.B { class C { } }` makes a reference to `C` from inside `A.B` resolve to `A.B.C`.
3. **File-level `using` directives.** For each `using Ns;` in the file (in source order), check whether `Ns.Name` is a known declared symbol in the index. First match wins.
4. **File-level `using static T;` directives.** For each, check whether `Name` is a public static member of `T`. (Type lookup ignores these — they apply only to identifier references, not type references. Skipped during type resolution.)
5. **File-level `using Alias = X.Y.Z;` directives.** If the identifier matches `Alias`, resolve to `X.Y.Z`. Aliases participate in type resolution.
6. **Global namespace fallthrough.** If still unresolved, look for a top-level type named `Name` in the indexed workspace. First match wins; on tie, leave unresolved.

If none of the above produce a match, `canonical_name = null`.

### Qualified names

`qualified_name` is interpreted left-to-right. The leftmost segment goes through the scope walk above (typically resolving against `using`s). The trailing segments are concatenated unchanged. `System.String` canonicalizes to `System.String` directly (assuming `System` is in scope, which any file with `using System;` provides).

### Generic arguments

Generic arguments are resolved independently and substituted into the canonical form: `List<Project>` becomes `System.Collections.Generic.List<ProjectHub.Api.Models.Project>` (when both `List` and `Project` resolve). If any argument fails to resolve, the *outer* type still gets a canonical name when its head resolved, with the unresolved argument left in unresolved form: `List<UnknownT>` → `System.Collections.Generic.List<UnknownT>`. The unresolved-ness is recorded by the inner row (its own `canonical_name` is `null`).

### Type parameters

A reference to a generic type parameter (e.g. `T` inside `class Box<T> { T Value; }`) is **unresolved**: `canonical_name = null`. This matches the schema convention — type parameters do not have a stable cross-file identity.

### Aliases

`using Foo = System.Collections.Generic.List<int>;` followed by `Foo x;` produces a `type` row with `display_name = "Foo"` and `canonical_name = "System.Collections.Generic.List<System.Int32>"`. Aliases canonicalize through to their RHS — they are display-name sugar, not new types.

### Primitive canonicalization

Predefined keywords resolve to their CLR full names:

| keyword | canonical |
|---|---|
| `bool` | `System.Boolean` |
| `byte` | `System.Byte` |
| `sbyte` | `System.SByte` |
| `char` | `System.Char` |
| `decimal` | `System.Decimal` |
| `double` | `System.Double` |
| `float` | `System.Single` |
| `int` | `System.Int32` |
| `uint` | `System.UInt32` |
| `long` | `System.Int64` |
| `ulong` | `System.UInt64` |
| `short` | `System.Int16` |
| `ushort` | `System.UInt16` |
| `object` | `System.Object` |
| `string` | `System.String` |
| `void` | `System.Void` |
| `nint` | `System.IntPtr` |
| `nuint` | `System.UIntPtr` |
| `dynamic` | `System.Object` (with a flag we don't currently track; canonicalized as `Object`) |

## Identity

Per [ADR-0003](adr/0003-level-3-types-and-references.md), `type.id = blake3(language | file_id | display_name)`. Concretely: `blake3("csharp" + "\0" + file_id + "\0" + display_name)` rendered as a 32-char hex prefix. The `display_name` used as input is the normalized form from the rules above — *not* the raw source slice. Two textually-different occurrences (`List<int>` and `List< int >`) in the same file produce the same `type.id`.

The dedup key is per-file; cross-file aggregation joins through `canonical_name`.

## Field types — `field_type` relation

Per the schema, every class/struct/record/interface field or property declaration with a typed declarator emits a `field_type {symbol_id, type_id}` row linking the field/property symbol to its `type` row. Auto-properties get one row keyed on the property symbol. Local variables and method parameters are not fields and use `parameter` / `references` wiring instead.

## Worked examples

All examples cite paths under `../virgil-skills/benchmarks/csharp/dotnet-api/`. For brevity, IDs use `<blake3-prefix>` rather than the full hash. File ids equal their relative path per ADR-0002.

### Example 1 — primitive (`int`)

Source: `src/ProjectHub.Api/Models/Project.cs` line 9

```csharp
public int Id { get; set; }
```

The property type node is `predefined_type` covering `int`.

`type` row:

| column | value |
|---|---|
| id | `<blake3>` |
| kind | `primitive` |
| language | `csharp` |
| display_name | `int` |
| canonical_name | `System.Int32` |

The property symbol id (ADR-0002): `src/ProjectHub.Api/Models/Project.cs\|9\|8\|Id\|property`. No `returns_type` row — `int` here is the property type, not a method return. (The schema currently has no `field_type`/`property_type` relation; this is a known gap and the row's *referrer* is recorded only through the `references` table with `ref_kind = "type_use"`.)

### Example 2 — named (`User`)

Source: `src/ProjectHub.Api/Models/Project.cs` line 25

```csharp
public User Owner { get; set; }
```

The type node is `identifier_name` containing `User`.

`type` row:

| column | value |
|---|---|
| id | `<blake3>` |
| kind | `named` |
| language | `csharp` |
| display_name | `User` |
| canonical_name | `ProjectHub.Api.Models.User` |

Resolution: enclosing namespace is `ProjectHub.Api.Models` (line 5 of the file). `User` is declared as `class User` at `src/ProjectHub.Api/Models/User.cs`. The scope walk finds it via step 2 (enclosing namespace scope).

### Example 3 — generic (`ICollection<TaskItem>`)

Source: `src/ProjectHub.Api/Models/Project.cs` line 36

```csharp
public ICollection<TaskItem> Tasks { get; set; } = new List<TaskItem>();
```

The property type is a `generic_name` whose head is `ICollection` and whose single argument is `TaskItem` (an `identifier_name`).

Two `type` rows are emitted (one for the outer generic, one for the argument):

Inner (`TaskItem`):

| column | value |
|---|---|
| id | `<blake3-inner>` |
| kind | `named` |
| language | `csharp` |
| display_name | `TaskItem` |
| canonical_name | `ProjectHub.Api.Models.TaskItem` |

Outer (`ICollection<TaskItem>`):

| column | value |
|---|---|
| id | `<blake3-outer>` |
| kind | `generic` |
| language | `csharp` |
| display_name | `ICollection<TaskItem>` |
| canonical_name | `System.Collections.Generic.ICollection<ProjectHub.Api.Models.TaskItem>` |

The initializer `new List<TaskItem>()` produces *additional* `type` rows for `List<TaskItem>` and `TaskItem` (the latter dedups to the same row as above — same `display_name`, same `file_id`, same `language`). The initializer's `type_use` rows are emitted from the `references` extractor.

### Example 4 — array (`string[]`)

Source: `src/ProjectHub.Api/Configuration/AppSettings.cs` line 9

```csharp
public string[] AllowedCorsOrigins { get; set; }
```

`array_type` over `predefined_type` `string`.

Two rows:

| column | value (inner) | value (outer) |
|---|---|---|
| id | `<blake3-s>` | `<blake3-arr>` |
| kind | `primitive` | `array` |
| language | `csharp` | `csharp` |
| display_name | `string` | `string[]` |
| canonical_name | `System.String` | `System.String[]` |

The `array` kind canonical form keeps the bracket notation. We do **not** canonicalize through `System.Array`, because CLR-level `T[]` is a distinct type, not `System.Array<T>`.

### Example 5 — nullable value-type (`DateTime?`)

Source: `src/ProjectHub.Api/Models/Project.cs` line 28

```csharp
public DateTime? UpdatedAt { get; set; }
```

`nullable_type` wrapping `identifier_name` `DateTime`.

Single row (per the nullable-handling rule — no separate row for the inner `DateTime` when wrapped):

| column | value |
|---|---|
| id | `<blake3>` |
| kind | `named` |
| language | `csharp` |
| display_name | `DateTime?` |
| canonical_name | `System.Nullable<System.DateTime>` |

Resolution: `DateTime` resolves to `System.DateTime` via the file-level `using System;` (line 1). The `?` triggers the value-type-nullable canonicalization (`System.Nullable<...>`). The inner `DateTime` is **not** emitted as a separate `type` row when it appears under a `nullable_type` — only the nullable form is recorded. Rationale: emitting both `DateTime` and `DateTime?` for the same source span would over-count `references` rows downstream.

### Example 6 — generic with nested generic (`Task<IActionResult>`)

Source: `src/ProjectHub.Api/Controllers/ProjectController.cs` line 23

```csharp
public async Task<IActionResult> GetAll()
```

`generic_name` `Task<IActionResult>`. Argument is `identifier_name` `IActionResult`.

Two rows:

Inner (`IActionResult`):

| column | value |
|---|---|
| id | `<blake3-iar>` |
| kind | `named` |
| language | `csharp` |
| display_name | `IActionResult` |
| canonical_name | `Microsoft.AspNetCore.Mvc.IActionResult` |

Outer (`Task<IActionResult>`):

| column | value |
|---|---|
| id | `<blake3-task>` |
| kind | `generic` |
| language | `csharp` |
| display_name | `Task<IActionResult>` |
| canonical_name | `System.Threading.Tasks.Task<Microsoft.AspNetCore.Mvc.IActionResult>` |

`returns_type` row for the method `GetAll`:

| column | value |
|---|---|
| function_id | `src/ProjectHub.Api/Controllers/ProjectController.cs\|23\|8\|GetAll\|method` |
| type_id | `<blake3-task>` |

The `async` modifier does **not** wrap the return type in another `Task<...>` — it is already explicit. (When the source elides `Task` and writes `async void`, the type row is `void`/`System.Void`; we do not synthesize a `Task` wrapper.)

### Example 7 — tuple (`(int Id, string Name)`)

Source (illustrative pattern, e.g. `src/ProjectHub.Api/Extensions/QueryExtensions.cs` line 60):

```csharp
public static async Task<(IEnumerable<T> Items, int Total)> PaginateAsync<T>(
```

The inner tuple is `(IEnumerable<T> Items, int Total)` — a `tuple_type` with two elements, each a `tuple_element` carrying a type and an optional element name.

Tuple `type` row:

| column | value |
|---|---|
| id | `<blake3-tup>` |
| kind | `tuple` |
| language | `csharp` |
| display_name | `(IEnumerable<T> Items, int Total)` |
| canonical_name | `(System.Collections.Generic.IEnumerable<T> Items, System.Int32 Total)` |

`T` here is a type parameter, so it stays `T` (unresolved per the type-parameter rule); the outer canonical_name is still emitted because the *outer* tuple's structure resolved. The inner `IEnumerable<T>` gets its own row with `canonical_name = "System.Collections.Generic.IEnumerable<T>"` — head resolved, argument unresolved, outer still canonicalizable per the "Generic arguments" rule.
