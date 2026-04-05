# Python Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 12 (8 classic tech-debt + 4 test-quality)
- **Trait types used:** All 12 use `GraphPipeline` (wrapped via `AnyPipeline::Graph`)
- **Overall assessment:** Implementations are solid tree-sitter based detectors with reasonable test coverage. However, most pipelines labeled `GraphPipeline` do not actually use the `CodeGraph` at all -- only `missing_type_hints` queries the graph (for cross-module caller filtering). The remaining 11 pipelines treat `GraphPipelineContext.graph` as dead weight, meaning they gain no benefit from the graph infrastructure. Detection is predominantly single-node or single-file; no pipeline performs cross-file duplicate detection, data flow analysis, or call-graph-based prioritization beyond `missing_type_hints`. Suppression annotation awareness (`# noqa`, `# type: ignore`, `# nosec`) is absent across the board. Severity graduation is minimal -- most pipelines emit a single static severity level regardless of how bad the finding is.

---

## bare_except

### Current Implementation
- **File:** `src/audit/pipelines/python/bare_except.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `untyped_exception_handler` -- bare `except:` without exception type
- **Detection method:** Tree-sitter query for `except_clause` nodes; checks if the clause has zero named children that are not `block` (i.e., no exception type specified)

### Problems Identified
1. **[Single-node detection (14)]:** Detection is purely AST-local. The graph is available but unused. A `bare except` inside a tiny retry helper called from one place is very different from one in a request handler called from 50 endpoints. Graph-based caller traversal could graduate severity.
2. **[No suppression/annotation awareness (11)]:** `# noqa: E722` and `# type: ignore` on the except line are not checked. A developer who has explicitly suppressed the linter warning will still receive this finding.
3. **[No severity graduation (15)]:** All bare excepts are emitted as `"warning"`. A bare except in a top-level `main()` that intentionally catches everything to log and exit is different from one in a library function that silently swallows `KeyboardInterrupt`.
4. **[High false positive rate (2)]:** `except:` followed by `raise` (re-raise) is a common safe pattern used for logging or cleanup. The pipeline does not check whether the except body contains a `raise` statement, so it flags these benign patterns.
5. **[Missing compound variants (9)]:** Only detects `except:`. Does not detect `except BaseException:` or `except Exception:` which, while typed, are nearly as broad and often equally problematic. These are separate patterns but related enough to warrant at least informational notes.
6. **[Language idiom ignorance (13)]:** In Python 2 compatibility code, `except Exception, e:` is a valid syntax variant. While tree-sitter-python parses Python 3, the pipeline makes no mention of this distinction.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** bare `except:`, typed `except Exception:`, `except Exception as e:`, tuple `except (ValueError, TypeError):`
- **What's NOT tested:** `except:` followed by `raise` (re-raise pattern), `except:` with a `# noqa` comment, `except:` inside nested try/except, multiple `except:` clauses in one try block, `except BaseException:` (related broad catch)

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `except_clause` nodes (same as current)
- **Why not graph:** Graph does not index exception handlers -- this is fundamentally a syntactic pattern. Tree-sitter is the correct first-pass tool.
- **Query:** `(except_clause) @except`
- **Returns:** List of `(file_path, line, except_node)` tuples

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST walk on each `except_clause`
- **Query:** For each except clause: (a) check if it has an exception type child, (b) check if the `block` body contains a `raise` statement (re-raise), (c) check preceding comment for `# noqa`
- **Returns:** Filtered list excluding re-raise patterns and suppressed lines

#### Step 3: False Positive Removal
- **Tool:** Graph query (CodeGraph.find_symbol + traverse_callers)
- **Query:** For each bare except, find the enclosing function via line range, look up the symbol in the graph, count cross-file callers. If the function is only called from tests or from one internal location, downgrade severity to "info".
- **Returns:** Findings with graduated severity

#### Graph Enhancement Required
- No new graph data needed. The existing `find_symbol(file, line)` and `traverse_callers()` are sufficient for caller-based severity graduation.

### New Test Cases
1. **bare_except_with_reraise** -- `try:\n    pass\nexcept:\n    logger.error(...)\n    raise` -> [] (no findings) -- Covers: high false positive rate (2)
2. **bare_except_with_noqa** -- `try:\n    pass\nexcept:  # noqa: E722\n    pass` -> [] (suppressed) -- Covers: no suppression awareness (11)
3. **bare_except_in_main_entrypoint** -- bare except in `if __name__ == "__main__"` block -> severity "info" not "warning" -- Covers: no severity graduation (15)
4. **multiple_bare_excepts** -- two bare excepts in separate try blocks in one file -> 2 findings -- Covers: missing edge cases (6)
5. **except_base_exception** -- `except BaseException:` -> separate pattern "broad_exception_handler" at "info" severity -- Covers: missing compound variants (9)
6. **nested_try_except_bare** -- bare except inside a nested try/except -> 1 finding at the inner except -- Covers: missing edge cases (6)

---

## mutable_default_args

### Current Implementation
- **File:** `src/audit/pipelines/python/mutable_default_args.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `mutable_default_arg` -- list `[]`, dictionary `{}`, or set `set()` literals as default parameter values
- **Detection method:** Tree-sitter query for `default_parameter` and `typed_default_parameter` nodes; checks the `value` child's kind against `MUTABLE_KINDS = ["list", "dictionary", "set"]`

### Problems Identified
1. **[High false negative rate (3)]:** `MUTABLE_KINDS` only includes `"list"`, `"dictionary"`, `"set"` (literal syntax). It misses mutable constructor calls: `list()`, `dict()`, `set()`, `defaultdict(list)`, `OrderedDict()`, `bytearray()`, `deque()`. These are all mutable defaults and equally dangerous.
2. **[Single-node detection (14)]:** Does not use the graph at all. Could use graph to check whether the function is actually called multiple times (single-call functions are less dangerous with mutable defaults).
3. **[No suppression/annotation awareness (11)]:** No check for `# noqa` or `# type: ignore` comments.
4. **[No severity graduation (15)]:** All mutable defaults are "warning" regardless of the function's call frequency or whether the parameter is ever mutated in the body.
5. **[Missing context (4)]:** The finding message tells the user to use `None` and initialize inside the function, but does not show which function contains the parameter. The `snippet` is just the parameter text, not the function signature for context.
6. **[Missing compound variants (9)]:** Does not detect `*` or `**` parameters with mutable defaults (uncommon but possible via decorators), nor does it detect mutable defaults assigned via function calls like `def foo(x=list()):`.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** list default `[]`, dict default `{}`, typed mutable default `list = []`, None default (negative), scalar default (negative)
- **What's NOT tested:** `set()` literal as default, `list()` call as default, `dict()` call as default, `bytearray()` as default, `defaultdict(list)` as default, `deque()` as default, `# noqa` suppression, method (inside class) with mutable default, lambda with mutable default (not supported by tree-sitter query for `function_definition`)

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `default_parameter` and `typed_default_parameter`
- **Why not graph:** Graph does not index parameter default values. This is a syntactic pattern.
- **Query:** Same as current: `[(default_parameter) @default_param (typed_default_parameter) @default_param]`
- **Returns:** List of parameter nodes with their parent function

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection of the `value` child
- **Query:** Check `value.kind()` against expanded `MUTABLE_KINDS` (add `"list"`, `"dictionary"`, `"set"` for literals) AND check if `value.kind() == "call"` and the function name is in `["list", "dict", "set", "bytearray", "deque", "defaultdict", "OrderedDict"]`
- **Returns:** Filtered parameter nodes confirmed as mutable defaults

#### Step 3: False Positive Removal
- **Tool:** Graph query (find the enclosing function symbol, traverse_callers)
- **Query:** If the function has 0 or 1 callers in the graph, downgrade to "info" (single-call functions are less dangerous). Check for `# noqa` comment on the same line.
- **Returns:** Findings with graduated severity

#### Graph Enhancement Required
- None. Existing graph API is sufficient.

### New Test Cases
1. **detects_list_call_default** -- `def foo(items=list()):` -> 1 finding -- Covers: high false negative rate (3)
2. **detects_dict_call_default** -- `def foo(data=dict()):` -> 1 finding -- Covers: high false negative rate (3)
3. **detects_set_call_default** -- `def foo(items=set()):` -> 1 finding -- Covers: high false negative rate (3)
4. **detects_defaultdict_default** -- `def foo(data=defaultdict(list)):` -> 1 finding -- Covers: high false negative rate (3)
5. **detects_deque_default** -- `def foo(q=deque()):` -> 1 finding -- Covers: high false negative rate (3)
6. **skips_noqa_suppressed** -- `def foo(items=[]):  # noqa` -> 0 findings -- Covers: no suppression awareness (11)
7. **method_mutable_default** -- `class C:\n    def m(self, items=[]):` -> 1 finding -- Covers: missing edge cases (6)
8. **detects_set_literal_default** -- `def foo(s={1, 2}):` -> 1 finding -- Covers: missing edge cases (6)

---

## magic_numbers

### Current Implementation
- **File:** `src/audit/pipelines/python/magic_numbers.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `magic_number` -- numeric literals outside exempt contexts
- **Detection method:** Tree-sitter query for `(integer)` and `(float)` nodes; filters through EXCLUDED_VALUES, COMMON_ALLOWED_NUMBERS, exempt context checks (ALL_CAPS assignment, subscript index, collection literal, return, keyword argument, default parameter, common builtin calls), test context checks

### Problems Identified
1. **[Hardcoded thresholds without justification (12)]:** `EXCLUDED_VALUES` contains 28 hardcoded "common" numbers including `256`, `512`, `1024`, `2048`, `4096`, `8192`, `60`, `3600`, `86400`, etc. Some are genuinely common (0, 1, -1) but others like `180`, `360`, `90` are domain-specific (degrees? days?) and may not be "common" in all codebases. No justification or configuration mechanism is provided.
2. **[High false positive rate (2)]:** Despite extensive exemptions, numbers used in mathematical expressions (`radius * 2 * 3.14159`), comparison operators (`if count > 5:`), and augmented assignments (`counter += 1`) are still flagged. The `is_exempt_context` check only looks at immediate parent kinds, missing these common patterns.
3. **[No suppression/annotation awareness (11)]:** No check for `# noqa` or inline comments explaining the number.
4. **[Single-node detection (14)]:** Does not use the graph. A magic number used consistently across many files (e.g., `timeout=30` everywhere) is a higher-priority finding than one used once. Cross-file deduplication via graph could improve signal.
5. **[Literal blindness (8)]:** Negative numbers are not handled as a unit. `-1` is in EXCLUDED_VALUES as `"-1"`, but the tree-sitter AST represents `-42` as a `unary_operator` with a child `integer` of `42`. The pipeline sees `42` (not in EXCLUDED_VALUES) and may flag it incorrectly. This depends on how tree-sitter represents negative numbers.
6. **[No severity graduation (15)]:** All magic numbers are "info" severity. A magic number deeply embedded in business logic (`price * 1.0825` -- tax rate) is more concerning than one in a print format string.
7. **[Missing context (4)]:** The `snippet` field is just the raw number value (e.g., `"9999"`), not the surrounding code context. This makes triage difficult.

### Test Coverage
- **Existing tests:** 8 tests
- **What's tested:** magic number detection, ALL_CAPS constant exemption, common value skip, subscript index skip, keyword argument skip, default parameter skip, float magic number, collection literal skip
- **What's NOT tested:** negative numbers (unary minus), numbers in comparison expressions (`if x > 42`), numbers in augmented assignments (`x += 5`), numbers in f-strings, numbers in decorators (`@retry(max_attempts=3)`), hex/octal/binary literals (`0xFF`), scientific notation (`1e-6`), `# noqa` suppression, test file skip (covered by `is_test_file` but not tested here)

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage for cross-file deduplication)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for numeric literals
- **Why not graph:** Graph does not index numeric literal positions. Syntactic detection required.
- **Query:** `[(integer) @number (float) @number]`
- **Returns:** All numeric literal nodes across all files

#### Step 2: Narrowing
- **Tool:** Tree-sitter parent-chain walk (enhanced `is_exempt_context`)
- **Query:** Expand exempt contexts to include: comparison operators, augmented assignments, f-string expressions, decorator arguments. Also handle unary minus by checking if `node.parent().kind() == "unary_operator"` and concatenating the sign. Check same-line comment for `# noqa`.
- **Returns:** Non-exempt magic numbers with resolved signed values

#### Step 3: False Positive Removal
- **Tool:** Graph query (cross-file frequency analysis)
- **Query:** Collect all magic number values across files. If a specific magic number appears in 3+ files (same value, non-constant), elevate severity to "warning" (indicates a project-wide constant that should be centralized). Single-file occurrences stay "info".
- **Returns:** Findings with graduated severity

#### Graph Enhancement Required
- None directly, but a future enhancement could add `Constant` nodes to the graph for better cross-file analysis.

### New Test Cases
1. **negative_magic_number** -- `x = -42` -> should detect (or not if -42 is composed of unary + 42) -- Covers: literal blindness (8)
2. **hex_literal** -- `mask = 0xDEADBEEF` -> 1 finding -- Covers: missing edge cases (6)
3. **scientific_notation** -- `epsilon = 1e-6` -> 0 findings (should be exempt or configurable) -- Covers: language idiom ignorance (13)
4. **comparison_context** -- `if retries > 42:` -> should be flagged (42 is magic) -- Covers: high false positive rate (2)
5. **noqa_suppressed** -- `x = 9999  # noqa` -> 0 findings -- Covers: no suppression awareness (11)
6. **augmented_assignment** -- `counter += 5` -> evaluate if 5 should be flagged (currently is) -- Covers: missing context (4)
7. **decorator_argument** -- `@retry(max_attempts=3)` -> 0 findings (keyword argument in decorator) -- Covers: missing edge cases (6)
8. **fstring_number** -- `f"page {42} of {100}"` -> should be exempt -- Covers: missing edge cases (6)

---

## god_functions

### Current Implementation
- **File:** `src/audit/pipelines/python/god_functions.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `god_function` -- functions exceeding 50 lines or 20 statements
- **Detection method:** Tree-sitter query for `function_definition` nodes; counts lines from body start/end positions and counts named children of the body node

### Problems Identified
1. **[Hardcoded thresholds without justification (12)]:** `LINE_THRESHOLD = 50` and `STATEMENT_THRESHOLD = 20` are hardcoded with no justification for why these specific values. Industry standards vary (50 lines is quite generous; some standards use 25 or 30). No configuration mechanism.
2. **[No severity graduation (15)]:** All god functions are "warning". A 51-line function is treated identically to a 500-line function. Severity should graduate: 50-100 lines = warning, 100-200 = error, 200+ = critical.
3. **[Single-node detection (14)]:** Does not use the graph. Graph-based analysis could determine: (a) how many callers the function has (high fan-in god functions are more urgent), (b) whether the function has high cyclomatic complexity (large but simple functions are less problematic).
4. **[Missing context (4)]:** Does not consider the function's role. `__init__` methods in data classes legitimately have many assignment statements. Test setup functions often have many statements. These should be treated differently.
5. **[Language idiom ignorance (13)]:** Python `@property` methods, `__init__`, `__str__`, and other dunder methods often have specific structural reasons for being long. No exemptions for these patterns.
6. **[No suppression/annotation awareness (11)]:** No check for `# noqa` or any suppression comment.
7. **[High false positive rate (2)]:** Counts ALL named children of `block` as "statements". In Python, comments and docstrings are also named children. A function with a long docstring and many comments but few actual statements would be flagged incorrectly on the statement count.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** long function (>50 lines), clean small function, many statements (>20)
- **What's NOT tested:** function at exact threshold boundary (50 lines, 20 statements), function exceeding both thresholds simultaneously, `__init__` method with many assignments, function with long docstring inflating line count, decorated function, async function, nested function, lambda (not matched by query), class method vs standalone function difference

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `function_definition`
- **Why not graph:** Graph has symbol start/end lines but not statement counts or body analysis.
- **Query:** Current query: `(function_definition name: (identifier) @fn_name parameters: (parameters) @params body: (block) @fn_body) @fn_def`
- **Returns:** Function definitions with their body nodes

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST analysis (enhanced counting)
- **Query:** Count lines (excluding docstring lines and blank lines), count statements (excluding `expression_statement` containing only a `string` node -- docstrings). Exempt `__init__` if the parent class has >10 attributes. Exempt test setup functions.
- **Returns:** Functions exceeding adjusted thresholds

#### Step 3: False Positive Removal
- **Tool:** Graph query (fan-in analysis)
- **Query:** For each god function, use `find_symbol()` + `traverse_callers()` to count callers. God functions with high caller count get severity "error"; those with 0-1 callers get "info". Also check if function has associated `function_cfgs` and compute cyclomatic complexity -- high-complexity god functions are worse.
- **Returns:** Findings with graduated severity based on callers and complexity

#### Graph Enhancement Required
- None. Existing `find_symbol`, `traverse_callers`, and `function_cfgs` are sufficient.

### New Test Cases
1. **boundary_50_lines_no_finding** -- function with exactly 50 body lines -> 0 findings -- Covers: hardcoded thresholds (12)
2. **boundary_51_lines_finding** -- function with 51 body lines -> 1 finding -- Covers: hardcoded thresholds (12)
3. **init_method_many_assignments** -- `__init__` with 25 assignments -> should be downgraded or exempt -- Covers: language idiom ignorance (13)
4. **function_with_long_docstring** -- 20-line docstring + 15 actual code lines -> should not trigger on statement count -- Covers: high false positive rate (2)
5. **both_thresholds_exceeded** -- function exceeding both line and statement thresholds -> message mentions both -- Covers: missing edge cases (6)
6. **test_setup_function** -- `setUp` method with many statements -> should be downgraded -- Covers: missing context (4)
7. **async_function** -- `async def` with many lines -> should detect -- Covers: missing edge cases (6)
8. **severity_graduation** -- 200-line function -> "error" severity, 51-line function -> "warning" -- Covers: no severity graduation (15)

---

## missing_type_hints

### Current Implementation
- **File:** `src/audit/pipelines/python/missing_type_hints.rs`
- **Trait type:** `GraphPipeline` (DOES use graph -- the only pipeline that does)
- **Patterns detected:** `missing_return_type`, `missing_param_type` -- public functions missing parameter or return type annotations
- **Detection method:** Two-phase: (1) Tree-sitter query finds `function_definition` nodes, checks for missing return type annotation and untyped parameters (excluding `self`, `cls`, and splat patterns). (2) Graph-based filter via `is_cross_module_api()` that checks if the function is in an API-facing file or has cross-module callers via `traverse_callers()`.

### Problems Identified
1. **[High false negative rate (3)]:** The `is_cross_module_api` filter suppresses findings for functions with no cross-module callers in the graph. However, call graph resolution is name-based (heuristic), so functions called indirectly (via dict dispatch, decorators, `getattr`, callbacks) will have no callers in the graph and be silently suppressed. This is especially common in Python codebases using frameworks like Flask/Django where route handlers are registered by decorator, not by direct call.
2. **[Language idiom ignorance (13)]:** Does not recognize or handle: `@overload` decorated functions (where type hints are on the overloads, not the implementation), `Protocol` classes, `@abstractmethod`, `@property` (return type on property is the getter's return type), `__init__` (return type is always `-> None`).
3. **[No suppression/annotation awareness (11)]:** No check for `# type: ignore`, `# noqa`, or `py.typed` marker files. Also does not check if the file has `from __future__ import annotations`.
4. **[Missing context (4)]:** The snippet is a generic `"def {fn_name}(...)"` string, not the actual code. This loses information about which parameters are typed vs untyped.
5. **[No severity graduation (15)]:** All findings are "info". A public API function in `__init__.py` missing all type hints is more serious than a single missing return type on a helper.
6. **[Weak test assertions (5)]:** The test `context_with_empty_graph_suppresses_non_api_findings` uses an empty graph, which means it only tests the "no callers = suppress" path. There is no test with a populated graph showing that a function WITH cross-module callers is kept.
7. **[High false positive rate (2)]:** Functions in API-facing files (`__init__.py`, `/api/`, `/views/`, `/routes/`, `/endpoints/`) are always flagged regardless of whether they are actually part of the public API. A private helper in `views/helpers.py` that happens to live in the `/views/` directory would be flagged.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** missing param + return, fully typed, private function skip, self param skip, missing return only, tree-sitter base check, empty graph suppression, `__init__.py` API file detection
- **What's NOT tested:** populated graph with cross-module caller (positive case), `@overload` decorated function, `@property` method, `@abstractmethod`, `*args` and `**kwargs` parameters, class methods (`@classmethod`), static methods (`@staticmethod`), nested function, `# type: ignore` suppression, function in `/api/` subdirectory with no actual callers

### Replacement Pipeline Design
**Target trait:** GraphPipeline (already uses graph, enhance usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `function_definition`
- **Why not graph:** Graph has symbol data but not parameter-level type annotation info. Tree-sitter needed for AST inspection.
- **Query:** Current query: `(function_definition name: (identifier) @fn_name parameters: (parameters) @params body: (block) @fn_body) @fn_def`
- **Returns:** All function definitions

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection (enhanced)
- **Query:** Same as current, plus: skip `@overload`-decorated functions, skip `@property` return type check (handled separately), recognize `__init__` as always `-> None`. Check preceding comment for `# type: ignore`.
- **Returns:** Functions with genuinely missing type hints

#### Step 3: False Positive Removal
- **Tool:** Graph query (enhanced cross-module detection)
- **Query:** Use `find_symbol()` + `traverse_callers(depth=2)` instead of depth=1 to catch indirect callers. For functions in framework-pattern files, check for decorator names that indicate route registration (`@app.route`, `@router.get`, etc.) -- these are always API functions even without direct callers.
- **Returns:** Findings with graduated severity: public API = "warning", internal with callers = "info"

#### Graph Enhancement Required
- Consider adding decorator information to `NodeWeight::Symbol` (e.g., a `decorators: Vec<String>` field) so the graph can be queried for framework-registered functions without re-parsing the AST.

### New Test Cases
1. **graph_with_cross_module_caller** -- function with a caller from another file in the graph -> finding kept -- Covers: weak test assertions (5)
2. **overloaded_function** -- `@overload` decorated function -> skip -- Covers: language idiom ignorance (13)
3. **property_method** -- `@property` with missing return type -> special handling -- Covers: language idiom ignorance (13)
4. **type_ignore_suppressed** -- `def foo(x):  # type: ignore` -> 0 findings -- Covers: no suppression awareness (11)
5. **init_missing_return_only** -- `__init__` without `-> None` -> lower severity or skip -- Covers: language idiom ignorance (13)
6. **args_kwargs_untyped** -- `def foo(*args, **kwargs):` -> should flag -- Covers: missing edge cases (6)
7. **classmethod_static** -- `@classmethod def foo(cls, x):` -> should check x but skip cls -- Covers: missing edge cases (6)
8. **api_dir_private_function** -- `_helper` in `/api/utils.py` -> should be skipped (private) -- Covers: high false positive rate (2)

---

## stringly_typed

### Current Implementation
- **File:** `src/audit/pipelines/python/stringly_typed.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `stringly_typed_comparison` -- string comparisons on variables with suspicious names (status, kind, type, mode, state, action, level, category, role, variant, phase, stage) where 3+ distinct string values are compared
- **Detection method:** Tree-sitter query for `comparison_operator` nodes; examines children for a string literal + identifier/attribute pair. Collects comparisons per variable name. Emits findings only when 3+ distinct string values are compared against the same variable.

### Problems Identified
1. **[High false negative rate (3)]:** Only detects `==` comparisons via `comparison_operator`. Misses `in` membership tests (`if status in ["active", "inactive", "pending"]`), `match`/`case` statements (Python 3.10+), and `dict` lookups using string keys that act as dispatch tables.
2. **[No scope awareness (7)]:** Comparisons are collected file-wide, not per-function or per-scope. If `status == "active"` appears in function A and `status == "inactive"` in function B and `status == "pending"` in function C, they are grouped together even though they may refer to completely different `status` variables.
3. **[Single-node detection (14)]:** Does not use the graph. Graph could determine if the same variable is compared against strings across multiple files (cross-file stringly-typed pattern), which is a stronger signal.
4. **[Missing compound variants (9)]:** Does not detect `if/elif` chains using `isinstance()` with string type names, `getattr()` dispatch patterns, or `type(x).__name__ == "..."` patterns -- all common Python stringly-typed anti-patterns.
5. **[No suppression/annotation awareness (11)]:** No check for `# noqa` comments.
6. **[Language idiom ignorance (13)]:** Python `Enum` values are often compared using `.value` attribute which produces string comparisons. If the codebase uses `Status.ACTIVE.value == status`, this is flagged but is actually proper enum usage.
7. **[Hardcoded thresholds without justification (12)]:** The threshold of 3+ distinct string values is hardcoded. In some codebases, even 2 distinct values for a `status` field warrant an enum.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** status with 3+ comparisons, attribute comparison (obj.state), few comparisons (1), two comparisons (2), numeric comparison, non-suspicious name
- **What's NOT tested:** `in` membership test, `match`/`case` statement, comparisons across different scopes/functions, Enum `.value` access, `# noqa` suppression, chained comparison (`a == "x" or a == "y"`), f-string or formatted string comparison

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `comparison_operator` + `match_statement` (Python 3.10+) + `if` with `in` operator
- **Why not graph:** Graph does not index comparison expressions. Syntactic detection required.
- **Query:** Extended comparison query + `(match_statement subject: (_) @match_subject)` + `(comparison_operator) @comparison` where operator is `in` and right side is a list/tuple of strings
- **Returns:** All string comparison patterns

#### Step 2: Narrowing
- **Tool:** Tree-sitter scope-aware grouping
- **Query:** Group comparisons by (enclosing_function, variable_name) instead of just variable_name. This prevents cross-function false grouping. Count distinct string values per group.
- **Returns:** Scoped comparison groups with 3+ distinct values

#### Step 3: False Positive Removal
- **Tool:** Graph query (cross-file analysis)
- **Query:** For each stringly-typed variable, check if any module in the project defines an `Enum` class with matching value members (via `find_symbols_by_name`). If an enum exists, suggest migration rather than just flagging.
- **Returns:** Findings with actionable suggestions

#### Graph Enhancement Required
- Consider adding Enum member values to the graph (as properties of Class nodes with kind=Enum) to enable automated enum-existence checking.

### New Test Cases
1. **in_membership_test** -- `if status in ["active", "inactive", "pending"]:` -> 1 finding -- Covers: high false negative rate (3)
2. **match_case_string** -- `match status: case "active": ... case "inactive": ... case "pending": ...` -> 1 finding -- Covers: high false negative rate (3)
3. **cross_function_no_grouping** -- status compared in 3 separate functions -> 0 findings (different scopes) -- Covers: no scope awareness (7)
4. **enum_value_comparison** -- `if status == Status.ACTIVE.value:` -> annotate differently -- Covers: language idiom ignorance (13)
5. **noqa_suppressed** -- comparison with `# noqa` -> 0 findings -- Covers: no suppression awareness (11)
6. **chained_or_comparison** -- `if x == "a" or x == "b" or x == "c":` -> 1 finding -- Covers: missing compound variants (9)
7. **dict_dispatch** -- `handlers = {"active": handle_active, "inactive": handle_inactive, "pending": handle_pending}` -> detect as dispatch pattern -- Covers: missing compound variants (9)

---

## deep_nesting

### Current Implementation
- **File:** `src/audit/pipelines/python/deep_nesting.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `excessive_nesting_depth` -- nesting depth > 4 levels of `if_statement`, `for_statement`, `while_statement`, `with_statement`, `try_statement`
- **Detection method:** Recursive tree walk counting nesting depth. When depth exceeds threshold and the current node is a nesting kind, emits a finding and stops recursing that branch (prevents duplicate reports).

### Problems Identified
1. **[Hardcoded thresholds without justification (12)]:** `NESTING_THRESHOLD = 4` is hardcoded. Some codebases have stricter standards (3) or more lenient ones (5). No configuration mechanism.
2. **[No severity graduation (15)]:** All deep nesting findings are "warning". Nesting depth 5 is different from nesting depth 10. Should graduate severity.
3. **[Single-node detection (14)]:** Does not use the graph. Could use graph to check function complexity (a deeply nested function with high cyclomatic complexity is worse than one with simple linear nesting).
4. **[Missing context (4)]:** The finding does not identify which function contains the deep nesting. The `file_path` and `line` are given but the enclosing function name would aid triage.
5. **[Language idiom ignorance (13)]:** In Python, `with` statements for context managers are often nested legitimately (e.g., `with open(f1) as a:\n    with open(f2) as b:` or multiple `with` items which are syntactically separate in some Python versions). Python 3.10+ supports `with (open(f1) as a, open(f2) as b):` as a single statement. The pipeline counts each `with_statement` as a nesting level even when they could be combined.
6. **[No suppression/annotation awareness (11)]:** No check for `# noqa` comments.
7. **[High false positive rate (2)]:** `function_definition` is NOT in `NESTING_KINDS`, which is correct. However, the nesting count starts from the module root. If a function is defined inside an `if __name__ == "__main__":` block, it already has depth 1 before any code runs, making it easier to trigger the threshold.
8. **[Missing compound variants (9)]:** Does not count `elif` as a nesting level (it shouldn't, but `elif` chains can still produce arrow patterns). Also does not count `except_clause` as nesting (debatable -- code inside `except:` adds visual nesting).

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** 5 nested ifs (depth 5, detected), 4 nested ifs (depth 4, clean), mixed control flow nesting, flat function
- **What's NOT tested:** nested `with` statements, nesting inside `if __name__ == "__main__"`, `try/except/finally` nesting, nesting in class method, `async for` / `async with`, depth exactly at threshold boundary, depth far exceeding threshold (severity graduation), `# noqa` suppression, function name in finding message

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter recursive walk (same as current)
- **Why not graph:** Graph does not track nesting depth. This is inherently syntactic.
- **Query:** Walk tree, count nesting kinds
- **Returns:** Nodes exceeding nesting threshold

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection (enhanced)
- **Query:** For each deeply nested node, find the enclosing `function_definition` to include function name in the finding. Collapse adjacent `with_statement` blocks into a single nesting level. Skip nesting inside `if __name__ == "__main__":` guards.
- **Returns:** Findings with function context and adjusted nesting counts

#### Step 3: False Positive Removal
- **Tool:** Graph query (complexity correlation)
- **Query:** For each enclosing function, look up its CFG from `function_cfgs` and compute cyclomatic complexity. Deep nesting + high complexity = "error". Deep nesting + low complexity (linear nesting like context managers) = "info".
- **Returns:** Severity-graduated findings

#### Graph Enhancement Required
- None. Existing `find_symbol` and `function_cfgs` are sufficient.

### New Test Cases
1. **nested_with_statements** -- 5 nested `with` statements -> 1 finding (but note context manager nesting) -- Covers: language idiom ignorance (13)
2. **inside_main_guard** -- deeply nested code inside `if __name__ == "__main__":` -> adjusted counting -- Covers: high false positive rate (2)
3. **function_name_in_message** -- deeply nested code in `def process_data():` -> message includes "process_data" -- Covers: missing context (4)
4. **severity_depth_5** -- depth 5 -> "warning" -- Covers: no severity graduation (15)
5. **severity_depth_8** -- depth 8 -> "error" -- Covers: no severity graduation (15)
6. **async_with_nesting** -- `async with` counted as nesting -> detected -- Covers: missing edge cases (6)
7. **try_except_finally_nesting** -- `try` + `except` + nested `if` -> proper counting -- Covers: missing compound variants (9)
8. **noqa_suppressed** -- deeply nested with `# noqa` -> 0 findings -- Covers: no suppression awareness (11)

---

## duplicate_logic

### Current Implementation
- **File:** `src/audit/pipelines/python/duplicate_logic.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `similar_function_signature` -- 3+ functions with identical normalized parameter signatures
- **Detection method:** Tree-sitter query for `function_definition`; normalizes parameter lists by type annotations (or parameter names if untyped). Groups by normalized signature string. Emits findings when 3+ functions share a signature. Skips trivial signatures (empty or sole `self`/`cls`).

### Problems Identified
1. **[Broken detection (1)]:** The normalization logic is fundamentally flawed. For untyped parameters (`identifier` kind), it uses the parameter NAME as the signature component (line 33-36). This means `def foo(name, age, email)` and `def bar(name, age, email)` share a signature because they have the same parameter names -- but `def baz(x, y, z)` would not match even if the functions do identical work. Signature matching should be based on arity and type annotations, not parameter names.
2. **[High false positive rate (2)]:** Matching by parameter names means common patterns like `def get_by_id(id, db)` and `def delete_by_id(id, db)` and `def update_by_id(id, db)` would be flagged as "duplicate logic" even though they perform entirely different operations. Parameter name coincidence is not evidence of duplication.
3. **[Single-node detection (14)]:** Does not use the graph. This is a single-file check only. True duplicate logic detection requires comparing function bodies across files, which the graph (or at minimum cross-file analysis) could enable.
4. **[No data flow tracking (10)]:** Does not compare function bodies at all. Two functions could have identical signatures but completely different implementations. True duplicate detection requires body similarity analysis (e.g., AST structure hashing).
5. **[Hardcoded thresholds without justification (12)]:** Threshold of 3+ matching functions is hardcoded. Also, `non_self.len() < 2` minimum parameter count means single-parameter functions are never checked.
6. **[No suppression/annotation awareness (11)]:** No check for `# noqa` comments.
7. **[Missing context (4)]:** The finding message lists all functions sharing the signature, but does not show file paths for cross-reference (since this is single-file, they are all in the same file, but still).
8. **[Overlapping detection (16)]:** This pipeline overlaps significantly with the `duplicate_code` pipeline in the code-style category. Both aim to detect duplication but use different approaches.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** 3 functions with identical parameter names, different signatures (negative), `self`-only methods (negative)
- **What's NOT tested:** typed parameters sharing type signatures, 2 functions (below threshold), functions with `*args`/`**kwargs`, methods inside a class sharing signatures, cross-file scenario (would require graph), functions with default parameters, single-parameter functions (excluded by minimum), `# noqa` suppression

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage for cross-file analysis)

#### Step 1: File Identification
- **Tool:** Graph query (symbol enumeration)
- **Why not tree-sitter first:** The graph already contains all function symbols with their names, kinds, and file locations. We can enumerate all functions from the graph without re-parsing.
- **Query:** Iterate `graph.file_entries()` to collect all Function/Method symbols. For each, retrieve the AST via tree-sitter to extract body structure.
- **Returns:** All function symbols with metadata

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST structural hashing
- **Query:** For each function body, compute a normalized AST hash: replace all identifiers with placeholders, strip comments, hash the resulting structure. Group by (arity, body_hash). Functions with identical arity AND similar body structure (hash match or edit distance < threshold) are genuine duplicates.
- **Returns:** Groups of structurally similar functions

#### Step 3: False Positive Removal
- **Tool:** Graph query (call graph analysis)
- **Query:** For each duplicate group, check if the functions call each other (one might be a wrapper). If A calls B and has the same structure, it is delegation, not duplication. Use `traverse_callees()` to detect this.
- **Returns:** Genuine duplicates with cross-file references

#### Graph Enhancement Required
- Consider adding a body hash or structural fingerprint to `NodeWeight::Symbol` for fast cross-file comparison without re-parsing.

### New Test Cases
1. **same_names_different_bodies** -- 3 functions with same param names but different bodies -> should NOT flag (or flag with low confidence) -- Covers: broken detection (1)
2. **same_types_different_names** -- 3 functions with `(x: int, y: str)`, `(a: int, b: str)`, `(foo: int, bar: str)` -> should flag based on type signature -- Covers: broken detection (1)
3. **arity_based_matching** -- 3 untyped functions with 4 params each (different names) -> should flag based on arity -- Covers: high false positive rate (2)
4. **wrapper_delegation** -- `def foo(x, y): return bar(x, y)` with same signature as `bar` -> should not flag -- Covers: missing context (4)
5. **cross_file_duplicates** -- identical functions in different files -> should detect (requires graph) -- Covers: single-node detection (14)
6. **noqa_suppressed** -- function with `# noqa` -> suppressed -- Covers: no suppression awareness (11)
7. **two_functions_below_threshold** -- only 2 matching functions -> 0 findings -- Covers: missing edge cases (6)
8. **typed_default_parameters** -- 3 functions with `(x: int = 0, y: str = "")` -> should match on type -- Covers: missing edge cases (6)

---

## test_assertions

### Current Implementation
- **File:** `src/audit/pipelines/python/test_assertions.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `missing_assertion` (test functions with no assertions), `trivial_assertion` (tests with `assert True/False/None/1/0`)
- **Detection method:** Tree-sitter query for `function_definition`; filters to `test_*` functions in test files. Recursively walks body to find assertions (`assert_statement`, `self.assert*()`, `pytest.raises/warns/approx`, `with` blocks containing `raises`/`warns`). Separately detects trivial assertions.

### Problems Identified
1. **[High false negative rate (3)]:** The `contains_assertion` function checks for `self.assert*()` but does not detect third-party assertion libraries: `assertpy` (`assert_that(x).is_equal_to(y)`), `expects` (`expect(x).to.equal(y)`), `hamcrest` (`assert_that(x, is_(y))`), `sure` (`x.should.equal(y)`). These are common in Python test suites.
2. **[Language idiom ignorance (13)]:** Does not recognize `unittest.mock.assert_called_with()`, `mock.assert_called_once()`, and other mock assertion methods as valid assertions. A test that sets up a mock and verifies it was called correctly would be flagged as "missing assertion".
3. **[No suppression/annotation awareness (11)]:** No check for `# noqa` or `@pytest.mark.skip`/`@pytest.mark.xfail` decorators. Skipped/xfail tests intentionally have no assertions.
4. **[No severity graduation (15)]:** Both missing and trivial assertions are "warning". A test with `assert True` as a placeholder (with a TODO comment) is different from a test that simply forgot assertions.
5. **[Weak test assertions (5)]:** The trivial assertion check only looks for `assert True/False/None/1/0`. It misses `assert ""` (always falsy), `assert []` (always falsy), `assert "constant string"` (always truthy), and `self.assertTrue(True)`.
6. **[High false positive rate (2)]:** The `with` block check for `raises`/`warns` uses substring matching on the full `with_statement` text (line 59). This could match false positives like `with database_raises_error_on_connect():` where "raises" is in the function name but it is not `pytest.raises`.
7. **[Missing context (4)]:** For `missing_assertion`, the pipeline does not distinguish between genuinely empty tests (`pass`) and tests that verify side effects (e.g., verifying a file was created, checking log output). Some tests are integration tests where the "assertion" is that no exception was raised.

### Test Coverage
- **Existing tests:** 14 tests
- **What's tested:** missing assertion (pass body), missing assertion (no assert), skips function with assert, pytest.raises, self.assertEqual, pytest.warns, trivial assert True/False/None/1, real assertion skip, non-test file skip, non-test function skip, mixed trivial and real, multiple test functions
- **What's NOT tested:** `unittest.mock` assertion methods, third-party assertion libraries, `@pytest.mark.skip` decorator, `@pytest.mark.xfail` decorator, `assert "constant string"` (truthy but trivial), parametrized tests, test fixtures that are not test functions, generator/yield-based tests, `self.assertTrue(True)` (trivial but not detected)

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `function_definition` in test files
- **Why not graph:** Graph has function symbols but not body-level assertion analysis.
- **Query:** Same as current. Filter to `test_*` functions.
- **Returns:** Test function definitions

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST walk (enhanced assertion detection)
- **Query:** Expand assertion detection to include: `mock.assert_*()`, `assertpy.assert_that()`, `expect()` calls, `hamcrest` matchers. Also check for `@pytest.mark.skip`/`@pytest.mark.xfail` decorators on the enclosing `decorated_definition` -- skip these. Expand trivial assertion detection to `self.assertTrue(True)`, `self.assertEqual(x, x)`, `assert "literal"`.
- **Returns:** Test functions with genuinely missing or trivial assertions

#### Step 3: False Positive Removal
- **Tool:** Graph query (call graph for side-effect tests)
- **Query:** For test functions flagged as "missing assertion", check if they call functions that are known to raise exceptions (via `traverse_callees()`). If the test calls a function known to raise and is wrapped in `try`/`except`, it may be a "no exception = success" test -- downgrade severity.
- **Returns:** Findings with context-aware severity

#### Graph Enhancement Required
- None. Existing graph API is sufficient.

### New Test Cases
1. **mock_assert_called** -- `def test_x():\n    mock.assert_called_once_with(42)` -> 0 findings -- Covers: language idiom ignorance (13)
2. **pytest_mark_skip** -- `@pytest.mark.skip\ndef test_pending():  pass` -> 0 findings -- Covers: no suppression awareness (11)
3. **pytest_mark_xfail** -- `@pytest.mark.xfail\ndef test_known_bug():  pass` -> 0 findings -- Covers: no suppression awareness (11)
4. **assert_truthy_string** -- `def test_x():\n    assert "always true"` -> trivial_assertion -- Covers: weak test assertions (5)
5. **self_assertTrue_true** -- `def test_x(self):\n    self.assertTrue(True)` -> trivial_assertion -- Covers: weak test assertions (5)
6. **third_party_assertion** -- `def test_x():\n    assert_that(result).is_equal_to(42)` -> 0 findings -- Covers: high false negative rate (3)
7. **with_raises_false_positive** -- `def test_x():\n    with database_raises_error():  pass` -> should still flag (not pytest.raises) -- Covers: high false positive rate (2)
8. **parametrized_test** -- `@pytest.mark.parametrize(...)\ndef test_x(val):  assert val > 0` -> 0 findings -- Covers: missing edge cases (6)

---

## test_pollution

### Current Implementation
- **File:** `src/audit/pipelines/python/test_pollution.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `global_mutable_test_state` (module-level mutable assignments in test files), `mutable_class_fixture` (class-level mutable assignments in `Test*` classes)
- **Detection method:** (1) Walks direct children of module root for `expression_statement` containing `assignment` with mutable RHS (list, dictionary, set literals, or calls to `list`/`dict`/`set`/`defaultdict`/`OrderedDict`). (2) Tree-sitter query for `class_definition` with `Test*` name; walks class body for same patterns.

### Problems Identified
1. **[High false negative rate (3)]:** Does not detect mutable state via other patterns: `CACHE: Dict[str, Any] = {}` (type-annotated assignment uses `type_alias` or `assignment` with different structure), `collections.deque()`, `io.BytesIO()`, custom mutable classes. Also misses `setUpClass` methods that assign to `cls.something = []`.
2. **[No scope awareness (7)]:** Only checks direct children of module/class body. Misses mutable state in nested scopes that leak: `conftest.py` fixtures that return mutable objects, or `setUp` methods that create mutable instance attributes.
3. **[Single-node detection (14)]:** Does not use the graph. Could cross-reference with test functions that mutate the global/class state (via `find_symbols_by_name` + call graph traversal) to determine if the mutable state is actually modified during tests. If the mutable state is never mutated in any test, it is a false positive.
4. **[No suppression/annotation awareness (11)]:** No check for `# noqa` comments or `pytest.fixture` decoration (fixtures are meant to be shared state, and `pytest.fixture(scope="function")` resets per test).
5. **[Language idiom ignorance (13)]:** `frozenset()` and `tuple()` are immutable constructors that are NOT flagged (correct), but `bytes()` and `str()` (also immutable) share similar constructor call syntax and could be confused. More importantly, `pytest.fixture` returning a mutable object per test is safe -- the pipeline cannot distinguish this from a raw global.
6. **[Missing compound variants (9)]:** Does not detect augmented assignment patterns at module level: `existing_list.append(item)` at module level (mutating an imported mutable). Does not detect `global` keyword usage in test functions that modifies module state.

### Test Coverage
- **Existing tests:** 18 tests
- **What's tested:** global list, dict, set(), list(), dict(), defaultdict, collections.OrderedDict (for global pattern); class-level list, dict, set() (for class pattern); string/int/tuple constants (negative); local variable inside function (negative); non-Test class (negative); non-test file (negative); multiple globals; both patterns; severity check
- **What's NOT tested:** type-annotated assignment (`CACHE: Dict = {}`), `deque()`, `BytesIO()`, `setUp`/`setUpClass` creating mutable state, `conftest.py` fixtures, `# noqa` suppression, mutable state that is never actually mutated, `@pytest.fixture` decorated functions returning mutable objects, nested class definitions

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter walk of module/class body (same as current, extended)
- **Why not graph:** Graph does not index individual assignments or their RHS types. Syntactic detection required.
- **Query:** Walk module-level and Test* class-level assignments. Expand mutable detection to include: `deque()`, `BytesIO()`, `Counter()`, and type-annotated assignments where RHS is mutable.
- **Returns:** Mutable state candidates

#### Step 2: Narrowing
- **Tool:** Tree-sitter decorator check + comment check
- **Query:** Check if the assignment is inside a `@pytest.fixture(scope="function")` decorated function (safe, resets per test). Check for `# noqa` on the assignment line.
- **Returns:** Filtered mutable state that is not fixture-scoped

#### Step 3: False Positive Removal
- **Tool:** Graph query (mutation analysis)
- **Query:** For each mutable variable, use `find_symbols_by_name(var_name)` to locate all references. Check if any test function modifies the variable (calls `.append()`, `.update()`, `[key] = ...` etc. on it). If no test mutates the variable, it is likely used as read-only shared data -- downgrade to "info".
- **Returns:** Findings where mutable state is genuinely mutated in tests

#### Graph Enhancement Required
- Consider adding variable/assignment nodes to the graph (currently only functions, classes, methods are symbols). This would enable tracking variable references and mutations.

### New Test Cases
1. **type_annotated_mutable** -- `CACHE: Dict[str, Any] = {}` -> 1 finding -- Covers: high false negative rate (3)
2. **deque_mutable** -- `QUEUE = deque()` -> 1 finding -- Covers: high false negative rate (3)
3. **pytest_fixture_safe** -- `@pytest.fixture\ndef data(): return []` -> 0 findings -- Covers: language idiom ignorance (13)
4. **setup_class_mutable** -- `@classmethod\ndef setUpClass(cls): cls.data = []` -> 1 finding -- Covers: no scope awareness (7)
5. **noqa_suppressed** -- `DATA = []  # noqa` -> 0 findings -- Covers: no suppression awareness (11)
6. **never_mutated_mutable** -- `DEFAULTS = {}` (never mutated in any test) -> "info" severity -- Covers: no severity graduation (15)
7. **conftest_fixture** -- mutable in `conftest.py` -> context-dependent -- Covers: language idiom ignorance (13)
8. **global_keyword_mutation** -- `def test_x(): global DATA; DATA.append(1)` -> finding on DATA definition -- Covers: missing compound variants (9)

---

## test_hygiene

### Current Implementation
- **File:** `src/audit/pipelines/python/test_hygiene.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `excessive_mocking` (>3 `@patch` decorators on a test function), `sleep_in_test` (`time.sleep()` or `asyncio.sleep()` in test context)
- **Detection method:** (1) Recursive walk for `decorated_definition` nodes; counts decorators containing "patch" text; flags if >3 on a `test_*` function. (2) Tree-sitter call query for `time.sleep` and `asyncio.sleep` in test context.

### Problems Identified
1. **[High false positive rate (2)]:** The `excessive_mocking` check uses substring match `decorator_text.contains("patch")` (line 125). This matches ANY decorator containing the word "patch", including `@dispatch`, `@hotpatch`, or custom decorators with "patch" in the name. Should specifically match `mock.patch`, `patch`, `unittest.mock.patch`.
2. **[Hardcoded thresholds without justification (12)]:** The threshold of >3 patches is hardcoded. Some test styles legitimately require 4-5 patches (e.g., testing a function with many external dependencies). No configuration mechanism.
3. **[High false negative rate (3)]:** The `sleep_in_test` check only detects `time.sleep` and `asyncio.sleep`. Misses: `trio.sleep()`, `anyio.sleep()`, `twisted.internet.defer.sleep()`, and custom sleep wrappers. Also misses `os.system("sleep 5")` and `subprocess.run(["sleep", ...])`.
4. **[No suppression/annotation awareness (11)]:** No check for `# noqa` comments.
5. **[No severity graduation (15)]:** `excessive_mocking` is "warning" and `sleep_in_test` is "info". But 10 patches should be more severe than 4 patches. Sleep of 0.001 seconds is different from sleep of 60 seconds.
6. **[Single-node detection (14)]:** Does not use the graph. Could use graph to check if the mocked targets actually exist in the codebase (if they do, the mock is replacing real code; if they are external, the mock is necessary and less concerning).
7. **[Missing compound variants (9)]:** Does not detect `with mock.patch(...)` context manager usage (only detects decorator form). Also does not detect `mock.patch.object()` used as a context manager inside the function body.

### Test Coverage
- **Existing tests:** 13 tests
- **What's tested:** 4 patches detected, mixed patch decorators, 2 patches (skip), 3 patches (skip), non-test function with patches (skip), time.sleep in test, asyncio.sleep in test, sleep in non-test file (skip), sleep in non-test function (skip), sleep in test class method, non-test file exclusion, both patterns, multiple sleeps
- **What's NOT tested:** `with mock.patch(...)` context manager, custom decorator with "patch" in name (false positive), `trio.sleep()`, sleep with very small duration, 10+ patches (severity graduation), `# noqa` suppression, `patch.object` as context manager, `mock.patch.dict` decorator, combination of decorator + context manager patches

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** Tree-sitter query for decorated definitions + call expressions
- **Why not graph:** Graph does not index decorators or sleep calls. Syntactic detection required.
- **Query:** (1) `decorated_definition` nodes, (2) call expressions matching sleep patterns
- **Returns:** Candidate test functions and sleep calls

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection (enhanced)
- **Query:** For patches: match specifically against `mock.patch`, `unittest.mock.patch`, `patch` (imported from mock). Count both decorator patches AND `with mock.patch(...)` context manager usage in the body. For sleep: expand to `trio.sleep`, `anyio.sleep`. Check `# noqa` on decorator/call lines.
- **Returns:** Verified excessive mocking and sleep findings

#### Step 3: False Positive Removal
- **Tool:** Graph query (mock target analysis)
- **Query:** For each mock patch string argument (e.g., `'a.b.c'`), check if the target module exists in the project via `graph.file_nodes`. External targets = necessary mocking, internal targets = potential design issue. Graduate severity: many internal target mocks = "warning", external target mocks = "info".
- **Returns:** Context-aware findings

#### Graph Enhancement Required
- Consider adding decorator metadata to `NodeWeight::Symbol` for easier mock detection without re-parsing.

### New Test Cases
1. **context_manager_patch** -- `def test_x():\n    with mock.patch('a.b'):\n        with mock.patch('c.d'):\n            with mock.patch('e.f'):\n                with mock.patch('g.h'):\n                    pass` -> 1 finding -- Covers: missing compound variants (9)
2. **custom_decorator_false_positive** -- `@hotpatch\n@dispatch\n@route_patch\n@api_patch\ndef test_x():` -> 0 findings -- Covers: high false positive rate (2)
3. **trio_sleep** -- `def test_x():\n    await trio.sleep(1)` -> 1 finding -- Covers: high false negative rate (3)
4. **severity_ten_patches** -- 10 `@mock.patch` decorators -> "error" severity -- Covers: no severity graduation (15)
5. **noqa_suppressed** -- `@mock.patch('a.b')  # noqa\n...` -> patch not counted -- Covers: no suppression awareness (11)
6. **small_sleep_duration** -- `time.sleep(0.001)` -> "info" (acceptable for timing) vs `time.sleep(60)` -> "warning" -- Covers: no severity graduation (15)
7. **mixed_decorator_and_context** -- 2 decorator patches + 2 context manager patches in same test -> total 4, should flag -- Covers: missing compound variants (9)
8. **mock_patch_dict** -- `@mock.patch.dict('os.environ', {})` -> counted as patch -- Covers: missing edge cases (6)

---

## empty_test_files

### Current Implementation
- **File:** `src/audit/pipelines/python/empty_test_files.rs`
- **Trait type:** `GraphPipeline` (does NOT use graph)
- **Patterns detected:** `empty_test_file` -- test files containing no `test_*` functions
- **Detection method:** Checks if file is a test file (via `is_test_file`), excludes `conftest.py` and `__init__.py`, then uses tree-sitter query for `function_definition` and counts those starting with `test_`. If count is 0, emits finding.

### Problems Identified
1. **[High false negative rate (3)]:** Only counts `function_definition` nodes. Does not detect test classes with `test_*` methods. A file containing `class TestSuite:\n    def test_something(self):` has no top-level `test_*` functions but is NOT an empty test file. The query only matches `function_definition`, not methods inside classes.
2. **[Language idiom ignorance (13)]:** Does not account for pytest parametrize on class methods, `unittest.TestCase` subclasses (where test methods are `test_*` methods inside the class), or `pytest` test classes (convention: `class Test*` with `test_*` methods).
3. **[No suppression/annotation awareness (11)]:** No check for `# noqa` or special markers indicating the file is intentionally empty (e.g., a placeholder for future tests).
4. **[Missing context (4)]:** The finding message says "may be an abandoned stub or discovery file" but does not distinguish between: (a) truly empty files (only imports), (b) files with helpers/fixtures but no tests, (c) files that are test utilities imported by other test files.
5. **[High false positive rate (2)]:** Files like `test_utils.py`, `test_helpers.py`, `test_fixtures.py` that contain helper functions for other test files will be flagged as "empty test files" even though they serve a legitimate purpose. The `is_test_file` check only looks at directory/filename patterns.
6. **[No severity graduation (15)]:** All empty test files are "info". A `test_critical_feature.py` with no tests is more concerning than `test_helpers.py`.

### Test Coverage
- **Existing tests:** 8 tests
- **What's tested:** file with only import, empty file, file with only helpers, conftest.py exclusion, `__init__.py` exclusion, non-test file exclusion, file with test function (negative), file with multiple test functions (negative)
- **What's NOT tested:** file with test class (contains `test_*` methods), `test_utils.py` helper file, file with parametrized tests, file with only fixtures, `# noqa` suppression, file with `unittest.TestCase` subclass

### Replacement Pipeline Design
**Target trait:** GraphPipeline (genuine graph usage)

#### Step 1: File Identification
- **Tool:** `is_test_file` check (same as current)
- **Why not graph:** Graph does not have test-file classification. File path heuristic is necessary.
- **Query:** Filter to test files, exclude conftest.py and __init__.py
- **Returns:** Test file candidates

#### Step 2: Narrowing
- **Tool:** Tree-sitter query (enhanced: function_definition + class methods)
- **Query:** Count both top-level `test_*` functions AND `test_*` methods inside classes (walk class bodies). Also detect `unittest.TestCase` subclasses. If any test-like entity exists, the file is not empty.
- **Returns:** Files with zero test entities

#### Step 3: False Positive Removal
- **Tool:** Graph query (import analysis)
- **Query:** For each empty test file, check if it is imported by other test files (via `reverse_file_edges()`). If another test file imports from this file, it is a test utility/helper file, not abandoned. Downgrade severity or suppress entirely.
- **Returns:** Genuinely abandoned test files

#### Graph Enhancement Required
- None. Existing `reverse_file_edges()` is sufficient for import analysis.

### New Test Cases
1. **test_class_with_methods** -- `class TestSuite:\n    def test_something(self): assert True` -> 0 findings -- Covers: high false negative rate (3)
2. **unittest_testcase** -- `class MyTest(unittest.TestCase):\n    def test_x(self): pass` -> 0 findings -- Covers: language idiom ignorance (13)
3. **test_helper_imported** -- `test_utils.py` with helpers imported by `test_main.py` -> suppress or downgrade -- Covers: high false positive rate (2)
4. **test_fixture_file** -- file with only `@pytest.fixture` functions -> context-dependent (fixture file, not abandoned) -- Covers: missing context (4)
5. **noqa_suppressed** -- empty test file with `# noqa: empty_test_file` header -> 0 findings -- Covers: no suppression awareness (11)
6. **parametrized_test** -- `@pytest.mark.parametrize(...)\ndef test_x():` -> file not empty -- Covers: missing edge cases (6)
7. **test_helpers_naming** -- `test_helpers.py` with only non-test functions -> lower severity or suppress -- Covers: high false positive rate (2)
8. **decorated_test_function** -- `@pytest.mark.slow\ndef test_x():` inside `decorated_definition` -> file not empty -- Covers: missing edge cases (6)

---

## Cross-Cutting Observations

### Graph Under-Utilization
Of the 12 pipelines registered in `tech_debt_pipelines()`, **only `missing_type_hints` uses the CodeGraph at all**. The other 11 pipelines are pure tree-sitter detectors wrapped in the `GraphPipeline` trait. This means:
- The audit engine builds the full CodeGraph (including CFGs, taint analysis, resource tracking) for every audit run, even when the tech-debt pipelines do not use it.
- Every pipeline receives `&CodeGraph` but ignores it, wasting the semantic information available.

### Universal Missing Features
These issues apply to ALL 12 pipelines:
1. **No suppression/annotation awareness:** None of the pipelines check for `# noqa`, `# type: ignore`, `# nosec`, `# pragma: no cover`, or any other suppression mechanism.
2. **No configuration/threshold mechanism:** All thresholds are hardcoded constants. There is no way for users to customize thresholds per-project.
3. **Single-file scope:** Except for `missing_type_hints`, all pipelines operate on a single file at a time. Cross-file analysis opportunities are missed everywhere.

### Recommended Priority Order for Replacement
1. **duplicate_logic** -- Broken detection logic (uses parameter names instead of types/arity)
2. **empty_test_files** -- Misses test classes entirely (high false negative)
3. **mutable_default_args** -- Misses common mutable constructors (high false negative)
4. **stringly_typed** -- Misses `in` and `match/case` patterns (high false negative)
5. **missing_type_hints** -- Good graph usage but framework-registered functions are missed
6. **test_assertions** -- Good coverage but misses mock assertions
7. **magic_numbers** -- Extensive exemptions already, needs refinement
8. **god_functions** -- Works correctly, needs severity graduation
9. **bare_except** -- Works correctly, needs re-raise exemption
10. **deep_nesting** -- Works correctly, needs function context
11. **test_hygiene** -- Works correctly, needs precision improvement
12. **test_pollution** -- Works correctly, needs extended mutable detection
