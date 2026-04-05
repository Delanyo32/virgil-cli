# PHP Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 7
- **Trait types used:** All 7 use the legacy `Pipeline` trait (tree-sitter only, no graph access)
- **Overall assessment:** The PHP tech debt pipelines are functional but operate entirely at the single-node/single-file tree-sitter level. None of them leverage the CodeGraph, CFG, or taint engine that are already built and available for PHP. Detection is uniformly shallow -- string prefix matching, AST child counting, or presence/absence of child nodes. There are significant false positive and false negative gaps across all pipelines, particularly around scope awareness, data flow, suppression annotations, and compound pattern variants. Every pipeline should be migrated to `GraphPipeline` to unlock cross-file analysis, call graph traversal, and taint-aware detection.

---

## deprecated_mysql_api

### Current Implementation
- **File:** `src/audit/pipelines/php/deprecated_mysql_api.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `deprecated_mysql_function` -- any function call whose name starts with `mysql_`
- **Detection method:** Tree-sitter `function_call_expression` query, then `fn_name.starts_with("mysql_")` string prefix check on the name capture

### Problems Identified
1. **[High false positive rate]:** The prefix `mysql_` check will match user-defined functions that happen to start with `mysql_`, e.g., `mysql_audit_log_custom()`. There is no verification that the function is actually a built-in PHP mysql_* extension function. (Line 57: `fn_name.starts_with("mysql_")`)
2. **[Missing compound variants]:** Only detects bare `mysql_*()` function calls. Does not detect usage via variable functions (`$fn = 'mysql_connect'; $fn(...)`) or `call_user_func('mysql_connect', ...)`.
3. **[No severity graduation]:** All findings are `"warning"` severity. `mysql_connect` (removed in PHP 7.0) should be `error`; less critical functions like `mysql_field_name` could remain `warning`.
4. **[No suppression/annotation awareness]:** No check for `@phpstan-ignore` or `// phpcs:ignore` or `@deprecated` migration-in-progress annotations on the surrounding code.
5. **[Single-node detection]:** Detection is purely lexical. Does not check whether the call is actually reachable (dead code), wrapped in a version check (`if (function_exists('mysql_connect'))`), or behind a compatibility layer.
6. **[No data flow tracking]:** Cannot detect indirect usage, e.g., a wrapper function that internally calls `mysql_query()` and is used throughout the codebase. The graph's call traversal could surface these.
7. **[Language idiom ignorance]:** Does not flag `mysql_*` constants like `MYSQL_ASSOC`, `MYSQL_NUM` which are also removed in PHP 7.0.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `mysql_connect` detection, `mysql_query` detection, clean `mysqli_connect`, clean PDO usage
- **What's NOT tested:** User-defined functions with `mysql_` prefix (false positive), `call_user_func('mysql_query', ...)`, variable function calls, version-guarded usage, `mysql_*` inside conditionals, multiple deprecated calls in one file

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_entries()` filtered to `Language::Php`
- **Why not higher-ranked tool:** Graph is the highest-ranked tool and is appropriate here.
- **Query:** Iterate `file_entries()`, collect all PHP file paths.
- **Returns:** `Vec<String>` of PHP file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `compile_function_call_query()` on each PHP file
- **Why not higher-ranked tool:** The graph's CallSite nodes record function names, but do not distinguish built-in from user-defined functions. We need the tree-sitter AST to also check for `call_user_func` wrappers and to inspect surrounding context (version guards).
- **Query:** Match `function_call_expression` nodes where the `fn_name` capture matches any of the 45 known deprecated `mysql_*` extension functions (use a `HashSet` lookup, not prefix matching).
- **Returns:** `Vec<(file_path, line, column, fn_name, call_node)>` of candidate deprecated calls.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (AST walk) + Graph (call graph)
- **Query:** For each candidate: (a) Walk ancestors to check if inside `if (function_exists('mysql_connect'))` guard -- if so, downgrade to `info` severity. (b) Check if the call site is inside a function marked `@deprecated` in its docblock -- if so, suppress. (c) Check for `// phpcs:ignore` or `@phpstan-ignore` on the same or preceding line -- if so, suppress.
- **Returns:** Filtered `Vec<AuditFinding>` with severity graduated: removed-in-7.0 functions as `error`, others as `warning`, version-guarded as `info`.

#### Graph Enhancement Required
- **CallSite enrichment:** The current `CallSite` node stores `name`, `file_path`, `line`. It would benefit from also storing whether the callee is a built-in function (requires a built-in function list per language). This would allow graph-only detection without tree-sitter fallback.

### New Test Cases
1. **user_defined_mysql_prefix** -- `function mysql_audit_log() {} mysql_audit_log();` -> no finding -- Covers: High false positive rate
2. **call_user_func_indirect** -- `call_user_func('mysql_connect', ...);` -> 1 finding -- Covers: Missing compound variants
3. **version_guarded_usage** -- `if (function_exists('mysql_connect')) { mysql_connect(...); }` -> 1 finding at `info` severity -- Covers: No severity graduation
4. **phpcs_ignore_suppression** -- `// phpcs:ignore\nmysql_query($sql);` -> no finding -- Covers: No suppression/annotation awareness
5. **mysql_constant_usage** -- `$result = mysql_fetch_array($r, MYSQL_ASSOC);` -> 1 finding for the function, note about constant -- Covers: Language idiom ignorance
6. **multiple_deprecated_in_one_file** -- 5 different `mysql_*` calls -> 5 findings -- Covers: basic regression
7. **wrapper_function_callees** -- `function dbQuery($sql) { return mysql_query($sql); } dbQuery('SELECT 1');` -> finding on `mysql_query`, graph traversal identifies `dbQuery` as transitive user -- Covers: No data flow tracking

---

## error_suppression

### Current Implementation
- **File:** `src/audit/pipelines/php/error_suppression.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `at_operator` -- any `error_suppression_expression` node in the AST
- **Detection method:** Tree-sitter query matching `(error_suppression_expression) @suppress`, reports every occurrence

### Problems Identified
1. **[High false positive rate]:** Reports every single `@` operator usage without context. Some uses are legitimate and intentional, e.g., `@unlink()` where file-not-found is expected and non-critical, or `@session_start()` which is a common PHP pattern. (Line 46: every match produces a finding)
2. **[No scope awareness]:** Does not distinguish `@` in test files, migration scripts, or compatibility layers from production code. A `@` in a test helper is less concerning than in a request handler.
3. **[No suppression/annotation awareness]:** Does not check for comments like `// intentional suppression` or `// @phpcs:ignore` or PHPDoc `@suppressWarnings`.
4. **[No severity graduation]:** All findings are `"warning"`. `@mysql_connect()` (deprecated API + suppression) should be `error`; `@unlink()` (common pattern) could be `info`.
5. **[Missing context]:** The finding message is generic. It should identify what function is being suppressed and whether the error could be handled by a try/catch instead.
6. **[Single-node detection]:** Does not check if there is a surrounding try/catch that could replace the `@`. Does not check if the suppressed expression's return value is checked (e.g., `if (@file_get_contents(...) === false)`).
7. **[Language idiom ignorance]:** `@` before `fopen`, `fclose`, `unlink` is an established PHP idiom for non-critical filesystem operations. These should be `info` severity at most, not `warning`.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Single `@` detection, multiple `@` detection, clean (no `@`) code
- **What's NOT tested:** `@` inside try/catch (false positive), `@` with return value check, `@` on deprecated functions (severity graduation), `@` in test files, intentional suppression comments, `@session_start()` idiom, nested `@`

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_entries()` filtered to PHP files
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** All PHP file paths from `file_entries()`.
- **Returns:** `Vec<String>` of PHP file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `compile_error_suppression_query()`
- **Why not higher-ranked tool:** `error_suppression_expression` is a syntactic construct not represented in the graph's node types (no equivalent in `NodeWeight`).
- **Query:** Match all `error_suppression_expression` nodes. For each, extract the suppressed expression's text (the child node of the `@` expression) to identify what function/operation is being suppressed.
- **Returns:** `Vec<(file_path, line, column, suppressed_expr_text, node)>` of candidates.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (ancestor walk) + Graph (symbol lookup)
- **Query:** For each candidate: (a) Check if the `@` expression is inside a try/catch -- if so, suppress (the try/catch is a better error handling mechanism, and `@` is redundant but not harmful). (b) Check if the suppressed function is in a known-safe set (`unlink`, `fopen`, `fclose`, `session_start`, `mkdir`) -- if so, downgrade to `info`. (c) Check if the return value is tested (parent is `if_statement` condition, assignment followed by comparison) -- if so, downgrade to `info`. (d) Check for `// intentional` or suppression comments on the same or preceding line.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- None strictly required. The error_suppression_expression is a syntactic feature best handled by tree-sitter with graph context for scope/call awareness.

### New Test Cases
1. **at_inside_try_catch** -- `try { @file_get_contents('x'); } catch (\Exception $e) {}` -> no finding or `info` -- Covers: No scope awareness
2. **at_with_return_check** -- `if (@file_get_contents('x') === false) { ... }` -> `info` severity -- Covers: Single-node detection
3. **at_on_deprecated_function** -- `@mysql_connect(...)` -> `error` severity -- Covers: No severity graduation
4. **at_unlink_idiom** -- `@unlink('/tmp/old.txt');` -> `info` severity -- Covers: Language idiom ignorance
5. **at_session_start** -- `@session_start();` -> `info` severity -- Covers: Language idiom ignorance
6. **at_with_suppression_comment** -- `// intentional\n@fopen('x', 'r');` -> no finding -- Covers: No suppression/annotation awareness
7. **nested_at_expressions** -- `@(@fopen('x', 'r'));` -> 2 findings (or 1 consolidated) -- Covers: Missing edge cases in tests

---

## missing_type_declarations

### Current Implementation
- **File:** `src/audit/pipelines/php/missing_type_declarations.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `missing_return_type` and `missing_param_type` -- functions/methods missing return type or parameter type declarations
- **Detection method:** Tree-sitter `function_definition` and `method_declaration` queries. Checks `return_type` field and `type` field on `simple_parameter` children. Skips a hardcoded list of magic methods.

### Problems Identified
1. **[High false negative rate]:** Only checks `simple_parameter` nodes for type absence (line 218: `.filter(|child| child.kind() == "simple_parameter")`). Misses `variadic_parameter` (e.g., `function foo(...$args)`) and `property_promotion_parameter` (PHP 8 constructor promotion, e.g., `public function __construct(public $name)`) which can also lack types.
2. **[Hardcoded thresholds without justification]:** The `MAGIC_METHODS` list (line 14-26) is incomplete. Missing: `__get`, `__set`, `__isset`, `__unset`, `__call`, `__callStatic`, `__set_state` is listed but `__get`/`__set`/`__isset`/`__unset` are not. These magic methods have well-known signatures that make type declarations optional/forced.
3. **[No severity graduation]:** All findings are `"info"`. Public API methods missing types should be `warning` (affects consumers); private internal functions could stay `info`.
4. **[Language idiom ignorance]:** Does not check for PHPDoc `@param` and `@return` annotations. In PHP 7.x codebases, PHPDoc type annotations are a valid alternative to native type declarations. Flagging functions with complete PHPDoc types is a false positive.
5. **[No scope awareness]:** Does not differentiate between public API functions (exported from classes/modules) and private helper functions. Missing types on public methods are more impactful.
6. **[Missing compound variants]:** Does not handle closure/arrow function (`fn($x) => $x + 1`) type checking. PHP 7.4+ arrow functions and closures can also have parameter and return types.
7. **[No suppression/annotation awareness]:** No check for `@phpstan-ignore` or `@psalm-suppress` annotations that indicate intentional type omission.
8. **[Literal blindness]:** The snippet field (line 203: `format!("function {name}(...)")`) is a hardcoded template, not the actual source snippet. This loses context for the user.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Missing return + param types, fully typed function, missing method types, magic method skipping, missing return only, fully typed method
- **What's NOT tested:** Variadic parameters without types, constructor promotion parameters, closures/arrow functions, PHPDoc-typed functions (false positive), public vs private severity, `__get`/`__set`/`__isset`/`__unset` magic methods, union types (PHP 8), intersection types (PHP 8.1), nullable types

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_entries()` filtered to PHP files
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** All PHP file paths.
- **Returns:** `Vec<String>` of PHP file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `compile_function_def_query()` and `compile_method_decl_query()`, plus a new closure/arrow function query
- **Why not higher-ranked tool:** The graph's `Symbol` nodes do not store parameter type information or return type presence. The AST is needed to inspect parameter children and return_type field.
- **Query:** For each function/method/closure/arrow function: check `return_type` field presence, check each parameter child (all parameter kinds: `simple_parameter`, `variadic_parameter`, `property_promotion_parameter`) for `type` field presence. Also check preceding comment/docblock for `@param`/`@return` PHPDoc annotations.
- **Returns:** `Vec<(file_path, line, fn_name, missing_returns: bool, untyped_params: Vec<String>, has_phpdoc_types: bool)>`

#### Step 3: False Positive Removal
- **Tool:** Graph (symbol lookup for visibility) + Tree-sitter (PHPDoc check)
- **Query:** (a) Look up the function/method in the graph via `find_symbol(file_path, start_line)` to check `exported` status. (b) If PHPDoc `@param` and `@return` cover all parameters and the return type, suppress the finding. (c) If method is in the complete magic method set (including `__get`, `__set`, `__isset`, `__unset`, `__call`, `__callStatic`), suppress. (d) Graduate severity: `warning` for exported methods missing types, `info` for private/internal.
- **Returns:** Filtered findings with severity graduation.

#### Graph Enhancement Required
- **Symbol parameter metadata:** Graph `Symbol` nodes could benefit from storing parameter count and whether parameters are typed. This would allow graph-only queries for "functions with many untyped parameters" without tree-sitter fallback.

### New Test Cases
1. **variadic_param_untyped** -- `function foo(...$args) {}` -> finding for untyped variadic param -- Covers: High false negative rate
2. **constructor_promotion_untyped** -- `class Foo { public function __construct(public $name) {} }` -> finding for untyped promoted param -- Covers: High false negative rate
3. **magic_method_get_set** -- `class Foo { public function __get($name) {} public function __set($name, $value) {} }` -> no finding -- Covers: Hardcoded thresholds without justification
4. **phpdoc_typed_function** -- `/** @param int $x @return bool */ function foo($x) { return true; }` -> no finding (PHPDoc covers types) -- Covers: Language idiom ignorance
5. **public_vs_private_severity** -- `class Foo { public function bar($x) {} private function baz($x) {} }` -> `bar` at `warning`, `baz` at `info` -- Covers: No severity graduation
6. **closure_missing_types** -- `$fn = function($x) { return $x; };` -> finding -- Covers: Missing compound variants
7. **arrow_function_missing_types** -- `$fn = fn($x) => $x + 1;` -> finding -- Covers: Missing compound variants
8. **phpstan_ignore_suppression** -- `/** @phpstan-ignore missingType */ function foo($x) {}` -> no finding -- Covers: No suppression/annotation awareness

---

## god_class

### Current Implementation
- **File:** `src/audit/pipelines/php/god_class.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `god_class` -- classes with more than 10 methods
- **Detection method:** Tree-sitter `class_declaration` query, counts `method_declaration` children of the `declaration_list` body node. Threshold: `METHOD_THRESHOLD = 10`.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** The threshold of 10 methods (line 13: `const METHOD_THRESHOLD: usize = 10`) is arbitrary. A class with 11 simple getters/setters is less of a god class than one with 8 complex business logic methods. No justification or configurability. The Python equivalent uses 50 lines or 20 statements which is more nuanced.
2. **[Single-node detection]:** Only counts `method_declaration` children. Does not count properties, constants, or trait-use declarations. A class with 5 methods but 30 properties and 50 constants is also a god class. Does not consider lines of code in the class body.
3. **[High false positive rate]:** Does not account for: (a) interfaces with many method signatures (no bodies), (b) trait declarations with many methods (designed to be mixed in), (c) abstract classes with many abstract methods (thin signatures). Line 67: `child.kind() == "method_declaration"` counts all methods including trivial getters/setters.
4. **[Missing context]:** The finding does not describe what kind of methods the class has. A breakdown (e.g., "8 public, 3 private, 2 abstract") would help the developer decide how to refactor.
5. **[No scope awareness]:** Does not distinguish between test classes (which often have many test methods) and production classes. A `UserControllerTest` with 15 test methods is normal.
6. **[No data flow tracking]:** Does not check method cohesion -- whether the methods share state (use the same properties). A class with 12 methods that all operate on different data is worse than 12 methods sharing 2 properties. The graph could help by analyzing property access patterns.
7. **[Overlapping detection]:** No coordination with the `function_length` complexity pipeline. A god class with short methods is different from one with long methods; severity should account for both.
8. **[Language idiom ignorance]:** Does not filter out trait methods (inherited via `use TraitName;` inside the class body). The class might have 12 methods but 8 of them come from traits -- the class itself only defines 4.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Class with 12 methods (detected), class with 3 methods (clean), exactly 10 methods (clean, boundary test)
- **What's NOT tested:** Interface with 12 abstract methods, trait with 12 methods, abstract class, class with trait-use imports, class with many properties but few methods, test class with many test methods, class at 11 methods (just over threshold)

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- iterate `symbols_by_name` or `file_entries()` to find Class-kind symbols
- **Why not higher-ranked tool:** Graph is the highest-ranked tool. The graph stores `Symbol` nodes with `kind: SymbolKind::Class` which directly identifies classes.
- **Query:** Collect all `NodeIndex` values where `NodeWeight::Symbol { kind: SymbolKind::Class, .. }`. For each, note `file_path`, `start_line`, `end_line`, `name`.
- **Returns:** `Vec<(NodeIndex, String, String, u32, u32)>` -- (node, name, file_path, start_line, end_line).

#### Step 2: Narrowing
- **Tool:** Graph (Contains edges) + Tree-sitter (for detailed child inspection)
- **Why not higher-ranked tool:** The graph's `Contains` edges link symbols to their children. However, to distinguish trait-imported methods from locally-defined methods, and to count properties/constants, tree-sitter is needed for AST-level inspection.
- **Query:** For each class symbol: (a) From graph, follow `Contains` edges to find child `Method` symbols -- count them. (b) From tree-sitter, parse the class body to count: locally-defined methods (excluding inherited trait methods), properties, constants, abstract methods, and total line count. (c) Apply composite threshold: `methods > 15 OR (methods > 10 AND lines > 300) OR (methods + properties + constants > 25)`.
- **Returns:** Candidates with method/property/constant counts and line count.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (AST inspection) + Graph (call graph for cohesion)
- **Query:** (a) If the node is an interface or trait declaration, suppress (they are not god classes, they are contracts/mixins). (b) If file path matches test patterns (`*Test.php`, `*_test.php`, `tests/*`), suppress. (c) Count trait-use declarations and subtract imported methods from the total. (d) Optionally compute method cohesion via graph property access analysis (future enhancement).
- **Returns:** Filtered findings with severity: `error` for extreme (>25 methods), `warning` for moderate (>15 or composite threshold), `info` for borderline (>10 simple methods).

#### Graph Enhancement Required
- **Contains edges for properties/constants:** Verify that `Contains` edges are emitted for `Property` and `Constant` symbol kinds within a class, not just methods. If not, the graph builder should be extended.
- **Trait-use resolution:** A new edge type or node annotation for trait-imported methods would allow graph-only cohesion analysis.

### New Test Cases
1. **interface_many_methods** -- `interface Foo { public function a(); ... (12 methods) }` -> no finding -- Covers: High false positive rate
2. **trait_many_methods** -- `trait Foo { public function a() {} ... (12 methods) }` -> no finding or downgraded -- Covers: High false positive rate
3. **class_with_trait_imports** -- `class Foo { use BigTrait; public function a() {} }` where BigTrait has 10 methods -> no finding (only 1 local method) -- Covers: Language idiom ignorance
4. **test_class_many_methods** -- `class UserTest extends TestCase { public function testA() {} ... (15 test methods) }` -> no finding -- Covers: No scope awareness
5. **class_many_properties_few_methods** -- Class with 3 methods but 30 properties -> finding under composite threshold -- Covers: Single-node detection
6. **severity_graduation** -- Class with 26 methods -> `error` severity; class with 11 methods -> `warning` -- Covers: No severity graduation
7. **abstract_class_many_abstract** -- `abstract class Base { abstract public function a(); ... (12 abstract methods) }` -> `info` severity at most -- Covers: High false positive rate

---

## extract_usage

### Current Implementation
- **File:** `src/audit/pipelines/php/extract_usage.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `extract_call` -- calls to the `extract()` function
- **Detection method:** Tree-sitter `function_call_expression` query, exact match on `fn_name == "extract"` (line 57)

### Problems Identified
1. **[High false positive rate]:** Reports all `extract()` calls equally. `extract($data, EXTR_IF_EXISTS)` with the `EXTR_IF_EXISTS` flag only creates variables that already exist in the current scope -- much safer than `extract($_POST)` which creates arbitrary variables. Similarly, `extract($data, EXTR_PREFIX_ALL, 'data')` prefixes all variables, reducing scope pollution risk. (Line 57: no argument inspection)
2. **[No severity graduation]:** All findings are `"warning"`. `extract($_POST)` or `extract($_GET)` (user input directly into scope) should be `error` (security risk). `extract($localArray)` with prefix flags should be `info`.
3. **[Missing context]:** The finding does not indicate what is being extracted. `extract($_POST)` is a security issue; `extract($config)` is a code quality issue. The message is generic.
4. **[No data flow tracking]:** Does not trace the source of the argument to `extract()`. If the argument comes from `$_POST`, `$_GET`, `$_REQUEST`, this is a security issue (taint), not just a tech debt issue. The taint engine could surface this.
5. **[Single-node detection]:** Does not check for `extract()` in variable function form: `$fn = 'extract'; $fn($data);` or `call_user_func('extract', $data)`.
6. **[No suppression/annotation awareness]:** No check for `@phpstan-ignore` or suppression comments.
7. **[Missing compound variants]:** Does not detect `compact()` in conjunction with `extract()`. `extract()`/`compact()` pairs are a common pattern that indicates the developer is using variable scope manipulation as a data passing mechanism.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** `extract($_POST)` detection, no extract (clean), `compact()` and `array_merge()` (clean)
- **What's NOT tested:** `extract()` with safe flags (`EXTR_IF_EXISTS`, `EXTR_PREFIX_ALL`), `extract()` with user input vs local data, variable function calls, `call_user_func('extract', ...)`, suppression comments

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_entries()` for PHP files
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** All PHP file paths.
- **Returns:** `Vec<String>` of PHP file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `compile_function_call_query()` to find `extract()` calls
- **Why not higher-ranked tool:** Graph CallSite nodes record call names, but we need the AST to inspect arguments (flags and source array). Graph `find_symbols_by_name("extract")` returns function definitions, not call sites.
- **Query:** Match `function_call_expression` where `fn_name == "extract"`. For each match, extract: (a) the first argument text (what is being extracted), (b) the second argument if present (flags like `EXTR_IF_EXISTS`), (c) the third argument if present (prefix).
- **Returns:** `Vec<(file_path, line, first_arg, flags_arg, prefix_arg, call_node)>`

#### Step 3: False Positive Removal
- **Tool:** Graph (taint analysis) + Tree-sitter (argument inspection)
- **Query:** (a) If the first argument is `$_GET`, `$_POST`, `$_REQUEST`, `$_COOKIE`, `$_SERVER`, `$_FILES` -- severity `error` (tainted input into scope). (b) If the second argument contains `EXTR_IF_EXISTS` or `EXTR_PREFIX_ALL` -- downgrade to `info`. (c) Use graph taint engine to check if the first argument has taint provenance (FlowsTo from an ExternalSource) -- if so, `error`. (d) Check for suppression comments.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- **Taint-aware call site arguments:** The current graph stores call site name and line but not argument values. To perform taint analysis on `extract()`'s first argument, the taint engine must be consulted via CFG analysis of the enclosing function. This is already supported via `function_cfgs` and the `TaintEngine`.

### New Test Cases
1. **extract_with_extr_if_exists** -- `extract($data, EXTR_IF_EXISTS);` -> `info` severity -- Covers: High false positive rate
2. **extract_with_prefix** -- `extract($data, EXTR_PREFIX_ALL, 'pfx');` -> `info` severity -- Covers: High false positive rate
3. **extract_user_input** -- `extract($_POST);` -> `error` severity -- Covers: No severity graduation
4. **extract_get_input** -- `extract($_GET);` -> `error` severity -- Covers: No data flow tracking
5. **extract_local_array** -- `$a = ['x' => 1]; extract($a);` -> `warning` severity -- Covers: No severity graduation
6. **call_user_func_extract** -- `call_user_func('extract', $_POST);` -> finding -- Covers: Missing compound variants
7. **extract_with_suppression** -- `/** @phpstan-ignore */ extract($data);` -> no finding -- Covers: No suppression/annotation awareness

---

## silent_exception

### Current Implementation
- **File:** `src/audit/pipelines/php/silent_exception.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `silent_catch` -- catch blocks that catch `Exception` or `Throwable` with empty or return-only bodies
- **Detection method:** Tree-sitter `catch_clause` query. Checks type_list children for `Exception`/`\Exception`/`Throwable`/`\Throwable`. Checks body: empty (0 named children) or single `return_statement`.

### Problems Identified
1. **[High false negative rate]:** Only flags catches of exactly `Exception`, `\Exception`, `Throwable`, or `\Throwable`. Does not flag catches of `\RuntimeException`, `\Error`, `\TypeError` etc. that also have empty bodies. Any empty catch block is suspicious, not just broad exception types. (Line 93-107: `catches_exception` function is too narrow)
2. **[Missing compound variants]:** The "trivial body" check (line 110-122) only considers: (a) completely empty, (b) single `return_statement`. Does not catch: (c) single `continue` statement, (d) single variable assignment (e.g., `$ignored = true;`), (e) single `pass`-equivalent comment-only body. A body with just `// TODO: handle this` should also be flagged.
3. **[No severity graduation]:** All findings are `"warning"`. An empty `catch (Throwable $e) {}` at the top level of a request handler should be `error`. An empty `catch (Exception $e) {}` in a cleanup/finally-like context could be `info`.
4. **[No scope awareness]:** Does not consider the context of the catch. If the catch is in a `__destruct` method or a `finally` equivalent cleanup, swallowing exceptions may be intentional.
5. **[No data flow tracking]:** Does not check if the caught exception variable `$e` is used later (e.g., in a `finally` block or stored for later processing). The graph could track whether the exception variable flows to any subsequent use.
6. **[Missing context]:** The finding does not mention what exception type is caught or what code is in the try block. This context is essential for the developer to assess the risk.
7. **[Language idiom ignorance]:** In PHP, `catch (Exception $e) { return null; }` is a common pattern for "try or return default" logic. While not ideal, it is different from a completely empty catch. The pipeline treats both as equally bad.
8. **[Literal blindness]:** Does not distinguish between `return;` (void return) and `return null;` or `return false;` (default value return). The latter is a recognized pattern in PHP.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Empty catch of Exception, return-only catch, catch with logging (clean), specific exception catch (skipped), Throwable catch
- **What's NOT tested:** Empty catch of RuntimeException, catch with single `continue`, catch with assignment only, catch in `__destruct`, catch with `return false`, multi-type catch (`Exception | Error`), catch where `$e` is used in finally, nested try/catch

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_entries()` for PHP files
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** All PHP file paths.
- **Returns:** `Vec<String>`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `compile_catch_clause_query()`
- **Why not higher-ranked tool:** Catch clauses are syntactic constructs not represented in the graph's node model. No `NodeWeight` variant for exception handling.
- **Query:** Match all `catch_clause` nodes. For each, extract: (a) the exception type(s) from `type_list`, (b) the exception variable name, (c) the body's named child count and kinds, (d) whether the body contains only trivial statements (empty, single return, single continue, single assignment, comment-only).
- **Returns:** `Vec<(file_path, line, exception_types, var_name, body_kind, catch_node)>` where `body_kind` is an enum: `Empty`, `ReturnOnly`, `ContinueOnly`, `AssignmentOnly`, `CommentOnly`, `Substantive`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter (ancestor walk) + Graph (symbol lookup)
- **Query:** (a) If the catch is inside a `__destruct` method, suppress or downgrade to `info`. (b) If the body has a substantive statement count > 1, skip. (c) If the exception variable `$e` is referenced in a subsequent `finally` block (sibling of the try_statement), suppress. (d) If there is a comment in the catch body containing "intentional", "ignore", "expected", suppress. (e) Graduate severity: `error` for Throwable/Exception in request handler context, `warning` for other broad catches, `info` for specific exception types.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Exception handling model:** The graph currently has no representation of try/catch/finally blocks. Adding `TryCatch` or `ExceptionHandler` as a NodeWeight variant with edges to the caught exception types would enable graph-level analysis of exception handling patterns across the codebase.

### New Test Cases
1. **empty_catch_runtime_exception** -- `try {} catch (RuntimeException $e) {}` -> finding (any empty catch is suspicious) -- Covers: High false negative rate
2. **catch_with_continue** -- `foreach ($items as $item) { try { process($item); } catch (Exception $e) { continue; } }` -> finding for trivial catch -- Covers: Missing compound variants
3. **catch_with_assignment** -- `catch (Exception $e) { $failed = true; }` -> finding for trivial catch -- Covers: Missing compound variants
4. **catch_return_false** -- `catch (Exception $e) { return false; }` -> finding at `info` severity (recognized pattern) -- Covers: Literal blindness
5. **catch_in_destruct** -- `public function __destruct() { try { ... } catch (Exception $e) {} }` -> `info` severity -- Covers: No scope awareness
6. **catch_var_used_in_finally** -- `try {} catch (Exception $e) {} finally { log($e); }` -> no finding -- Covers: No data flow tracking
7. **multi_type_catch** -- `catch (Exception | Error $e) {}` -> finding for broad multi-type catch -- Covers: Missing compound variants
8. **catch_with_intentional_comment** -- `catch (Exception $e) { // intentionally ignored }` -> no finding -- Covers: No suppression/annotation awareness

---

## logic_in_views

### Current Implementation
- **File:** `src/audit/pipelines/php/logic_in_views.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `db_in_view` -- database function/method calls in files that also contain HTML output
- **Detection method:** Two-phase: (1) Check if the file has HTML output via `text` nodes (inline HTML) or `echo_statement` nodes. (2) If so, scan for DB function calls (`mysql_query`, `mysqli_query`, `pg_query`, `sqlite_query`) and DB method calls (`query`, `exec`, `execute`, `prepare`, `fetch`, `fetchAll`).

### Problems Identified
1. **[High false positive rate]:** The DB method name list (line 17: `DB_METHODS`) is extremely broad. `->query()`, `->execute()`, `->fetch()`, `->prepare()` match any object's methods with those names, not just database objects. A `$cache->fetch()` call or `$queue->execute()` would trigger a false positive. There is no object type resolution. (Line 17: `DB_METHODS = &["query", "exec", "execute", "prepare", "fetch", "fetchAll"]`)
2. **[High false positive rate]:** The HTML detection heuristic (line 156-168) treats `echo 'plain text'` as HTML output. A CLI script that echoes plain text to stdout and also queries a database would be flagged incorrectly.
3. **[Missing compound variants]:** Only checks `function_call_expression` (bare function calls) and `member_call_expression` (method calls). Does not detect: static method calls (`PDO::query()`), scope resolution (`parent::query()`), or calls through variable variables.
4. **[No data flow tracking]:** Does not trace whether the DB query result actually flows into the HTML output. A file that queries a DB at the top, processes the data, and then outputs HTML at the bottom is flagged -- but the real concern is when raw DB data is directly echoed into HTML. The graph's taint engine could distinguish these cases.
5. **[No scope awareness]:** Does not distinguish between Blade templates, Twig templates, and plain PHP files. In a Blade template context, any PHP logic beyond simple variable display is suspicious; in a controller, mixing query + view rendering is normal for legacy code.
6. **[No severity graduation]:** All findings are `"info"`. Direct `echo $db->query(...)->fetch()['name'];` in HTML should be `warning` or `error`. A controller method that queries and then renders should be `info`.
7. **[Language idiom ignorance]:** In legacy PHP (pre-MVC frameworks), mixing HTML and DB calls in the same file was the standard pattern. This pipeline is most useful for codebases migrating to MVC, but the message does not acknowledge the migration context or suggest specific refactoring patterns.
8. **[Broken detection]:** The `has_html_output` function (line 156) uses the `text_query` to check for inline HTML `(text)` nodes. In the PHP tree-sitter grammar, `text` nodes represent raw text outside `<?php ... ?>` tags. However, a file that is entirely PHP (starts with `<?php` and never closes the tag) would never have `text` nodes, even if it uses `echo` with HTML strings. The echo check partially covers this, but `echo $var;` where `$var` happens to contain HTML would not be detected as HTML output.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** DB function call with inline HTML, method call with echo, no HTML output (clean), no DB calls (clean)
- **What's NOT tested:** `$cache->fetch()` false positive, `echo 'plain text'` false positive, static DB calls, DB query result flowing into echo vs not, Blade/Twig template context, `echo` with variable containing HTML, `PDO::query()` static call

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_entries()` for PHP files
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** All PHP file paths. Additionally, check file path patterns for view/template indicators: `**/views/**`, `**/templates/**`, `**/*.blade.php`, `**/*.twig`.
- **Returns:** `Vec<(String, bool)>` -- (file_path, is_likely_view_file).

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- multiple queries
- **Why not higher-ranked tool:** The graph does not model HTML output or inline HTML. The `text` and `echo_statement` nodes are purely syntactic.
- **Query:** (a) Check for HTML indicators: `text` nodes containing HTML tags (`<` characters), `echo_statement` nodes containing string literals with `<`. This reduces false positives from `echo 'plain text'`. (b) For files with confirmed HTML output: match `member_call_expression` and `function_call_expression` nodes. For member calls, extract the full expression `$obj->method()` to get the object variable name, not just the method name. (c) Use a more precise DB method set: only flag `->query()` when the object is named `$db`, `$pdo`, `$conn`, `$connection`, `$mysqli`, `$stmt`, or similar.
- **Returns:** `Vec<(file_path, line, call_text, object_name, method_name)>` of DB call candidates in HTML-producing files.

#### Step 3: False Positive Removal
- **Tool:** Graph (taint/data flow) + Tree-sitter (object name heuristic)
- **Query:** (a) If the object name does not match known DB object patterns and the method name is generic (`fetch`, `execute`), suppress. (b) Use graph taint engine: check if data flows from the DB call result to an echo/print/text output -- if so, severity `warning`; if not, `info`. (c) If file is in a known view directory but uses an ORM pattern (e.g., Eloquent `Model::all()`), flag as `warning` (business logic in view). (d) Check for MVC framework patterns (Laravel controllers, Symfony controllers) and adjust messaging.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- **HTML output model:** Adding an `HtmlOutput` node weight or edge type that connects to echo/text nodes would allow graph-level analysis of data flow from DB queries to HTML output without tree-sitter re-parsing.
- **Object type inference:** A basic type inference pass in the graph builder that tracks `$var = new PDO(...)` assignments and propagates the type to method calls on `$var` would dramatically reduce false positives for method name matching.

### New Test Cases
1. **cache_fetch_false_positive** -- `<?php $data = $cache->fetch('key'); ?><h1>Data</h1>` -> no finding (not a DB fetch) -- Covers: High false positive rate
2. **echo_plain_text_false_positive** -- `<?php $rows = $db->query('SELECT 1'); echo "done\n";` -> `info` severity (echo is not HTML) -- Covers: High false positive rate
3. **static_pdo_call** -- `<?php $stmt = PDO::prepare('SELECT 1'); ?><h1>Results</h1>` -> finding -- Covers: Missing compound variants
4. **db_result_to_echo_flow** -- `<?php $name = $db->query('SELECT name')->fetch()['name']; echo "<h1>$name</h1>";` -> `warning` (data flows to output) -- Covers: No data flow tracking
5. **controller_query_and_render** -- `<?php $users = User::all(); return view('users', compact('users'));` -> `info` severity -- Covers: No scope awareness
6. **blade_template_with_db** -- File at `resources/views/users.blade.php` with `<?php $db->query(...) ?>` -> `error` severity -- Covers: No severity graduation
7. **html_in_echo_variable** -- `<?php $html = '<h1>Title</h1>'; echo $html;` -> correctly detected as HTML output -- Covers: Broken detection

---

## Overall Cross-Pipeline Issues

### Shared Structural Problems

1. **All 7 pipelines use the Legacy `Pipeline` trait.** They receive only `(tree, source, file_path)` and have no access to the `CodeGraph`. This means no cross-file analysis, no call graph traversal, no taint tracking, and no CFG-based analysis -- all of which are already built and available for PHP.

2. **No pipeline uses the PHP CFG builder** (`src/graph/cfg_languages/php.rs`), which supports if/else, for/foreach/while, switch, try/catch/finally, and return. CFG-based analysis would enable data flow tracking within functions.

3. **No pipeline uses the taint engine** (`src/graph/taint.rs`), which already has PHP-specific sources (`$_GET`, `$_POST`, `$_REQUEST`, `$_COOKIE`, `$_SERVER`, `$_FILES`, `getenv`, `php://input`), sinks (`mysqli_query`, `pg_query`, `preg_replace`, `include`, `require`, `unserialize`), and sanitizers (`htmlspecialchars`, `htmlentities`, `addslashes`, `mysqli_real_escape_string`, `pg_escape_string`, `filter_var`, `filter_input`).

4. **Zero suppression/annotation awareness** across all pipelines. No pipeline checks for `@phpstan-ignore`, `@psalm-suppress`, `// phpcs:ignore`, `// @codingStandardsIgnoreLine`, or intentional-suppression comments.

5. **No severity graduation** -- most pipelines emit a fixed severity regardless of context, risk, or exposure level.

6. **Snippet quality varies.** Some pipelines use `extract_snippet()` from primitives (good), while `missing_type_declarations` uses a hardcoded template string (bad).

### Migration Priority

| Priority | Pipeline | Reason |
|----------|----------|--------|
| 1 | `logic_in_views` | Highest false positive rate, most benefit from graph data flow |
| 2 | `silent_exception` | High false negative rate, would benefit from CFG exception flow analysis |
| 3 | `extract_usage` | Taint engine integration for security-relevant detection |
| 4 | `missing_type_declarations` | PHPDoc awareness and visibility-based severity graduation |
| 5 | `god_class` | Graph symbol counting and cohesion analysis |
| 6 | `deprecated_mysql_api` | Call graph traversal for transitive usage detection |
| 7 | `error_suppression` | Mostly complete as-is; context improvements are incremental |
