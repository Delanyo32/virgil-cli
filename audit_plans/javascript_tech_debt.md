# JavaScript Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 12
- **Trait types used:** All 12 use the legacy `Pipeline` trait (not `NodePipeline` or `GraphPipeline`)
- **Overall assessment:** The pipelines are functional but shallow. Every pipeline operates on single-file, single-node tree-sitter queries with no graph awareness. Most lack suppression/annotation awareness, severity graduation, and scope tracking. False positive rates are moderate to high depending on pipeline. Test coverage is minimal (3-5 tests per pipeline, no edge cases). The entire suite should migrate to `GraphPipeline` or at minimum `NodePipeline` to leverage cross-file analysis and reduce false positives.

---

## var_usage

### Current Implementation
- **File:** `src/audit/pipelines/javascript/var_usage.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `var_usage` -- any `var` declaration
- **Detection method:** Tree-sitter query `(variable_declaration) @var_decl` matches all `var` declarations. In the tree-sitter JavaScript grammar, `variable_declaration` is the node kind for `var` specifically, while `let`/`const` produce `lexical_declaration`. Every match produces a warning.

### Problems Identified
1. **[No suppression/annotation awareness]:** No recognition of `// eslint-disable-next-line no-var`, `/* eslint-disable */`, or `// @ts-ignore`. A codebase that has intentionally suppressed this rule will get duplicate noise. (Lines 38-53, no comment checking.)
2. **[No scope awareness]:** Reports `var` in all contexts equally. A `var` at file/module scope is less harmful than one inside a function. A `var` inside a `for` loop (`for (var i = ...)`) is the most classic case, but the pipeline does not distinguish it from a top-level `var`. (Lines 38-53.)
3. **[No severity graduation]:** Every `var` is "warning" severity regardless of context. A `var` inside a function that is only used within the declaring block is low-risk; a `var` in a loop that leaks into the outer function scope is high-risk. (Line 43.)
4. **[High false positive rate]:** In legacy codebases, `var` is ubiquitous. Reporting every single one without any context or deduplication produces overwhelming noise. No throttling or deduplication.
5. **[Missing compound variants]:** Does not detect `var` inside `for` statements (`for (var i = 0; ...)`) where the variable leaks to function scope -- the most dangerous variant.
6. **[Language idiom ignorance]:** Does not recognize that in strict mode (`"use strict"`) or ES modules (files with `import`/`export`), `var` hoisting is already partially mitigated. Still worth flagging but severity should differ.
7. **[Single-node detection]:** Purely single-node. Does not check if the `var` variable is actually re-assigned (in which case `let` is correct) vs. never re-assigned (in which case `const` is better). The message says "prefer `let` or `const`" but cannot distinguish which.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Detects single `var`, skips `let`, skips `const`, detects multiple `var` declarations.
- **What's NOT tested:** `var` inside `for` loops, `var` in nested functions, `var` at top-level vs. function scope, files with eslint-disable comments, `var` with destructuring patterns, `var` in `catch` clauses, multiple declarators in one `var` statement (`var a = 1, b = 2`).

### Replacement Pipeline Design
**Target trait:** NodePipeline (this is inherently per-node; graph not needed for core detection)

#### Step 1: File Identification
- **Tool:** Tree-sitter query `(variable_declaration) @var_decl`
- **Why not graph:** Graph has Symbol nodes but does not track `var` vs. `let`/`const` declaration kind. This is an AST-level detail.
- **Query:** Same as current `compile_variable_declaration_query()`
- **Returns:** All `variable_declaration` nodes with positions.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST walk on each matched node
- **Query:** For each `variable_declaration` node:
  - Check parent: is it a `for_statement`, `for_in_statement`, `for_of_statement`? Tag as `var_in_loop`.
  - Check if enclosing scope is module-level (parent chain reaches `program` without passing through a function). Tag as `var_at_module_scope`.
  - Check if the variable is never re-assigned in its scope (walk scope body for `assignment_expression` targeting same name). If never re-assigned, suggest `const`; otherwise suggest `let`.
- **Returns:** Enriched findings with pattern sub-type (`var_in_loop`, `var_at_module_scope`, `var_in_function`) and suggested replacement (`const` or `let`).

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** For each finding, check preceding sibling nodes and parent's preceding siblings for comment nodes containing `eslint-disable`, `eslint-disable-next-line no-var`, or `@ts-ignore`.
- **Returns:** Filtered findings with suppressed entries removed.

#### Graph Enhancement Required
None -- this pipeline does not benefit from graph data.

### New Test Cases
1. **var_in_for_loop** -- `for (var i = 0; i < 10; i++) {}` -> detected, pattern `var_in_loop`, severity `warning` -- Covers: missing compound variants
2. **var_at_module_scope** -- `var config = {};` (top-level) -> detected, severity `info` -- Covers: no scope awareness, no severity graduation
3. **var_with_eslint_disable** -- `// eslint-disable-next-line no-var\nvar x = 1;` -> suppressed, no finding -- Covers: no suppression/annotation awareness
4. **var_never_reassigned** -- `function f() { var x = 1; return x; }` -> message suggests `const` -- Covers: single-node detection
5. **var_reassigned** -- `function f() { var x = 1; x = 2; return x; }` -> message suggests `let` -- Covers: single-node detection
6. **var_multiple_declarators** -- `var a = 1, b = 2;` -> one finding per `var` statement (not per declarator) -- Covers: missing edge cases in tests
7. **var_in_catch** -- `try {} catch(e) { var msg = e.message; }` -> detected with `var_in_function` context -- Covers: missing edge cases in tests
8. **var_with_destructuring** -- `var { a, b } = obj;` -> detected -- Covers: missing edge cases in tests

---

## callback_hell

### Current Implementation
- **File:** `src/audit/pipelines/javascript/callback_hell.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `nested_callback` -- callback nesting depth exceeding threshold (3)
- **Detection method:** Manual recursive AST walk (`walk_tree`). Counts depth of `arrow_function` or `function_expression` nodes whose parent is `arguments`. When depth exceeds `NESTING_THRESHOLD` (3), reports a finding at depth 4+. Uses early return to stop recursing once a finding is reported.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `NESTING_THRESHOLD = 3` (line 9) is hardcoded. No justification or configurability. The description says ">3 levels" but the condition is `new_depth > NESTING_THRESHOLD` which means depth 4+ triggers it. This is reasonable but not configurable.
2. **[High false positive rate]:** Express middleware chains, gulp tasks, and Mocha test suites commonly use deeply nested callbacks intentionally. No awareness of test contexts (`describe`/`it` nesting) or common frameworks.
3. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line max-nested-callbacks` or similar suppression comments. (Lines 25-52.)
4. **[Missing compound variants]:** Does not detect mixed callback + promise patterns (`.then(() => { fs.readFile(..., (err, data) => { ... })})`). The nesting depth resets between `.then()` and the callback.
5. **[No severity graduation]:** Every finding is "warning" regardless of depth. Depth 4 is mildly concerning; depth 8 is an emergency. (Line 44.)
6. **[Single-node detection]:** Reports the innermost callback node. Does not report the entire callback chain. A developer seeing line N has no context about the outer chain without reading the snippet.
7. **[Overlapping detection]:** The early `return` on line 51 prevents reporting multiple findings for the same chain, but if there are two separate deep chains in one function, only the first one encountered in tree-walk order gets reported for the deepest point -- sibling chains are handled correctly, but chains that share a common prefix will only flag once.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Deep nesting with `function(){}`, shallow nesting (3 levels, not flagged), deep nesting with arrow functions, flat code.
- **What's NOT tested:** Mixed arrow + function expression nesting, callbacks inside `.then()` chains, nesting inside test contexts (describe/it), nesting at exactly threshold boundary (depth 3 = 3 callbacks), callbacks in event handlers, callbacks with named functions (not detected because named functions outside `arguments` parent wouldn't match).

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter -- manual AST walk (same approach)
- **Why not graph:** Callback nesting is a structural AST pattern. The graph does not encode nesting depth of anonymous callback functions.
- **Query:** Walk the full AST, tracking depth of `arrow_function`/`function_expression` nodes whose parent is `arguments`.
- **Returns:** All nodes where callback depth exceeds threshold, with exact depth count.

#### Step 2: Narrowing
- **Tool:** Tree-sitter context checking
- **Query:** For each finding:
  - Check if inside a test context (walk ancestors for `call_expression` with callee `describe`, `it`, `test`) -- if so, suppress or reduce severity.
  - Compute the full chain: walk up the callback chain to find the outermost callback, include all intermediate function names in the message.
  - Graduate severity: depth 4-5 = "info", depth 6-7 = "warning", depth 8+ = "error".
- **Returns:** Findings with graduated severity and enriched context.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check for suppression comments (`eslint-disable`, `max-nested-callbacks`).
- **Returns:** Filtered findings.

#### Graph Enhancement Required
None.

### New Test Cases
1. **mixed_arrow_and_function** -- Arrow inside function expression inside arrow -> depth 3 -> no finding (at threshold) -- Covers: missing edge cases in tests
2. **depth_4_vs_depth_8_severity** -- Depth 4 -> info, depth 8 -> error -- Covers: no severity graduation
3. **callback_in_test_context** -- `describe(() => { it(() => { doA(() => { doB(() => { ... })})})})` -> suppressed or reduced severity -- Covers: high false positive rate
4. **callback_with_eslint_disable** -- `// eslint-disable-next-line max-nested-callbacks` before deep chain -> suppressed -- Covers: no suppression/annotation awareness
5. **callback_in_then_chain** -- `.then(() => { fs.readFile('f', (err, data) => { parse(data, (err, result) => { ... })})})` -> detected -- Covers: missing compound variants
6. **named_function_in_arguments** -- `doA(function handler() { doB(function inner() { ... }) })` -> still counts as callback nesting -- Covers: missing edge cases in tests

---

## implicit_globals

### Current Implementation
- **File:** `src/audit/pipelines/javascript/implicit_globals.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `implicit_global` -- assignment to undeclared variable
- **Detection method:** Tree-sitter query for `assignment_expression` nodes. Filters to only bare identifier LHS (not member expressions). Skips known globals (window, document, console, etc.). Walks up to enclosing scope (function/program) and searches for `variable_declarator` or `formal_parameters` with matching name.

### Problems Identified
1. **[High false negative rate]:** The `search_declarations` function (lines 70-111) only checks `variable_declarator` and `formal_parameters`. It misses:
   - `function_declaration` names (a `function foo(){}` declares `foo` in scope)
   - `class_declaration` names
   - `import` specifiers (`import { x } from ...` declares `x`)
   - `catch_clause` parameter (`catch(e)` declares `e`)
   - Destructured parameters (`function({a, b})` -- these create bindings but `formal_parameters` iteration only looks for direct `identifier` children, missing `object_pattern`/`array_pattern`)
   - `for...in`/`for...of` variable bindings
2. **[No scope awareness]:** The scope walk (lines 50-61) stops at `function_declaration`, `function_expression`, `arrow_function`, or `program`. This misses block scoping -- `let`/`const` in a block create a scope. While `var` hoists to function scope (which this pipeline targets), `let`/`const` in an inner block would be incorrectly found as "declared" even if the assignment is in an outer block where it's not in scope. (Actually, since the search walks the entire subtree of the scope, it would find declarations in inner blocks too -- potentially false negative if a `let x` in an inner block shadows the assignment target in an outer context.)
3. **[Missing context]:** The `KNOWN_GLOBALS` list (lines 14-33) is incomplete. Missing: `Array`, `Object`, `String`, `Number`, `Boolean`, `Date`, `Math`, `JSON`, `Promise`, `Map`, `Set`, `WeakMap`, `WeakSet`, `Symbol`, `Proxy`, `Reflect`, `Error`, `TypeError`, `RangeError`, `RegExp`, `parseInt`, `parseFloat`, `isNaN`, `isFinite`, `encodeURIComponent`, `decodeURIComponent`, `encodeURI`, `decodeURI`, `atob`, `btoa`, `fetch`, `performance`, `navigator`, `location`, `history`, `alert`, `confirm`, `prompt`, `requestAnimationFrame`, `cancelAnimationFrame`, `queueMicrotask`, `structuredClone`, `AbortController`, `TextEncoder`, `TextDecoder`, `URL`, `URLSearchParams`, `Headers`, `Request`, `Response`, `ReadableStream`, `WritableStream`, `Blob`, `File`, `FormData`, `WebSocket`, `EventSource`, `XMLHttpRequest`, `Worker`, `SharedWorker`, `MessageChannel`, `MessagePort`, `BroadcastChannel`, `Notification`, `IntersectionObserver`, `MutationObserver`, `ResizeObserver`, `CustomEvent`, `Event`, `EventTarget`, `HTMLElement`, `SVGElement`, and other Web API globals.
4. **[No suppression/annotation awareness]:** No check for `/* global myVar */`, `/* eslint-disable no-implicit-globals */`, `// eslint-disable-next-line no-undef`, or JSDoc `@global` annotations. (Lines 123-168.)
5. **[Language idiom ignorance]:** In Node.js, `module.exports = ...` pattern commonly assigns to `exports` or other module-level names. The pipeline does allow `module` and `exports` in `KNOWN_GLOBALS` but doesn't handle patterns like `exports.foo = bar` (which is a member expression and already skipped) vs. `exports = { foo }` (identifier assignment -- would be flagged even though it's a standard Node.js pattern). Actually `exports` is in the known list so this specific case is fine, but similar CJS patterns like `self = this` are not handled.
6. **[Single-node detection]:** Only examines individual assignment expressions. Does not trace whether the implicit global is read elsewhere (which would confirm it's truly an implicit global leak vs. a one-off assignment typo).
7. **[High false positive rate]:** In sloppy-mode scripts, augmenting the global scope is sometimes intentional (polyfills, shims). The pipeline cannot distinguish intentional from accidental.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Detects implicit global, skips declared variable, skips known globals, skips member expression, skips parameter.
- **What's NOT tested:** Function declarations as scope bindings, class declarations, import bindings, catch clause parameters, destructured parameters (`function({a}) { a = 1; }`), for-in/for-of bindings, `/* global */` comments, assignment in nested block where `let` is in outer block, augmented assignment operators (`x += 1`), TypeScript-specific `declare` statements, ES module context where strict mode is implicit.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all Symbol nodes with kind `Function`/`ArrowFunction` in JS files
- **Why not tree-sitter first:** We want to scope the analysis to functions, and the graph already has function boundaries with start/end lines.
- **Query:** `graph.file_nodes` filtered to JS language, then outgoing edges from file nodes to Symbol nodes of function kinds.
- **Returns:** List of (file_path, function_start_line, function_end_line) tuples.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (graph does not encode individual assignment statements or variable declarations within function bodies)
- **Why not graph:** The graph does not track local variable declarations or assignment expressions.
- **Query:** For each function body, use `compile_assignment_expression_query()` to find assignments. For each bare identifier LHS, check:
  - All `variable_declarator` names in the function scope
  - All parameter names (including destructured patterns -- walk `object_pattern`/`array_pattern` recursively)
  - All `function_declaration` names in scope
  - All `class_declaration` names in scope
  - All `import` specifier names (from graph `Imports` edges or tree-sitter walk)
  - All `catch_clause` parameters
  - An expanded `KNOWN_GLOBALS` list including all ECMAScript built-ins, Web API globals, and Node.js globals
- **Returns:** Findings for assignments where no declaration is found.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning + AI prompt
- **Query:** 
  - Check for `/* global varName */` at file top
  - Check for `// eslint-disable` comments
  - Check if file has `"use strict"` or is an ES module (has `import`/`export`) -- in strict mode, implicit globals throw a ReferenceError, so the finding should be elevated to "error" severity
- **AI Prompt (if needed):** For ambiguous cases where a variable name matches a well-known library global (e.g., `$`, `_`, `jQuery`, `React`, `angular`), use an AI prompt:
  - **Context gathering query:** The 5 lines surrounding the assignment + the file's import list
  - **Context shape:** `{ "assignment": "$ = jQuery.noConflict()", "imports": ["import jQuery from 'jquery'"], "file_path": "src/app.js" }`
  - **Prompt:** "Is this assignment to `$` intentional (a common jQuery alias pattern) or an accidental implicit global? Answer: intentional or accidental."
  - **Expected return:** `{ "verdict": "intentional" | "accidental" }`
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Missing:** Per-function local variable declaration tracking. The graph currently tracks Symbol-level declarations but not local variables within function bodies. Adding `LocalBinding` nodes (name, declaration_kind: var/let/const/param) connected via `DeclaredIn` edges to their enclosing function Symbol would allow graph-only detection.

### New Test Cases
1. **function_declaration_as_binding** -- `function outer() { function inner() {} inner = 42; }` -> no finding (inner is declared) -- Covers: high false negative rate
2. **import_binding** -- `import { x } from 'mod'; function f() { x = 1; }` -> no finding (x is imported) -- Covers: high false negative rate
3. **catch_parameter** -- `try {} catch(e) { e = null; }` -> no finding (e is declared) -- Covers: high false negative rate
4. **destructured_parameter** -- `function f({a}) { a = 1; }` -> no finding (a is declared via destructuring) -- Covers: high false negative rate
5. **global_comment** -- `/* global myLib */\nfunction f() { myLib = {}; }` -> suppressed -- Covers: no suppression/annotation awareness
6. **strict_mode_severity** -- `"use strict"; function f() { x = 1; }` -> severity "error" -- Covers: no severity graduation
7. **builtin_global_not_flagged** -- `function f() { JSON = null; }` -> not flagged (JSON is a known global) -- Covers: missing context
8. **augment_assignment** -- `function f() { x += 1; }` -> detected (augmented assignment to undeclared) -- Covers: missing compound variants
9. **for_in_binding** -- `for (var k in obj) { k = 'x'; }` -> no finding (k declared by for-in var) -- Covers: missing edge cases in tests

---

## loose_equality

### Current Implementation
- **File:** `src/audit/pipelines/javascript/loose_equality.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `loose_equality` (`==`), `loose_inequality` (`!=`)
- **Detection method:** Tree-sitter query `(binary_expression) @binary` matches all binary expressions. Then iterates unnamed children to find the operator token. If `==` or `!=`, reports a finding.

### Problems Identified
1. **[High false positive rate]:** The pipeline flags ALL `==` and `!=` usage. However, `== null` is a recognized JavaScript idiom that checks for both `null` and `undefined` simultaneously (`x == null` is equivalent to `x === null || x === undefined`). This is explicitly allowed by many style guides (including ESLint's `eqeqeq` rule with `"smart"` option). (Lines 38-66.)
2. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line eqeqeq` or block-level `/* eslint-disable eqeqeq */`. (Lines 33-66.)
3. **[No severity graduation]:** All findings are "warning". `== null` should be "info" at most. `== ""` or `== 0` (where type coercion can cause real bugs) should be "warning" or "error". (Line 53.)
4. **[Language idiom ignorance]:** Does not recognize the `typeof x == "string"` pattern. Since `typeof` always returns a string, `==` vs. `===` is irrelevant here. Many linters allow this. (Lines 38-66.)
5. **[Single-node detection]:** Reports each `==`/`!=` independently. Does not aggregate findings per function or file.
6. **[Missing compound variants]:** Does not handle negated patterns like `!(x == y)` which should suggest `x !== y` instead.
7. **[Literal blindness]:** Does not examine what the operands are. `x == true` is much more dangerous than `x == "hello"` (string-to-string comparison is safe with `==`).

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Detects `==`, detects `!=`, skips `===`, skips `!==`, skips `+`.
- **What's NOT tested:** `== null` idiom, `typeof x == "string"` pattern, `==` inside ternary, `==` in template literal, eslint-disable comments, chained comparisons, `==` with literal on both sides.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter query `(binary_expression) @binary`
- **Why not graph:** Graph does not encode operator tokens in binary expressions.
- **Query:** Same as current `compile_binary_expression_query()`
- **Returns:** All binary expression nodes.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection on each node
- **Query:** For each binary expression with `==` or `!=` operator:
  - Check if either operand is `null` or `undefined` -> pattern `null_equality_idiom`, severity `info`
  - Check if the LHS is a `typeof` expression -> pattern `typeof_comparison`, severity `info` (or suppress entirely)
  - Check if both operands are string literals -> suppress (no coercion risk)
  - Otherwise -> pattern `loose_equality`/`loose_inequality`, severity `warning`
- **Returns:** Findings with context-appropriate severity and pattern.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check preceding comments for `eslint-disable-next-line eqeqeq` or block-level `/* eslint-disable eqeqeq */`.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
None.

### New Test Cases
1. **null_check_idiom** -- `if (x == null) {}` -> severity `info`, pattern `null_equality_idiom` -- Covers: high false positive rate, language idiom ignorance
2. **typeof_comparison** -- `if (typeof x == "string") {}` -> suppressed or severity `info` -- Covers: language idiom ignorance
3. **string_vs_string** -- `if (name == "admin") {}` -> suppressed (no coercion risk, both are strings at parse time is unknowable, but literal RHS is string) -- Covers: literal blindness
4. **eslint_disable** -- `// eslint-disable-next-line eqeqeq\nif (x == 1) {}` -> suppressed -- Covers: no suppression/annotation awareness
5. **dangerous_coercion** -- `if (x == 0) {}` -> severity `warning` (0 coercion is dangerous) -- Covers: no severity graduation
6. **undefined_check** -- `if (x == undefined) {}` -> severity `info` -- Covers: high false positive rate

---

## unhandled_promise

### Current Implementation
- **File:** `src/audit/pipelines/javascript/unhandled_promise.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `unhandled_then` -- `.then()` without `.catch()`
- **Detection method:** Tree-sitter query for call expressions with member expression function. Filters to `.then()` calls. Checks if `.then()` has 2+ arguments (rejection handler) or if the parent is a member_expression accessing `.catch()` or `.finally()`.

### Problems Identified
1. **[High false negative rate]:** The `is_handled` function (lines 27-47) checks if `.then()` is the **object** of a `.catch()` member expression. But it checks `call_node.parent()` -- the parent of the call_expression. In `fetch(url).then(...).catch(...)`, the `.then(...)` call_expression's parent would be a `member_expression` (the `.catch` access). This works for direct chaining but misses:
   - Promise stored in a variable and `.catch()` called later: `const p = fetch().then(...); p.catch(...)` -- flagged incorrectly.
   - `.then().then().catch()` -- only the outermost `.then()` is checked. The inner `.then()` call's parent is the `.then` member_expression of the second `.then()`, not a `.catch()`. This means the inner `.then()` would be flagged as unhandled.
   - `await` usage: `await fetch().then(...)` -- the promise rejection is handled by the enclosing try/catch or the caller, but this pipeline flags it.
2. **[No data flow tracking]:** Cannot track promise references across statements. `const p = fetch(); p.then(...); p.catch(...)` -- the `.then()` appears unhandled because the `.catch()` is on a separate statement. (Lines 59-98.)
3. **[Missing compound variants]:** Does not detect:
   - Unhandled `Promise.all()`, `Promise.race()`, `Promise.allSettled()` without `.catch()`
   - Floating promises (async function calls without `await` or `.then()/.catch()`)
   - `async` functions that don't have try/catch around `await` calls
4. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line no-floating-promises` or similar. (Lines 59-98.)
5. **[Single-node detection]:** Only looks at individual `.then()` call sites. Does not analyze the promise chain holistically.
6. **[Language idiom ignorance]:** In test code (Jest, Mocha), returning a promise from a test is sufficient handling. `it('test', () => { return fetch().then(...); })` is correctly handled by the test runner.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `.then()` without `.catch()`, `.then().catch()`, `.then(onSuccess, onError)`, non-`.then()` method.
- **What's NOT tested:** `.then().then().catch()` chain, `await fetch().then(...)`, promise stored in variable, `.then().finally()`, `.then()` inside async function with try/catch, `.then()` returned from a function (caller handles rejection), test context handling.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all CallSite nodes where name is "then"
- **Why not tree-sitter first:** The graph already indexes call sites by name, making the initial scan faster.
- **Query:** `graph.find_symbols_by_name("then")` to find `.then()` call sites, plus tree-sitter to verify they're method calls on promise-like objects.
- **Returns:** All `.then()` call site nodes with file paths and lines.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST walk from each `.then()` call
- **Query:** For each `.then()` call:
  - Walk up the call chain: is there a `.catch()` or `.finally()` anywhere in the chain? (not just immediate parent)
  - Is the `.then()` call `await`ed? Check if parent (possibly nested) is an `await_expression`.
  - Is the `.then()` result stored in a variable? If so, search the scope for `.catch()` on that variable.
  - Does `.then()` have 2+ arguments?
- **Returns:** Findings for truly unhandled `.then()` calls.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter context + AI prompt for ambiguous cases
- **Query:**
  - Check if inside a test context (`describe`/`it`/`test`) and the `.then()` is returned.
  - Check for suppression comments.
- **AI Prompt (for variable-stored promises):**
  - **Context gathering query:** Tree-sitter: extract all statements in the enclosing function that reference the promise variable
  - **Context shape:** `{ "promise_var": "p", "statements": ["const p = fetch().then(handler);", "p.catch(onError);"], "function_name": "loadData" }`
  - **Prompt:** "Given these statements referencing promise variable `p`, is the promise rejection handled? Look for .catch(), try/catch around await, or the promise being returned to a caller that handles it. Answer: handled or unhandled."
  - **Expected return:** `{ "verdict": "handled" | "unhandled" }`
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Missing:** Promise chain tracking. The graph builds call edges (`Calls`) but does not model `.then()` / `.catch()` chaining as a distinct relationship. Adding a `PromiseChain` edge type between CallSite nodes that are part of the same promise chain would enable graph-level analysis of promise handling.

### New Test Cases
1. **chained_then_then_catch** -- `fetch().then(a).then(b).catch(e)` -> no finding (chain is handled) -- Covers: high false negative rate
2. **awaited_then** -- `await fetch().then(handler)` -> no finding -- Covers: high false negative rate
3. **variable_stored_catch** -- `const p = fetch().then(h); p.catch(e)` -> no finding -- Covers: no data flow tracking
4. **then_in_test_returned** -- `it('test', () => { return fetch().then(h); })` -> no finding (test runner handles) -- Covers: language idiom ignorance
5. **promise_all_unhandled** -- `Promise.all([p1, p2]).then(r)` -> finding (no .catch) -- Covers: missing compound variants
6. **eslint_disable_floating** -- `// eslint-disable-next-line no-floating-promises\nfetch().then(h)` -> suppressed -- Covers: no suppression/annotation awareness
7. **then_finally_no_catch** -- `fetch().then(h).finally(cleanup)` -> finding (`.finally()` does not handle rejection) -- Covers: missing edge cases in tests

---

## argument_mutation

### Current Implementation
- **File:** `src/audit/pipelines/javascript/argument_mutation.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `argument_mutation` -- assignment to member of a function parameter
- **Detection method:** Tree-sitter query for function/arrow/expression with params and body. Extracts parameter names. Walks body for `assignment_expression` where LHS is `member_expression` with root object matching a parameter name.

### Problems Identified
1. **[High false negative rate]:** Only detects `assignment_expression` with `member_expression` LHS. Misses:
   - `param.push(item)` / `param.splice(0, 1)` -- mutating methods on arrays/objects
   - `param[0] = value` -- subscript assignment (LHS is `subscript_expression`, not `member_expression`)
   - `delete param.key` -- delete expression
   - `param.sort()` -- in-place mutation methods
   - `Object.assign(param, { key: value })` -- mutation via utility function
   - Destructured parameter mutation: `function f({a}) { a.x = 1; }` -- `a` is extracted from the param but the pipeline only finds direct identifier params and `assignment_pattern` left-side identifiers (lines 28-43)
2. **[Missing compound variants]:** Does not detect indirect mutation through aliasing: `function f(obj) { const ref = obj; ref.x = 1; }` -- `ref` is an alias of `obj`, so mutating `ref.x` mutates the argument.
3. **[No data flow tracking]:** Does not trace parameter references through the function body. An alias, spread, or destructure of a parameter can still mutate the original. (Lines 66-119.)
4. **[Language idiom ignorance]:** Some mutation is intentional and idiomatic:
   - Builder/configuration patterns: `function configure(options) { options.defaults = {...}; }` -- intentional API.
   - Prototype extension: `function extend(proto) { proto.newMethod = ...; }` -- intentional.
   - Express middleware: `function middleware(req, res, next) { req.user = decoded; }` -- standard practice.
   The pipeline does not distinguish intentional from accidental mutation. (Lines 74-98.)
5. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line no-param-reassign` or JSDoc `@mutates` annotations. (Lines 144-175.)
6. **[No severity graduation]:** All mutations are "warning". Mutating a deeply nested property (`config.nested.deep = true`) is higher risk than mutating a top-level property. (Line 88.)
7. **[No scope awareness]:** Does not skip nested functions (actually it does on line 102-105: it skips `function_declaration`, `function_expression`, `arrow_function`). However, it skips `function_declaration` but the tree-sitter node kind for `function` keyword in expressions is `function_expression`, not `function`. Line 103 has `"function"` which is not a valid tree-sitter node kind -- this may be dead code or a bug (depending on grammar).

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Direct member mutation, local variable mutation (skipped), no params (skipped), deep member chain mutation, arrow function mutation.
- **What's NOT tested:** Subscript assignment (`param[0] = value`), mutating methods (`.push()`, `.sort()`), destructured parameter mutation, aliased parameter mutation, `delete param.key`, Express middleware pattern (req/res mutation), nested function correctly skipped, augmented assignment (`param.x += 1`).

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all Symbol nodes of kind Function/ArrowFunction/Method
- **Why not tree-sitter first:** Graph gives us function boundaries and parameter nodes (if populated).
- **Query:** Iterate `graph.symbol_nodes` for JS files, filter to function kinds.
- **Returns:** List of function symbols with file paths, start/end lines.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST walk within each function body
- **Why not graph:** The graph does not track individual statements or local variable assignments within function bodies.
- **Query:** For each function:
  - Extract parameter names (including destructured: walk `object_pattern`/`array_pattern` recursively)
  - Walk body for ALL mutation patterns:
    - `assignment_expression` with `member_expression` LHS (current detection)
    - `assignment_expression` with `subscript_expression` LHS
    - `augmented_assignment_expression` with `member_expression`/`subscript_expression` LHS
    - `call_expression` where object is a param and method is a known mutating method (`push`, `pop`, `shift`, `unshift`, `splice`, `sort`, `reverse`, `fill`, `copyWithin`, `delete`)
    - `delete` expression targeting param member
    - `Object.assign()` / `Object.defineProperty()` calls with param as first arg
  - Track aliases: if `const ref = param` or `const ref = param.nested`, add `ref` to the tracked names set
- **Returns:** Findings for all detected mutations.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning + pattern matching
- **Query:**
  - Check for `// eslint-disable-next-line no-param-reassign`
  - Check if function name matches common intentional-mutation patterns: `configure`, `extend`, `init`, `setup`, `middleware`, `mutate`, `patch`, `update`
  - Check if parameter name is `req`, `res`, `ctx`, `state` (framework patterns where mutation is expected)
  - Check for JSDoc `@mutates` or `@param {Object} param - Modified in place` annotations
- **Returns:** Filtered findings with context.

#### Graph Enhancement Required
- **Missing:** Parameter nodes in the graph do exist (`Parameter` node weight) but local variable alias tracking (data flow within a function) is not available. Adding intra-function FlowsTo edges from parameters to local variables that receive their value would enable alias detection.

### New Test Cases
1. **subscript_mutation** -- `function f(arr) { arr[0] = 'x'; }` -> finding -- Covers: high false negative rate
2. **push_mutation** -- `function f(arr) { arr.push(1); }` -> finding -- Covers: high false negative rate
3. **sort_mutation** -- `function f(arr) { arr.sort(); }` -> finding -- Covers: high false negative rate
4. **delete_mutation** -- `function f(obj) { delete obj.key; }` -> finding -- Covers: high false negative rate
5. **alias_mutation** -- `function f(obj) { const ref = obj; ref.x = 1; }` -> finding -- Covers: no data flow tracking
6. **express_middleware_suppressed** -- `function middleware(req, res, next) { req.user = decoded; }` -> suppressed or severity `info` -- Covers: language idiom ignorance
7. **eslint_disable** -- `// eslint-disable-next-line no-param-reassign\nfunction f(obj) { obj.x = 1; }` -> suppressed -- Covers: no suppression/annotation awareness
8. **augmented_assignment** -- `function f(obj) { obj.count += 1; }` -> finding -- Covers: missing compound variants
9. **object_assign_mutation** -- `function f(obj) { Object.assign(obj, defaults); }` -> finding -- Covers: missing compound variants
10. **destructured_param** -- `function f({nested}) { nested.x = 1; }` -> finding -- Covers: missing edge cases in tests

---

## console_log_in_prod

### Current Implementation
- **File:** `src/audit/pipelines/javascript/console_log_in_prod.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `console_log` -- any `console.log/warn/error/debug/info/trace` call
- **Detection method:** Tree-sitter query for call expressions with member expression. Checks if object is `console` and method is in `CONSOLE_METHODS` list.

### Problems Identified
1. **[High false positive rate]:** Flags `console.error()` which is commonly used for legitimate error reporting in production server code (Node.js). Also flags `console.warn()` which is used for deprecation warnings. Not all console methods are equal. (Lines 14, 55.)
2. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line no-console`, `/* eslint-disable no-console */`, or `/* istanbul ignore next */`. (Lines 37-75.)
3. **[No severity graduation]:** All console methods get the same `info` severity. `console.log` (debug leftover) should be `warning`, `console.error` (possibly intentional) should be `info`, `console.debug` (definitely debug) should be `warning`. (Line 58.)
4. **[Language idiom ignorance]:** In Node.js server code, `console.error` and `console.warn` are standard. In browser code, any console statement is suspect. The pipeline cannot distinguish browser vs. Node.js context. (Lines 37-75.)
5. **[Missing context]:** Does not check if the file is already using a logging library (winston, pino, bunyan, log4js). If a file imports a logger, a remaining `console.log` is more likely a leftover. If no logger is imported, `console.log` might be the intentional logging mechanism.
6. **[Single-node detection]:** Does not track whether `console` has been reassigned or wrapped (`const log = console.log; log('test');` -- not detected).
7. **[Overlapping detection]:** `console.error` in a `catch` block is standard error handling, not tech debt. No context-aware filtering.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** `console.log`, `console.warn`, `console.error`, other objects (`logger.log`), other methods (`console.clear`).
- **What's NOT tested:** `console.debug`, `console.info`, `console.trace`, console in test files, console in catch blocks, console with eslint-disable, console.error in Node.js server context, aliased console (`const log = console.log`), `console.table`, `console.dir`, `console.assert`, `console.time`/`console.timeEnd`, `console.group`/`console.groupEnd`.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all CallSite nodes, filter by name containing "log", "warn", "error", "debug", "info", "trace" on `console` object
- **Why not tree-sitter first:** The graph CallSite nodes index call names. However, the graph stores the method name (e.g., "log") not the full `console.log`. So tree-sitter is actually more appropriate here.
- **Tool (revised):** Tree-sitter query (same as current) -- graph CallSite does not store the receiver object.
- **Query:** Same `compile_call_expression_query()`, filter obj=console, method in expanded list.
- **Returns:** All console.* call nodes.

#### Step 2: Narrowing
- **Tool:** Tree-sitter + Graph
- **Query:** For each console call:
  - Check file imports (via graph `Imports` edges): does the file import a logging library (winston, pino, bunyan, log4js, debug, loglevel)? If yes, finding severity = `warning` (definite leftover).
  - Check if `console.error`/`console.warn` and inside a `catch` block -> suppress or severity `info`.
  - Check if file is a test file (`is_test_file()`) -> suppress.
  - Graduate severity: `console.log` -> `warning`, `console.debug` -> `warning`, `console.trace` -> `warning`, `console.info` -> `info`, `console.warn` -> `info`, `console.error` -> `info`.
- **Returns:** Findings with graduated severity.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check for `// eslint-disable-next-line no-console` or block-level disables.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Useful but not required:** The graph's `Imports` edges could be used to check for logging library imports. This data is already available.

### New Test Cases
1. **console_debug** -- `console.debug('x');` -> finding, severity `warning` -- Covers: no severity graduation
2. **console_error_in_catch** -- `try {} catch(e) { console.error(e); }` -> suppressed or severity `info` -- Covers: high false positive rate, language idiom ignorance
3. **console_in_test_file** -- (file path `test.spec.js`) `console.log('debug')` -> suppressed -- Covers: high false positive rate
4. **console_with_eslint_disable** -- `// eslint-disable-next-line no-console\nconsole.log('ok')` -> suppressed -- Covers: no suppression/annotation awareness
5. **file_with_logger_import** -- `import logger from 'winston'; console.log('leftover')` -> severity `warning` -- Covers: missing context
6. **aliased_console** -- `const log = console.log; log('test');` -> finding (stretch goal) -- Covers: single-node detection
7. **console_table** -- `console.table(data);` -> finding with appropriate severity -- Covers: missing edge cases in tests

---

## event_listener_leak

### Current Implementation
- **File:** `src/audit/pipelines/javascript/event_listener_leak.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `missing_remove_listener` -- `addEventListener` without any `removeEventListener` in the same file
- **Detection method:** Tree-sitter query for call expressions. Collects all `addEventListener` calls and checks if any `removeEventListener` exists in the same file. If ANY `removeEventListener` exists anywhere in the file, ALL `addEventListener` calls are suppressed (line 63: `if has_remove`).

### Problems Identified
1. **[Broken detection]:** The suppression logic is file-level: a single `removeEventListener` anywhere in the file suppresses ALL `addEventListener` findings. If a file has 5 `addEventListener` calls and 1 `removeEventListener` for a different event, the other 4 are silently passed. (Line 63.)
2. **[High false negative rate]:** Due to the broken suppression logic above. Also misses:
   - `addEventListener` with `{ once: true }` option -- these auto-remove and should never be flagged
   - `AbortController` signal pattern: `el.addEventListener('click', fn, { signal: controller.signal })` -- cleanup via `controller.abort()`
   - Listeners added in constructor/componentDidMount should be removed in destructor/componentWillUnmount -- but this requires cross-method analysis
3. **[No scope awareness]:** Does not check if `addEventListener` and `removeEventListener` are for the same event type and same handler. Even a matching pair might have different event names. (Lines 42-65.)
4. **[Single-node detection]:** Looks only at individual files. In React/Vue/Angular, event listeners are often added in one lifecycle method and removed in another (same file but different functions). The current approach can't correlate them correctly.
5. **[No data flow tracking]:** Cannot determine if the handler function reference is the same between `add` and `remove`. `el.addEventListener('click', fn); el.removeEventListener('click', fn)` is correct. `el.addEventListener('click', fn); el.removeEventListener('click', otherFn)` is a bug but looks correct to the pipeline.
6. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line` or intentional patterns. (Lines 35-84.)
7. **[Language idiom ignorance]:** React class components with `componentDidMount`/`componentWillUnmount`, React hooks with `useEffect` return cleanup, and Vue `mounted`/`beforeDestroy` lifecycle patterns are not recognized.
8. **[Missing compound variants]:** Does not detect `window.addEventListener` vs. `element.addEventListener` -- they have different cleanup requirements. Also misses jQuery `.on()` without `.off()`, and EventEmitter `.on()` without `.off()`/`.removeListener()`.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** `addEventListener` without `removeEventListener`, both present (suppressed), only `removeEventListener`.
- **What's NOT tested:** Multiple `addEventListener` with one `removeEventListener` (broken detection), `{ once: true }` option, `AbortController` signal pattern, different event types between add/remove, React lifecycle patterns, jQuery event binding, anonymous handler function (impossible to remove), named vs. anonymous handlers, event listener inside loop.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all CallSite nodes where name is "addEventListener"
- **Why not tree-sitter first:** Graph indexes call sites by name for fast lookup across the codebase.
- **Query:** `graph.find_symbols_by_name("addEventListener")` for call sites.
- **Returns:** All addEventListener call sites with file paths and lines.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST analysis per call site
- **Why not graph:** Need to extract the event type string, handler reference, and options from arguments -- which are AST details.
- **Query:** For each `addEventListener` call:
  - Extract event type (first arg, usually string literal)
  - Extract handler reference (second arg: identifier, function expression, or arrow function)
  - Extract options (third arg: check for `{ once: true }` or `{ signal: ... }`) -- if `once: true`, suppress finding
  - Search the same file for `removeEventListener` calls with matching event type and handler reference
  - If handler is anonymous (arrow function or function expression), flag as `anonymous_listener_leak` (cannot be removed)
  - If handler is an identifier, check if `removeEventListener` uses the same identifier for the same event type
- **Returns:** Findings for unmatched `addEventListener` calls.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter context analysis + Graph
- **Query:**
  - Check if `addEventListener` is inside a `useEffect` callback that returns a cleanup function containing `removeEventListener` (React hook pattern)
  - Check if the file has lifecycle methods (`componentDidMount`/`componentWillUnmount`, `mounted`/`beforeDestroy`) and the `removeEventListener` is in the unmount method
  - Check for suppression comments
  - Check if the `addEventListener` is on `{ once: true }` or uses `AbortController.signal`
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Missing:** Event listener pairing. A new `ListensTo { event_type, handler_ref }` edge type from element nodes to handler function nodes would enable graph-level matching of add/remove pairs. This is a significant enhancement.

### New Test Cases
1. **multiple_add_one_remove** -- 3 `addEventListener` + 1 `removeEventListener` for different event -> 2 findings -- Covers: broken detection
2. **once_option** -- `el.addEventListener('click', fn, { once: true })` -> no finding -- Covers: high false negative rate
3. **abort_signal** -- `el.addEventListener('click', fn, { signal: ctrl.signal })` -> no finding -- Covers: high false negative rate
4. **anonymous_handler** -- `el.addEventListener('click', () => {})` -> finding with pattern `anonymous_listener_leak` -- Covers: no data flow tracking
5. **mismatched_event_type** -- `el.addEventListener('click', fn); el.removeEventListener('keyup', fn)` -> finding -- Covers: no scope awareness
6. **react_useeffect_cleanup** -- `useEffect(() => { el.addEventListener('x', fn); return () => el.removeEventListener('x', fn); })` -> no finding -- Covers: language idiom ignorance
7. **jquery_on_without_off** -- `$(el).on('click', fn)` -> finding -- Covers: missing compound variants
8. **listener_in_loop** -- `items.forEach(el => el.addEventListener('click', handler))` -> finding with severity `warning` -- Covers: missing edge cases in tests

---

## loose_truthiness

### Current Implementation
- **File:** `src/audit/pipelines/javascript/loose_truthiness.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `loose_length_check` -- `if(arr.length)` without explicit comparison
- **Detection method:** Tree-sitter query for `if_statement` with `parenthesized_expression` condition. Gets inner expression and checks if it's a `member_expression` with property `length`.

### Problems Identified
1. **[High false positive rate]:** `if (arr.length)` is an extremely common and widely accepted JavaScript idiom. Most style guides do not flag this. The suggestion to use `if (arr.length > 0)` is stylistic, not a correctness issue. `.length` is always a non-negative integer, so truthiness check is safe (only 0 is falsy). (Lines 59-76.)
2. **[Missing compound variants]:** Only checks `if` statements. Misses:
   - Ternary: `arr.length ? x : y`
   - Logical AND: `arr.length && doSomething()`
   - While loop: `while (arr.length) { arr.pop(); }`
   - Negation: `if (!arr.length)` (not a member_expression, it's a unary_expression wrapping member_expression)
3. **[Literal blindness]:** Only looks for `.length`. Other loose truthiness patterns are arguably more dangerous:
   - `if (str)` where str could be `""` (empty string is falsy)
   - `if (count)` where count could be `0` (0 is falsy but may be valid)
   - `if (obj)` where obj could be `null` or `undefined`
   These are the real loose truthiness bugs, but the pipeline ignores them entirely.
4. **[No suppression/annotation awareness]:** No check for eslint-disable comments. (Lines 35-83.)
5. **[No severity graduation]:** All findings are "info" which is appropriate for this specific pattern, but there's no graduation for the more dangerous variants that should be added.
6. **[Language idiom ignorance]:** `if (arr.length)` is idiomatic JavaScript. Flagging it creates noise without preventing real bugs.
7. **[Overlapping detection]:** This pipeline's domain overlaps with where a more comprehensive "implicit type coercion" pipeline would operate.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `if (arr.length)`, explicit comparison `arr.length > 0`, other property `obj.visible`, bare identifier `if (x)`.
- **What's NOT tested:** `while (arr.length)`, ternary `arr.length ? ...`, logical AND `arr.length && ...`, `if (!arr.length)`, `if (str.length)` (string length), nested expression `if (foo.bar.length)`, `if (arr.length == 0)` (not flagged since it's explicit), `for` loop condition `for (; arr.length; )`.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter query for all condition contexts: `if_statement`, `while_statement`, `do_statement`, `for_statement`, `ternary_expression`, `binary_expression` (logical AND/OR)
- **Why not graph:** Graph does not encode control flow conditions.
- **Query:** Expanded queries:
  - `(if_statement condition: (parenthesized_expression) @condition) @stmt`
  - `(while_statement condition: (parenthesized_expression) @condition) @stmt`
  - `(ternary_expression condition: (_) @condition) @expr`
- **Returns:** All condition expressions.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection
- **Query:** For each condition:
  - Is it a bare `member_expression` with property `length`? -> `loose_length_check` (severity `info`)
  - Is it a bare identifier that could be a string or number? -> too noisy without type info, skip unless context helps
  - Is it a `unary_expression` (`!`) wrapping a `.length` check? -> `negated_length_check` (severity `info`)
  - Is it a `.length` inside a `&&` or `||`? -> `loose_length_in_logical` (severity `info`)
- **Returns:** Findings with specific pattern names.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check for eslint-disable comments. Also consider making the entire `.length` check pattern configurable (opt-in, since it's so common and harmless).
- **Returns:** Filtered findings.

#### Graph Enhancement Required
None.

### New Test Cases
1. **while_loop_length** -- `while (arr.length) { arr.pop(); }` -> finding -- Covers: missing compound variants
2. **ternary_length** -- `const x = arr.length ? arr[0] : null;` -> finding -- Covers: missing compound variants
3. **logical_and_length** -- `arr.length && process(arr)` -> finding -- Covers: missing compound variants
4. **negated_length** -- `if (!arr.length) { return; }` -> finding -- Covers: missing compound variants
5. **nested_member_length** -- `if (foo.bar.length) {}` -> finding -- Covers: missing edge cases in tests
6. **eslint_disable** -- `// eslint-disable-next-line\nif (arr.length) {}` -> suppressed -- Covers: no suppression/annotation awareness

---

## no_optional_chaining

### Current Implementation
- **File:** `src/audit/pipelines/javascript/no_optional_chaining.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `deep_property_chain` -- member expression chain >= 4 segments without `?.`
- **Detection method:** Tree-sitter query for `member_expression`. Filters to outermost expressions only (skips if parent is also `member_expression`). Counts chain depth. Checks if any node in the chain uses optional chaining (`optional_chain_expression`).

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `DEPTH_THRESHOLD = 4` (line 12). No justification for why 4 and not 3 or 5. Many real-world property chains are 4+ segments and perfectly safe when the object structure is known (e.g., `document.body.style.display`, `process.env.NODE_ENV`).
2. **[High false positive rate]:** Chains on well-known, always-defined objects are safe:
   - `document.body.style.display` -- DOM is always present in browser
   - `process.env.NODE_ENV` -- only 3 segments, but `config.database.host.port` at 4 would be flagged
   - `Math.PI` (2 segments, fine), `window.location.href.split(...)` (4 segments, flagged unnecessarily)
   - `this.props.data.items` in React -- `this` and `props` are always defined; `data` might be null but this is context-dependent
3. **[No severity graduation]:** All findings are "info". Depth 4 should be "info", depth 6+ should be "warning", depth 8+ should be "error". (Line 110.)
4. **[No suppression/annotation awareness]:** No eslint-disable checks. (Lines 76-122.)
5. **[Missing compound variants]:** Does not detect:
   - Method call chains: `a.b().c.d` -- `.c` is after a call, which may return null
   - Mixed access: `a[b].c.d.e` -- subscript access mixed with member access
   - Computed property access: `a.b[key].c.d` -- not detected because subscript_expression breaks the member_expression chain
6. **[Language idiom ignorance]:** TypeScript code with strict null checks already handles this at the type level. Running this on `.ts` files (which share the same grammar) produces noise for code that's already type-safe.
7. **[Literal blindness]:** Does not check if the chain starts with a known-safe root: `document`, `window`, `Math`, `console`, `process`, `module`, `require`, `JSON`. These roots are always defined.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Deep chain (4 segments), shallow chain (3), single member (2).
- **What's NOT tested:** Chain with optional chaining present, chain starting with `document`/`window`, chain including method calls, chain with computed properties, chain at exactly threshold boundary (depth=4 flagged, depth=3 not), depth 6+, eslint-disable comment, TypeScript file context.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter query for `member_expression` (same as current)
- **Why not graph:** Chain depth is an AST structural property.
- **Query:** Same `compile_member_expression_query()`, filter to outermost expressions.
- **Returns:** All outermost member expression chains with depth.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST inspection
- **Query:** For each chain:
  - Compute depth (same as current `chain_depth`)
  - Check if optional chaining is used (same as current `has_optional_chaining`)
  - Check root of chain: if it's a well-known safe root (`document`, `window`, `Math`, `console`, `process`, `module`, `JSON`, `this`), increase threshold by 1 or suppress
  - Check if chain includes method calls (`.foo().bar`) -- these are higher risk since methods can return null
  - Graduate severity: depth 4-5 = `info`, depth 6-7 = `warning`, depth 8+ = `error`
- **Returns:** Findings with graduated severity and enriched context.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check for eslint-disable comments. Check if file is TypeScript (if so, reduce severity since TS null checks may handle this).
- **Returns:** Filtered findings.

#### Graph Enhancement Required
None.

### New Test Cases
1. **chain_with_optional_chaining** -- `a?.b.c.d` -> no finding (optional chaining present) -- Covers: missing edge cases in tests
2. **safe_root_document** -- `document.body.style.display` -> suppressed or reduced severity -- Covers: high false positive rate, literal blindness
3. **depth_6_severity** -- `a.b.c.d.e.f` -> severity `warning` -- Covers: no severity graduation
4. **method_in_chain** -- `a.b().c.d.e` -> finding (method may return null) -- Covers: missing compound variants
5. **eslint_disable** -- `// eslint-disable-next-line\nlet x = a.b.c.d;` -> suppressed -- Covers: no suppression/annotation awareness
6. **process_env** -- `process.env.NODE_ENV.toLowerCase()` -> suppressed (known safe root) -- Covers: language idiom ignorance
7. **this_props_chain** -- `this.props.data.items` -> severity `info` (this is semi-safe) -- Covers: high false positive rate

---

## magic_numbers

### Current Implementation
- **File:** `src/audit/pipelines/javascript/magic_numbers.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `magic_number` -- numeric literals outside const contexts
- **Detection method:** Tree-sitter query for `(number) @number`. Skips test files, excluded values (0, 1, 2, 10, 100, etc.), `COMMON_ALLOWED_NUMBERS` (HTTP codes, ports, etc.), exempt contexts (const declarations, switch cases, array indices), and test contexts (describe/it blocks).

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `EXCLUDED_VALUES` (line 13) includes some questionable entries: `256`, `512`, `1024`, `2048`, `4096`, `8192` are byte/memory sizes but not universally safe. `10`, `100`, `1000` are common but could be meaningful magic numbers in certain contexts (e.g., `timeout = 1000` is `1000ms` which should be a named constant).
2. **[High false positive rate]:** Despite the extensive exclusion lists, common patterns still trigger:
   - Millisecond conversions: `* 60 * 1000` -> `60` and `1000` are excluded but `60000` is not
   - Percentage calculations: `* 100` -> 100 is excluded, fine
   - Array/string operations: `.slice(0, 5)` -> `5` is excluded via `COMMON_ALLOWED_NUMBERS`, fine
   - However, hex literals (`0xFF`, `0x1A`) are not in the exclusion list and will be flagged
3. **[Missing context]:** Does not consider what the number is used for:
   - RHS of timing: `setTimeout(fn, 3000)` -- 3000 is a timeout, should suggest named constant with high confidence
   - RHS of comparison: `if (statusCode === 404)` -- 404 is in COMMON_ALLOWED_NUMBERS, fine
   - Enum-like object: `const Modes = { READ: 1, WRITE: 2, EXEC: 4 }` -- these are in const context (exempt), fine
   - But `let mode = 4` is flagged even though `4` is a common small integer
4. **[No suppression/annotation awareness]:** No check for `// eslint-disable-next-line no-magic-numbers` or `/* eslint-disable no-magic-numbers */`. (Lines 75-123.)
5. **[Literal blindness]:** Does not handle negative numbers (`-1` is common for "not found" patterns), hex literals (`0xFF`), binary literals (`0b1010`), octal literals (`0o777`), BigInt literals (`100n`), exponential notation (`1e6`).
6. **[Overlapping detection]:** The pipeline's exclusion lists overlap with `COMMON_ALLOWED_NUMBERS` from helpers. The effective exclusion set is the union of `EXCLUDED_VALUES` and `COMMON_ALLOWED_NUMBERS`, making it hard to reason about what's actually flagged.
7. **[No severity graduation]:** All findings are "info". A magic number in a function body is more concerning than one in a class property initialization. (Line 111.)
8. **[Weak test assertions]:** The test `detects_magic_number` (line 142) uses `parse_and_check("let x = 42;")` but the parse function uses file path `"test.js"` which would be detected as a test file by `is_test_file()` (line 77). However, looking at the helpers, `is_test_file` checks for `_test.`, `.test.`, `__tests__`, etc. -- `"test.js"` matches `test.` prefix but actually the function checks path components. Let me re-examine: the test file path is `"test.js"` and `is_test_file` checks `path.contains("/tests/")`, `path.contains("_test.")`, `path.ends_with(".test.js")`, etc. `"test.js"` doesn't match any of these patterns, so it's not skipped. This is fine but fragile.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Magic number detected, const context skipped, common values (0, 1, 2) skipped, array index skipped, let context not skipped, float magic number.
- **What's NOT tested:** Hex literals (`0xFF`), negative numbers (`-1`), numbers in setTimeout arguments, numbers in eslint-disable context, numbers in enum-like objects (non-const), numbers in return statements, numbers in ternary expressions, numbers used as bitflags, test file skipping, BigInt literals.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter query `(number) @number` (same as current)
- **Why not graph:** Graph does not encode numeric literal values.
- **Query:** Same `compile_numeric_literal_query()`
- **Returns:** All numeric literal nodes.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST context analysis
- **Query:** For each numeric literal:
  - Check if value is in exclusion list (maintain current lists)
  - Check if in const/enum context (maintain current exempt check)
  - Check context: is parent a `call_expression` where function is `setTimeout`/`setInterval`? -> pattern `magic_timeout`, severity `warning`
  - Check if negative (parent is `unary_expression` with `-` operator) -> treat `-1` as allowed
  - Check format: hex/binary/octal/exponential -> reduce severity (these are often intentional bit manipulation)
  - Check if in test context (maintain current check)
  - Check if parent is `variable_declaration` with `let`/`var` -> severity `warning` (should be const)
  - Check if parent is comparison/return -> severity `info`
- **Returns:** Findings with context-enriched messages and graduated severity.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check for `// eslint-disable-next-line no-magic-numbers`.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
None.

### New Test Cases
1. **hex_literal** -- `let mask = 0xFF;` -> finding (not in exclusion list) -- Covers: literal blindness
2. **negative_one** -- `const idx = arr.indexOf(x); if (idx === -1) {}` -> `-1` suppressed (common pattern) -- Covers: literal blindness
3. **setTimeout_magic** -- `setTimeout(fn, 3000);` -> finding, severity `warning`, pattern `magic_timeout` -- Covers: missing context
4. **eslint_disable** -- `// eslint-disable-next-line no-magic-numbers\nlet x = 42;` -> suppressed -- Covers: no suppression/annotation awareness
5. **bigint_literal** -- `let n = 100n;` -> finding or suppressed depending on value -- Covers: literal blindness
6. **exponential** -- `let n = 1e6;` -> finding with reduced severity -- Covers: literal blindness
7. **binary_literal** -- `let flags = 0b1010;` -> finding with reduced severity -- Covers: literal blindness
8. **enum_like_non_const** -- `let Status = { OK: 200, ERR: 500 };` -> finding for 200 and 500 (in COMMON_ALLOWED_NUMBERS, actually suppressed) -- Covers: missing edge cases

---

## shallow_spread_copy

### Current Implementation
- **File:** `src/audit/pipelines/javascript/shallow_spread_copy.rs`
- **Trait type:** `Pipeline` (legacy)
- **Patterns detected:** `shallow_spread_copy` -- `{ ...identifier }` spread in object literal
- **Detection method:** Tree-sitter query for `(object (spread_element (_) @target) @spread) @obj`. Reports when the spread target is an identifier (skips function calls like `{ ...getDefaults() }`).

### Problems Identified
1. **[High false positive rate]:** Object spread is the standard, idiomatic way to create copies in JavaScript. Nearly every React state update, Redux reducer, and configuration merge uses `{ ...obj, key: value }`. Flagging every single one is extremely noisy. The message "nested objects are still shared references" is true but only problematic when the nested objects are actually mutated later. Without data flow analysis, the pipeline cannot determine this. (Lines 33-68.)
2. **[No data flow tracking]:** The finding is only useful if the spread copy's nested properties are subsequently mutated. Without tracking reads/writes after the spread, every spread is flagged even when it's safe. (Lines 33-68.)
3. **[Missing compound variants]:** Only checks object spread (`{ ...obj }`). Misses:
   - Array spread: `[...arr]` -- same shallow copy semantics for arrays of objects
   - `Object.assign({}, obj)` -- equivalent to `{ ...obj }`
   - `Array.from(arr)` -- shallow copy of array
   - `.slice()` / `.concat()` -- shallow copy methods
4. **[No suppression/annotation awareness]:** No check for eslint-disable or other suppression. (Lines 33-68.)
5. **[No severity graduation]:** All findings are "info", which is appropriate but there's no distinction between a spread used in an immutable context (React state update, never mutated) vs. a spread where the copy is later deeply mutated.
6. **[Language idiom ignorance]:** In React/Redux, spread copy followed by property override is THE standard pattern: `return { ...state, loading: true }`. This is correct, immutable usage. Flagging it is pure noise.
7. **[Single-node detection]:** Reports the spread node in isolation. Does not check if the resulting object's nested properties are ever written to (which is what would make the shallow copy dangerous).

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Spread of identifier, spread of function call (skipped), no spread.
- **What's NOT tested:** Spread in React state update pattern, spread in Redux reducer, multiple spreads in one object (`{ ...a, ...b }`), spread in function argument (`fn({ ...obj })`), array spread, `Object.assign`, nested spread (`{ ...obj, nested: { ...obj.nested } }`), spread of member expression (`{ ...this.state }`).

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Tree-sitter query (same as current) for object spread
- **Why not graph:** Graph does not track spread operations.
- **Query:** Same `compile_spread_in_object_query()`
- **Returns:** All spread-in-object nodes.

#### Step 2: Narrowing
- **Tool:** Tree-sitter AST context + data flow heuristics
- **Query:** For each spread:
  - Check if the result variable is ever used in a mutation context (walk subsequent statements for assignment to `.nested.prop` on the result variable)
  - Check if in a React return statement / Redux reducer (walk ancestors for `return` in a function that matches reducer/component patterns)
  - If the spread target is a member expression (`{ ...this.state }`, `{ ...props }`), reduce severity (common safe pattern)
  - If the spread is followed by a nested spread (`{ ...obj, nested: { ...obj.nested, key: val } }`), suppress (developer is already doing deep copy of the mutated path)
- **Returns:** Only findings where the shallow copy is demonstrably dangerous (nested properties are mutated later).

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter comment scanning
- **Query:** Check eslint-disable comments. Check if file imports React/Redux (if so, suppress unless mutation is detected).
- **AI Prompt (for complex cases):**
  - **Context gathering query:** Tree-sitter: the spread statement + next 10 lines of code
  - **Context shape:** `{ "spread_line": "const copy = { ...state, loading: true };", "subsequent_lines": ["copy.nested.items.push(newItem);"], "file_imports": ["import { useReducer } from 'react'"] }`
  - **Prompt:** "This code creates a shallow copy via object spread. Are any nested properties of the copy subsequently mutated (which would affect the original due to shared references)? Answer: 'mutation_detected' with the line, or 'safe'."
  - **Expected return:** `{ "verdict": "mutation_detected", "mutation_line": "copy.nested.items.push(newItem);" } | { "verdict": "safe" }`
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Missing:** Assignment tracking / data flow for local variables. To detect "is the spread result's nested property later mutated," we need intra-function FlowsTo edges or a dedicated "MutatedAt" edge type.

### New Test Cases
1. **react_state_update** -- `return { ...state, loading: true };` -> suppressed (idiomatic React) -- Covers: high false positive rate, language idiom ignorance
2. **spread_then_deep_mutation** -- `const copy = { ...obj }; copy.nested.x = 1;` -> finding with severity `warning` -- Covers: no data flow tracking
3. **array_spread** -- `const copy = [...arr];` -> finding (not currently detected, should be) -- Covers: missing compound variants
4. **object_assign** -- `const copy = Object.assign({}, obj);` -> finding -- Covers: missing compound variants
5. **nested_spread_safe** -- `return { ...state, user: { ...state.user, name: 'new' } };` -> suppressed (deep copy of mutated path) -- Covers: high false positive rate
6. **spread_of_member** -- `const copy = { ...this.state };` -> reduced severity -- Covers: missing edge cases in tests
7. **eslint_disable** -- `// eslint-disable-next-line\nconst copy = { ...obj };` -> suppressed -- Covers: no suppression/annotation awareness
8. **multiple_spreads** -- `const merged = { ...defaults, ...overrides };` -> finding (both sources shallow) -- Covers: missing edge cases in tests

---

## Summary of Cross-Cutting Issues

### All 12 pipelines share these problems:

1. **Legacy `Pipeline` trait:** All use the legacy trait, missing graph and id_counts access. Should migrate to `GraphPipeline` (for cross-file analysis) or `NodePipeline` (for per-node metrics).

2. **No suppression/annotation awareness (all 12):** None of the pipelines check for `// eslint-disable`, `/* eslint-disable */`, `// eslint-disable-next-line <rule>`, `/* istanbul ignore */`, `// @ts-ignore`, `// @ts-expect-error`, or `// noinspection` (WebStorm). This is the single most impactful improvement across all pipelines.

3. **Minimal test coverage (all 12):** Tests average 3-5 per pipeline with only happy-path and basic negative cases. No edge cases, no suppression tests, no framework-specific patterns, no compound variant tests.

4. **No severity graduation (most):** Most pipelines use a single severity level. Real-world usefulness requires context-dependent severity.

5. **No graph usage (all 12):** None leverage the CodeGraph for cross-file analysis, even where it would significantly reduce false positives (e.g., event_listener_leak checking lifecycle methods, unhandled_promise tracking promise chains, argument_mutation tracking aliases).

### Priority ranking for fixes (highest impact first):

1. **event_listener_leak** -- Broken detection (file-level suppress-all logic). Must fix.
2. **implicit_globals** -- High false negative rate (missing declaration kinds). Must fix.
3. **unhandled_promise** -- High false negative rate (chained `.then()` mishandled). Must fix.
4. **argument_mutation** -- High false negative rate (only detects member assignment, misses push/splice/delete/subscript). Should fix.
5. **shallow_spread_copy** -- Extremely high false positive rate (every spread flagged). Should redesign or make opt-in.
6. **loose_equality** -- High false positive rate (`== null` idiom). Should fix.
7. **console_log_in_prod** -- No severity graduation (console.error vs console.log). Should fix.
8. **magic_numbers** -- Literal blindness (hex, negative, BigInt). Should fix.
9. **no_optional_chaining** -- High false positive rate (known-safe roots). Should fix.
10. **callback_hell** -- No severity graduation. Low priority fix.
11. **loose_truthiness** -- High false positive rate (idiomatic pattern). Consider removing or making opt-in.
12. **var_usage** -- Works correctly but noisy in legacy codebases. Low priority improvements.
