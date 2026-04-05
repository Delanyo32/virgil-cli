# C Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 12
- **Trait types used:** All 12 use the legacy `Pipeline` trait (no `GraphPipeline` or `NodePipeline`)
- **Overall assessment:** The 12 C tech debt pipelines cover meaningful C-specific anti-patterns but universally suffer from **single-node / single-function detection** (rubric #14), operating entirely via tree-sitter queries on individual AST nodes without any graph awareness. Cross-function data flow, resource lifecycle tracking via CFGs, and call graph traversal are completely absent despite the CodeGraph infrastructure being available. Several pipelines have high false positive rates due to coarse heuristics (substring matching on identifier names, blanket flagging of all function-like macros), and multiple pipelines have high false negative rates due to narrow detection windows (e.g., only checking 3 siblings for null checks, missing `realloc` in leak detection, not tracking pointer aliasing). Test coverage is minimal (2-4 tests per pipeline) and misses important edge cases.

---

## buffer_overflows

### Current Implementation
- **File:** `src/audit/pipelines/c/buffer_overflows.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `unsafe_string_function`
- **Detection method:** Uses `compile_call_expression_query()` to find all `call_expression` nodes, then checks if the function name (`identifier` child) matches a hardcoded list: `strcpy`, `strcat`, `sprintf`, `vsprintf`, `gets`, `scanf`. Every match is flagged as severity `error`.

### Problems Identified
1. **[High false negative rate (#3)]:** The unsafe function list is incomplete. Missing: `wcscpy`, `wcscat`, `swprintf`, `_tcscpy`, `lstrcpy`, `lstrcpyA`, `lstrcpyW` (Windows), `stpcpy`, `stpncpy` (less common but equally dangerous). Also missing `sscanf` which has the same format-string vulnerability as `scanf`.
2. **[No severity graduation (#15)]:** All findings are `error` regardless of context. `gets()` is unconditionally dangerous (removed from C11), but `strcpy` with a provably bounded source is merely a style issue. No graduation between `gets` (always error) vs `sprintf` (warning if source is bounded).
3. **[Missing context (#4)]:** The pipeline does not report what the safe alternative is per function. The message generically says "e.g. strncpy, snprintf" but `gets` has no bounded version -- the alternative is `fgets`. `scanf` should suggest `scanf_s` or `fgets` + `sscanf`.
4. **[Single-node detection (#14)]:** Does not check if the call is inside a bounds-checked wrapper function or behind a size assertion. A function that calls `strcpy` after `assert(strlen(src) < sizeof(dest))` is less dangerous than a bare `strcpy`.
5. **[No suppression/annotation awareness (#11)]:** No mechanism to suppress findings via comments like `/* NOLINT */` or `// NOSONAR`.
6. **[Language idiom ignorance (#13)]:** Does not detect calls through function pointers or macro-wrapped calls like `STRCPY(dest, src)` which expand to `strcpy`.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `strcpy` detection, `sprintf` detection, `gets` detection, safe alternatives not flagged (`strncpy`, `snprintf`, `memcpy`)
- **What's NOT tested:** `strcat`, `vsprintf`, `scanf` detection; calls via member access (e.g., `lib.strcpy`); calls inside macros; calls inside wrapper functions; multiple unsafe calls in one function; calls in header files; calls inside `#ifdef` blocks.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_nodes` filtered to `Language::C` files. For each file node, traverse `Contains` edges to find `CallSite` nodes where `name` matches the expanded unsafe function set.
- **Returns:** List of `(file_path, call_site_node_index, function_name)` tuples.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (CodeGraph `CallSite` nodes lack argument details)
- **Why not graph:** Graph `CallSite` stores only name/file/line; we need argument AST to check if bounds are being passed.
- **Query:** For each flagged call site, parse the file and locate the `call_expression` node at the reported line. Extract the argument list to determine argument count and types.
- **Returns:** `(call_site, function_name, arg_count, arg_texts)` tuples.

#### Step 3: False Positive Removal
- **Tool:** CodeGraph CFG traversal + tree-sitter
- **Query:** For each call site, find the enclosing function via `graph.symbol_nodes`. Look up the function's CFG in `graph.function_cfgs`. Walk the CFG basic blocks preceding the call site to check for `Guard` statements containing bounds checks (`sizeof`, `strlen`, `assert`). If a preceding guard constrains the source buffer size, downgrade severity from `error` to `info`.
- **Returns:** Final filtered findings with graduated severity.

#### Graph Enhancement Required
- **CallSite argument capture:** Currently `CallSite` nodes store only `name`, `file_path`, `line`. Adding argument text or argument count would allow Step 2 to be pure graph.

### New Test Cases
1. **test_detects_all_unsafe_functions** -- Input: one call per unsafe function (strcpy, strcat, sprintf, vsprintf, gets, scanf, wcscpy, sscanf) -> Expected: 8 findings -- Covers: #3 false negatives
2. **test_gets_always_error** -- Input: `gets(buf)` with preceding size check -> Expected: severity `error` (no safe alternative exists) -- Covers: #15 severity graduation
3. **test_strcpy_after_bounds_check** -- Input: `assert(strlen(src) < sizeof(dest)); strcpy(dest, src);` -> Expected: finding with severity `info` not `error` -- Covers: #14 single-node detection
4. **test_macro_wrapped_call** -- Input: `#define COPY(d,s) strcpy(d,s)\nCOPY(dest, src);` -> Expected: 1 finding on the macro expansion -- Covers: #13 language idiom
5. **test_suppression_comment** -- Input: `strcpy(dest, src); /* NOLINT */` -> Expected: 0 findings -- Covers: #11 suppression
6. **test_multiple_unsafe_in_one_function** -- Input: function with `strcpy` + `sprintf` -> Expected: 2 findings -- Covers: #6 missing edge cases

---

## unchecked_malloc

### Current Implementation
- **File:** `src/audit/pipelines/c/unchecked_malloc.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `unchecked_allocation`
- **Detection method:** Uses `compile_function_definition_query()` to iterate function bodies. Within each body, recursively walks the AST looking for `declaration` nodes with `init_declarator` whose value is a call to `malloc`/`calloc`/`realloc`, and `expression_statement` nodes with `assignment_expression` whose right side is an alloc call. Handles `cast_expression` wrapping (e.g., `(int *)malloc(...)`). Then checks the next 3 named siblings of the allocation node for an `if_statement` whose condition contains the variable name and either `NULL`, `null`, or `!`.

### Problems Identified
1. **[High false negative rate (#3)]:** The null-check detection (`has_null_check_after`) only examines 3 sibling nodes after the allocation. If there is an intervening assignment or function call between the allocation and the check, the check is missed. Example: `int *p = malloc(n); init_defaults(p); if (!p) return;` -- the `if` is the 2nd sibling but `init_defaults(p)` dereferences `p` before the check, which is the real bug (use-before-check), yet the pipeline would report "no null check" instead of "use before null check".
2. **[Single-node detection (#14)]:** Does not use CFG to trace control flow paths. A null check in a different branch (e.g., inside a helper function called immediately after allocation) is invisible.
3. **[Missing compound variants (#9)]:** Does not detect `aligned_alloc`, `posix_memalign`, `mmap`, `valloc`, or `pvalloc` -- all allocate memory that can return NULL/MAP_FAILED.
4. **[Literal blindness (#8)]:** The null-check detection uses substring matching (`cond_text.contains(var_name)`) which could false-match on `if (p2 == NULL)` when checking for variable `p`. If a variable `p` exists and the condition is `if (pp == NULL)`, this is a false negative (pp contains "p").
5. **[No data flow tracking (#10)]:** Does not track pointer aliasing. If `int *p = malloc(n); int *q = p; if (!q) return;`, the pipeline reports `p` as unchecked because it only looks for the original variable name.
6. **[No scope awareness (#7)]:** Does not distinguish between early-return patterns (correct: `if (!p) return;`) and deferred-check patterns (incorrect: check is after first use).

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Unchecked malloc detection, `!p` null check skipped, `p == NULL` null check skipped.
- **What's NOT tested:** `calloc` detection, `realloc` detection, cast-wrapped allocations like `(int*)malloc(...)`, allocation via assignment (not declaration), null check more than 3 siblings away, pointer aliasing before check, check inside nested block, allocation in a loop, multiple allocations in one function (only first tested).

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** For each C file, iterate all `CallSite` nodes. Filter where `name` is in `{malloc, calloc, realloc, aligned_alloc, posix_memalign, mmap, valloc}`. Collect the enclosing function's `NodeIndex` via the `DefinedIn` edge from the call site.
- **Returns:** List of `(function_node, call_site_node, alloc_function_name, file_path, line)`.

#### Step 2: Narrowing
- **Tool:** CodeGraph CFG
- **Why not tree-sitter:** The CFG provides structured control flow path analysis which is the whole point.
- **Query:** For each function with an allocation call site, look up `graph.function_cfgs[function_node]`. Find the basic block containing the allocation (match by line number). Walk all forward-reachable paths from that block. On each path, check for a `Guard` statement that references the allocated variable before any `Call` or `Assignment` that dereferences it.
- **Returns:** `(call_site, alloc_var, has_check_on_all_paths: bool, first_use_before_check: Option<line>)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Why not graph:** Need source text to check for `__attribute__((malloc))` or custom allocator annotations.
- **Query:** Parse the function body at the call site line. Check if the return value is immediately used in a ternary or short-circuit expression (e.g., `int *p = malloc(n) ?: abort()`). Check if the function is annotated with `__attribute__((returns_nonnull))`.
- **Returns:** Filtered findings with accurate severity (`error` if no check on any path, `warning` if check exists but use-before-check detected).

#### Graph Enhancement Required
- **Variable assignment tracking in CFG:** The CFG's `Assignment` statements need to capture the right-hand-side call name so that `target = malloc(...)` can be identified as a resource acquisition without re-parsing.

### New Test Cases
1. **test_detects_calloc** -- Input: `int *p = calloc(10, sizeof(int)); p[0] = 1;` -> Expected: 1 finding -- Covers: #9 compound variants
2. **test_detects_realloc** -- Input: `p = realloc(p, new_size); p[0] = 1;` -> Expected: 1 finding -- Covers: #9
3. **test_cast_wrapped_malloc** -- Input: `int *p = (int *)malloc(10 * sizeof(int)); p[0] = 1;` -> Expected: 1 finding -- Covers: #6 missing edge cases
4. **test_null_check_far_away** -- Input: allocation followed by 5 statements then `if (!p) return;` -> Expected: 0 findings (currently would be 1 due to 3-sibling limit) -- Covers: #3 false negatives
5. **test_aliased_pointer_check** -- Input: `int *p = malloc(n); int *q = p; if (!q) return;` -> Expected: 0 findings -- Covers: #10 data flow
6. **test_use_before_check** -- Input: `int *p = malloc(n); p[0] = 1; if (!p) return;` -> Expected: finding with note about use-before-check -- Covers: #7 scope awareness
7. **test_substring_false_match** -- Input: variables `p` and `pp`, check is `if (!pp)` -> Expected: `p` still flagged as unchecked -- Covers: #8 literal blindness

---

## memory_leaks

### Current Implementation
- **File:** `src/audit/pipelines/c/memory_leaks.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `potential_memory_leak`
- **Detection method:** Walks each function body to find `malloc`/`calloc` allocation declarations. Simultaneously scans for any `free()` call anywhere in the function and records returned variable names. If the function has allocations but no `free()` call at all, and the allocated variable is not returned, it flags a leak. Severity: `warning`.

### Problems Identified
1. **[High false positive rate (#2)]:** The pipeline checks for the *existence* of any `free()` call in the function. If a function has `int *p = malloc(n); int *q = malloc(n); free(q);`, `p` is NOT flagged because `has_free` is true for the entire function. This is a **broken detection** that causes false negatives: the single `free(q)` silences the leak on `p`.
2. **[Broken detection (#1)]:** The `has_free` flag is a single boolean for the entire function body (line 120: `if has_free { continue; }`). This means one `free()` call for ANY variable suppresses leak detection for ALL allocations in that function. This is fundamentally broken.
3. **[Missing compound variants (#9)]:** Only checks `malloc` and `calloc`. Missing: `realloc`, `strdup`, `strndup`, `asprintf`, `aligned_alloc`, `mmap`, `fopen` (file handle leaks). Note: the `UncheckedMallocPipeline` includes `realloc` but this one does not.
4. **[No data flow tracking (#10)]:** Does not track if the pointer is stored in a struct field (e.g., `ctx->buf = malloc(n)` -- not a leak if the struct outlives the function). Does not track if the pointer is passed to another function that takes ownership.
5. **[Single-node detection (#14)]:** Does not use CFGs. A function with multiple return paths where some paths free and others don't (the most common real leak pattern) is invisible because the pipeline checks the entire function body as a flat scan.
6. **[No suppression/annotation awareness (#11)]:** No way to suppress findings.
7. **[Language idiom ignorance (#13)]:** Does not recognize common C patterns: `goto cleanup` idiom, `__attribute__((cleanup))`, ownership transfer via return in struct field, reference counting patterns.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Missing free detected, free present skipped, returned pointer skipped.
- **What's NOT tested:** Multiple allocations where only one is freed (exposes the broken `has_free` logic), `calloc` detection, pointer stored in struct, pointer passed to ownership-taking function, `goto cleanup` pattern, allocation in a loop, conditional free (free on one branch but not another), `realloc` (not in alloc list).

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph resource lifecycle edges
- **Why not higher-ranked tool:** Graph is highest-ranked. The graph already computes `Acquires` and `ReleasedBy` edges via the `ResourceAnalyzer` module.
- **Query:** Iterate all function symbol nodes. For each, check outgoing `Acquires` edges with `resource_type == "memory"`. For each acquire edge, check if there is a corresponding `ReleasedBy` edge on all CFG exit paths.
- **Returns:** List of `(function_node, acquire_target, released_on_all_paths: bool)`.

#### Step 2: Narrowing
- **Tool:** CodeGraph CFG path analysis
- **Why not tree-sitter:** CFG provides structured path analysis that handles `goto cleanup`, early returns, and conditional frees.
- **Query:** For each function with an unreleased allocation, walk the CFG from the allocation's basic block. For each exit path, check whether a `ResourceRelease` statement targeting the same variable (or an alias) appears. Collect the specific exit paths that lack a release.
- **Returns:** `(function_node, var_name, leaking_exit_paths: Vec<line>)`.

#### Step 3: False Positive Removal
- **Tool:** CodeGraph call graph + tree-sitter
- **Why not graph alone:** Need to check if pointer is passed to an ownership-transfer function not captured in graph.
- **Query:** Check if the allocated pointer is: (a) stored in a struct field that outlives the function (via `FlowsTo` edges), (b) passed as argument to a function whose name suggests ownership transfer (e.g., `*_take`, `*_own`, `list_append`), (c) returned indirectly via output parameter.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Per-variable resource tracking:** The `ResourceAnalyzer` currently creates `Acquires`/`ReleasedBy` edges at the function level. Per-variable tracking (which specific allocated variable is released) would eliminate the broken single-boolean-for-all-allocations problem.

### New Test Cases
1. **test_multiple_allocs_one_freed** -- Input: `int *p = malloc(n); int *q = malloc(n); free(q);` -> Expected: 1 finding for `p` (currently 0 due to broken has_free) -- Covers: #1 broken detection
2. **test_realloc_not_freed** -- Input: `p = realloc(p, new_size);` with no free -> Expected: 1 finding -- Covers: #9 compound variants
3. **test_strdup_not_freed** -- Input: `char *s = strdup(input);` with no free -> Expected: 1 finding -- Covers: #9
4. **test_goto_cleanup_pattern** -- Input: function with `goto cleanup;` ... `cleanup: free(p); return;` -> Expected: 0 findings -- Covers: #13 language idiom
5. **test_conditional_free** -- Input: `if (err) return; free(p);` -> Expected: 1 finding (leak on error path) -- Covers: #14 single-node
6. **test_pointer_stored_in_struct** -- Input: `ctx->buf = malloc(n);` -> Expected: 0 findings -- Covers: #10 data flow
7. **test_ownership_transfer_via_function** -- Input: `list_append(list, malloc(n));` -> Expected: 0 findings (ownership transferred) -- Covers: #10

---

## signed_unsigned_mismatch

### Current Implementation
- **File:** `src/audit/pipelines/c/signed_unsigned_mismatch.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `signed_unsigned_mismatch`
- **Detection method:** Uses `compile_for_statement_query()` to find `for` loops. Checks if the initializer declares a variable of type `int` (exact match). Then checks if the condition text contains any substring from `SIZE_LIKE_IDENTIFIERS` (`size`, `len`, `length`, `count`, `num`, `sz`) or `SIZE_FUNCTIONS` (`strlen`, `sizeof`, `wcslen`). Severity: `warning`.

### Problems Identified
1. **[High false positive rate (#2)]:** Substring matching on identifier names is extremely coarse. `for (int i = 0; i < recount; i++)` flags because "recount" contains "count". `for (int i = 0; i < intensity; i++)` flags because "intensity" contains "len". Any variable whose name happens to contain "size", "len", "count", "num", or "sz" triggers a false positive. (Lines 41-44: `if cond_text.contains(ident)`)
2. **[High false negative rate (#3)]:** Only detects `int` as the type (exact match, line 31: `type_text == "int"`). Does not flag `short`, `long`, `long long`, `char`, or any signed type that is not literally `int`. Also misses `signed int`, `signed`, etc.
3. **[Literal blindness (#8)]:** Does not check if the size variable is actually unsigned. `for (int i = 0; i < size; i++)` is flagged even if `size` was declared as `int size = 10;` (signed), where there is no mismatch at all.
4. **[No scope awareness (#7)]:** Does not look at the type of the comparison operand. A proper check would verify that the right-hand side of the comparison is actually an unsigned type or a function returning `size_t`.
5. **[Missing compound variants (#9)]:** Only checks `for` loops. `while` loops with similar patterns (`while (i < len)`) are not checked. Comparisons outside loops (e.g., `if (index < size)`) are also missed.
6. **[No data flow tracking (#10)]:** Does not trace the type of the comparison target through assignments. If `size_t n = strlen(s); int limit = n;`, the `int limit` conversion is the real problem, not just the loop comparison.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `int` vs `strlen()` detected, `int` vs variable named `size` detected, `size_t` counter skipped, `int` vs literal `10` skipped.
- **What's NOT tested:** False positive on variable containing "len" as substring (e.g., `length_flag`), `short`/`long`/`long long` counter types, signed size variable, `while` loop patterns, comparison outside loop, `unsigned int` counter (should skip), `for` loop without init declaration.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter (Graph does not track variable types)
- **Why not graph:** CodeGraph `Symbol` nodes do not store type information for local variables. Type analysis requires AST inspection.
- **Query:** Find all `for_statement` and `while_statement` nodes. For `for_statement`, extract the initializer declaration's type specifier. For conditions, extract the comparison operator's operands.
- **Returns:** List of `(loop_node, counter_type, comparison_operands)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Type resolution of local variables requires AST walking.
- **Query:** For each loop, check: (a) counter type is a signed integer type (not just `int` -- include `short`, `long`, `long long`, `signed`, `char`), (b) the comparison operand is a function call returning `size_t` (check against known functions: `strlen`, `sizeof`, `wcslen`, `strnlen`, `fread`, `fwrite`, array `.length` patterns) OR is a variable declared with `size_t`/`unsigned` type.
- **Returns:** Findings where both conditions are confirmed.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Check if the counter variable is cast to `size_t` before comparison, or if the comparison is inside a bounds-checked macro. Check if the size variable is actually a signed type (trace its declaration).
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Local variable type tracking:** Adding type information to `Symbol` nodes or a separate type map would allow graph-based type mismatch detection without re-parsing.

### New Test Cases
1. **test_false_positive_substring** -- Input: `for (int i = 0; i < recount; i++) {}` where `recount` is an `int` -> Expected: 0 findings (no actual unsigned type) -- Covers: #2 false positive
2. **test_long_counter** -- Input: `for (long i = 0; i < strlen(s); i++) {}` -> Expected: 1 finding -- Covers: #3 false negative
3. **test_signed_size_variable** -- Input: `int size = 10; for (int i = 0; i < size; i++) {}` -> Expected: 0 findings (both signed) -- Covers: #8 literal blindness
4. **test_while_loop** -- Input: `int i = 0; while (i < strlen(s)) { i++; }` -> Expected: 1 finding -- Covers: #9 compound variants
5. **test_unsigned_counter** -- Input: `for (unsigned int i = 0; i < strlen(s); i++) {}` -> Expected: 0 findings -- Covers: #6 missing edge cases
6. **test_size_t_variable_not_function** -- Input: `size_t len = 10; for (int i = 0; i < len; i++) {}` -> Expected: 1 finding (len is size_t) -- Covers: #7 scope awareness

---

## magic_numbers

### Current Implementation
- **File:** `src/audit/pipelines/c/magic_numbers.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `magic_number`
- **Detection method:** Uses `compile_numeric_literal_query()` to find all `number_literal` nodes. Filters out values in `EXCLUDED_VALUES` (0, 1, 2, 0.0, 1.0, -1, common powers of 2, hex masks) and `COMMON_ALLOWED_NUMBERS` (HTTP codes, ports, small integers 3-8, 16-128). Filters out numbers in exempt ancestor contexts: `preproc_def`, `preproc_function_def`, `enumerator`, `bitfield_clause`, `field_declaration`, `array_declarator`, `initializer_list`, and `const` declarations. Also filters out array subscript indices. Cap: 200 findings per file. Severity: `info`.

### Problems Identified
1. **[Hardcoded thresholds without justification (#12)]:** The `EXCLUDED_VALUES` list includes `10`, `100`, `1000` which are themselves magic numbers in most contexts. `256` and `512` are only non-magic in bitwise/buffer contexts. The `COMMON_ALLOWED_NUMBERS` list is extremely broad (includes all HTTP status codes, common ports, timeouts) which makes sense for web languages but not for C code where these values are rarely used directly. The 200-finding cap per file is arbitrary.
2. **[High false positive rate (#2)]:** Any number not in the allowlists and not in an exempt context is flagged. Numbers in `return` statements (e.g., `return 42;` as an error code) and function arguments (e.g., `sleep(5)`) are flagged. Numbers in `switch` case labels are not exempt.
3. **[High false negative rate (#3)]:** Does not detect string-embedded magic numbers (e.g., format strings with hardcoded widths). Does not detect magic numbers used as bit shift counts (e.g., `x << 24`) -- these are often meaningful constants that should be named.
4. **[Missing context (#4)]:** The finding message just says "consider extracting to a named constant" without any context about what the number might represent. A number `3600` is likely seconds-in-an-hour; `0x1F` is likely a bitmask. Context-aware messages would be more useful.
5. **[Language idiom ignorance (#13)]:** Does not exempt numbers in `static_assert`, `_Static_assert`, or `sizeof` comparisons. Does not exempt numbers in `#if` / `#elif` preprocessor conditions (these are not in the AST as `preproc_def`). Does not exempt `case` labels in `switch` statements.
6. **[No suppression/annotation awareness (#11)]:** No way to suppress individual findings.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Magic number in function detected, `#define` skipped, `const` skipped, `enum` skipped, common values (0, 1, 2) skipped, array index skipped.
- **What's NOT tested:** Numbers in `switch` case labels, numbers in `return` statements, numbers in `#if` conditions, numbers as bit shift operands, numbers in `static_assert`, numbers with type suffix (e.g., `42L`, `3.14f`), negative magic numbers, hex magic numbers not in allowlist (e.g., `0xDEADBEEF`), the 200-cap behavior, numbers in `initializer_list` (exempt but untested), bitfield widths.

### Replacement Pipeline Design
**Target trait:** GraphPipeline (for cross-reference with symbol definitions)

#### Step 1: File Identification
- **Tool:** Tree-sitter (numeric literals are purely syntactic)
- **Why not graph:** Graph does not track numeric literal nodes. This is inherently an AST-level check.
- **Query:** Find all `number_literal` nodes. Filter through allowlists and exempt contexts.
- **Returns:** List of `(file_path, line, column, value, parent_context)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Query:** For each candidate, check additional exempt contexts: `switch` case labels (`case_statement` ancestor), bit shift operands (`binary_expression` with `<<`/`>>` operator), `static_assert` / `_Static_assert`, `return_statement` in functions with `int` return type (common for error codes). Also check `#if`/`#elif` preprocessor contexts.
- **Returns:** Filtered list.

#### Step 3: False Positive Removal
- **Tool:** AI prompt (for semantic classification of remaining numbers)
- **Why not graph/tree-sitter:** Determining whether a number like `3600` is "seconds in hour" or `0xCAFE` is a magic constant requires semantic understanding beyond syntax.
- **Context gathering query:** Extract 3 lines of context around each remaining finding.
- **Context shape:** `{"file": "...", "line": N, "value": "3600", "context": "timeout = 3600; // ..."}`
- **Prompt:** "For each numeric literal, classify as: MAGIC (should be named constant), ACCEPTABLE (common idiom), or UNCERTAIN. Respond with JSON array of classifications."
- **Expected return:** `[{"value": "3600", "classification": "MAGIC", "suggested_name": "SECONDS_PER_HOUR"}]`

#### Graph Enhancement Required
- None for the core detection. Graph could optionally provide cross-file analysis to find if a magic number is duplicated across multiple files (suggesting it should be a shared constant).

### New Test Cases
1. **test_switch_case_not_flagged** -- Input: `switch(x) { case 42: break; }` -> Expected: 0 findings -- Covers: #13 language idiom
2. **test_bit_shift_operand** -- Input: `int x = val << 24;` -> Expected: 1 finding for `24` (should be named) -- Covers: #4 missing context
3. **test_return_error_code** -- Input: `int f() { return 42; }` -> Expected: 1 finding -- Covers: #6 missing edge cases
4. **test_hex_magic** -- Input: `int x = 0xDEADBEEF;` -> Expected: 1 finding -- Covers: #6
5. **test_type_suffix_number** -- Input: `long x = 42L;` -> Expected: 1 finding -- Covers: #6
6. **test_static_assert_exempt** -- Input: `_Static_assert(sizeof(int) == 4, "");` -> Expected: 0 findings for `4` -- Covers: #13
7. **test_200_cap_reached** -- Input: file with 250 magic numbers -> Expected: exactly 200 findings -- Covers: #12 threshold

---

## global_mutable_state

### Current Implementation
- **File:** `src/audit/pipelines/c/global_mutable_state.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `global_mutable_state`
- **Detection method:** Uses `compile_declaration_query()` to find all `declaration` nodes. Filters to only those at `translation_unit` scope (file level). Skips `const` declarations, `extern` declarations, and function prototypes. Everything remaining is flagged. Severity: `warning`.

### Problems Identified
1. **[High false positive rate (#2)]:** Flags ALL non-const file-scope variables, including: `static` variables (which are file-local and encapsulated -- not shared state), variables that are only read after initialization (effectively immutable), variables that are thread-local (`_Thread_local` / `__thread`). File-local `static` variables are a common and acceptable C pattern for encapsulation.
2. **[No severity graduation (#15)]:** All findings are `warning`. A truly global mutable variable (non-static, accessed from multiple files) is more dangerous than a file-local `static` variable. An unprotected global accessed from multiple threads deserves `error`.
3. **[Missing context (#4)]:** Does not report whether the variable is `static` (file-local) vs non-static (truly global). Does not report if the variable is used from multiple functions.
4. **[Single-node detection (#14)]:** Does not analyze how many functions read/write the variable. A global variable used by one function is less problematic than one used by 10 functions across 5 files.
5. **[Language idiom ignorance (#13)]:** Does not recognize `_Thread_local` / `__thread` qualifier which makes the variable thread-safe. Does not recognize `volatile` variables used for hardware registers (common in embedded C). Does not exempt common patterns like `static const char *` arrays (string tables).
6. **[No data flow tracking (#10)]:** Does not check whether the global is actually mutated after initialization. `int debug_level = 0;` set once at startup and never changed is effectively const.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Global mutable detected, `const` skipped, `extern` skipped, function prototype skipped, local variable skipped.
- **What's NOT tested:** `static` non-const variable (should be lower severity), `_Thread_local` variable, `volatile` variable, global array, global struct, global pointer, global with `__attribute__` annotations, multiple globals in one file.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** Iterate `graph.file_nodes` for C files. For each file, look at `Symbol` nodes connected via `DefinedIn` edges where `kind == Variable` and `exported == true` (non-static). These are the truly global mutable candidates.
- **Returns:** List of `(file_path, symbol_name, symbol_node, is_exported)`.

#### Step 2: Narrowing
- **Tool:** CodeGraph call graph + tree-sitter
- **Why not graph alone:** Graph `Symbol` does not store type qualifiers (const, volatile, thread_local).
- **Query:** For each candidate variable, (a) use tree-sitter to check for `const`, `_Thread_local`/`__thread`, `volatile` qualifiers, (b) use `graph.find_symbols_by_name(var_name)` cross-referenced with `CallSite` nodes to count how many functions reference this variable, (c) check reverse file edges to see if the variable is accessed from multiple files.
- **Returns:** `(symbol, qualifier_set, reference_count, cross_file: bool)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Check if the variable is only assigned once (at declaration) and never re-assigned. Parse the file and walk all `assignment_expression` nodes to check if any target the global variable.
- **Returns:** Graduated findings: `error` for cross-file mutable globals, `warning` for file-local statics used by multiple functions, `info` for file-local statics used by one function.

#### Graph Enhancement Required
- **Variable reference tracking:** Currently `CallSite` tracks function calls. A `VariableRef` node type (or extending `CallSite` to cover variable references) would allow graph-based usage counting without re-parsing.

### New Test Cases
1. **test_static_variable_lower_severity** -- Input: `static int count = 0;` -> Expected: finding with severity `info` (not `warning`) -- Covers: #15 severity graduation
2. **test_thread_local_skipped** -- Input: `_Thread_local int tls_var = 0;` -> Expected: 0 findings -- Covers: #13 language idiom
3. **test_volatile_hardware_register** -- Input: `volatile int *MMIO_REG = (volatile int *)0x40000000;` -> Expected: finding with context noting volatile -- Covers: #4 missing context
4. **test_global_array** -- Input: `int lookup_table[256] = {0};` -> Expected: 1 finding -- Covers: #6 missing edge cases
5. **test_cross_file_usage_escalated** -- Graph input: variable referenced from 3 files -> Expected: severity `error` -- Covers: #14 single-node, #15 severity
6. **test_effectively_const** -- Input: `int debug_level = 0;` with no re-assignment -> Expected: `info` severity -- Covers: #10 data flow

---

## typedef_pointer_hiding

### Current Implementation
- **File:** `src/audit/pipelines/c/typedef_pointer_hiding.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `typedef_hides_pointer`
- **Detection method:** Uses `compile_type_definition_query()` to find all `type_definition` nodes. Checks if the declarator contains a `pointer_declarator` node. Excludes function pointer typedefs (contains `function_declarator`). Everything else with a pointer is flagged. Severity: `info`.

### Problems Identified
1. **[High false positive rate (#2)]:** Flags ALL pointer typedefs indiscriminately. Common and widely accepted patterns like `typedef char *string_t;` in embedded systems, or opaque handle typedefs like `typedef struct Foo_s *Foo;` (opaque pointer idiom) are flagged. The opaque pointer pattern is a recommended C design pattern from CERT/MISRA.
2. **[Language idiom ignorance (#13)]:** Does not recognize the opaque pointer idiom (`typedef struct Impl *Handle;`). Does not recognize Windows-style handle typedefs (`typedef void *HANDLE;` -- though `void*` is excluded, `typedef HWND__ *HWND;` is not). Does not exempt typedefs in system/vendor headers.
3. **[No severity graduation (#15)]:** All findings are `info`. A `typedef int ***TriplePtr;` is much more concerning than `typedef const char *cstring;`.
4. **[Missing context (#4)]:** Does not report the underlying type or pointer depth. `typedef int *IntPtr;` (1 level) vs `typedef int **IntPtrPtr;` (2 levels) should have different messaging.
5. **[Overlapping detection (#16)]:** A `typedef void *Handle;` would be caught by both this pipeline and the `void_pointer_abuse` pipeline (if it were a parameter). However, since this checks typedefs and void_pointer_abuse checks parameters/returns, actual overlap is minimal. Potential overlap: the typedef creates a type that is then used as a parameter.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Pointer typedef detected, non-pointer typedef skipped, function pointer typedef skipped.
- **What's NOT tested:** Double pointer typedef (`typedef int **IntPtrPtr;`), opaque pointer typedef (`typedef struct Impl *Handle;`), const pointer typedef (`typedef const char *cstring;`), array of pointers typedef, typedef in header file, typedef using another typedef.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** Find all `Symbol` nodes where `kind == Typedef`. Use `graph.find_symbols_by_name` to collect typedef symbols.
- **Returns:** List of `(file_path, symbol_name, symbol_node)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (graph does not store whether a typedef hides a pointer)
- **Why not graph:** `Symbol` nodes for typedefs do not store the underlying type or pointer depth.
- **Query:** For each typedef, parse the `type_definition` node. Check if the declarator contains `pointer_declarator`. If so, compute pointer depth (count nested `pointer_declarator` nodes). Check if it is an opaque pointer pattern (underlying type is `struct_specifier` with no `body` -- forward declaration). Check for `const` qualifier.
- **Returns:** `(typedef_name, pointer_depth, is_opaque, is_const, underlying_type)`.

#### Step 3: False Positive Removal
- **Tool:** CodeGraph usage analysis
- **Query:** For each flagged typedef, check how many times it is used in the codebase (via `graph.find_symbols_by_name` for parameters/variables of that type). If the typedef is used extensively, it is a deliberate API design choice. If it is only used once, it is more likely accidental.
- **Returns:** Graduated findings: `info` for single-pointer with wide usage, `warning` for double+ pointers, skip opaque pointer pattern entirely.

#### Graph Enhancement Required
- **Typedef underlying type:** Storing the underlying type (or at least "is_pointer" and "pointer_depth") in the `Symbol` node for typedefs would allow pure graph-based detection.

### New Test Cases
1. **test_opaque_pointer_skipped** -- Input: `typedef struct Impl *Handle;` -> Expected: 0 findings -- Covers: #13 language idiom
2. **test_double_pointer_higher_severity** -- Input: `typedef int **IntPtrPtr;` -> Expected: finding with severity `warning` (not `info`) -- Covers: #15 severity graduation
3. **test_const_pointer_typedef** -- Input: `typedef const char *cstring;` -> Expected: 0 findings or `info` with reduced concern -- Covers: #4 missing context
4. **test_pointer_depth_reported** -- Input: `typedef int ***TriplePtr;` -> Expected: message mentions "3 levels of indirection" -- Covers: #4
5. **test_function_pointer_still_skipped** -- Input: `typedef int (*Comparator)(const void *, const void *);` -> Expected: 0 findings -- Covers: #6 edge case

---

## define_instead_of_inline

### Current Implementation
- **File:** `src/audit/pipelines/c/define_instead_of_inline.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `function_like_macro`
- **Detection method:** Uses `compile_preproc_function_def_query()` to find all `preproc_function_def` nodes (macros with parameter lists). Every single one is flagged. Severity: `info`.

### Problems Identified
1. **[High false positive rate (#2)]:** Flags ALL function-like macros unconditionally. This is an extremely noisy pipeline. Many function-like macros cannot be replaced with inline functions: (a) macros that operate on types (e.g., `#define ARRAY_SIZE(arr) (sizeof(arr)/sizeof((arr)[0]))`), (b) macros that use stringification (`#`) or token pasting (`##`), (c) macros used in `#if` expressions, (d) macros that need to work across C89 compilers where `inline` is not guaranteed, (e) variadic macros (`__VA_ARGS__`), (f) macros that return from the enclosing function (`#define CHECK(x) if(!(x)) return -1`). The current pipeline is essentially "flag every function-like macro" which is not useful.
2. **[Broken detection (#1)]:** The pipeline does not examine the macro body at all. It cannot distinguish between macros that genuinely should be inline functions (simple arithmetic) and macros that structurally cannot be (token pasting, type-generic, variadic).
3. **[Missing context (#4)]:** Does not report why the macro should be an inline function or what the inline equivalent would look like.
4. **[Language idiom ignorance (#13)]:** In C (as opposed to C++), function-like macros are the standard mechanism for type-generic programming before `_Generic` (C11). Flagging `#define MAX(a,b) ((a) > (b) ? (a) : (b))` ignores that the inline alternative would lose type genericity.
5. **[No suppression/annotation awareness (#11)]:** No way to suppress findings.

### Test Coverage
- **Existing tests:** 2 tests
- **What's tested:** Function-like macro detected, value macro skipped.
- **What's NOT tested:** Macros with `#`/`##` operators, variadic macros, macros using `do { } while(0)` pattern, macros with `return` statements, macros using `typeof`/`__typeof__`, multi-line macros, guard macros, macros in system headers.

### Replacement Pipeline Design
**Target trait:** GraphPipeline (for cross-file macro usage analysis)

#### Step 1: File Identification
- **Tool:** Tree-sitter
- **Why not graph:** Graph `Symbol` nodes for macros (`kind == Macro`) do not store the macro body or parameter information.
- **Query:** Find all `preproc_function_def` nodes. Extract the macro name, parameter list, and body text (the `value` field).
- **Returns:** List of `(file_path, macro_name, params, body_text)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (body text analysis)
- **Query:** For each macro, analyze the body text for features that prevent inline replacement: (a) contains `#` (stringification) or `##` (token pasting), (b) contains `__VA_ARGS__` or `...`, (c) contains `return` / `break` / `continue` / `goto` (control flow from enclosing scope), (d) contains `typeof` / `__typeof__` / `_Generic`, (e) contains `do { } while(0)` statement wrapper. Only flag macros whose bodies are pure expressions without these features.
- **Returns:** Filtered list of macros that could genuinely be inline functions.

#### Step 3: False Positive Removal
- **Tool:** AI prompt
- **Why not graph/tree-sitter:** Determining whether a macro is "type-generic on purpose" vs "should be typed" requires semantic judgment.
- **Context gathering query:** Extract the full macro definition text.
- **Context shape:** `{"name": "MAX", "params": "(a, b)", "body": "((a) > (b) ? (a) : (b))"}`
- **Prompt:** "For each C function-like macro, determine if it can be safely replaced with a `static inline` function without losing functionality (type genericity, stringification, control flow). Respond with JSON: {replaceable: bool, reason: string}."
- **Expected return:** `{"replaceable": false, "reason": "Type-generic comparison macro; inline would require fixed types"}`

#### Graph Enhancement Required
- **Macro body storage:** Storing the macro body text or at least feature flags (has_stringify, has_token_paste, has_va_args) in the `Symbol` node would allow graph-based filtering.

### New Test Cases
1. **test_stringify_macro_skipped** -- Input: `#define STR(x) #x` -> Expected: 0 findings -- Covers: #2 false positive
2. **test_token_paste_macro_skipped** -- Input: `#define CONCAT(a, b) a##b` -> Expected: 0 findings -- Covers: #2
3. **test_variadic_macro_skipped** -- Input: `#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)` -> Expected: 0 findings -- Covers: #13 language idiom
4. **test_simple_arithmetic_flagged** -- Input: `#define DOUBLE(x) ((x) * 2)` -> Expected: 1 finding -- Covers: correct positive
5. **test_control_flow_macro_skipped** -- Input: `#define CHECK(x) if(!(x)) return -1` -> Expected: 0 findings -- Covers: #2
6. **test_do_while_wrapper_skipped** -- Input: `#define SWAP(a,b) do { int t = a; a = b; b = t; } while(0)` -> Expected: 0 findings (statement macro, not expression) -- Covers: #13
7. **test_type_generic_macro** -- Input: `#define MAX(a,b) ((a) > (b) ? (a) : (b))` -> Expected: 0 findings or `info` with note about type genericity -- Covers: #13

---

## ignored_return_values

### Current Implementation
- **File:** `src/audit/pipelines/c/ignored_return_values.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `ignored_return_value`
- **Detection method:** Uses `compile_expression_statement_call_query()` which matches `expression_statement > call_expression` -- i.e., function calls whose return value is discarded (used as a statement, not assigned). Checks if the function name is in `DANGEROUS_FUNCTIONS`: `fwrite`, `fread`, `fclose`, `fopen`, `fgets`, `fputs`, `strncpy`, `snprintf`, `memcpy`, `memmove`, `read`, `write`, `close`, `open`. Severity: `warning`.

### Problems Identified
1. **[High false positive rate (#2)]:** `memcpy` and `memmove` return the destination pointer, which is rarely useful and commonly ignored. `strncpy` also returns dest, commonly ignored. These are acceptable to ignore in most codebases. `close()` return value is debated -- POSIX says to check it but many codebases intentionally ignore it.
2. **[High false negative rate (#3)]:** The function list is incomplete. Missing: `malloc` (return value is the pointer -- covered by unchecked_malloc but conceptually overlaps), `realloc`, `fprintf`, `fscanf`, `send`, `recv`, `connect`, `bind`, `listen`, `accept`, `pthread_*` functions, `setvbuf`, `setenv`, `unlink`, `rename`, `chmod`.
3. **[No severity graduation (#15)]:** All findings are `warning`. Ignoring `fopen()` return (file handle leak + null deref) is `error`-level. Ignoring `memcpy()` return is at most `info`.
4. **[Missing context (#4)]:** Does not explain what the return value means for each function. `fwrite` returns number of bytes written (partial write detection); `fopen` returns NULL on failure (crash if unchecked).
5. **[Single-node detection (#14)]:** Does not check if the return value is checked via an enclosing `if` expression (e.g., `if (!fopen(...))` would not match the query since it is not an `expression_statement`). But it also cannot detect patterns like `(void)fclose(fp);` where the cast to void is an explicit suppression.
6. **[No suppression/annotation awareness (#11)]:** Does not recognize `(void)func()` cast as intentional suppression. Does not check for `__attribute__((warn_unused_result))` on function declarations.
7. **[Language idiom ignorance (#13)]:** In C, casting to `(void)` is the standard way to suppress "unused return value" warnings. This pipeline should respect that.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Ignored `fwrite` detected, assigned `fwrite` skipped, ignored `fclose` detected.
- **What's NOT tested:** `(void)fclose(fp)` suppression, `memcpy` (debatable -- should it be flagged?), `fopen` (critical to check), `read`/`write` (POSIX), return used in `if` condition, return used in comma expression, chained calls (e.g., `fwrite(...), fclose(fp)`), function calls through pointers.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** Find all `CallSite` nodes in C files. For each, check if the call site's name is in the expanded dangerous function list. Cross-reference with the enclosing symbol to determine context.
- **Returns:** List of `(call_site_node, function_name, enclosing_function)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (to determine if return value is used)
- **Why not graph:** Graph `CallSite` does not record whether the return value was consumed.
- **Query:** For each call site at a given line, parse the AST and check if the call_expression is: (a) direct child of `expression_statement` (unused), (b) wrapped in `(void)` cast (intentionally suppressed), (c) used as operand in assignment/if/return (used). Only flag category (a).
- **Returns:** `(call_site, function_name, return_value_status: Unused|Suppressed|Used)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + severity table
- **Query:** Apply per-function severity rules: `fopen`/`open` -> `error` (null/fd leak), `fwrite`/`fread`/`read`/`write` -> `warning` (data corruption), `memcpy`/`memmove`/`strncpy` -> `info` (return is dest, rarely useful), `close`/`fclose` -> `info` (debatable). Skip findings where `(void)` cast is present.
- **Returns:** Graduated severity findings.

#### Graph Enhancement Required
- **Return value usage tracking:** If `CallSite` nodes stored whether the return value was consumed, Step 2 could be pure graph.

### New Test Cases
1. **test_void_cast_suppression** -- Input: `(void)fclose(fp);` -> Expected: 0 findings -- Covers: #11 suppression, #13 language idiom
2. **test_fopen_is_error_severity** -- Input: `fopen("file.txt", "r");` -> Expected: severity `error` -- Covers: #15 severity graduation
3. **test_memcpy_is_info_severity** -- Input: `memcpy(dest, src, n);` -> Expected: severity `info` -- Covers: #15
4. **test_return_in_if_not_flagged** -- Input: `if (fwrite(buf, 1, n, fp) < n) { ... }` -> Expected: 0 findings -- Covers: #6 edge cases
5. **test_pthread_create_ignored** -- Input: `pthread_create(&t, NULL, func, arg);` -> Expected: 1 finding -- Covers: #3 false negatives
6. **test_recv_ignored** -- Input: `recv(sock, buf, len, 0);` -> Expected: 1 finding -- Covers: #3
7. **test_chained_comma** -- Input: `fwrite(buf, 1, n, fp), fclose(fp);` -> Expected: 2 findings -- Covers: #6

---

## void_pointer_abuse

### Current Implementation
- **File:** `src/audit/pipelines/c/void_pointer_abuse.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `void_pointer_parameter` (used for both parameter and return findings)
- **Detection method:** Two passes: (1) Find all `parameter_declaration` nodes where type is `void` and declarator is `pointer_declarator`. (2) Find all `function_definition` nodes where return type is `void` and top-level declarator is `pointer_declarator`. Both are flagged with severity `info`.

### Problems Identified
1. **[High false positive rate (#2)]:** `void*` is the standard C mechanism for generic programming (e.g., `qsort`, `bsearch`, `pthread_create`, callback contexts). Flagging every `void*` parameter is extremely noisy. Standard library functions and callback patterns use `void*` by design.
2. **[Language idiom ignorance (#13)]:** Does not recognize: (a) callback context patterns (`void *ctx` in callback signatures), (b) generic container implementations (`void *data` in linked list nodes), (c) system call wrappers that must use `void*` for POSIX compliance, (d) `main`'s thread entry point signature.
3. **[Overlapping detection (#16)]:** The pattern name `void_pointer_parameter` is used for both parameter and return-type findings (line 87 for params, line 135 for returns). Should use distinct pattern names for accurate categorization.
4. **[No severity graduation (#15)]:** All findings are `info`. A function with 5 `void*` parameters is more concerning than one with a single `void *ctx` callback parameter.
5. **[Missing context (#4)]:** Does not suggest what the parameter type should be. If the `void*` is immediately cast to a specific type inside the function body, that type should be suggested.
6. **[Single-node detection (#14)]:** Does not check how the `void*` is used inside the function. If it is immediately cast to a concrete type, the fix is obvious. If it flows through multiple functions unchecked, it is more dangerous.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `void*` parameter detected, `void*` return detected, `int*` parameter skipped, `void` (no pointer) skipped.
- **What's NOT tested:** Multiple `void*` parameters, `const void*` parameter, `void**` parameter, callback context pattern, `void*` in function prototype (not definition), `void*` cast immediately in function body, `void*` in variadic function context.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** Find all `Symbol` nodes with `kind == Function` in C files. For each, check `Parameter` nodes connected via edges. Filter to parameters where `name` suggests void pointer usage.
- **Returns:** List of candidate functions with void pointer usage.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (graph `Parameter` nodes do not store type information)
- **Why not graph:** Parameter type is not stored in the graph.
- **Query:** For each candidate function, parse the function signature. Check for `void *` parameters. For the function body, check if the `void*` parameter is cast to a specific type within the first 5 lines (suggests the concrete type is known). Check for callback patterns: parameter named `ctx`, `data`, `arg`, `userdata`, `opaque` with `void*` type.
- **Returns:** `(function, param_name, is_callback_pattern: bool, cast_target_type: Option<string>)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Skip findings where: (a) the parameter name matches common callback context names and the function signature includes a function pointer parameter (callback pattern), (b) the function is implementing a known interface (`pthread_create` entry, `qsort` comparator), (c) the `void*` is `const void*` used for read-only generic data.
- **Returns:** Filtered findings with context (suggest concrete type if cast found).

#### Graph Enhancement Required
- **Parameter type storage:** Adding type information to `Parameter` nodes would allow graph-based filtering.

### New Test Cases
1. **test_callback_context_skipped** -- Input: `void handler(void *ctx, int event) {}` -> Expected: 0 findings -- Covers: #13 language idiom
2. **test_qsort_comparator_skipped** -- Input: `int cmp(const void *a, const void *b) {}` -> Expected: 0 findings -- Covers: #13
3. **test_void_star_star** -- Input: `void process(void **data) {}` -> Expected: 1 finding -- Covers: #6 edge cases
4. **test_const_void_star_lower** -- Input: `void process(const void *data) {}` -> Expected: 0 findings or lower severity -- Covers: #15
5. **test_immediate_cast_suggests_type** -- Input: `void process(void *data) { struct Foo *f = (struct Foo *)data; }` -> Expected: finding with suggestion "use struct Foo *" -- Covers: #4 missing context
6. **test_distinct_pattern_names** -- Input: `void *create()` and `void f(void *p)` -> Expected: different pattern names for return vs parameter -- Covers: #16 overlapping detection

---

## missing_const

### Current Implementation
- **File:** `src/audit/pipelines/c/missing_const.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `missing_const_param`
- **Detection method:** Uses `compile_parameter_declaration_query()` to find all `parameter_declaration` nodes. Checks if the type is not `void` (deferred to void_pointer_abuse), the declarator is a pointer, and there is no `const` qualifier. Severity: `info`.

### Problems Identified
1. **[High false positive rate (#2)]:** Flags ALL non-const pointer parameters. Many functions legitimately modify data through pointer parameters (`memcpy` dest, `strcpy` dest, any output parameter). Without analyzing whether the function body modifies the pointed-to data, every non-const pointer is flagged.
2. **[Single-node detection (#14)]:** Does not analyze the function body to determine if the pointer parameter is actually modified. The finding says "add `const` if the function does not modify the data" but the pipeline cannot determine this. This makes it a suggestion generator, not a bug detector.
3. **[No data flow tracking (#10)]:** The critical analysis for this pipeline is: "does the function body write through this pointer?" This requires data flow analysis (or at least AST walking of the function body) which is completely absent.
4. **[High false negative rate (#3)]:** Only checks function parameters. Does not check local pointer variables that could be `const` (e.g., `char *s = "hello";` should be `const char *s`).
5. **[Missing context (#4)]:** Does not report which parameter specifically (by name) or what the parameter type is.
6. **[No suppression/annotation awareness (#11)]:** No suppression mechanism.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Non-const pointer detected, const pointer skipped, non-pointer skipped, void pointer skipped (deferred).
- **What's NOT tested:** Output parameter that modifies data (false positive), double pointer parameter, function with multiple pointer params, `restrict` qualifier, `char *` that points to string literal, function prototype (not definition), array parameter syntax (`int arr[]`).

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** Find all `Symbol` nodes with `kind == Function` in C files. For each, collect `Parameter` nodes.
- **Returns:** List of `(function_node, parameter_nodes)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (parameter type and function body analysis)
- **Why not graph:** Parameter types and function body mutation analysis require AST walking.
- **Query:** For each function, parse the function definition. For each pointer parameter without `const`: (a) extract the parameter name, (b) walk the function body looking for write operations through this parameter (assignment to `*param`, `param[i] = ...`, passing to a non-const function). If no writes found, the parameter can be `const`.
- **Returns:** `(function, param_name, param_type, has_writes: bool)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Additional checks: (a) the parameter is passed to a function that takes non-const pointer (transitive mutation), (b) the parameter is returned (can't add const without changing return type), (c) the parameter is stored in a non-const global/struct field. Only flag if no writes or transitive mutations found.
- **Returns:** Findings where `const` can be safely added.

#### Graph Enhancement Required
- **Parameter type storage:** Type information on `Parameter` nodes.
- **Write-through-pointer tracking:** CFG `Assignment` statements could track writes through pointer dereferences, enabling graph-based mutation analysis.

### New Test Cases
1. **test_output_parameter_not_flagged** -- Input: `void init(int *out) { *out = 42; }` -> Expected: 0 findings -- Covers: #2 false positive, #10 data flow
2. **test_read_only_parameter_flagged** -- Input: `int sum(int *arr, int n) { int s=0; for(int i=0;i<n;i++) s+=arr[i]; return s; }` -> Expected: 1 finding for `arr` -- Covers: correct positive
3. **test_double_pointer_output** -- Input: `void alloc(int **out) { *out = malloc(10); }` -> Expected: 0 findings -- Covers: #6 edge cases
4. **test_restrict_pointer** -- Input: `void copy(int *restrict dest, int *restrict src, int n) {}` -> Expected: 1 finding for `src` only -- Covers: #6
5. **test_array_parameter** -- Input: `void process(int arr[], int n) {}` -> Expected: 1 finding if arr is not modified -- Covers: #3 false negatives
6. **test_passed_to_non_const_function** -- Input: `void process(char *buf) { strlen(buf); }` -> Expected: 1 finding (strlen takes const, so buf could be const) -- Covers: #10

---

## raw_struct_serialization

### Current Implementation
- **File:** `src/audit/pipelines/c/raw_struct_serialization.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `raw_struct_serialization`
- **Detection method:** Uses `compile_call_expression_query()` to find calls to `fwrite`/`fread`. Checks if the argument list contains a `sizeof_expression` whose operand is a struct type. Struct detection heuristics: (a) operand text contains "struct", (b) child is `type_identifier` or `struct_specifier`, (c) for `sizeof(TypedefName)`, uppercase-starting identifiers are treated as likely type names. Severity: `warning`.

### Problems Identified
1. **[High false positive rate (#2)]:** The uppercase-first-letter heuristic for typedef detection (line 88: `c.is_ascii_uppercase()`) will flag `sizeof(FILE)` (standard library type), `sizeof(DIR)`, `sizeof(NULL)` (if used erroneously). It will also flag typedefs of primitive types (e.g., `typedef int Status; fwrite(&s, sizeof(Status), 1, fp)`).
2. **[High false negative rate (#3)]:** Does not detect: (a) `fwrite` with `sizeof(var)` where `var` is a struct instance (only checks `sizeof(type)`, not `sizeof(variable)`), (b) `fwrite` with computed size that happens to be `sizeof(struct)`, (c) `send`/`sendto` with raw struct serialization (network I/O, not just file I/O), (d) `write()` system call with raw struct.
3. **[Missing context (#4)]:** Does not explain what makes struct serialization non-portable (padding, endianness, field alignment). Does not suggest alternatives (field-by-field serialization, protocol buffers, packed structs with `__attribute__((packed))`).
4. **[Language idiom ignorance (#13)]:** Does not check for `__attribute__((packed))` on the struct, which eliminates padding concerns. Does not check for `#pragma pack` directives. A packed struct with `fwrite` is acceptable.
5. **[Literal blindness (#8)]:** Relies on text matching (`inner.contains("struct")`) which could match on a variable named `restructure` or a comment within the sizeof expression.
6. **[Single-node detection (#14)]:** Does not check if the struct being serialized has any padding. A struct with all same-type fields (e.g., `struct Vec3 { float x, y, z; }`) has no padding on most platforms and is safe to serialize raw.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** `fwrite` with `sizeof(struct Record)` detected, `fwrite` with `sizeof(char)` skipped, `fread` with `sizeof(Point)` (typedef) detected.
- **What's NOT tested:** `sizeof(variable)` instead of `sizeof(type)`, packed struct, `write()` system call, `send()`, struct with all same-type fields, `sizeof` in a macro argument, `fwrite` where size is a variable computed from sizeof elsewhere, uppercase non-struct typedef.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** CodeGraph query
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** Find all `CallSite` nodes where `name` is in `{fwrite, fread, write, send, sendto, writev}`. Collect enclosing file and function.
- **Returns:** List of `(call_site, function_name, file_path, line)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Need to inspect call arguments for sizeof patterns, which graph does not store.
- **Query:** For each call site, parse the AST at that line. Check if any argument is or contains a `sizeof_expression`. If so, resolve the sizeof operand: (a) if it is a `type_identifier` or `struct_specifier`, look up the struct definition in the file, (b) if it is an `identifier` (variable), find its declaration and check if the type is a struct. Check if the struct has `__attribute__((packed))` or is under `#pragma pack`.
- **Returns:** `(call_site, struct_name, is_packed: bool, has_padding: Option<bool>)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (struct definition analysis)
- **Query:** For each flagged struct, check if all fields are the same type and same size (no padding expected). Check for `__attribute__((packed))` or `#pragma pack`. Skip packed structs.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Struct field tracking:** If `Symbol` nodes for structs stored field types and `__attribute__` annotations, padding analysis could be graph-based.

### New Test Cases
1. **test_sizeof_variable_not_type** -- Input: `struct S s; fwrite(&s, sizeof(s), 1, fp);` -> Expected: 1 finding -- Covers: #3 false negative
2. **test_packed_struct_skipped** -- Input: `struct __attribute__((packed)) S { int x; char y; }; fwrite(&s, sizeof(S), 1, fp);` -> Expected: 0 findings -- Covers: #13 language idiom
3. **test_write_syscall** -- Input: `write(fd, &record, sizeof(struct Record));` -> Expected: 1 finding -- Covers: #3 false negative
4. **test_send_network** -- Input: `send(sock, &msg, sizeof(struct Msg), 0);` -> Expected: 1 finding -- Covers: #3
5. **test_primitive_typedef_not_flagged** -- Input: `typedef int Status; fwrite(&s, sizeof(Status), 1, fp);` -> Expected: 0 findings (not a struct) -- Covers: #2 false positive
6. **test_all_same_type_struct** -- Input: `struct Vec3 { float x, y, z; }; fwrite(&v, sizeof(struct Vec3), 1, fp);` -> Expected: `info` severity (likely no padding) -- Covers: #15 severity graduation
7. **test_FILE_not_flagged** -- Input: `fread(buf, sizeof(FILE), 1, fp);` -> Expected: 0 findings or different pattern -- Covers: #2 false positive
