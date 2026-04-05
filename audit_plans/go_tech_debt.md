# Go Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 10
- **Trait types used:** Pipeline (all 10 use the legacy `Pipeline` trait)
- **Overall assessment:** The Go tech debt pipelines are uniformly implemented using the legacy `Pipeline` trait with tree-sitter-only detection. None leverage the CodeGraph, CFGs, or taint analysis. Several have solid false-positive suppression (error_swallowing, god_struct, context_not_propagated), but the majority suffer from single-node detection without data-flow context, no suppression/annotation awareness, and missing severity graduation. The goroutine_leak pipeline uses fragile text-contains heuristics instead of AST analysis for its select/Done check. The concrete_return_type pipeline only matches single bare pointer returns, missing the far more common `(*T, error)` tuple pattern. The stringly_typed_config pipeline has no scope awareness for legitimate uses (e.g., HTTP headers, environment maps). All pipelines should migrate to GraphPipeline to exploit cross-file symbol resolution, call-graph traversal, and CFG-based flow analysis.

---

## error_swallowing

### Current Implementation
- **File:** `src/audit/pipelines/go/error_swallowing.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `error_swallowed`
- **Detection method:** Matches `short_var_declaration` and `assignment_statement` nodes where the LHS `expression_list` contains a blank identifier `_` and the RHS `expression_list` contains a `call_expression`. Skips nodes inside `defer_statement` and calls to safe cleanup methods (`Close`, `Flush`, `Remove`).

### Problems Identified
1. **High false positive rate (Rubric 2):** Flags `_, _ = someFunc()` even when both return values are intentionally discarded (e.g., writing to a known-good destination). There is no check for whether the `_` position corresponds to an error type specifically -- it flags any blank identifier in a multi-return call, including `val, _ := strconv.Atoi(knownGoodInput)` where the programmer has validated the input upstream.
2. **High false negative rate (Rubric 3):** Does not detect error swallowing via explicit `var err error; err = someFunc(); _ = err` -- only catches the blank identifier at the declaration site. Also misses the pattern `if err := someFunc(); err == nil { ... }` where the else branch is missing (error silently dropped).
3. **Missing context / No data flow tracking (Rubric 4, 10):** Uses tree-sitter only. The CodeGraph has FlowsTo edges and CFGs that could determine whether the error variable flows to any error-handling code path. Graph-based analysis could distinguish "error genuinely discarded" from "error handled via a different code path."
4. **No suppression/annotation awareness (Rubric 11):** No check for `//nolint:errcheck` or `// intentionally discarded` comments adjacent to the line.
5. **No severity graduation (Rubric 15):** All findings are "warning" regardless of what is being called. Discarding the error from `os.Open()` is far more dangerous than discarding from `fmt.Println()`.
6. **Language idiom ignorance (Rubric 13):** The safe cleanup list is hardcoded to only 3 methods (`Close`, `Flush`, `Remove`). Idiomatic Go commonly discards errors from `fmt.Fprintf(w, ...)`, `writer.Write(...)` in logging paths, and `conn.SetDeadline(...)`. These are not covered.
7. **Literal blindness (Rubric 8):** Does not check whether the RHS call is actually a function that returns an error. Many Go functions return `(T, bool)` -- e.g., type assertions, map lookups. While map lookups are filtered (no `call_expression`), type assertion results `v, _ := x.(Type)` would be flagged because the tree-sitter node for type assertion is different, but if wrapped in a function call it could still false-positive.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Short var decl with blank identifier, assignment with blank identifier, map access (no call_expression), single blank without call, clean code with proper error handling.
- **What's NOT tested:** Defer context skip, safe cleanup method skip (Close/Flush/Remove), multiple blank identifiers in one LHS, `//nolint` comment suppression, nested function calls, error swallowing via re-assignment pattern, method calls (as opposed to package-level selector calls).

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate `graph.file_nodes` filtered to `Language::Go` files. For each file, look up Symbol nodes via `DefinedIn` edges to find functions/methods.
- **Returns:** Set of Go file paths and their function symbol NodeIndexes.

#### Step 2: Narrowing
- **Tool:** Graph query + Tree-sitter
- **Why not graph-only:** Graph cannot express blank identifier patterns in assignment LHS -- this is a syntactic pattern requiring AST inspection. However, graph's `function_cfgs` can identify all `Call` statements and `Assignment` statements within each function.
- **Query:** For each function's CFG, find `CfgStatementKind::Assignment` where `target == "_"` and `CfgStatementKind::Call` on the same line. Cross-reference with tree-sitter to confirm the blank identifier is in an error position (last position in multi-return).
- **Returns:** List of (file_path, line, call_name, receiver) tuples where `_` absorbs a return value from a call.

#### Step 3: False Positive Removal
- **Tool:** Graph query + Tree-sitter
- **Query:** (a) Check `symbols_by_name` for the called function to determine if it is a known safe-cleanup or logging function. (b) Walk parent chain for `defer_statement`. (c) Check for `//nolint:errcheck` comment on the same or preceding line via tree-sitter comment query. (d) Graduate severity: "error" for I/O, DB, network calls; "warning" for general; "info" for logging/formatting calls.
- **Returns:** Filtered findings with graduated severity and suppression support.

#### Graph Enhancement Required
- **Missing:** Return type information on Symbol nodes (specifically: whether a function returns `error` as part of its signature).
- **Why needed:** To definitively determine whether the `_` in position N corresponds to an `error` return value vs. a `bool` or other type.
- **Proposed change:** Add `return_types: Vec<String>` field to `NodeWeight::Symbol` populated during graph building from tree-sitter `result` field of function declarations.

### New Test Cases
1. **test_nolint_comment_suppression** -- Input: `_, _ := someFunc() //nolint:errcheck` -> Expected: not flagged -- Covers: suppression/annotation awareness (Rubric 11)
2. **test_defer_cleanup_not_flagged** -- Input: `defer func() { _ = f.Close() }()` -> Expected: not flagged -- Covers: language idiom ignorance (Rubric 13)
3. **test_fmt_fprintf_not_flagged** -- Input: `_, _ = fmt.Fprintf(w, "msg")` -> Expected: not flagged (logging/formatting) -- Covers: severity graduation (Rubric 15)
4. **test_os_open_error_high_severity** -- Input: `f, _ := os.Open(path)` -> Expected: flagged as "error" severity -- Covers: severity graduation (Rubric 15)
5. **test_type_assertion_not_flagged** -- Input: `v, _ := x.(string)` -> Expected: not flagged (not a call_expression) -- Covers: literal blindness (Rubric 8)
6. **test_multiple_blanks** -- Input: `_, _, _ := multiReturn()` -> Expected: flagged once -- Covers: missing edge cases (Rubric 6)
7. **test_reassignment_swallow** -- Input: `err := someFunc(); _ = err` -> Expected: flagged -- Covers: high false negative rate (Rubric 3)

---

## god_struct

### Current Implementation
- **File:** `src/audit/pipelines/go/god_struct.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `large_struct` (>= 15 fields), `large_method_set` (>= 10 methods)
- **Detection method:** Uses `compile_struct_type_query` to find struct type declarations and counts `field_declaration` children. Uses `compile_method_decl_query` to find method declarations, extracts receiver type name, and accumulates method counts per struct. Skips structs with Config/Options/Settings/Params suffixes. Skips structs with DTO tags (`json:`, `yaml:`, etc.).

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric 12):** `FIELD_THRESHOLD = 15` and `METHOD_THRESHOLD = 10` are magic numbers. No documentation of why these specific values. Industry standards vary (e.g., some linters use 8 fields as the threshold).
2. **Missing context (Rubric 4):** Uses tree-sitter only. The CodeGraph has Symbol nodes with `kind == Struct` and method counts could be derived from call-graph edges. More importantly, graph traversal could determine if a struct's methods operate on different subsets of fields (indicating the struct should be split), vs. all methods using all fields (legitimate large struct).
3. **High false negative rate (Rubric 3):** Embedded structs are counted as a single `field_declaration` even though they pull in many fields. A struct with 5 direct fields but 3 embedded structs with 10 fields each would not be flagged despite having 35 effective fields.
4. **No scope awareness (Rubric 7):** Does not distinguish between production code and generated code (e.g., protobuf-generated structs, which are typically large DTOs). The DTO tag check partially covers this, but generated code without tags would still be flagged.
5. **No suppression/annotation awareness (Rubric 11):** No `//nolint:govet` or custom suppression comment support.
6. **Overlapping detection across pipelines (Rubric 16):** A struct with many fields AND many methods could be flagged by both `large_struct` and `large_method_set` on different lines, which is arguably correct but could also be consolidated into a single "god struct" finding.
7. **Single-node detection (Rubric 14):** Method count is per-file only. If a struct's methods are spread across multiple files (Go allows this), the per-file count would be lower than the actual total, leading to false negatives.
8. **No severity graduation (Rubric 15):** Both findings are "warning" regardless of how far above the threshold. A struct with 16 fields is very different from one with 50 fields.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Large struct detection (16 fields), small struct (5 fields, clean), large method set (11 methods), small method set (3 methods, clean).
- **What's NOT tested:** Config suffix skip, DTO tag skip, embedded struct counting, pointer receiver vs. value receiver, methods split across files, boundary values (exactly 15 fields, exactly 10 methods), structs with both patterns simultaneously.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate all Symbol nodes where `kind == Struct` and `language == Go`. Use `graph.symbols_by_name` to collect struct names, then use `traverse_callees` from the struct's symbol node to find all method nodes that have the struct as receiver (via `Contains` edge or `DefinedIn` cross-reference).
- **Returns:** Map of struct name -> (field_count from tree-sitter, total_method_count from graph across all files, file_paths).

#### Step 2: Narrowing
- **Tool:** Graph query + Tree-sitter
- **Why not graph-only:** Graph does not store field counts or embedded struct information -- these require AST inspection of the struct body.
- **Query:** For each struct above threshold, use tree-sitter to count fields (including recursive embedded struct field resolution). Check for DTO tags and config suffixes. Compute cross-file method totals from graph.
- **Returns:** Filtered list of structs above threshold with effective field count and cross-file method count.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph query
- **Query:** (a) Check if file path matches generated code patterns (`*.pb.go`, `*_gen.go`, `*_generated.go`). (b) Check for `//go:generate` directive in file. (c) Check for `//nolint` comment on struct declaration line. (d) Graduate severity: 15-25 fields = "info", 25-40 = "warning", 40+ = "error".
- **Returns:** Final findings with graduated severity.

#### Graph Enhancement Required
- **Missing:** `Contains` edges from struct Symbol nodes to their method Symbol nodes.
- **Why needed:** To count methods across files for a given struct without relying on receiver-type text parsing.
- **Proposed change:** During graph building, when processing a `method_declaration` with a receiver, add a `Contains` edge from the struct's Symbol node to the method's Symbol node.

### New Test Cases
1. **test_config_suffix_skip** -- Input: struct `AppConfig` with 20 fields -> Expected: not flagged -- Covers: language idiom (Rubric 13)
2. **test_dto_tag_skip** -- Input: struct with 20 fields all having `json:` tags -> Expected: not flagged -- Covers: language idiom (Rubric 13)
3. **test_boundary_14_fields** -- Input: struct with exactly 14 fields -> Expected: not flagged -- Covers: missing edge cases (Rubric 6)
4. **test_boundary_15_fields** -- Input: struct with exactly 15 fields -> Expected: flagged -- Covers: missing edge cases (Rubric 6)
5. **test_embedded_struct_expansion** -- Input: struct with 5 fields and 3 embedded structs each with 5 fields -> Expected: flagged (effective 20 fields) -- Covers: high false negative rate (Rubric 3)
6. **test_generated_file_skip** -- Input: file named `model.pb.go` with large struct -> Expected: not flagged -- Covers: no scope awareness (Rubric 7)
7. **test_severity_graduation** -- Input: struct with 50 fields -> Expected: flagged as "error" not "warning" -- Covers: no severity graduation (Rubric 15)
8. **test_pointer_receiver_counted** -- Input: methods with `(s *Svc)` receiver -> Expected: counted same as `(s Svc)` -- Covers: missing edge cases (Rubric 6)

---

## naked_interface

### Current Implementation
- **File:** `src/audit/pipelines/go/naked_interface.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `empty_interface_param`, `empty_interface_field`
- **Detection method:** Uses `compile_param_decl_query` and `compile_field_decl_query` to find parameter and field declarations. Checks if the type is `interface_type` with 0 named children (empty interface `interface{}`) or `type_identifier` with text `any`. Reports as "info" severity.

### Problems Identified
1. **High false positive rate (Rubric 2):** Flags legitimate uses of `interface{}`/`any` that are idiomatic in Go: (a) `json.Unmarshal` target variables, (b) template data parameters, (c) `context.Value` return types, (d) variadic functions like `fmt.Sprintf` wrappers, (e) generic container types before Go 1.18 generics. The pipeline has no context for whether the empty interface usage is justified.
2. **Missing context (Rubric 4):** Uses tree-sitter only. The CodeGraph could determine whether the parameter is used in type assertions downstream (indicating the developer knows concrete types flow through it), or whether the function is called with diverse types (legitimate polymorphism).
3. **No scope awareness (Rubric 7):** Does not distinguish test code, generated code, or framework-required signatures. Interface adapters and middleware often legitimately use `any`.
4. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support.
5. **Language idiom ignorance (Rubric 13):** In Go, `any` is the official alias for `interface{}` since Go 1.18 and is widely used in standard library functions. Flagging every occurrence creates enormous noise in codebases using generics with `any` constraint.
6. **High false negative rate (Rubric 3):** Does not detect `[]interface{}` (slice of empty interface), `map[string]interface{}` (very common JSON pattern), or `chan interface{}`. These are equally problematic from a type-safety perspective.
7. **No severity graduation (Rubric 15):** All findings are "info" regardless of context. An exported API function taking `any` is more problematic than an internal helper.
8. **Missing compound variants (Rubric 9):** Does not check for `*interface{}` (pointer to empty interface), though this is rare.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Empty interface param, `any` param, concrete interface (clean), empty interface field.
- **What's NOT tested:** `[]interface{}` slice, `map[string]interface{}`, variadic `...interface{}`, return types using empty interface, nested/pointer empty interface, test file skip, generated code skip, `//nolint` suppression.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate `graph.file_nodes` for Go files. For each file, find Symbol nodes (functions/methods) via `DefinedIn` edges.
- **Returns:** List of Go files with function/method symbols.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph cannot express AST-level type node inspection for parameters. The `NodeWeight::Symbol` does not store parameter type information. Tree-sitter is needed to inspect the type subtree of each parameter/field declaration.
- **Query:** Run `compile_param_decl_query` and `compile_field_decl_query`. For each match, recursively check if the type node contains `interface_type` with 0 children or `type_identifier == "any"`. This catches compound types like `[]any`, `map[string]any`, `*any`.
- **Returns:** List of (file_path, line, pattern, context) tuples.

#### Step 3: False Positive Removal
- **Tool:** Graph query + Tree-sitter
- **Query:** (a) Check if function is exported (uppercase name) -- graduate to "warning" for exported, keep "info" for unexported. (b) Skip if enclosing function name matches known framework patterns (e.g., `ServeHTTP`, `Handle`, `Middleware`). (c) Check for `//nolint` comment. (d) Skip test files and generated files. (e) Use graph `traverse_callers` to determine if the function is called from diverse sites with different concrete types (legitimate polymorphism = lower severity).
- **Returns:** Filtered findings with severity graduation.

#### Graph Enhancement Required
- **Missing:** Parameter type information on Symbol nodes or Parameter nodes.
- **Why needed:** To determine at the graph level whether a function accepts `any`/`interface{}` without re-parsing the AST.
- **Proposed change:** Add `type_annotation: Option<String>` field to `NodeWeight::Parameter`.

### New Test Cases
1. **test_slice_of_interface** -- Input: `func Process(items []interface{}) {}` -> Expected: flagged -- Covers: missing compound variants (Rubric 9)
2. **test_map_string_interface** -- Input: `func Process(data map[string]interface{}) {}` -> Expected: flagged -- Covers: missing compound variants (Rubric 9)
3. **test_variadic_interface** -- Input: `func Log(args ...interface{}) {}` -> Expected: flagged -- Covers: missing compound variants (Rubric 9)
4. **test_json_unmarshal_target_skip** -- Input: `var result interface{}; json.Unmarshal(data, &result)` -> Expected: lower severity or skip -- Covers: language idiom ignorance (Rubric 13)
5. **test_exported_function_higher_severity** -- Input: `func Process(v any) {}` -> Expected: "warning" severity -- Covers: no severity graduation (Rubric 15)
6. **test_test_file_skip** -- Input: file `handler_test.go` with `any` params -> Expected: not flagged -- Covers: no scope awareness (Rubric 7)
7. **test_nolint_suppression** -- Input: `func Process(v any) {} //nolint:naked_interface` -> Expected: not flagged -- Covers: suppression awareness (Rubric 11)

---

## context_not_propagated

### Current Implementation
- **File:** `src/audit/pipelines/go/context_not_propagated.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `context_background_in_func`, `context_todo_in_func`
- **Detection method:** Uses `compile_selector_call_query` to find `context.Background()` and `context.TODO()` calls. Skips calls in `main()`, `init()`, test functions, `_test.go` files, and `New*`/`Init*` constructor functions. Reports as "warning".

### Problems Identified
1. **High false positive rate (Rubric 2):** Flags `context.Background()` in package-level variable initialization (e.g., `var bgCtx = context.Background()` at module scope), which is a common and acceptable pattern. Also flags it in `func TestMain(m *testing.M)` helper functions that are not caught by the `Test*` prefix check since `TestMain` is a special case.
2. **Missing context / No data flow tracking (Rubric 4, 10):** Does not check whether the function actually has a `context.Context` parameter available to propagate. If a function takes no context parameter and creates `context.Background()`, that might be the correct behavior (it's the root of a context chain). The real anti-pattern is when a function RECEIVES a context but ignores it and creates a new one.
3. **No suppression/annotation awareness (Rubric 11):** No `//nolint` or `// TODO: propagate context` awareness.
4. **No severity graduation (Rubric 15):** Both `context.Background()` and `context.TODO()` get the same "warning" severity, but `context.TODO()` is arguably less severe because it explicitly signals intent to fix later.
5. **High false negative rate (Rubric 3):** Does not detect `context.WithCancel(context.Background())` where Background is nested inside a With* call -- the tree-sitter match should still work here, but the more important miss is functions that receive `ctx context.Context` but never pass it to downstream calls (the real context propagation failure).
6. **Single-node detection (Rubric 14):** Only checks the call site itself without examining the enclosing function's parameter list. A function that takes `ctx context.Context` and then calls `context.Background()` is FAR worse than a function that takes no context at all.
7. **Language idiom ignorance (Rubric 13):** Skipping `New*`/`Init*` is good, but misses other bootstrap patterns: `Setup*`, `Start*`, `Run*`, `Serve*` functions that are legitimate context roots.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Background in service function (flagged), TODO in service function (flagged), clean in main, clean in init.
- **What's NOT tested:** Test function skip (`TestFoo`), test file skip (`_test.go`), `New*` constructor skip, function that receives context but ignores it, nested context.Background inside WithCancel, package-level variable initialization, `//nolint` suppression, `Setup*`/`Start*` patterns.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate Symbol nodes where `kind == Function || kind == Method` in Go files. For each, check via tree-sitter if the function's parameter list contains a `context.Context` parameter.
- **Returns:** Two sets: (a) functions WITH context parameter, (b) functions WITHOUT context parameter.

#### Step 2: Narrowing
- **Tool:** Graph query + Tree-sitter
- **Why not graph-only:** Graph does not store parameter types; tree-sitter is needed to inspect parameter declarations for `context.Context` type and to find `context.Background()`/`context.TODO()` call sites.
- **Query:** For functions WITH a context parameter: flag any `context.Background()` or `context.TODO()` call as "error" severity (they already have a context they should use). For functions WITHOUT a context parameter: flag as "info" severity (they may be context roots). Additionally, use `traverse_callees` from each function to check if downstream calls accept context -- if so and the function doesn't take one, flag as "warning" (propagation break in the chain).
- **Returns:** Findings with graduated severity based on context availability.

#### Step 3: False Positive Removal
- **Tool:** Graph query + Tree-sitter
- **Query:** (a) Skip `main`, `init`, `Test*`, `Benchmark*`, `New*`, `Init*`, `Setup*`, `Start*`, `Run*`. (b) Skip `_test.go` and generated files. (c) Check for `//nolint` or `// TODO:` adjacent comments. (d) Skip package-level variable declarations (not inside a function body).
- **Returns:** Final filtered findings.

#### Graph Enhancement Required
- **Missing:** Parameter type information on `NodeWeight::Parameter` nodes.
- **Why needed:** To determine at graph level whether a function accepts `context.Context` without re-parsing the AST.
- **Proposed change:** Add `type_name: Option<String>` field to `NodeWeight::Parameter`.

### New Test Cases
1. **test_function_with_context_param_creating_background** -- Input: `func Handle(ctx context.Context) { bg := context.Background() }` -> Expected: flagged as "error" -- Covers: no data flow tracking (Rubric 10)
2. **test_function_without_context_param** -- Input: `func helper() { bg := context.Background() }` -> Expected: flagged as "info" -- Covers: severity graduation (Rubric 15)
3. **test_context_todo_lower_severity** -- Input: `func handle() { ctx := context.TODO() }` -> Expected: flagged as "info" -- Covers: severity graduation (Rubric 15)
4. **test_setup_function_skip** -- Input: `func SetupRoutes() { ctx := context.Background() }` -> Expected: not flagged -- Covers: language idiom ignorance (Rubric 13)
5. **test_package_level_skip** -- Input: `var bgCtx = context.Background()` (package scope) -> Expected: not flagged -- Covers: high false positive rate (Rubric 2)
6. **test_nested_background_in_withcancel** -- Input: `ctx, cancel := context.WithCancel(context.Background())` -> Expected: flagged -- Covers: high false negative rate (Rubric 3)
7. **test_test_function_skip** -- Input: `func TestFoo(t *testing.T) { ctx := context.Background() }` -> Expected: not flagged -- Covers: scope awareness (Rubric 7)

---

## init_abuse

### Current Implementation
- **File:** `src/audit/pipelines/go/init_abuse.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `init_side_effect`
- **Detection method:** Two-pass approach: (1) Find all `init()` function bodies via `compile_function_decl_query`. (2) Find all selector calls via `compile_selector_call_query` and check if they are inside an init body range AND match the `SUSPICIOUS_CALLS` list (sql.Open, http.Get, http.Post, http.ListenAndServe, os.Open, os.Create, log.Fatal, log.Fatalf, net.Listen, net.Dial).

### Problems Identified
1. **High false negative rate (Rubric 3):** The `SUSPICIOUS_CALLS` list is incomplete. Missing: `grpc.Dial`, `redis.NewClient`, `mongo.Connect`, `kafka.NewReader`, `prometheus.MustRegister`, `flag.Parse` (controversial but common), `os.Setenv`, `os.MkdirAll`. Also misses indirect side effects: calling a helper function that internally does I/O.
2. **Missing context (Rubric 4):** Uses tree-sitter only. The CodeGraph's call graph could trace calls FROM init() to determine transitive side effects -- if init() calls `setup()` which calls `sql.Open()`, that's equally problematic but not detected.
3. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support. Some init() side effects are intentional and well-documented.
4. **No severity graduation (Rubric 15):** `log.Fatal` in init (which exits the process) and `os.Open` in init (which may fail gracefully) get the same "warning" severity. `log.Fatal` should be "error" as it can crash the application silently.
5. **Single-node detection (Rubric 14):** Only checks direct `pkg.Method()` calls. Does not detect: (a) method calls on previously acquired objects (e.g., `db := getDB(); db.Ping()`), (b) function calls that are not selector expressions (e.g., bare function calls that wrap I/O).
6. **Language idiom ignorance (Rubric 13):** `log.Fatal` in init is actually a Go convention for fail-fast initialization (e.g., `log.Fatalf("failed to load config: %v", err)`). Some teams prefer this over returning errors from init. Could be "info" rather than "warning".
7. **Hardcoded thresholds without justification (Rubric 12):** The `SUSPICIOUS_CALLS` list is effectively a hardcoded threshold -- there is no way to configure which calls are considered suspicious.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** sql.Open in init (flagged), variable-only init (clean), sql.Open in regular function (not flagged), log.Fatal in init (flagged).
- **What's NOT tested:** Multiple suspicious calls in one init, http.Get/Post/ListenAndServe, os.Open/Create, net.Listen/Dial, indirect side effects through helper functions, multiple init() functions in one file (Go allows this), `//nolint` suppression, non-selector calls in init.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Find all Symbol nodes where `name == "init"` and `kind == Function` in Go files. These are the init functions to analyze.
- **Returns:** List of init function NodeIndexes and their file paths.

#### Step 2: Narrowing
- **Tool:** Graph query
- **Query:** For each init function, use `traverse_callees(seed, depth=3)` to find all direct and transitive callees. Check if any callee's name matches the expanded suspicious calls list, or if any callee has `ExternalSource` edges (indicating I/O, DB, network).
- **Returns:** List of (init_function, suspicious_callee, call_depth) tuples.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph query
- **Query:** (a) Check for `//nolint` comments on init function declaration or on the specific call line. (b) Graduate severity: `log.Fatal*` = "info" (Go convention), database/network opens = "warning", `http.ListenAndServe` = "error" (blocks forever). (c) For transitive calls (depth > 1), use "info" severity and include the call chain in the message.
- **Returns:** Filtered findings with severity and call chain context.

### New Test Cases
1. **test_http_get_in_init** -- Input: `func init() { http.Get("http://example.com") }` -> Expected: flagged -- Covers: missing edge cases (Rubric 6)
2. **test_multiple_init_functions** -- Input: Two `init()` functions in one file, each with side effects -> Expected: both flagged -- Covers: missing edge cases (Rubric 6)
3. **test_indirect_side_effect** -- Input: `func init() { setup() }` where `setup()` calls `sql.Open()` -> Expected: flagged (requires graph) -- Covers: missing context (Rubric 4)
4. **test_nolint_suppression** -- Input: `func init() { //nolint:init_abuse\n sql.Open(...) }` -> Expected: not flagged -- Covers: suppression awareness (Rubric 11)
5. **test_log_fatal_lower_severity** -- Input: `func init() { log.Fatalf("...") }` -> Expected: flagged as "info" -- Covers: severity graduation (Rubric 15)
6. **test_http_listen_and_serve_error** -- Input: `func init() { http.ListenAndServe(":8080", nil) }` -> Expected: flagged as "error" -- Covers: severity graduation (Rubric 15)
7. **test_os_setenv_in_init** -- Input: `func init() { os.Setenv("KEY", "val") }` -> Expected: flagged -- Covers: high false negative rate (Rubric 3)

---

## mutex_misuse

### Current Implementation
- **File:** `src/audit/pipelines/go/mutex_misuse.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `lock_without_defer_unlock`
- **Detection method:** Uses `compile_method_call_query` to find all method calls. Filters for `.Lock()` and `.RLock()` calls. Extracts the receiver (operand) text. Walks the enclosing function body to check if `.Unlock()` / `.RUnlock()` is called on the same receiver anywhere. Flags if no matching unlock is found.

### Problems Identified
1. **High false negative rate (Rubric 3):** The pipeline only flags when there is NO unlock at all. It does NOT flag the far more dangerous pattern: `Lock()` followed by code that can `return` or `panic` before reaching `Unlock()` -- the classic "Lock without defer Unlock" anti-pattern. If `Unlock()` exists but is not deferred, an early return skips it. The pipeline description says "not immediately followed by defer" but the implementation only checks for existence, not defer usage.
2. **Missing context / No data flow tracking (Rubric 4, 10):** The walk-based approach does textual matching on receiver names. If the mutex is accessed through different paths (e.g., `s.mu.Lock()` vs `mu.Lock()` via alias), the receiver text comparison would fail. CodeGraph's CFG could properly track lock/unlock pairs through control flow paths.
3. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support.
4. **No severity graduation (Rubric 15):** All findings are "warning". A Lock with no Unlock at all is "error" severity. A Lock with Unlock but not deferred is "warning". A Lock in a function that never returns (infinite loop) could be "info".
5. **Single-node detection (Rubric 14):** Only checks within a single function body. Cross-function patterns (Lock in one function, Unlock expected in caller) are not detected.
6. **Language idiom ignorance (Rubric 13):** Does not distinguish `sync.Mutex` from `sync.RWMutex` for severity purposes. Also, some codebases use wrapper types around mutexes (e.g., `type SafeMap struct { mu sync.Mutex; ... }`) where Lock/Unlock is called via the wrapper methods -- these are missed or cause false receiver matching.
7. **Missing compound variants (Rubric 9):** Does not detect `TryLock()` (Go 1.18+), which should have the same unlock requirement. Does not detect embedded mutex patterns where the struct itself is the receiver: `s.Lock()` instead of `s.mu.Lock()`.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Lock without any unlock (flagged), Lock with Unlock later (clean), Lock with defer Unlock (clean), Lock followed by wrong defer (flagged).
- **What's NOT tested:** RLock/RUnlock pair, Lock with Unlock but early return path between them, TryLock, different receiver aliasing, func_literal (closure) lock patterns, cross-function lock/unlock, embedded mutex (`s.Lock()` pattern), `//nolint` suppression.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Find all Symbol nodes for functions/methods in Go files. For each function, check the function's CFG (`graph.function_cfgs`) for `CfgStatementKind::Call` with name containing "Lock" or "RLock".
- **Returns:** List of function NodeIndexes that contain lock calls.

#### Step 2: Narrowing
- **Tool:** Graph query (CFG analysis)
- **Query:** For each function with a lock call, analyze the CFG paths: (a) From the Lock call's basic block, enumerate all paths to function exit. (b) Check if EVERY path passes through a matching Unlock call. (c) Check specifically if the Unlock is in a defer statement (via tree-sitter `defer_statement` ancestor check). Classify: "no unlock on any path" = error, "unlock exists but not all paths reach it" = warning, "unlock deferred" = clean.
- **Returns:** Findings with path analysis: which specific paths miss the unlock.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph query
- **Query:** (a) Check for `//nolint` comments. (b) Verify receiver identity by resolving through aliases (tree-sitter variable tracking within function scope). (c) Skip if Lock is inside a test helper function.
- **Returns:** Final findings with CFG path evidence.

#### Graph Enhancement Required
- **Missing:** `ResourceAcquire`/`ResourceRelease` edges for mutex Lock/Unlock operations are not classified as resource operations in the current graph builder.
- **Why needed:** The CFG already has `CfgStatementKind::ResourceAcquire` and `ResourceRelease` variants. If the Go CFG builder classifies Lock as Acquire and Unlock as Release, the `ResourceAnalyzer` could automatically check proper lock lifecycle.
- **Proposed change:** In `src/graph/cfg_languages/go.rs`, classify `Lock()`/`RLock()` as `ResourceAcquire { resource_type: "mutex" }` and `Unlock()`/`RUnlock()` as `ResourceRelease { resource_type: "mutex" }`.

### New Test Cases
1. **test_lock_with_early_return** -- Input: `mu.Lock(); if err { return }; mu.Unlock()` -> Expected: flagged ("warning", unlock exists but not all paths) -- Covers: high false negative rate (Rubric 3)
2. **test_rlock_without_runlock** -- Input: `mu.RLock(); doWork()` -> Expected: flagged -- Covers: missing edge cases (Rubric 6)
3. **test_trylock_without_unlock** -- Input: `if mu.TryLock() { doWork() }` -> Expected: flagged -- Covers: missing compound variants (Rubric 9)
4. **test_embedded_mutex_lock** -- Input: `type SafeMap struct { sync.Mutex }; func (m *SafeMap) Get() { m.Lock() }` -> Expected: flagged -- Covers: missing compound variants (Rubric 9)
5. **test_different_receiver_alias** -- Input: `m := &s.mu; m.Lock(); s.mu.Unlock()` -> Expected: flagged (receiver mismatch) -- Covers: no data flow tracking (Rubric 10)
6. **test_nolint_suppression** -- Input: `mu.Lock() //nolint:mutex_misuse` -> Expected: not flagged -- Covers: suppression awareness (Rubric 11)
7. **test_closure_lock** -- Input: `go func() { mu.Lock(); doWork() }()` -> Expected: flagged within closure scope -- Covers: missing edge cases (Rubric 6)
8. **test_no_unlock_error_severity** -- Input: `mu.Lock(); doWork()` (no unlock anywhere) -> Expected: flagged as "error" -- Covers: severity graduation (Rubric 15)

---

## goroutine_leak

### Current Implementation
- **File:** `src/audit/pipelines/go/goroutine_leak.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `goroutine_missing_done_channel`
- **Detection method:** Uses `compile_go_statement_query` to find `go` statements. Checks if the go expression is a `call_expression` wrapping a `func_literal` (inline goroutine). Checks if the func_literal's body contains a `for_statement`. If so, checks if any for loop contains a `select_statement` with `<-` (channel receive). Flags if the goroutine has a for loop but no select with channel receive.

### Problems Identified
1. **Broken detection / High false positive rate (Rubric 1, 2):** The `walk_for_select` function checks if a `select_statement` contains `<-` by doing TEXT-BASED checking (`text.contains("<-")`). This is extremely fragile -- it would match any `<-` in the select statement's text representation, including comments, string literals containing `<-`, or even unrelated channel operations. Should use AST-based detection of `communication_case` with `receive_statement` nodes.
2. **High false positive rate (Rubric 2):** Flags `for range ch` patterns where the goroutine reads from a channel that WILL be closed by the sender. The `for range ch` pattern is the idiomatic Go fan-out pattern and will naturally terminate when the channel is closed. Only `for {}` (infinite loop) or `for { select {} }` without Done truly leak.
3. **High false negative rate (Rubric 3):** Only detects inline `go func() { ... }()` goroutines. Does NOT detect: (a) `go namedFunction()` where the named function contains an infinite loop, (b) `go obj.Method()` with a leaky method, (c) goroutines that leak through blocking channel operations without a for loop (e.g., `go func() { ch <- value }()` where nothing ever reads from `ch`).
4. **Missing context (Rubric 4):** Uses tree-sitter only. The CodeGraph call graph could resolve `go namedFunction()` to the function's body and check for loops/select patterns in it. The CFG could determine if all paths through the goroutine eventually exit.
5. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support.
6. **No severity graduation (Rubric 15):** All findings are "warning". A goroutine with `for {}` and no exit condition is "error". A goroutine with `for range ch` is more likely "info" (will terminate when channel closes).
7. **No scope awareness (Rubric 7):** Does not skip test code where goroutine leaks in tests are often acceptable (test helper goroutines with cleanup via `t.Cleanup`).
8. **Language idiom ignorance (Rubric 13):** `for range ch` is THE idiomatic Go pattern for consuming from a channel. Flagging it as a potential leak is misleading. The real leak pattern is `for { ... }` without `select { case <-ctx.Done(): return }`.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Goroutine with for loop and no done (flagged), goroutine with select and ctx.Done (clean), goroutine without for loop (clean).
- **What's NOT tested:** `for range ch` pattern (should be clean), `go namedFunction()`, text-based `<-` false positive in comments/strings, goroutine with timer-based exit, goroutine in test code, `//nolint` suppression, `for` loop with `break` condition, channel send blocking.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query + Tree-sitter
- **Why not graph-only:** Graph does not represent `go` statements as distinct nodes. Tree-sitter is needed to find `go_statement` nodes.
- **Query:** Use tree-sitter `compile_go_statement_query` to find all `go` statements. For `go namedFunction()` calls, resolve to Symbol nodes via `graph.find_symbols_by_name(name)`.
- **Returns:** List of goroutine launch sites with either inline body or resolved function NodeIndex.

#### Step 2: Narrowing
- **Tool:** Graph query (CFG) + Tree-sitter
- **Query:** For each goroutine body (inline or resolved): (a) Check CFG for infinite loops (cycles with no exit edge). (b) Use tree-sitter to classify for-loop type: `for range ch` (channel-drain, safe), `for { select { case <-ctx.Done(): return } }` (properly guarded), `for { ... }` without exit (leak candidate). (c) For `go namedFunction()`, use `traverse_callees` to check if the named function has any for-loops in its body.
- **Returns:** Goroutine sites classified as: safe (channel drain, select with done), suspicious (infinite loop without done), dangerous (no exit path).

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph query
- **Query:** (a) Skip test files and test functions. (b) Skip goroutines with `time.After`/`time.Tick` in select (timeout-based exit). (c) Check for `//nolint` comments. (d) Graduate severity: `for range ch` = not flagged, `for` with `break`/`return` reachable = "info", `for { select {...} }` without Done = "warning", `for {}` with no exit = "error".
- **Returns:** Final findings with severity.

### New Test Cases
1. **test_for_range_channel_not_flagged** -- Input: `go func() { for item := range ch { process(item) } }()` -> Expected: not flagged -- Covers: language idiom ignorance (Rubric 13)
2. **test_go_named_function** -- Input: `go worker()` where `func worker() { for {} }` -> Expected: flagged -- Covers: high false negative rate (Rubric 3)
3. **test_text_based_arrow_false_positive** -- Input: select with comment containing `<-` but no actual channel receive -> Expected: correctly classified -- Covers: broken detection (Rubric 1)
4. **test_timer_based_exit** -- Input: `go func() { for { select { case <-time.After(5*time.Second): return } } }()` -> Expected: not flagged -- Covers: language idiom (Rubric 13)
5. **test_blocking_channel_send** -- Input: `go func() { ch <- value }()` where ch is unbuffered and nothing reads -> Expected: flagged (requires data flow) -- Covers: high false negative rate (Rubric 3)
6. **test_test_file_skip** -- Input: goroutine leak in `handler_test.go` -> Expected: not flagged -- Covers: scope awareness (Rubric 7)
7. **test_infinite_loop_error_severity** -- Input: `go func() { for { doWork() } }()` -> Expected: flagged as "error" -- Covers: severity graduation (Rubric 15)

---

## stringly_typed_config

### Current Implementation
- **File:** `src/audit/pipelines/go/stringly_typed_config.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `string_map_param`, `string_map_field`
- **Detection method:** Uses `compile_param_decl_query` and `compile_field_decl_query` to find parameter and field declarations. Checks if the type is `map_type` with both key and value being `string` (via `child_by_field_name("key")` and `child_by_field_name("value")`). Reports as "info" severity.

### Problems Identified
1. **High false positive rate (Rubric 2):** Flags every `map[string]string` regardless of context. Many legitimate uses: (a) HTTP headers (`http.Header` is `map[string][]string` but wrappers often use `map[string]string`), (b) environment variable maps, (c) string-to-string lookup tables (e.g., country code to name), (d) template data, (e) query parameters. The pipeline has no way to distinguish "this is configuration that should be a typed struct" from "this is genuinely a string-to-string mapping."
2. **Missing context (Rubric 4):** Uses tree-sitter only. The CodeGraph could determine how the map is used downstream -- if keys are accessed with string literals (e.g., `cfg["port"]`), that's a strong signal it should be a struct. If keys are dynamic, it's a legitimate map.
3. **No scope awareness (Rubric 7):** Does not skip test code, generated code, or framework-standard patterns.
4. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support.
5. **High false negative rate (Rubric 3):** Only detects `map[string]string`. Does not detect `map[string]interface{}` which is an even more common stringly-typed config pattern (used heavily with `viper`, `mapstructure`, etc.). Does not detect `map[string]any` (Go 1.18+).
6. **No severity graduation (Rubric 15):** All findings are "info". An exported function parameter `map[string]string` in a public API is more problematic than an internal helper field.
7. **Language idiom ignorance (Rubric 13):** `map[string]string` is idiomatic for labels, annotations (Kubernetes), tags, and metadata in Go. The pipeline should check if the enclosing struct or function name suggests configuration (e.g., `Config`, `Options`, `Settings`) before flagging.
8. **Overlapping detection across pipelines (Rubric 16):** A `map[string]string` field could also be flagged by `naked_interface` if it were `map[string]interface{}`, creating overlapping findings for similar issues.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** map[string]string param (flagged), typed config (clean), map[string]int (clean), map[string]string field (flagged).
- **What's NOT tested:** map[string]interface{} (should flag), map[string]any (should flag), HTTP header pattern (should not flag), test file skip, function named with "Labels"/"Tags"/"Metadata" (should not flag), return type map[string]string, `//nolint` suppression.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate Symbol nodes for functions/methods and structs in Go files.
- **Returns:** List of Go files with function/struct symbols.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not store field types or parameter types. Tree-sitter is required to inspect the type subtree of parameter/field declarations.
- **Query:** Run `compile_param_decl_query` and `compile_field_decl_query`. Check if type is `map_type` with string key and (string OR interface{} OR any) value. Also check for `map[string]map[string]string` nested patterns.
- **Returns:** List of findings with map type details and enclosing function/struct name.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph query
- **Query:** (a) Check enclosing function/struct name for legitimate map uses: `*Header*`, `*Label*`, `*Tag*`, `*Annotation*`, `*Metadata*`, `*Env*`. (b) Check if the map's keys are accessed with dynamic keys (variable lookups) vs. string literal keys -- string literal keys = config smell, dynamic keys = legitimate map. This requires walking the function body for `index_expression` nodes on the map variable. (c) Skip test files and generated code. (d) Check for `//nolint`. (e) Graduate severity: string literal key access pattern = "warning", no evidence of literal keys = "info".
- **Returns:** Filtered findings with context-aware severity.

### New Test Cases
1. **test_map_string_interface_flagged** -- Input: `func New(cfg map[string]interface{}) {}` -> Expected: flagged -- Covers: high false negative rate (Rubric 3)
2. **test_map_string_any_flagged** -- Input: `func New(cfg map[string]any) {}` -> Expected: flagged -- Covers: high false negative rate (Rubric 3)
3. **test_http_header_pattern_skip** -- Input: `func SetHeaders(headers map[string]string) {}` -> Expected: not flagged (name contains "Header") -- Covers: language idiom (Rubric 13)
4. **test_labels_pattern_skip** -- Input: `type Pod struct { Labels map[string]string }` -> Expected: not flagged -- Covers: language idiom (Rubric 13)
5. **test_test_file_skip** -- Input: map[string]string in `config_test.go` -> Expected: not flagged -- Covers: scope awareness (Rubric 7)
6. **test_nolint_suppression** -- Input: `Opts map[string]string //nolint:stringly_typed` -> Expected: not flagged -- Covers: suppression awareness (Rubric 11)
7. **test_exported_api_higher_severity** -- Input: exported function with map[string]string param -> Expected: "warning" not "info" -- Covers: severity graduation (Rubric 15)

---

## concrete_return_type

### Current Implementation
- **File:** `src/audit/pipelines/go/concrete_return_type.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `exported_concrete_pointer_return`
- **Detection method:** Custom tree-sitter query matching `function_declaration` with `result: (pointer_type (type_identifier))`. Only flags exported functions (uppercase first letter). Skips `New*` constructors. Reports as "info" severity.

### Problems Identified
1. **High false negative rate (Rubric 3):** The query ONLY matches functions with a single bare `*Type` return. It does NOT match the far more common Go pattern of `(*Type, error)` -- a `parameter_list` result with two types. This is the dominant return pattern in Go. The query also misses method declarations entirely (only matches `function_declaration`, not `method_declaration`).
2. **High false positive rate (Rubric 2):** In Go, the "accept interfaces, return structs" principle means returning concrete types is actually idiomatic. The pipeline flags a pattern that many Go style guides RECOMMEND. The anti-pattern is specifically returning concrete types from PACKAGE-LEVEL APIs that should be mockable, not all exported functions.
3. **Missing context (Rubric 4):** Uses tree-sitter only. The CodeGraph could check whether the returned type implements any interfaces in the codebase (if so, the function could return the interface instead). Without this check, there's no way to know if returning an interface would even be possible.
4. **No scope awareness (Rubric 7):** Does not distinguish internal packages from public API packages. Returning concrete types from `internal/` packages is perfectly fine.
5. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support.
6. **Language idiom ignorance (Rubric 13):** "Accept interfaces, return structs" is explicitly recommended by Go proverbs. This pipeline contradicts standard Go guidance in many cases. Should only flag when the returned type is from a different package or when the function is part of a factory pattern that should return an interface.
7. **Single-node detection (Rubric 14):** Only checks the function signature. Does not consider whether callers use the concrete type's methods directly (would break if changed to interface) or only use interface methods (safe to change).

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Exported function returning `*RedisCache` (flagged), `New*` constructor skip, interface return (clean), unexported function skip.
- **What's NOT tested:** Method declarations, tuple returns `(*Type, error)`, functions in `internal/` packages, functions returning non-pointer concrete types, return type that implements a known interface, `//nolint` suppression, functions returning standard library types (e.g., `*http.Server`).

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Find all Symbol nodes where `kind == Function` and `exported == true` in Go files. Cross-reference with file path to exclude `internal/` and `_test.go` paths.
- **Returns:** List of exported function symbols in non-internal Go files.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not store return type information. Tree-sitter is needed to inspect the function's `result` field for concrete pointer types.
- **Query:** For each exported function, parse the `result` node. Handle both single return (`pointer_type`) and tuple return (`parameter_list` containing `pointer_type`). Extract the concrete return type name. Skip `New*`, `Create*`, `Build*` factory functions.
- **Returns:** List of (function_name, return_type_name, file_path, line) tuples.

#### Step 3: False Positive Removal
- **Tool:** Graph query
- **Query:** (a) Check if the return type name corresponds to a struct that implements any interface in the codebase (search for interface declarations and compare method sets). If no compatible interface exists, the finding is irrelevant -- there's nothing to return instead. (b) Use `traverse_callers` to check how many callers use concrete-type-specific methods vs. interface-compatible methods. (c) Skip standard library return types. (d) Check for `//nolint`.
- **Returns:** Only flag functions where a compatible interface exists and callers don't use concrete-specific methods.

#### Graph Enhancement Required
- **Missing:** Interface-implementation relationship tracking (which structs implement which interfaces).
- **Why needed:** To determine whether an interface alternative exists for the concrete return type.
- **Proposed change:** Add `Implements` edge type from struct Symbol nodes to interface Symbol nodes. This requires method-set comparison during graph building.

### New Test Cases
1. **test_tuple_return_detected** -- Input: `func GetCache() (*RedisCache, error) { ... }` -> Expected: flagged (currently missed) -- Covers: high false negative rate (Rubric 3)
2. **test_method_declaration_detected** -- Input: `func (s *Server) GetDB() *Database { ... }` -> Expected: flagged (currently missed) -- Covers: high false negative rate (Rubric 3)
3. **test_internal_package_skip** -- Input: file path `internal/cache/redis.go` with concrete return -> Expected: not flagged -- Covers: scope awareness (Rubric 7)
4. **test_no_compatible_interface_skip** -- Input: function returns `*UniqueType` that implements no interface -> Expected: not flagged -- Covers: missing context (Rubric 4)
5. **test_create_factory_skip** -- Input: `func CreateCache() *RedisCache {}` -> Expected: not flagged (factory pattern) -- Covers: language idiom (Rubric 13)
6. **test_stdlib_return_skip** -- Input: `func GetServer() *http.Server {}` -> Expected: not flagged -- Covers: false positive rate (Rubric 2)
7. **test_nolint_suppression** -- Input: `func GetCache() *Cache {} //nolint:concrete_return` -> Expected: not flagged -- Covers: suppression awareness (Rubric 11)

---

## magic_numbers

### Current Implementation
- **File:** `src/audit/pipelines/go/magic_numbers.rs`
- **Trait type:** Pipeline
- **Patterns detected:** `magic_number`
- **Detection method:** Uses `compile_numeric_literal_query` to find all `int_literal` and `float_literal` nodes. Filters out values in `EXCLUDED_VALUES` (0, 1, 2, common powers of 2, hex constants) and `COMMON_ALLOWED_NUMBERS` (HTTP status codes, ports, timeouts). Skips numbers in exempt ancestor contexts (`const_declaration`, `const_spec`, `case_clause`, `expression_case`, `call_expression`). Skips index expressions. Skips test files. Reports as "info" severity.

### Problems Identified
1. **High false positive rate (Rubric 2):** Exempts `call_expression` ancestor, which means ANY number used as a function argument is skipped. This is too broad -- `setLimit(42)` is a magic number that should be flagged, while `fmt.Println(42)` is fine. The exemption should be for specific safe call targets (logging, formatting), not all calls.
2. **Hardcoded thresholds without justification (Rubric 12):** The `EXCLUDED_VALUES` list mixes different categories (small integers, powers of 2, hex masks) without clear rationale. `10`, `100`, `1000` are excluded, which hides legitimate magic numbers like `timeout = 100` (milliseconds? seconds?). `256` through `65536` are powers-of-2 that are excluded, but `131072` (2^17) is not -- the cutoff seems arbitrary.
3. **Missing compound variants (Rubric 9):** Does not detect magic numbers in: (a) negative literals (`-42` is a unary expression, not an `int_literal`), (b) computed constants (`timeout := 5 * time.Second` -- the `5` is a magic number), (c) bitwise operations (`flags & 0x1F` -- if `0x1F` is not in the exclusion list, it gets flagged).
4. **No data flow tracking (Rubric 10):** Does not check whether the number is assigned to a well-named variable. `maxRetries := 3` is arguably not a magic number because the variable name provides context. Only truly inline magic numbers (e.g., `if count > 42`) should be flagged.
5. **No suppression/annotation awareness (Rubric 11):** No `//nolint` support.
6. **No severity graduation (Rubric 15):** All findings are "info". A magic number in a comparison (`if x > 42`) is more problematic than one in an assignment with a descriptive variable name.
7. **Language idiom ignorance (Rubric 13):** Go's `iota` constants in const blocks are exempt (via `const_declaration` ancestor check), which is good. But the `call_expression` blanket exemption is overly broad for Go idioms -- `make([]byte, 4096)` should not be flagged (and isn't, since 4096 is in excluded values), but `make([]byte, 8000)` would be flagged even though it's a buffer size in a call.
8. **Literal blindness (Rubric 8):** `0xFF` is in the exclusion list but `255` is not (they are the same value). Similarly, `0x80` is excluded but `128` is in `COMMON_ALLOWED_NUMBERS`. The exclusion should normalize values, not match on text.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Magic number 42 (flagged), const context (clean), common values 0/1/2 (clean), index expression (clean), float magic number 3.14159 (flagged).
- **What's NOT tested:** Call expression exemption, HTTP status codes in COMMON_ALLOWED_NUMBERS, test file skip, negative numbers, hex values not in exclusion list, `//nolint` suppression, assignment to well-named variable, numbers in struct literal initialization, `var_declaration` context.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate `graph.file_nodes` for Go files. Skip files matching test patterns (`_test.go`, `testdata/`) and generated patterns (`*.pb.go`, `*_gen.go`).
- **Returns:** List of non-test, non-generated Go file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not represent individual numeric literals. Tree-sitter is the right tool for finding literal nodes in the AST.
- **Query:** Find all `int_literal` and `float_literal` nodes. Apply value normalization (convert hex to decimal for comparison). Check exempt contexts: (a) `const_declaration`/`const_spec` ancestor, (b) `case_clause` ancestor, (c) index expression. Remove the blanket `call_expression` exemption. Instead, check if the call target is a known-safe function (make, len, cap, append, fmt.*, log.*).
- **Returns:** Raw findings with value, context, and enclosing expression.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** (a) Check if the number is assigned to a descriptively-named variable (heuristic: variable name is not single letter and is > 3 chars). If so, reduce to not flagged. (b) Check for `//nolint:mnd` (magic number detector) or `//nolint:magic_numbers` comments. (c) Handle negative literals by checking parent `unary_expression` with `-` operator. (d) Graduate severity: comparison context = "warning", arithmetic context = "info", standalone assignment = "info".
- **Returns:** Filtered findings.

### New Test Cases
1. **test_call_expression_still_flagged** -- Input: `setLimit(42)` -> Expected: flagged (42 is magic even in a call) -- Covers: high false positive rate (Rubric 2)
2. **test_make_call_not_flagged** -- Input: `make([]byte, 4096)` -> Expected: not flagged -- Covers: language idiom (Rubric 13)
3. **test_negative_number** -- Input: `x := -42 + y` -> Expected: flagged (42 is magic) -- Covers: missing compound variants (Rubric 9)
4. **test_hex_decimal_equivalence** -- Input: `x := 255 + y` -> Expected: same treatment as `0xFF` -- Covers: literal blindness (Rubric 8)
5. **test_well_named_variable_skip** -- Input: `maxRetries := 3` -> Expected: not flagged (descriptive name) -- Covers: data flow tracking (Rubric 10)
6. **test_comparison_magic_number** -- Input: `if count > 42 { ... }` -> Expected: flagged as "warning" -- Covers: severity graduation (Rubric 15)
7. **test_nolint_suppression** -- Input: `x := 42 //nolint:mnd` -> Expected: not flagged -- Covers: suppression awareness (Rubric 11)
8. **test_generated_file_skip** -- Input: magic number in `types.pb.go` -> Expected: not flagged -- Covers: scope awareness (Rubric 7)

---

## Pipeline Cross-Cutting Concerns

### Shared Issues Across All 10 Pipelines

1. **All use legacy `Pipeline` trait:** None have been migrated to `GraphPipeline` or `NodePipeline`. This means none can access the CodeGraph, CFGs, taint analysis, or cross-file symbol resolution that the graph provides.

2. **No suppression/annotation awareness:** Zero pipelines check for `//nolint`, `//go:generate`, or any comment-based suppression mechanism. This is table-stakes for a Go linter (all major Go linters support `//nolint` directives).

3. **No severity graduation:** 8 of 10 pipelines use a fixed severity for all findings. Only `naked_interface` and `stringly_typed_config` use "info", while the others use "warning". None graduate based on risk level.

4. **No scope awareness for generated code:** Only `magic_numbers` skips test files. None skip generated code (`*.pb.go`, `*_gen.go`, `*_generated.go`, files with `// Code generated` header).

5. **All are per-file only:** None leverage cross-file analysis. God struct method counts miss methods in other files. Mutex misuse cannot track lock/unlock across function boundaries. Context propagation cannot trace the context chain across callers.

### Overlapping Detection Matrix

| Finding | error_swallowing | init_abuse | Notes |
|---------|-----------------|------------|-------|
| `_, _ := sql.Open()` in init() | Flagged | Flagged | Same line, different patterns |

| Finding | naked_interface | stringly_typed_config | Notes |
|---------|----------------|----------------------|-------|
| `map[string]interface{}` field | Would flag if extended | Would flag if extended | Similar stringly-typed concern |

No other significant overlaps identified between current pipeline implementations.

### Priority Order for Migration

1. **mutex_misuse** -- Highest impact. CFG-based path analysis would eliminate the critical false negative (lock without defer unlock when early return exists).
2. **goroutine_leak** -- High impact. Replacing text-based `<-` detection with AST analysis and adding call-graph resolution for named goroutines.
3. **context_not_propagated** -- High impact. Graph parameter analysis would enable the critical check: "function receives context but creates new one."
4. **error_swallowing** -- Medium impact. Graph return-type information would eliminate false positives on non-error blanks.
5. **init_abuse** -- Medium impact. Transitive call-graph analysis would catch indirect side effects.
6. **god_struct** -- Medium impact. Cross-file method counting via graph Contains edges.
7. **concrete_return_type** -- Medium impact. Interface-implementation checking via graph.
8. **magic_numbers** -- Lower impact. Primarily needs better exclusion logic, not graph.
9. **naked_interface** -- Lower impact. Needs better false-positive suppression and compound variant detection.
10. **stringly_typed_config** -- Lowest impact. Needs contextual awareness more than graph features.
