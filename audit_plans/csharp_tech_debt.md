# C# Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 12
- **Trait types used:** All 12 use the legacy `Pipeline` trait (tree-sitter only, no graph access)
- **Overall assessment:** The pipelines are structurally sound tree-sitter implementations with correct query compilation and reasonable pattern detection. However, every single pipeline is a pure AST-pattern matcher with no graph awareness, no data flow tracking, no suppression/annotation handling, and no severity graduation. The `CodeGraph` (which includes C# CFGs, taint analysis, and resource lifecycle tracking) is completely unused despite being available. Several pipelines have hardcoded thresholds without justification, and false positive/negative rates are moderate to high due to single-node detection and missing context. The test suites are minimal (3-5 tests each) and test only the happy path, with no edge case coverage.

---

## sync_over_async

### Current Implementation
- **File:** `src/audit/pipelines/csharp/sync_over_async.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `blocking_result_access` (.Result), `blocking_wait_call` (.Wait/.WaitAll/.WaitAny), `async_void` (async void methods)
- **Detection method:** Tree-sitter `member_access_expression`, `invocation_expression`, and `method_declaration` queries. Checks member name == "Result", function text ends with ".Wait"/".WaitAll"/".WaitAny", or method has `async` modifier with `void` return type.

### Problems Identified
1. **[High false positive rate]:** `.Result` is matched on ANY member access named "Result", not just `Task<T>.Result`. A property named `Result` on a non-Task type (e.g., `ValidationResult.Result`, `OperationResult.Result`) will trigger a false positive. (Line 63-64: `node_text(name_node, source) == "Result"`)
2. **[High false negative rate]:** Does not detect `.GetAwaiter().GetResult()` which is an equally common sync-over-async pattern. Does not detect `Task.Run(...).Result` or `.ConfigureAwait(false).GetAwaiter().GetResult()`.
3. **[No scope awareness]:** The `blocking_result_access` and `blocking_wait_call` patterns do not check if the calling method is already synchronous (where blocking may be intentional, e.g., `Main` method, console app entry points). No check for whether the call is inside a `lock` statement (where async would be incorrect).
4. **[Missing compound variants]:** Does not detect `Task.WhenAll(...).Wait()`, `Task.WhenAny(...).Wait()`, or `.Result` inside LINQ expressions (common pattern).
5. **[No suppression/annotation awareness]:** Does not check for `#pragma warning disable` or `[SuppressMessage]` attributes. Does not check for `// intentional` or `// sync context` comments.
6. **[No severity graduation]:** All findings are "warning". `async void` is arguably more severe (process crash on unhandled exception) than `.Result` (which may deadlock but not crash). Event handlers (`async void OnClick`) are legitimate C# patterns that should be excluded or downgraded.
7. **[Language idiom ignorance]:** `async void` event handlers are idiomatic in WPF/WinForms. The pipeline flags them without checking if the method is an event handler (e.g., standard event handler signature `(object sender, EventArgs e)`).
8. **[Single-node detection]:** Checks `.Wait()` by string suffix matching on the function expression text (line 101), which could match a custom `Wait` method on a non-Task type.
9. **[No data flow tracking]:** Does not trace whether the task being awaited synchronously was created in the same method or received as a parameter. A `Task` parameter being `.Wait()`-ed in a synchronous method may be correct design.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Basic .Result detection, basic .Wait() detection, basic async void detection, clean async Task method, clean await usage
- **What's NOT tested:** `.GetAwaiter().GetResult()`, `.WaitAll()`/`.WaitAny()`, non-Task types with `.Result` property (false positive), async void event handlers (false positive), `Task.Run().Result`, `.Result` inside lock statements, suppression annotations, nested async lambdas

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query (symbol nodes with `kind: Method` in C# files) **Why not higher-ranked tool:** Graph IS the highest-ranked tool. **Query/Prompt:** Iterate `graph.file_nodes` for C# files. For each file, find all Symbol nodes with `kind == Method` via outgoing `DefinedIn` edges. Filter to methods that have `async` modifier or contain `.Result`/`.Wait` call sites via `CallSite` nodes. **Returns:** List of `(file_path, method_node_index, method_name)` tuples for candidate methods.

#### Step 2: Narrowing
- **Tool:** Graph query (call sites + CFG) **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** For each candidate method, inspect `function_cfgs[method_node]` to find CFG statements with `CfgStatementKind::Call` where `name` contains "Wait", "WaitAll", "WaitAny", "GetAwaiter", or `CfgStatementKind::Assignment` where `source_vars` contain references resolved to `.Result` member access. For async void detection, check Symbol node return type via tree-sitter AST (graph does not store return types). **Returns:** Candidate findings with call site line numbers and method context.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (graph does not store event handler signatures or parameter types) **Why not higher-ranked tool:** Graph Symbol nodes do not store parameter signatures or base class info needed to identify event handlers. **Query/Prompt:** For async void findings, check if the method has a standard event handler signature: `(object sender, EventArgs e)` or similar. For `.Result`/`.Wait()` findings, check if the enclosing method is `Main`, a constructor, or a `static void` entry point. Check parent scope for `lock` statements. Check for `#pragma warning disable` and `[SuppressMessage("...", "AsyncFixer")]` annotations. **Returns:** Filtered findings with severity: "error" for async void non-event-handlers, "warning" for `.Result`/`.Wait()` in async methods, "info" for `.Result`/`.Wait()` in synchronous methods.

#### Graph Enhancement Required
- **Missing:** Graph Symbol nodes do not store return type or parameter list. Adding `return_type: Option<String>` and `parameters: Vec<(String, String)>` to `NodeWeight::Symbol` would allow full graph-only detection of async void and event handler patterns without tree-sitter fallback.
- **Missing:** Taint source table does not include `Task.Result` or `Task.Wait()` as sink patterns that could be tracked via FlowsTo edges from async call sites.

### New Test Cases
1. **get_awaiter_get_result** -- `someTask.GetAwaiter().GetResult()` -> detected as `blocking_get_result` -- Covers: high false negative rate
2. **non_task_result_property** -- `class Foo { public int Result { get; set; } } ... foo.Result` -> NOT detected -- Covers: high false positive rate
3. **async_void_event_handler** -- `async void OnClick(object sender, EventArgs e)` -> NOT detected (idiomatic) -- Covers: language idiom ignorance
4. **wait_in_main** -- `static void Main() { task.Wait(); }` -> severity "info" not "warning" -- Covers: no scope awareness, no severity graduation
5. **task_run_result** -- `Task.Run(() => Compute()).Result` -> detected -- Covers: missing compound variants
6. **suppression_pragma** -- `#pragma warning disable CS0618\n someTask.Result;` -> NOT detected -- Covers: no suppression/annotation awareness
7. **wait_all_detection** -- `Task.WaitAll(t1, t2)` -> detected as `blocking_wait_call` -- Covers: missing compound variants
8. **configure_await_get_result** -- `task.ConfigureAwait(false).GetAwaiter().GetResult()` -> detected -- Covers: high false negative rate

---

## null_reference_risk

### Current Implementation
- **File:** `src/audit/pipelines/csharp/null_reference_risk.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `explicit_null_return` (return null), `deep_member_chain` (a.b.c.d without ?.)
- **Detection method:** Tree-sitter `return_statement` with `null_literal` child, and `member_access_expression` with recursive depth counting >= 3. Checks for `conditional_access_expression` ancestor to allow ?. chains.

### Problems Identified
1. **[High false positive rate]:** `return null` is flagged everywhere, including in methods with nullable return types (`string?`, `int?`, `T?`) where it is perfectly valid. Also flags `return null` in methods implementing interfaces that require nullable returns (e.g., `IComparer<T>.Compare` returning `T?`). (Lines 52-64)
2. **[High false positive rate]:** `deep_member_chain` threshold of 3 flags common fluent API patterns: `builder.WithName("x").WithAge(30).Build()`, LINQ chains `list.Where(...).Select(...).ToList()`, and chained string operations `str.Trim().ToLower().Replace(...)`. These are not null-safety risks. (Lines 82-83: `if depth >= 3`)
3. **[High false negative rate]:** Does not detect null dereference after null checks: `if (x != null) { } x.DoSomething();` (missing the fact that x may be null in the else path). Does not detect `!` (null-forgiving operator) overuse.
4. **[No data flow tracking]:** The `deep_member_chain` check does not track whether intermediate values could actually be null. `DateTime.Now.Year.ToString()` is safe but flagged. No flow analysis from `null_literal` to dereference points.
5. **[Literal blindness]:** Does not detect `default` returns which are equivalent to `null` for reference types: `return default;` or `return default(string);`.
6. **[No suppression/annotation awareness]:** Does not check `#nullable enable/disable` context. In a `#nullable enable` context, the compiler already handles null safety. Does not respect `[AllowNull]`, `[NotNull]`, `[MaybeNull]` attributes.
7. **[Hardcoded thresholds without justification]:** Chain depth threshold of 3 is not justified. The real risk is null-unsafe chains, not chain length. A chain of 2 (`a.b` where `a` can be null) is equally dangerous.
8. **[Single-node detection]:** `has_conditional_access_ancestor` walks UP the tree looking for `?.` ancestors, but the real check should be whether ALL links in the chain use `?.`. A chain like `a?.b.c.d` has `?.` at the root but `.` for inner accesses -- still unsafe.
9. **[Missing compound variants]:** Does not detect `as` casts followed by member access without null check: `(obj as Foo).Bar()`. Does not detect dictionary indexer access (`dict["key"].Value`) which can throw `KeyNotFoundException`.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Basic return null, deep chain a.b.c.d, clean short chain a.b, clean non-null return
- **What's NOT tested:** Nullable return type (`string?`) false positive, fluent API chains, LINQ chains, `?.` partial chains, `return default`, `#nullable enable` context, `as` cast without null check, `!` null-forgiving operator, dictionary indexer access

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph IS the highest-ranked tool. **Query/Prompt:** Iterate C# file nodes. Collect all Symbol nodes (methods) that have `return_statement` with `null_literal` via tree-sitter, or that contain member access chains. Filter files that have any methods. **Returns:** List of `(file_path, method_node_indices)`.

#### Step 2: Narrowing
- **Tool:** Graph query + CFG **Why not higher-ranked tool:** CFG provides data flow paths. **Query/Prompt:** For `explicit_null_return`: Walk CFG for each method, find `CfgStatementKind::Return` nodes where `value_vars` resolves to a null assignment. Track whether the method's callers (via `traverse_callers`) check for null. For `deep_member_chain`: Use tree-sitter to find chains, but filter out known-safe patterns (fluent builders, LINQ, string methods) by checking the root identifier's type if available via local declaration. **Returns:** Candidate null-return methods and unsafe chains with context.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (for `#nullable` directives and attributes) **Why not higher-ranked tool:** Graph does not store `#nullable` context or method-level attributes. **Query/Prompt:** Check if the file has `#nullable enable` at top. Check if the method has `[return: MaybeNull]` or `[return: NotNull]` attributes. For chains, check if any link uses `?.` (partial safety). Exclude fluent API patterns where method names match common builder/LINQ patterns (Where, Select, OrderBy, WithXxx, AddXxx). **Returns:** Final findings with severity: "error" for null dereference after failed null check (if detectable), "warning" for unguarded null returns in non-nullable context, "info" for long chains in nullable context.

#### Graph Enhancement Required
- **Missing:** Graph does not store `#nullable enable/disable` file-level or block-level context. A `nullable_context: Option<bool>` on `NodeWeight::File` would help.
- **Missing:** CFG does not track null state (definite assignment analysis). Adding a `NullState` lattice (NotNull / MaybeNull / Null) per variable at each CFG block would enable precise null-dereference detection.

### New Test Cases
1. **nullable_return_type** -- `string? GetName() { return null; }` -> NOT detected (valid nullable return) -- Covers: high false positive rate
2. **fluent_api_chain** -- `builder.WithName("x").WithAge(30).Build()` -> NOT detected -- Covers: high false positive rate
3. **linq_chain** -- `list.Where(x => x > 0).Select(x => x * 2).ToList()` -> NOT detected -- Covers: high false positive rate
4. **return_default** -- `string M() { return default; }` -> detected as null_return -- Covers: literal blindness
5. **partial_null_conditional** -- `a?.b.c.d` -> detected (inner links unguarded) -- Covers: single-node detection
6. **as_cast_without_check** -- `(obj as Foo).Bar()` -> detected -- Covers: missing compound variants
7. **nullable_enable_context** -- `#nullable enable\n string? M() { return null; }` -> NOT detected -- Covers: no suppression/annotation awareness
8. **null_forgiving_operator** -- `string s = GetNullable()!; s.Length;` -> "info" severity -- Covers: language idiom ignorance

---

## exception_control_flow

### Current Implementation
- **File:** `src/audit/pipelines/csharp/exception_control_flow.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `empty_catch` (catch with no statements), `catch_return_default` (catch { return null; }), `overly_broad_catch` (catch (Exception e) with body)
- **Detection method:** Tree-sitter `catch_clause` query. Counts `named_child_count` of body for emptiness, checks single child for `return_statement` with `null_literal`, checks catch declaration type text for "Exception".

### Problems Identified
1. **[High false positive rate]:** `overly_broad_catch` flags `catch (Exception e)` even when the catch body logs AND rethrows (`catch (Exception e) { logger.Error(e); throw; }`). The test `clean_catch_with_logging` covers `ArgumentException` but NOT `Exception` with rethrow, which is a common legitimate pattern. (Lines 94-108: no check for `throw;` rethrow in body)
2. **[High false negative rate]:** Does not detect `catch { }` (bare catch without type) which is even broader than `catch (Exception)`. Does not detect `catch (Exception)` without the variable name (e.g., `catch (Exception)`).
3. **[Missing compound variants]:** Does not detect catch blocks that only contain `return false;`, `return -1;`, `return string.Empty;`, or `return new List<T>()` -- all are "return default" patterns equivalent to `return null`. Only checks for `null_literal`. (Lines 75-77: `if named_count == 1` and `is_return_null`)
4. **[No scope awareness]:** Does not distinguish between `empty_catch` in test code (acceptable in some test frameworks for negative testing) and production code.
5. **[No suppression/annotation awareness]:** Does not check for `#pragma warning disable CA1031` (common Roslyn analyzer code for "do not catch general exception types").
6. **[Overlapping detection]:** An empty `catch (Exception e) { }` triggers BOTH `empty_catch` and `overly_broad_catch` would be skipped due to `continue` after `empty_catch`, but the `catch_return_default` and `overly_broad_catch` checks are not mutually exclusive -- a `catch (Exception e) { return null; }` triggers `catch_return_default` but not `overly_broad_catch` due to `continue`. This is actually correct behavior but the priority ordering is implicit and fragile.
7. **[Language idiom ignorance]:** In top-level `try`/`catch` in `Main` or global exception handlers, `catch (Exception)` is the standard pattern. The pipeline does not check enclosing method context.
8. **[No severity graduation]:** All patterns are "warning". An empty catch is arguably worse than a broad catch that logs, but both are "warning".
9. **[Literal blindness]:** `get_catch_type_text` only checks for the identifier "Exception" (line 96). Does not match `System.Exception`, which is the fully-qualified form sometimes used.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Empty catch, catch return null, broad catch (Exception) with logging, specific catch, catch with logging and rethrow
- **What's NOT tested:** Bare `catch { }` without type, catch returning `false`/`-1`/empty string, `catch (Exception)` with `throw;` rethrow (false positive), `System.Exception` fully-qualified, `#pragma warning disable CA1031`, nested try/catch, multiple catch clauses on same try, catch-when filters (`catch (Exception e) when (e is TimeoutException)`)

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Iterate C# file nodes. For each file, check if file source contains "catch" via simple string check. Collect file paths with potential catch clauses. **Returns:** List of file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (graph CFG already models try/catch via CfgEdge::Exception but does not capture catch clause specifics like type and body emptiness) **Why not higher-ranked tool:** The CFG models the control flow edges but not the semantic details of catch clauses (type, body contents). Tree-sitter is needed for the structural matching. **Query/Prompt:** Query `catch_clause` nodes. For each: (a) Check `named_child_count` of body for emptiness; (b) Check if body contains only return-default-value statements (null, false, -1, empty string, default, new empty collection); (c) Extract catch type (identifier or qualified_name), check for "Exception" or "System.Exception"; (d) Check for bare `catch` without `catch_declaration`; (e) Check body for `throw;` (rethrow) to exclude from broad-catch findings. **Returns:** Candidate findings with catch type, body contents summary, and rethrow status.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph **Why not higher-ranked tool:** Need tree-sitter for annotations and graph for method context. **Query/Prompt:** Check for `#pragma warning disable CA1031` in scope. Check for `catch ... when (...)` filter expressions (these narrow the catch and should not be flagged as broad). Check enclosing method: if `Main` or has `[ExceptionHandler]` attribute, downgrade broad-catch severity. Use graph `traverse_callers` to check if the method is called from an exception-safe context. **Returns:** Final findings with severity: "error" for empty catch in non-test code, "warning" for catch-return-default, "info" for broad catch with logging.

#### Graph Enhancement Required
- **Missing:** CFG `CfgEdge::Exception` edges exist but the catch clause metadata (type, body analysis) is not stored in the CFG or graph. Adding `catch_type: Option<String>` to Exception edges or a dedicated `CatchClause` node type in CFG would enable graph-only detection.

### New Test Cases
1. **bare_catch_no_type** -- `try { } catch { }` -> detected as `empty_catch` -- Covers: high false negative rate
2. **broad_catch_with_rethrow** -- `catch (Exception e) { logger.Error(e); throw; }` -> NOT detected -- Covers: high false positive rate
3. **catch_return_false** -- `catch (Exception e) { return false; }` -> detected as `catch_return_default` -- Covers: missing compound variants
4. **catch_return_empty_string** -- `catch (Exception e) { return ""; }` -> detected as `catch_return_default` -- Covers: missing compound variants
5. **system_exception_qualified** -- `catch (System.Exception e) { }` -> detected as `overly_broad_catch` -- Covers: literal blindness
6. **pragma_suppress** -- `#pragma warning disable CA1031\n catch (Exception e) { ... }` -> NOT detected -- Covers: no suppression/annotation awareness
7. **catch_when_filter** -- `catch (Exception e) when (e is TimeoutException)` -> NOT detected as broad (it is narrow) -- Covers: language idiom ignorance
8. **severity_graduation** -- empty catch -> "error", broad catch with logging -> "info" -- Covers: no severity graduation

---

## static_global_state

### Current Implementation
- **File:** `src/audit/pipelines/csharp/static_global_state.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `mutable_static_field` (static field without readonly/const)
- **Detection method:** Tree-sitter `field_declaration` query. Checks for `static` modifier AND absence of `readonly` and `const` modifiers.

### Problems Identified
1. **[High false positive rate]:** Flags `static` fields that are thread-safe by design, such as `private static readonly ConcurrentDictionary<K,V> _cache = new()` if `readonly` is missing from the declaration but the type itself is thread-safe. Also flags `private static volatile int _counter` used with `Interlocked` operations (thread-safe usage). (Lines 55-58)
2. **[High false positive rate]:** Flags `private static int _counter` even when used with proper locking (`lock` statements). The field itself is mutable static, but the usage is safe.
3. **[Missing context]:** Does not check if the field is accessed from multiple threads. A mutable static field in a single-threaded console app is not a problem.
4. **[High false negative rate]:** Does not detect mutable static properties (`public static int Counter { get; set; }`) -- only checks `field_declaration`, not `property_declaration`. Properties with auto-getters/setters are equally problematic.
5. **[No suppression/annotation awareness]:** Does not check for `[ThreadStatic]` attribute (which makes the field per-thread, not truly global). Does not check `[field: ThreadStatic]`.
6. **[Language idiom ignorance]:** Singleton pattern via `private static Foo _instance` is a common C# pattern. The field is mutable (set once in a factory method or lazy initialization) but the pattern is intentional. No exclusion for `Lazy<T>` wrapper types.
7. **[No severity graduation]:** All findings are "warning". A `public static` mutable field (accessible from anywhere) is far worse than a `private static` field (encapsulated). No distinction.
8. **[Single-node detection]:** Only looks at the field declaration, never at how the field is used. A static field that is only written in a static constructor and read everywhere else is effectively readonly.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Mutable static detected, static readonly clean, const clean, instance field clean, metadata correct
- **What's NOT tested:** Static properties (false negative), `[ThreadStatic]` attribute, `Lazy<T>` wrapper, `ConcurrentDictionary` fields, `volatile` fields with `Interlocked`, `public static` vs `private static` severity, static constructor-only writes, singleton pattern

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Iterate C# file nodes. Find all Symbol nodes with `kind == Variable` (fields) or `kind == Property` via DefinedIn edges. Filter to those that will need static analysis. **Returns:** List of `(file_path, symbol_node_index)` for fields and properties.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (graph Symbol nodes do not store modifiers like `static`/`readonly`/`const`/`volatile`) **Why not higher-ranked tool:** The graph stores `exported` (public/internal) but not the full modifier set. Need tree-sitter to check for `static`, `readonly`, `const`, `volatile`. **Query/Prompt:** For each candidate field/property: check modifiers. Include `property_declaration` with `{ get; set; }` that has `static` modifier. Exclude `readonly`, `const`, `[ThreadStatic]`, and `Lazy<T>` type. **Returns:** Mutable static fields/properties with their visibility, type, and attributes.

#### Step 3: False Positive Removal
- **Tool:** Graph (CFG analysis for write patterns) **Why not higher-ranked tool:** N/A, using graph. **Query/Prompt:** For each mutable static field, use `find_symbols_by_name` to locate all call sites that reference this field. Via CFG, check if all writes occur in static constructors or `Interlocked.*` calls. If yes, downgrade to "info". Check if field type is inherently thread-safe (`ConcurrentDictionary`, `ConcurrentBag`, `ImmutableArray`, etc.). **Returns:** Final findings with severity: "error" for `public static` mutable fields, "warning" for `private static` mutable fields with multi-method writes, "info" for `private static` with constructor-only writes or thread-safe types.

#### Graph Enhancement Required
- **Missing:** Graph Symbol nodes do not store modifier sets. Adding `modifiers: Vec<String>` to `NodeWeight::Symbol` would enable graph-only filtering for `static`, `readonly`, `const`, `volatile`, etc.
- **Missing:** Graph does not track field write sites. Adding `WritesTo` edges from CallSite/Symbol to field Symbol nodes would enable graph-only write pattern analysis.

### New Test Cases
1. **static_property_mutable** -- `public static int Counter { get; set; }` -> detected as `mutable_static_property` -- Covers: high false negative rate
2. **threadstatic_attribute** -- `[ThreadStatic] private static int _perThread;` -> NOT detected -- Covers: no suppression/annotation awareness
3. **lazy_wrapper** -- `private static Lazy<Foo> _instance = new(() => new Foo());` -> NOT detected -- Covers: language idiom ignorance
4. **concurrent_dictionary** -- `private static ConcurrentDictionary<string, int> _cache;` -> "info" severity -- Covers: missing context
5. **public_vs_private_severity** -- `public static int X;` -> "error"; `private static int Y;` -> "warning" -- Covers: no severity graduation
6. **static_constructor_only** -- static field written only in `static Foo() { _val = 42; }` -> "info" -- Covers: single-node detection
7. **volatile_with_interlocked** -- `private static volatile int _count;` used with `Interlocked.Increment` -> NOT detected -- Covers: high false positive rate

---

## disposable_not_disposed

### Current Implementation
- **File:** `src/audit/pipelines/csharp/disposable_not_disposed.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `missing_using` (IDisposable type created outside `using` statement)
- **Detection method:** Tree-sitter `local_declaration_statement` query. Matches declared type against hardcoded `DISPOSABLE_TYPES` list (20 types). Walks parent chain to check for `using_statement`/`using_declaration` ancestor.

### Problems Identified
1. **[High false negative rate]:** The `DISPOSABLE_TYPES` list is hardcoded to 20 types. Any user-defined `IDisposable` implementation (e.g., custom `DbContext`, `ServiceScope`, `TransactionScope`, `Font`, `Bitmap`, `Graphics`, `Pen`, `Brush`, `SolidBrush`, `Image`, `Icon`, `WebClient`, `SmtpClient`, `EventLog`, `PerformanceCounter`) is completely missed. (Lines 12-33)
2. **[High false negative rate]:** Does not detect `var` declarations: `var fs = new FileStream(...)` -- the type is `var` (implicit), not `FileStream`, so the type check fails. The `var_type` capture will be `implicit_type` or `var`, not the actual type. This is a critical miss since `var` is the dominant style in modern C#.
3. **[High false negative rate]:** Does not detect factory method returns: `var conn = DbFactory.CreateConnection()` -- no `new` expression with a type name to match against.
4. **[High false positive rate]:** Flags `HttpClient` outside `using`, but Microsoft explicitly recommends NOT disposing `HttpClient` per-request (use `IHttpClientFactory` instead). `HttpClient` in the list is a false positive for the recommended pattern.
5. **[No data flow tracking]:** Does not track if the disposable is passed to another method that takes ownership (e.g., `return new StreamReader(stream)` where `StreamReader` takes ownership of `stream`). Does not track if the disposable is assigned to a field (class-level disposal via `IDisposable` implementation).
6. **[No scope awareness]:** `is_inside_using` walks up to `method_declaration`/`constructor_declaration`/`class_declaration` boundaries, but misses `using` declarations in the same scope at a later point (C# 8 `using var` at statement level).
7. **[Language idiom ignorance]:** C# 8+ `using var fs = new FileStream(...)` (using declaration without braces) should be clean, but the detection may or may not catch `using_declaration` depending on tree-sitter grammar. The parent walk checks for it, but the local_declaration_statement query might not match `using_declaration` at all since it is a different node type.
8. **[Missing compound variants]:** Does not detect disposables created in method arguments: `Process.Start(new ProcessStartInfo(...))` -- the `ProcessStartInfo` is not disposable but the `Process` return value is, and it is not captured at all.
9. **[No suppression/annotation awareness]:** No check for `[SuppressMessage]` or `#pragma warning disable CA2000` (standard Roslyn analyzer for disposable tracking).

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** FileStream without using, FileStream with using statement, non-disposable type, HttpClient metadata
- **What's NOT tested:** `var` declarations (critical miss), factory methods, `using var` (C# 8 declaration), user-defined IDisposable types, ownership transfer, field assignment, `HttpClient` recommended pattern, `#pragma warning disable CA2000`

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query (resource lifecycle via Acquires/ReleasedBy edges) **Why not higher-ranked tool:** Graph IS the highest-ranked tool. The `ResourceAnalyzer` already builds `Acquires`/`ReleasedBy` edges from CFG `ResourceAcquire`/`ResourceRelease` statements. The C# CFG builder emits these for `using_statement`. **Query/Prompt:** Iterate all function nodes with CFGs. For each CFG, find `CfgStatementKind::ResourceAcquire` statements. Check if there is a corresponding `CfgStatementKind::ResourceRelease` on all exit paths. Functions with Acquires edges but missing ReleasedBy edges are candidates. **Returns:** List of `(function_node, resource_type, file_path, line)` where release is missing.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (to identify the actual type being created, handling `var` inference) **Why not higher-ranked tool:** Graph CFG stores `resource_type: "IDisposable"` generically. Need tree-sitter to identify the concrete type from `object_creation_expression` for meaningful messages. **Query/Prompt:** For each candidate, find the `local_declaration_statement` or `object_creation_expression` at the identified line. Extract the concrete type from `new TypeName(...)` expression regardless of whether `var` or explicit type is used on the left side. Also detect factory method patterns by checking if the right-hand side is an `invocation_expression` whose name matches known factory patterns (Create*, Open*, Get*). **Returns:** Concrete type names and creation patterns.

#### Step 3: False Positive Removal
- **Tool:** Graph (call graph for ownership transfer) + Tree-sitter (for annotations) **Why not higher-ranked tool:** Need graph for cross-method analysis and tree-sitter for suppression attributes. **Query/Prompt:** Check if the variable is passed as an argument to another method call in the same scope (potential ownership transfer). Check if the variable is assigned to a class field (class-level IDisposable pattern). Exclude `HttpClient` unless it is created per-request inside a loop. Check for `[SuppressMessage("...", "CA2000")]` and `#pragma warning disable CA2000`. **Returns:** Final findings with severity: "warning" for known-disposable types without using, "info" for potential disposables from factory methods.

#### Graph Enhancement Required
- **Missing:** The CFG `ResourceAcquire` is only emitted for `using_statement` bodies by the C# CFG builder. Plain `new FileStream(...)` without `using` does NOT emit a `ResourceAcquire` statement. The CFG builder should be enhanced to emit `ResourceAcquire` for all `object_creation_expression` nodes where the type is known-disposable.
- **Missing:** `ResourceAnalyzer.CALL_BASED_RELEASE_NAMES` includes "Dispose" and "dispose" but the detection is call-name-based, not type-based. This works for explicit `.Dispose()` calls but misses the `using` declaration pattern (`using var x = ...`) where the compiler generates the Dispose call.

### New Test Cases
1. **var_declaration** -- `var fs = new FileStream("test.txt", FileMode.Open);` -> detected -- Covers: high false negative rate
2. **factory_method** -- `var conn = SqlConnection.Create();` -> detected -- Covers: high false negative rate
3. **using_var_declaration** -- `using var fs = new FileStream("test.txt", FileMode.Open);` -> NOT detected -- Covers: language idiom ignorance
4. **custom_disposable** -- `MyDbContext ctx = new MyDbContext();` where MyDbContext : IDisposable -> detected (with graph) -- Covers: high false negative rate
5. **httpclient_singleton** -- `private static readonly HttpClient _client = new HttpClient();` -> NOT detected (recommended pattern) -- Covers: high false positive rate
6. **ownership_transfer** -- `var reader = new StreamReader(stream); return reader;` -> NOT detected (caller disposes) -- Covers: no data flow tracking
7. **pragma_suppress_ca2000** -- `#pragma warning disable CA2000\n FileStream fs = new FileStream(...);` -> NOT detected -- Covers: no suppression/annotation awareness
8. **field_assignment** -- `this._connection = new SqlConnection(...)` -> NOT detected (class-level dispose) -- Covers: no scope awareness

---

## god_class

### Current Implementation
- **File:** `src/audit/pipelines/csharp/god_class.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `too_many_methods` (>10 methods), `too_many_fields` (>15 fields)
- **Detection method:** Tree-sitter `class_declaration` query. Iterates body children counting `method_declaration`/`constructor_declaration` and `field_declaration` kinds.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `MAX_METHODS = 10` and `MAX_FIELDS = 15` have no justification. Common C# patterns like classes implementing multiple interfaces, or Entity Framework DbContext classes, easily exceed 10 methods legitimately. (Lines 12-13)
2. **[High false negative rate]:** Does not count `property_declaration` which in C# often replaces fields. A class with 2 fields but 30 auto-properties (`{ get; set; }`) is a god class but would not be flagged on the fields metric.
3. **[High false negative rate]:** Does not count nested classes, events, indexers, operators, or destructors. A class with 5 methods but 20 events and 10 nested classes is complex but undetected.
4. **[Missing context]:** Does not check class inheritance. A class that overrides 8 virtual methods from a base class is not a god class -- it is implementing a required interface contract. Similarly, partial classes may appear small in one file but large across files.
5. **[No scope awareness]:** Counts all methods including `private` helper methods. A class with 3 public methods and 8 private helpers (good encapsulation) is flagged, while a class with 9 public methods (poor encapsulation) is not.
6. **[Language idiom ignorance]:** Does not exclude generated code patterns (Entity Framework migrations, designer files, `*.g.cs`). Does not exclude `partial class` where the total is split across files.
7. **[No severity graduation]:** Both findings are "warning". A class with 11 methods gets the same severity as a class with 50 methods.
8. **[Overlapping detection]:** The `god_class` pipeline and the `god_controller` pipeline overlap for `*Controller` classes. A `FooController` with 12 public methods triggers BOTH `too_many_methods` (god_class) and `oversized_controller` (god_controller).
9. **[Single-node detection]:** Only counts direct children of the class body. Does not consider class complexity metrics like LCOM (Lack of Cohesion of Methods), WMC (Weighted Methods per Class), or coupling between methods.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Too many methods (12), too many fields (16), clean small class, metadata
- **What's NOT tested:** Properties not counted (false negative), partial classes, generated code, inheritance-driven methods, nested classes/events, public vs private method distinction, very large classes (50+ methods severity), overlap with god_controller

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Use `graph.file_entries()` to get file entries with `symbol_count`. Filter files with high symbol counts (>15) as candidates. Alternatively, iterate graph nodes to find Symbol nodes with `kind == Class` and count child Symbol nodes (methods, properties, fields) via `Contains` edges. **Returns:** List of `(file_path, class_node_index, class_name, child_counts)`.

#### Step 2: Narrowing
- **Tool:** Graph query **Why not higher-ranked tool:** Using graph. **Query/Prompt:** For each class Symbol node, count outgoing `Contains` edges by child kind: methods, properties, fields, nested types, events. Calculate composite complexity metric: `total = methods + properties + fields/2 + nested_types*3`. Also check LCOM: count how many method pairs share field access (via CallSite edges to field names). **Returns:** Per-class complexity metrics.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (for modifiers, partial keyword, generated code markers) **Why not higher-ranked tool:** Graph does not store `partial` modifier or file-level attributes like `[GeneratedCode]`. **Query/Prompt:** Check for `partial` modifier on class (need cross-file aggregation). Check for `[GeneratedCode]` attribute. Check file path for generated code patterns (`*.g.cs`, `*.designer.cs`, `Migrations/`). Exclude classes that are primarily interface implementations (check base_list for interface names). Distinguish public vs private member counts. **Returns:** Final findings with severity: "error" for classes >30 total members, "warning" for >20, "info" for >15. Exclude generated code and pure interface implementations.

#### Graph Enhancement Required
- **Missing:** Graph `Contains` edges between Symbol nodes (class -> method, class -> property, class -> field) exist but are built from DefinedIn edges. Verify that all member kinds (properties, events, nested types) generate Symbol nodes that link to the parent class.
- **Missing:** Graph does not track `partial class` across files. A cross-file aggregation pass would be needed to merge partial class symbol counts.

### New Test Cases
1. **properties_counted** -- class with 2 fields and 20 auto-properties -> detected -- Covers: high false negative rate
2. **partial_class** -- `partial class Foo` with 6 methods in file A and 6 methods in file B -> detected as total -- Covers: missing context
3. **generated_code_excluded** -- `[GeneratedCode("EF")] class Migration { ... 50 methods ... }` -> NOT detected -- Covers: language idiom ignorance
4. **interface_implementation** -- class implementing IFoo with 12 required methods -> NOT detected or "info" -- Covers: missing context
5. **severity_graduation** -- 11 methods -> "info", 25 methods -> "warning", 50 methods -> "error" -- Covers: no severity graduation
6. **controller_overlap_excluded** -- `FooController` with 12 methods -> flagged ONLY by god_controller, not god_class -- Covers: overlapping detection
7. **nested_types_weighted** -- class with 5 methods and 5 nested classes -> detected (high complexity) -- Covers: single-node detection
8. **public_private_distinction** -- 3 public methods + 10 private helpers -> NOT detected (good encapsulation) -- Covers: no scope awareness

---

## stringly_typed

### Current Implementation
- **File:** `src/audit/pipelines/csharp/stringly_typed.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `stringly_typed` (string parameters/fields/properties with names suggesting enum usage)
- **Detection method:** Tree-sitter `parameter`, `field_declaration`, and `property_declaration` queries. Checks if type is "string"/"String" and name matches against `SUSPICIOUS_NAMES` list (17 names) using lowercase equality, `_suffix` pattern, or PascalCase suffix.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** The `SUSPICIOUS_NAMES` list has 17 entries with no source citation. Names like "action" and "event_type" may have legitimate string uses (e.g., `string action` in a logging method, `string event_type` for dynamic event routing). (Lines 15-33)
2. **[High false positive rate]:** Flags `string status` in DTOs, API response models, and serialization classes where string is the correct type (external API contract). The pipeline checks for DTO suffix exclusion in `anemic_domain_model` but not here. (Lines 94-95: no DTO exclusion)
3. **[High false positive rate]:** Flags `string type` which is extremely common in reflection, serialization, and generic programming contexts where string is correct.
4. **[High false negative rate]:** Does not detect `string errorCode`, `string httpMethod`, `string dayOfWeek`, `string gender`, `string paymentMethod`, `string orderState`, `string userRole` -- many domain-specific names that should be enums are not in the list.
5. **[No scope awareness]:** Does not check if the string parameter/field is used in switch/if-else chains (which would strongly suggest it should be an enum). Just matching name without usage analysis.
6. **[Language idiom ignorance]:** Does not check if an enum with a matching name already exists in the same file/project. If `enum Status` exists but the parameter is `string status`, that is a stronger signal than just the name.
7. **[Missing compound variants]:** Only checks direct type "string"/"String". Does not check `string?` (nullable string), `String?`, or `IEnumerable<string>` where the collection element should be an enum.
8. **[No suppression/annotation awareness]:** No check for `[JsonProperty]` or `[DataMember]` attributes that indicate external serialization contract (string is required).
9. **[No severity graduation]:** All findings are "info". A `public string Status` property with switch statements in 5 methods is more concerning than a `private string _mode` field used once.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** String status parameter, string Role property, clean normal string param (name), clean enum typed, compound name (orderStatus)
- **What's NOT tested:** DTO/serialization context (false positive), `string type` reflection context, nullable string, `[JsonProperty]` attribute, field in switch/if-else usage, enum exists with same name, collection of strings

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Find all Symbol nodes with `kind == Method` or `kind == Property` or `kind == Variable` (fields) in C# files. **Returns:** List of candidates.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (graph does not store parameter/field types) **Why not higher-ranked tool:** Graph Symbol nodes do not include type information. **Query/Prompt:** Query parameters, fields, and properties with string type. Match names against suspicious name list (expanded to ~30+ names). Check if the enclosing class name ends with DTO/ViewModel/Request/Response suffixes -- exclude if so. **Returns:** Candidate findings.

#### Step 3: False Positive Removal
- **Tool:** Graph + Tree-sitter **Why not higher-ranked tool:** Need graph for cross-reference analysis. **Query/Prompt:** Use `graph.find_symbols_by_name` to check if an enum with a matching name exists (e.g., if field is `string status`, check for Symbol with `kind == Enum` and `name == "Status"`). If found, upgrade severity. Check if the string is used in switch/if-else patterns via CFG `CfgStatementKind::Guard` conditions containing the variable. Check for serialization attributes via tree-sitter. **Returns:** Final findings with severity: "warning" if matching enum exists or used in switch, "info" otherwise.

#### Graph Enhancement Required
- **Missing:** Graph does not store parameter types or field types. Adding `type_name: Option<String>` to Symbol nodes or a separate `TypeOf` edge would enable graph-only type filtering.

### New Test Cases
1. **dto_excluded** -- `class OrderDto { public string Status { get; set; } }` -> NOT detected -- Covers: high false positive rate
2. **serialization_attribute** -- `[JsonProperty("status")] public string Status { get; set; }` -> NOT detected -- Covers: no suppression/annotation awareness
3. **matching_enum_exists** -- `enum Status { Active, Inactive } class Foo { string status; }` -> "warning" severity -- Covers: language idiom ignorance
4. **used_in_switch** -- `string status` used in `switch (status) { case "active": ... }` -> "warning" -- Covers: no scope awareness
5. **nullable_string** -- `void M(string? status)` -> detected -- Covers: missing compound variants
6. **reflection_context** -- `string type` in a method with reflection calls -> NOT detected -- Covers: high false positive rate
7. **expanded_names** -- `string paymentMethod`, `string httpMethod`, `string errorCode` -> detected -- Covers: high false negative rate

---

## god_controller

### Current Implementation
- **File:** `src/audit/pipelines/csharp/god_controller.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `oversized_controller` (Controller class with >8 public action methods)
- **Detection method:** Tree-sitter `class_declaration` query. Filters classes with names ending in "Controller". Counts children of kind `method_declaration` that have `public` modifier.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `MAX_ACTIONS = 8` is arbitrary. RESTful controllers typically have 5-7 CRUD actions plus custom actions. An 8-action threshold is aggressive for real-world APIs. (Line 12)
2. **[High false positive rate]:** Counts ALL public methods, not just action methods. Public helper methods, static factory methods, and `Dispose()` override are counted. In ASP.NET Core, non-action methods can be marked `[NonAction]` -- this is not checked. (Lines 73-74)
3. **[High false negative rate]:** Only checks for name ending in "Controller". ASP.NET Core also supports `[ApiController]` attribute on classes not ending in "Controller" (via `[Controller]` attribute). Minimal API endpoints (not controller-based) are completely missed.
4. **[Language idiom ignorance]:** Does not distinguish between MVC controllers (view-returning actions) and API controllers (data-returning actions). API controllers often have more actions for different query variations. Also does not check for `[Area]` attribute routing which may indicate intentional grouping.
5. **[No suppression/annotation awareness]:** Does not check for `[NonAction]` attribute on methods. Does not check for `[ApiExplorerSettings(IgnoreApi = true)]` which hides methods from API surface.
6. **[Overlapping detection]:** A `FooController` with 15 methods triggers both `god_controller` (oversized_controller) and `god_class` (too_many_methods). These are redundant findings.
7. **[No severity graduation]:** All findings are "warning". A controller with 9 actions is barely over threshold while one with 30 actions is severely bloated. Same severity for both.
8. **[Single-node detection]:** Only counts methods in the class body. Does not account for inherited action methods from base controller classes (which also contribute to the API surface).
9. **[Missing compound variants]:** Does not check route complexity. A controller with 8 actions each taking 5 parameters is worse than one with 10 simple CRUD actions. Does not detect controllers that should be split by resource (e.g., `AdminController` handling users, roles, and settings).

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Oversized controller (10 public methods), clean small controller (2 methods), non-controller class ignored, counts only public methods (5 public + 10 private = clean)
- **What's NOT tested:** `[NonAction]` attribute, `[ApiController]` without "Controller" suffix, inherited action methods, `[Area]` routing, overlap with god_class, severity graduation, controllers with many parameters, minimal API patterns

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Find all Symbol nodes with `kind == Class` where `name` ends with "Controller" or where the source contains `[ApiController]` attribute. Use `graph.find_symbols_by_name` with partial matching. **Returns:** List of controller class node indices.

#### Step 2: Narrowing
- **Tool:** Graph query + Tree-sitter **Why not higher-ranked tool:** Graph does not store attributes. **Query/Prompt:** For each controller class, use graph `Contains` edges to find child Symbol nodes with `kind == Method` and `exported == true` (public). Use tree-sitter to check for `[NonAction]` attribute on each method -- exclude those. Count remaining action methods. Also check inherited methods by looking at base class (tree-sitter base_list) and finding parent class Symbol via `find_symbols_by_name`. **Returns:** Action counts per controller with attribute filtering.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (attributes) **Why not higher-ranked tool:** Graph does not store attributes or decorators. **Query/Prompt:** Check for `[Area]` or route-based grouping that explains high action count. Check if controller is partial (methods split across files). Suppress if `[SuppressMessage]` present. **Returns:** Final findings with severity: "info" for 9-12 actions, "warning" for 13-20, "error" for >20.

#### Graph Enhancement Required
- **Missing:** Graph does not store class inheritance relationships (base classes, implemented interfaces). A `InheritsFrom` edge between class Symbol nodes would enable inherited method counting.
- **Missing:** Graph does not store attributes/decorators on symbols. An `attributes: Vec<String>` on Symbol nodes would enable `[NonAction]`, `[ApiController]`, `[Area]` detection.

### New Test Cases
1. **non_action_excluded** -- public method with `[NonAction]` attribute -> not counted -- Covers: no suppression/annotation awareness
2. **api_controller_attribute** -- `[ApiController] class Orders { ... 10 public methods ... }` (no "Controller" suffix) -> detected -- Covers: high false negative rate
3. **inherited_actions** -- `class FooController : CrudController { ... 3 extra methods ... }` where base has 6 -> detected as 9 total -- Covers: single-node detection
4. **severity_graduation** -- 9 actions -> "info", 15 -> "warning", 25 -> "error" -- Covers: no severity graduation
5. **no_overlap_with_god_class** -- controller flagged by god_controller should not also be flagged by god_class -- Covers: overlapping detection
6. **area_routing** -- `[Area("Admin")] class AdminController { ... 10 actions ... }` -> "info" (area justifies grouping) -- Covers: language idiom ignorance

---

## thread_sleep

### Current Implementation
- **File:** `src/audit/pipelines/csharp/thread_sleep.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `thread_sleep_call` (Thread.Sleep() invocations)
- **Detection method:** Tree-sitter `invocation_expression` query. Checks function text == "Thread.Sleep" or ends with ".Thread.Sleep".

### Problems Identified
1. **[No scope awareness]:** Flags `Thread.Sleep` everywhere including test code, console applications, and deliberately synchronous polling loops where it is appropriate. No check for whether the enclosing method is async (where `Task.Delay` would be better) vs sync (where `Thread.Sleep` may be acceptable). (Lines 54-55)
2. **[High false negative rate]:** Does not detect `System.Threading.Thread.Sleep(...)` (fully-qualified form). The check `fn_text == "Thread.Sleep"` will fail for the fully-qualified call. Also does not detect `Thread.Sleep` via a static import alias.
3. **[No severity graduation]:** All findings are "warning". `Thread.Sleep` in a tight loop inside an async method is critical; `Thread.Sleep(100)` in a CLI tool's retry logic is benign.
4. **[Single-node detection]:** Only detects the call, not the context. Does not check if there is a `CancellationToken` that should be used with `Task.Delay` instead. Does not check the sleep duration (a 0ms sleep for yielding is different from a 30-second sleep).
5. **[No suppression/annotation awareness]:** No check for `#pragma warning disable` or custom suppression comments.
6. **[Missing compound variants]:** Does not detect `Task.Wait(TimeSpan)` or `ManualResetEvent.WaitOne(timeout)` which are equivalent blocking sleep patterns. Does not detect `SpinWait.SpinUntil(...)`.
7. **[Language idiom ignorance]:** `Thread.Sleep(0)` and `Thread.Sleep(1)` are documented .NET idioms for yielding the thread to the OS scheduler. These should be treated differently from actual sleeps.
8. **[Literal blindness]:** Does not check the argument value. A `Thread.Sleep(0)` yield is different from `Thread.Sleep(60000)` production-blocking call.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Basic Thread.Sleep detection, clean Task.Delay, unrelated invocation ignored
- **What's NOT tested:** Fully-qualified `System.Threading.Thread.Sleep`, `Thread.Sleep(0)` yield idiom, async vs sync method context, test code exclusion, sleep duration analysis, `ManualResetEvent.WaitOne`, `SpinWait.SpinUntil`

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Find CallSite nodes where `name` contains "Sleep", "WaitOne", "SpinUntil", or "Thread.Sleep" via `graph.symbols_by_name` or iteration over CallSite nodes. **Returns:** List of call site node indices with file paths and lines.

#### Step 2: Narrowing
- **Tool:** Graph CFG + Tree-sitter **Why not higher-ranked tool:** CFG provides the call context (async vs sync method, loop detection). Tree-sitter needed for argument analysis. **Query/Prompt:** For each call site, find the enclosing function via graph edges (incoming DefinedIn or containment). Check if function has `async` modifier (via tree-sitter). Check if the call is inside a loop (via CFG back-edges). Extract the sleep duration argument from tree-sitter. **Returns:** Enriched findings with async context, loop presence, and duration.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (file path analysis for test exclusion, argument analysis) **Why not higher-ranked tool:** Need tree-sitter for attribute and argument extraction. **Query/Prompt:** Exclude test files (path contains "Test", "Spec", or has `[TestClass]`/`[Fact]`/`[Test]` attributes). Exclude `Thread.Sleep(0)` and `Thread.Sleep(1)` yields. Check for `#pragma warning disable`. **Returns:** Final findings with severity: "error" for `Thread.Sleep` in async method inside loop, "warning" for `Thread.Sleep` in async method, "info" for `Thread.Sleep` in sync method.

#### Graph Enhancement Required
- None strictly required. The existing CFG and CallSite infrastructure is sufficient. However, storing the call arguments in `CallSite` nodes would enable argument analysis without tree-sitter fallback.

### New Test Cases
1. **fully_qualified** -- `System.Threading.Thread.Sleep(1000)` -> detected -- Covers: high false negative rate
2. **sleep_zero_yield** -- `Thread.Sleep(0)` -> NOT detected (yield idiom) -- Covers: language idiom ignorance, literal blindness
3. **async_method_context** -- `async Task M() { Thread.Sleep(1000); }` -> "error" severity -- Covers: no severity graduation, no scope awareness
4. **sync_method_context** -- `void Main() { Thread.Sleep(1000); }` -> "info" severity -- Covers: no severity graduation
5. **in_loop** -- `while (true) { Thread.Sleep(100); }` inside async method -> "error" -- Covers: single-node detection
6. **test_code_excluded** -- `[TestClass] class Tests { void T() { Thread.Sleep(500); } }` -> NOT detected -- Covers: no scope awareness
7. **manual_reset_event** -- `ManualResetEvent.WaitOne(5000)` -> detected as blocking wait -- Covers: missing compound variants

---

## missing_cancellation_token

### Current Implementation
- **File:** `src/audit/pipelines/csharp/missing_cancellation_token.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `no_cancellation_token` (async methods without CancellationToken parameter)
- **Detection method:** Tree-sitter `method_declaration` query. Checks for `async` modifier, then iterates parameters looking for type == "CancellationToken".

### Problems Identified
1. **[High false positive rate]:** Flags ALL async methods without CancellationToken, including event handlers (`async void OnClick`), top-level program entry points, `IHostedService.StartAsync`/`StopAsync` (which receive CT from the framework), and methods that call only non-cancellable APIs. (Lines 64-66: any async method without CT)
2. **[High false positive rate]:** Flags async methods in interface implementations where the interface does not define a CancellationToken parameter. The developer cannot add it without changing the interface.
3. **[High false negative rate]:** Does not check if the CancellationToken parameter is actually USED in the method body. A method with `CancellationToken ct` that never passes it to any `await`-ed call is equally problematic.
4. **[No scope awareness]:** Does not check the method's visibility. Private async helper methods called from a single public method that passes CancellationToken may not need their own CT parameter (CT passed via closure or field).
5. **[Language idiom ignorance]:** ASP.NET Core controller actions automatically receive CancellationToken from model binding -- the parameter is optional in the signature but available via `HttpContext.RequestAborted`. Flagging controller actions is a false positive.
6. **[No suppression/annotation awareness]:** No check for `[SuppressMessage]` or custom attributes that indicate intentional omission.
7. **[Missing compound variants]:** Does not check for `CancellationToken?` (nullable), `CancellationToken cancellationToken = default` (optional parameter), or methods that accept `IProgress<T>` (often paired with CT). Does not check if the method wraps `TaskCompletionSource` (often does not need CT directly).
8. **[No severity graduation]:** All findings are "info". A public async method in a library should have CT (important), while a private async method called once is less important.
9. **[Single-node detection]:** Only checks the method signature, not the method body. Does not check if the method calls other async methods that accept CT (indicating it should propagate CT).

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Async without CT, async with CT, sync method ignored, metadata
- **What's NOT tested:** Event handlers (false positive), interface implementations, unused CT parameter, ASP.NET controller actions, private helpers, optional CT parameter (`= default`), nullable CT, CT propagation through call chain

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Find all Symbol nodes with `kind == Method` in C# files. Use tree-sitter to filter to async methods (graph does not store `async` modifier). **Returns:** Async method candidates.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (for parameter type checking, since graph does not store parameter types) + Graph (for call chain analysis) **Why not higher-ranked tool:** Graph does not store parameter types. **Query/Prompt:** For each async method, check if any parameter has type containing "CancellationToken" (including `CancellationToken?`, `CancellationToken cancellationToken = default`). If no CT, use graph `traverse_callees` to check if the method calls other async methods that accept CT (indicating it should propagate CT). **Returns:** Methods missing CT that should have it.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (for attributes and interface context) + Graph (for caller analysis) **Why not higher-ranked tool:** Need tree-sitter for attributes. **Query/Prompt:** Exclude: (1) Methods with `async void` signature (event handlers); (2) Methods in classes ending with "Controller" (ASP.NET CT via model binding); (3) Methods overriding interface methods without CT; (4) Private methods called from a single caller that has CT. Check for `[SuppressMessage]`. **Returns:** Final findings with severity: "warning" for public async methods in libraries, "info" for private/internal methods.

#### Graph Enhancement Required
- **Missing:** Graph does not store parameter types. `Parameter` nodes exist but only store `name`, `function_node`, `position`, `is_taint_source` -- no type info. Adding `type_name: String` to `NodeWeight::Parameter` would enable graph-only analysis.

### New Test Cases
1. **event_handler_excluded** -- `async void OnClick(object sender, EventArgs e)` -> NOT detected -- Covers: high false positive rate
2. **controller_action_excluded** -- `public async Task<IActionResult> Get()` in `FooController` -> NOT detected -- Covers: language idiom ignorance
3. **optional_ct_parameter** -- `async Task M(CancellationToken ct = default)` -> NOT detected (has CT) -- Covers: missing compound variants
4. **unused_ct** -- `async Task M(CancellationToken ct) { await Task.Delay(1000); }` (ct not used) -> detected as "unused_cancellation_token" -- Covers: high false negative rate
5. **interface_implementation** -- method implementing interface without CT -> NOT detected -- Covers: high false positive rate
6. **call_chain_propagation** -- async method calling `HttpClient.GetAsync` (accepts CT) but not passing CT -> "warning" -- Covers: single-node detection
7. **public_vs_private** -- public async method -> "warning", private async helper -> "info" -- Covers: no severity graduation

---

## hardcoded_config

### Current Implementation
- **File:** `src/audit/pipelines/csharp/hardcoded_config.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `hardcoded_config_value` (string literals containing connection strings, API keys, secrets, endpoints)
- **Detection method:** Tree-sitter `string_literal` query. Iterates all string literals, strips quotes, checks if inner text contains any of 11 `SUSPICIOUS_PATTERNS`.

### Problems Identified
1. **[High false positive rate]:** `"secret"` as a pattern matches ANY string containing the word "secret", including `"This is not a secret"`, `"secret_name"` (configuration key name, not value), and `"SecretManager"` (class name reference). (Lines 22: `("secret", "secret value")`)
2. **[High false positive rate]:** `"https://api."` matches ALL API URLs including well-known public APIs and documentation examples. A constant like `"https://api.github.com"` in a test file is not hardcoded config.
3. **[High false positive rate]:** Matches strings in comments, XML doc strings, and log messages: `Console.WriteLine("Server=localhost is the default")`.
4. **[High false negative rate]:** Does not detect interpolated strings: `$"Server={host};Database={db};Password={password}"` -- the `string_literal` query does not match `interpolated_string_expression`. This is ironic because interpolated strings are MORE likely to contain sensitive data.
5. **[High false negative rate]:** Does not detect verbatim strings `@"Server=localhost;..."` or raw string literals (`"""..."""` in C# 11).
6. **[No scope awareness]:** Flags strings in test files, documentation, and constants that reference configuration KEY NAMES (not values). `const string PasswordKey = "password";` -- this is a config key, not a hardcoded password.
7. **[No suppression/annotation awareness]:** No check for `[SuppressMessage]` or test-file exclusion.
8. **[Literal blindness]:** Strips `"` and `@` but does not handle `$"..."` (interpolated), `@$"..."` (verbatim interpolated), or `"""..."""` (raw) string literal prefixes.
9. **[No severity graduation]:** All findings are "warning". A literal containing `"Password=admin123"` (actual credential) should be "error", while `"https://api.example.com"` (endpoint) should be "info".
10. **[Missing compound variants]:** Does not check string concatenation: `"Server=" + host + ";Password=" + password` -- the individual fragments may not match but the composed string would.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Connection string, bearer token, secret key (sk_live_), clean normal strings, metadata
- **What's NOT tested:** False positive on "secret" in normal text, interpolated strings, verbatim strings, test file exclusion, string concatenation, config key names vs values, log message strings, `@$""` strings

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Iterate C# file nodes. For each file, do a quick string search for suspicious patterns in the file source to pre-filter. Exclude test files (path patterns). **Returns:** Candidate file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (need full string literal analysis including interpolated strings) **Why not higher-ranked tool:** Graph does not contain string literal data. **Query/Prompt:** Query both `string_literal` AND `interpolated_string_expression` AND `verbatim_string_literal` (if separate node type) AND `raw_string_literal`. For each, extract the full text content. Match against suspicious patterns with context: check if the string is assigned to a variable whose name suggests configuration (conn, connection, password, key, secret, token) vs documentation (message, description, label). **Returns:** Candidate findings with variable context.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (for assignment context, attribute checking) **Why not higher-ranked tool:** Need AST context for the string's usage. **Query/Prompt:** Check if the string is: (1) inside a test class; (2) inside a string constant whose name ends with "Key"/"Name"/"Path" (config key, not value); (3) inside a comment or XML doc; (4) assigned to an `[Option]` or `[Description]` attribute; (5) in a log/trace method call. Check for `[SuppressMessage]`. **Returns:** Final findings with severity: "error" for literal containing both pattern AND actual value (e.g., full connection string), "warning" for suspicious pattern match, "info" for API endpoint patterns.

#### Graph Enhancement Required
- None strictly required. String literal analysis is inherently tree-sitter work. However, if taint analysis tracked string literal sources, the graph could detect when hardcoded strings flow to security-sensitive sinks (SQL queries, HTTP headers).

### New Test Cases
1. **interpolated_string** -- `$"Server={host};Password=admin123"` -> detected -- Covers: high false negative rate
2. **word_secret_in_text** -- `string msg = "This is not a secret value";` -> NOT detected -- Covers: high false positive rate
3. **config_key_not_value** -- `const string PasswordKey = "password";` -> NOT detected (key name) -- Covers: no scope awareness
4. **verbatim_string** -- `@"Server=localhost;Database=mydb"` -> detected -- Covers: literal blindness
5. **test_file_excluded** -- string in `FooTests.cs` -> NOT detected -- Covers: no scope awareness
6. **severity_graduation** -- full connection string with password -> "error", API URL -> "info" -- Covers: no severity graduation
7. **log_message_excluded** -- `logger.Info("Server=localhost is starting")` -> NOT detected -- Covers: high false positive rate
8. **string_concatenation** -- `"Server=" + host + ";Password=" + pass` -> detected -- Covers: missing compound variants

---

## anemic_domain_model

### Current Implementation
- **File:** `src/audit/pipelines/csharp/anemic_domain_model.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `anemic_class` (classes with >= 3 properties and 0 methods, excluding DTO/ViewModel suffixes)
- **Detection method:** Tree-sitter `class_declaration` query. Counts `property_declaration` and `method_declaration` children. Flags if `property_count >= MIN_PROPERTIES (3)` and `method_count == 0`. Excludes classes whose names end with any of 13 `EXCLUDED_SUFFIXES`.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `MIN_PROPERTIES = 3` means a class with 2 properties and 0 methods is clean, but 3 properties and 0 methods is flagged. No justification for why 3 is the threshold. (Line 12)
2. **[High false positive rate]:** Flags record-like classes, POCO classes, and entity classes used with ORMs like Entity Framework. These classes intentionally have only properties (the framework provides behavior). The `EXCLUDED_SUFFIXES` list covers some but misses: `Entity`, `Model`, `Record`, `State`, `Args`, `Params`, `Result`, `Payload`, `Schema`, `Spec`, `Attribute`. (Lines 13-27)
3. **[High false negative rate]:** Requires `method_count == 0` (strictly zero). A class with 15 properties and 1 trivial method (e.g., `ToString()`) is not flagged but is still anemic. Should check ratio of properties to methods.
4. **[Missing context]:** Does not check if the class has a constructor with logic, which indicates behavior. Does not check if the class is used with a service class that provides behavior (anemic domain model antipattern requires both data class and separate service class).
5. **[No scope awareness]:** Does not check file location. Classes in a `Models/` or `Entities/` directory are expected to be data-only. Classes in a `Domain/` or `Services/` directory should have behavior.
6. **[Language idiom ignorance]:** C# records (`record Order(int Id, string Name, decimal Price)`) are data-focused by design and should not be flagged. The query uses `class_declaration` which would not match `record_declaration`, but the excluded suffixes list should handle this. Also, `struct` declarations are not checked.
7. **[Single-node detection]:** Does not check inheritance. A class that inherits from a base class with methods is not truly anemic. Also does not check if the class implements interfaces that define behavior contracts.
8. **[No suppression/annotation awareness]:** No check for `[SuppressMessage]` or custom attributes like `[DataContract]`, `[Table]` (EF), `[ProtoContract]` (protobuf) that indicate intentional data-only design.
9. **[Missing compound variants]:** Does not count fields with initializers (`private readonly List<Item> _items = new()`) which contain behavior-like setup. Does not count static methods or extension methods in the same file.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Anemic class (3 props, 0 methods), class with methods clean, DTO suffix excluded, ViewModel suffix excluded, small class (2 props) ignored
- **What's NOT tested:** Entity suffix, Model suffix, record type, class with constructor logic, class in Models/ directory, class with 1 trivial method (still anemic), `[Table]`/`[DataContract]` attributes, inheritance from behavior-rich base, partial class with methods in another file

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query **Why not higher-ranked tool:** Graph is highest. **Query/Prompt:** Find all Symbol nodes with `kind == Class` in C# files. For each, count child symbols with `kind == Property` vs `kind == Method` via `Contains` or `DefinedIn` edges. Filter to classes where property count >= 3 and method count <= 1. **Returns:** Candidate anemic class node indices.

#### Step 2: Narrowing
- **Tool:** Graph + Tree-sitter **Why not higher-ranked tool:** Need tree-sitter for class name suffix and attribute checking. **Query/Prompt:** For each candidate: (1) Check name against expanded exclusion list (add Entity, Model, Record, State, Args, Params, Result, Payload); (2) Check for data-oriented attributes via tree-sitter: `[Table]`, `[DataContract]`, `[ProtoContract]`, `[JsonObject]`; (3) Check file path for `Models/`, `Entities/`, `DTOs/` directories; (4) Check if class inherits from a base class (tree-sitter base_list). **Returns:** Filtered candidates.

#### Step 3: False Positive Removal
- **Tool:** Graph (cross-file analysis) **Why not higher-ranked tool:** Using graph. **Query/Prompt:** For each candidate, use `graph.find_symbols_by_name` to check if a corresponding service class exists (e.g., `OrderService` for `Order`). If both exist, this IS the anemic domain model antipattern -- upgrade severity. If no service class, the class may just be a legitimate POCO. Check if the class is referenced by a Repository pattern (via call graph analysis). **Returns:** Final findings with severity: "warning" if paired service exists (antipattern confirmed), "info" if standalone data class.

#### Graph Enhancement Required
- **Missing:** Graph does not store class inheritance/implementation relationships. A `InheritsFrom`/`Implements` edge between class Symbol nodes would enable checking if the class inherits behavior.

### New Test Cases
1. **entity_suffix_excluded** -- `class OrderEntity { ... 5 props ... }` -> NOT detected -- Covers: high false positive rate
2. **ef_table_attribute** -- `[Table("orders")] class Order { ... 5 props ... }` -> NOT detected -- Covers: no suppression/annotation awareness
3. **one_trivial_method** -- class with 10 props and 1 `ToString()` override -> detected (still anemic) -- Covers: high false negative rate
4. **paired_service_class** -- `class Order` (anemic) + `class OrderService` (behavior) -> "warning" (antipattern confirmed) -- Covers: missing context
5. **models_directory** -- class in `/Models/Order.cs` -> NOT detected (expected pattern) -- Covers: no scope awareness
6. **record_type** -- `record Order(int Id, string Name)` -> NOT detected -- Covers: language idiom ignorance
7. **inheritance_with_behavior** -- `class Order : BaseEntity` where BaseEntity has methods -> NOT detected -- Covers: single-node detection
8. **properties_to_methods_ratio** -- 15 properties, 2 methods -> detected (high ratio) -- Covers: hardcoded thresholds

---

## Cross-Cutting Issues

### All Pipelines Share These Problems

1. **Legacy Pipeline trait:** All 12 pipelines use `Pipeline` (not `GraphPipeline`). They receive `(tree, source, file_path)` with no access to the `CodeGraph`. The C# CFG builder, taint analysis, and resource lifecycle analysis are all implemented but completely inaccessible to these pipelines.

2. **No suppression/annotation awareness:** Zero pipelines check for `#pragma warning disable`, `[SuppressMessage]`, or any C#-specific suppression mechanism. This is a universal gap.

3. **No severity graduation:** 10 of 12 pipelines use a single severity level ("warning" or "info"). Only stringly_typed uses "info" and missing_cancellation_token uses "info", while all others use "warning" regardless of severity.

4. **No test file exclusion:** Zero pipelines exclude test files or test classes. All findings apply equally to test and production code.

5. **Thin test suites:** Average 4.3 tests per pipeline. No pipeline has more than 5 tests. No negative test for false positives on common C# patterns (LINQ, fluent APIs, DTOs, generated code, etc.).

### Graph Enhancement Summary

| Enhancement | Pipelines Benefiting |
|---|---|
| `modifiers: Vec<String>` on Symbol | static_global_state, sync_over_async, missing_cancellation_token, god_controller |
| `return_type: Option<String>` on Symbol | sync_over_async, null_reference_risk, missing_cancellation_token |
| `type_name: String` on Parameter | missing_cancellation_token, stringly_typed, disposable_not_disposed |
| `attributes: Vec<String>` on Symbol | all 12 pipelines (suppression), god_controller, missing_cancellation_token |
| `InheritsFrom` edge | god_class, god_controller, anemic_domain_model |
| `WritesTo` edge | static_global_state |
| `nullable_context` on File | null_reference_risk |
| `catch_type` on CFG Exception edge | exception_control_flow |
