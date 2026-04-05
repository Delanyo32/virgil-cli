# C++ Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 12
- **Trait types used:** All 12 use `Pipeline` (legacy trait)
- **Overall assessment:** The 12 C++ tech debt pipelines are entirely text/string-matching and single-node tree-sitter detectors. None use the CodeGraph, CFG, taint engine, or cross-file analysis. Several pipelines rely heavily on `text.contains()` string matching rather than structured AST traversal, leading to false positives on comments and string literals. The pipelines have no suppression/annotation awareness, no severity graduation, and no data flow tracking. Test coverage is minimal (3-7 tests per pipeline, no adversarial cases). All should be migrated to `GraphPipeline` where cross-file or cross-function analysis would improve detection, and to `NodePipeline` for purely per-node checks.

---

## raw_memory_management

### Current Implementation
- **File:** `src/audit/pipelines/cpp/raw_memory_management.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `raw_new_delete`, `raw_array_allocation`
- **Detection method:** Uses `compile_new_expression_query` (`(new_expression) @new_expr`) and `compile_delete_expression_query` (`(delete_expression) @delete_expr`) to find all raw `new` and `delete` expressions. Filters out `new` inside smart pointer constructors by walking parent nodes to check for `unique_ptr`/`shared_ptr` in call_expression function text or init_declarator parent declaration text. `delete` has no suppression. Array allocations (`new int[100]`) are distinguished by checking if the node text contains `[`.

### Problems Identified
1. **High false negative rate (Rubric #3):** `is_smart_ptr_init` only checks direct parent chain (parent = argument_list, grandparent = call_expression or init_declarator). It misses `new` inside `make_unique`/`make_shared` factory wrappers, lambda captures, or multi-level nesting. A `new` passed to a helper function that wraps it in a smart pointer would still be flagged.
2. **Literal blindness (Rubric #8):** Array allocation detection uses `text.contains("[")` on the node text (line 88). This is fragile string matching -- it would also match `new SomeClass` if a template argument contained `[` in its text representation.
3. **No data flow tracking (Rubric #10):** A `new` expression whose result is immediately assigned to a `unique_ptr` variable in a separate statement (e.g., `auto raw = new int; unique_ptr<int> p(raw);`) is flagged because there's no data flow tracking to see the pointer is eventually wrapped.
4. **No suppression/annotation awareness (Rubric #11):** No support for `// NOLINT`, `// NOLINTNEXTLINE`, or any custom suppression comment.
5. **No severity graduation (Rubric #15):** Both `raw_new_delete` and `raw_array_allocation` are always "warning". Array allocations are arguably more dangerous than single-object allocations (risk of `delete` vs `delete[]` mismatch) and should be "error" or at least differentiated.
6. **Single-node detection (Rubric #14):** Each `new` and `delete` is analyzed in isolation. No pairing analysis to check whether a `new` has a matching `delete` (or vice versa) within the same scope/function/class.
7. **Missing compound variants (Rubric #9):** Does not detect `malloc`/`calloc`/`realloc`/`free` -- C-style raw memory management is equally problematic in C++ code but handled only by the C pipeline (if at all).

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** raw new, raw delete, array new, make_unique (no finding), smart ptr constructor (no finding), both new+delete together, metadata.
- **What's NOT tested:** `new` inside a lambda, `new` in a template function, placement new (`new (buf) T()`), `new` result stored in raw pointer then later assigned to smart ptr, `::new` (global operator new), overloaded operator new, `new` in header files, `delete[]` (array delete), suppression comments, nothrow new (`new(std::nothrow) T`).

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- iterate `graph.file_nodes` filtered to `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hxx`/`.hh` files.
- **Why not higher-ranked tool:** N/A, graph is highest.
- **Query:** `graph.file_nodes.iter().filter(|(path, _)| matches Language::Cpp)`
- **Returns:** Set of file paths and their NodeIndex values.

#### Step 2: Narrowing
- **Tool:** Tree-sitter query on each file.
- **Why not graph:** Graph does not index `new_expression`/`delete_expression` nodes; only Symbol, CallSite, Parameter, ExternalSource nodes exist. Raw `new`/`delete` are expression-level, below Symbol granularity.
- **Query:** `(new_expression) @new_expr` and `(delete_expression) @delete_expr` (existing queries).
- **Returns:** List of (file_path, line, node) for each raw new/delete.

#### Step 3: False Positive Removal
- **Tool:** Graph + tree-sitter combined.
- **Query/Prompt:**
  1. For each `new_expression`, walk the parent chain (tree-sitter) to check for smart pointer context (existing logic, extended to cover `make_unique`/`make_shared` wrapper calls).
  2. For each `new_expression` that survives, use `graph.find_symbol(file_path, function_start_line)` to find the enclosing function, then use `graph.function_cfgs[fn_node]` to check if the CFG has a `ResourceAcquire` followed by a matching `ResourceRelease` or smart-ptr assignment on the same variable (data flow via CFG `Assignment` statements).
  3. Check for NOLINT/suppression comments on the same or preceding line via tree-sitter comment query.
  4. For `delete` expressions, check if they are inside a destructor (parent function name starts with `~`), which is expected RAII cleanup -- downgrade to "info".
- **Returns:** Filtered list of findings with graduated severity.

#### Graph Enhancement Required
- **Resource lifecycle edges (Acquires/ReleasedBy):** Currently built by `ResourceAnalyzer` in `src/graph/resource.rs`. Verify that `new_expression` in C++ is recognized as a resource acquisition and that smart pointer assignment counts as a release. If not, extend `resource.rs` to handle C++ `new`/`delete`/smart-ptr patterns.
- **CFG CfgStatementKind::ResourceAcquire:** Verify the C++ CFG builder (`src/graph/cfg_languages/cpp.rs`) emits `ResourceAcquire` for `new` and `ResourceRelease` for `delete`/smart-ptr-reset.

### New Test Cases
1. **placement_new_skipped** -- `void f() { alignas(T) char buf[sizeof(T)]; new (buf) T(); }` -> No finding -- Covers: missing compound variants (#9)
2. **nothrow_new_detected** -- `void f() { int* p = new(std::nothrow) int; }` -> Finding -- Covers: missing compound variants (#9)
3. **new_in_lambda_detected** -- `void f() { auto fn = [&]() { int* p = new int; }; }` -> Finding -- Covers: missing edge cases (#6)
4. **global_operator_new** -- `void f() { int* p = ::new int; }` -> Finding -- Covers: missing edge cases (#6)
5. **delete_array_detected** -- `void f(int* p) { delete[] p; }` -> Finding with pattern `raw_new_delete` -- Covers: missing edge cases (#6)
6. **new_assigned_to_smartptr_later** -- `void f() { int* raw = new int; std::unique_ptr<int> p(raw); }` -> No finding (with graph data flow) -- Covers: no data flow tracking (#10)
7. **nolint_suppression** -- `void f() { int* p = new int; // NOLINT }` -> No finding -- Covers: no suppression awareness (#11)
8. **delete_in_destructor_downgraded** -- `class Foo { ~Foo() { delete data; } };` -> Finding with severity "info" -- Covers: no severity graduation (#15)
9. **malloc_in_cpp** -- `void f() { void* p = malloc(100); }` -> Finding for C-style allocation -- Covers: missing compound variants (#9)

---

## rule_of_five

### Current Implementation
- **File:** `src/audit/pipelines/cpp/rule_of_five.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `missing_rule_of_five`
- **Detection method:** Uses `compile_class_specifier_query` and `compile_struct_specifier_query` to find all classes and structs. For each, walks the `field_declaration_list` children looking for `function_definition` or `declaration` nodes. Detection of special members is entirely string-based: checks if the node text `contains("~ClassName")` for destructor, `contains("ClassName(const ClassName")` for copy constructor, `contains("operator=") && contains("const ClassName")` for copy assignment, `contains("ClassName(ClassName&&")` for move constructor, `contains("operator=") && contains("ClassName&&")` for move assignment. Only triggers if a destructor is found but fewer than 4 of the other special members are present.

### Problems Identified
1. **High false positive rate (Rubric #2):** String matching on node text is extremely fragile. A comment like `// See ClassName(const ClassName&) for details` inside a function body would cause `has_copy_constructor = true` (line 54). Conversely, `text.contains("operator=")` would match an unrelated operator= overload that isn't copy/move assignment.
2. **High false negative rate (Rubric #3):** The pipeline only checks `= default` and `= delete` for destructors (lines 78-80), not for the other four special members. A class with `~Foo() = default; Foo(const Foo&) = delete;` would see the destructor but not properly detect the deleted copy constructor via the general text matching (the `= delete` check is only on the destructor branch).
3. **Missing context (Rubric #4):** Does not check if the class manages resources. A class with a virtual destructor (polymorphism) but no raw resources does not necessarily need all five special members. The pipeline flags all classes with destructors regardless.
4. **No scope awareness (Rubric #7):** The `check_class_body` function only walks direct children of `field_declaration_list`. Special members defined in private/protected sections using access specifiers are skipped with `continue` at line 86-88, but the text matching still runs on `function_definition` and `declaration` children. However, out-of-line definitions (declared in-class, defined outside) are completely missed -- a class that declares `~Foo();` in-class and defines `Foo::~Foo() {}` outside the class body won't match.
5. **Missing compound variants (Rubric #9):** Does not check for `= delete` on any of the four copy/move members. A class that explicitly `= delete`s all copy/move operations is a valid pattern (non-copyable, non-movable) and should not be flagged, but the string matching does not reliably distinguish `= delete` from a real implementation.
6. **Language idiom ignorance (Rubric #13):** Does not recognize the "Rule of Zero" idiom -- classes that delegate resource management to RAII members (smart pointers, containers) should not define any special members. A class with only a user-defined destructor for debug logging should perhaps be "info" not "warning."
7. **Single-node detection (Rubric #14):** Analysis is strictly within one class body. If special members are defined out-of-line (common in C++), they are invisible.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Missing all four members, complete rule of five, no destructor, partial special members, metadata.
- **What's NOT tested:** `= delete` on copy/move, `= default` on copy/move, out-of-line definitions, templated classes, inherited special members, nested classes, comments containing special member signatures, virtual destructor without resource management, structs with inheritance.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_nodes` filtered to C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Graph query -- `graph.symbols_by_name` or iterate Symbol nodes where `kind == SymbolKind::Class || kind == SymbolKind::Struct`.
- **Why not tree-sitter first:** Graph already has all class/struct symbols indexed with file locations.
- **Query:** Iterate graph nodes, filter for `NodeWeight::Symbol { kind: Class | Struct, .. }`.
- **Returns:** List of (class_name, file_path, start_line, end_line) for each class/struct.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter on each class body.
- **Why not graph:** The graph does not index individual special member functions as distinct from other methods. Need AST to identify destructor, copy/move constructor/assignment by their declarator structure (not string matching).
- **Query:** Parse the class body using tree-sitter. For each `function_definition` or `declaration` child:
  - Destructor: child's declarator contains a `destructor_name` node (tree-sitter C++ grammar).
  - Copy constructor: declarator name matches class name AND parameters include `const ClassName&`.
  - Move constructor: declarator name matches class name AND parameters include `ClassName&&`.
  - Copy assignment: declarator is `operator_name` == "operator=" AND parameters include `const ClassName&`.
  - Move assignment: declarator is `operator_name` == "operator=" AND parameters include `ClassName&&`.
  - Check for `= default` and `= delete` on ALL special members (look for `default` or `delete` child nodes in the function body or declarator).
- **Additional graph check:** Use `graph.find_symbols_by_name("ClassName")` to find out-of-line definitions in other translation units. Check if special members are defined externally by looking for Symbol nodes named `ClassName::~ClassName`, `ClassName::ClassName`, `ClassName::operator=` across all files.
- **Returns:** Filtered findings that exclude classes with all five members (in-class or out-of-line), classes using `= delete` pattern, and classes without resource-managing members.

#### Graph Enhancement Required
- **Method-level Symbol resolution:** Currently, Symbol nodes are indexed at function/class level. Out-of-line method definitions like `Foo::~Foo()` may be indexed as Function symbols with name `~Foo` or qualified name. Verify that `symbols_by_name` indexes them properly with qualified names.

### New Test Cases
1. **default_all_five** -- Class with `= default` on all five -> No finding -- Covers: high false negative (#3)
2. **delete_copy_move** -- Class with `~Foo() {}; Foo(const Foo&) = delete; Foo& operator=(const Foo&) = delete;` and no move -> No finding (explicit non-copyable) -- Covers: missing compound variants (#9)
3. **out_of_line_destructor** -- `class Foo { ~Foo(); }; Foo::~Foo() { delete p; }` -> Finding (in-class body has no copy/move) -- Covers: no scope awareness (#7)
4. **comment_false_positive** -- Class with `// ~Foo() is defined elsewhere` in a comment -> Should NOT trigger destructor detection -- Covers: high false positive (#2)
5. **virtual_destructor_no_resources** -- `class Base { virtual ~Base() = default; };` -> "info" severity (polymorphic but no resources) -- Covers: language idiom ignorance (#13)
6. **rule_of_zero_smart_ptrs** -- Class with only smart pointer members and user-defined destructor for logging -> "info" -- Covers: language idiom ignorance (#13)
7. **templated_class** -- `template<typename T> class Holder { ~Holder() { delete data; } T* data; };` -> Finding -- Covers: missing edge cases (#6)
8. **nested_class** -- Inner class with destructor but missing others -> Finding on inner class only -- Covers: missing edge cases (#6)

---

## c_style_cast

### Current Implementation
- **File:** `src/audit/pipelines/cpp/c_style_cast.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `c_style_cast`
- **Detection method:** Uses `compile_cast_expression_query` (`(cast_expression type: (_) @cast_type value: (_) @cast_value) @cast_expr`) to find all C-style casts. Reports the cast type in the message. No filtering -- every `cast_expression` node is flagged.

### Problems Identified
1. **High false positive rate (Rubric #2):** Flags ALL C-style casts uniformly. Many are benign: `(void)unused_var` to suppress unused variable warnings, `(int)enum_value` for explicit enum-to-int conversions, casts in C compatibility code. These should be at most "info".
2. **No suppression/annotation awareness (Rubric #11):** No support for `// NOLINT` or any suppression mechanism.
3. **No severity graduation (Rubric #15):** All casts are "warning". Pointer casts (`(int*)p`) are far more dangerous than arithmetic casts (`(int)3.14`) and should be "error" or at least a higher severity.
4. **Missing context (Rubric #4):** Does not check what kind of cast it is. A `(void)x` cast is idiomatic C++ for suppressing unused variable warnings. A pointer-to-pointer cast is dangerous. A numeric cast is moderate risk.
5. **Single-node detection (Rubric #14):** Each cast is analyzed in isolation. No check for whether the cast is inside a template or macro expansion, which might justify it.
6. **Language idiom ignorance (Rubric #13):** Does not recognize `(void)` cast as an accepted C++ idiom. Modern C++ uses `static_cast<void>(x)` but `(void)x` is universally accepted and explicitly permitted by many style guides.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** C-style cast detection, static_cast skipped, pointer cast detected, metadata.
- **What's NOT tested:** `(void)` cast (should be info or skipped), `const_cast` equivalents (casting away const via C-style), `reinterpret_cast` equivalents (pointer type punning), casts in macro expansions, function-style casts (`int(3.14)` -- different tree-sitter node kind `functional_cast_expression`), suppression comments, multiple casts in one expression.

### Replacement Pipeline Design
**Target trait:** NodePipeline (purely per-node, no cross-file analysis needed)

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_nodes` filtered to C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `(cast_expression) @cast_expr` (existing query).
- **Why not graph:** Cast expressions are expression-level, not indexed in the graph.
- **Returns:** All C-style cast nodes with their type and value children.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter AST inspection.
- **Query/Prompt:**
  1. Check cast type: if `(void)`, mark as "info" or skip entirely.
  2. Check if cast involves pointer types (type text contains `*`): mark as "error" (pointer casts can violate type safety).
  3. Check if cast is in a macro expansion context (parent chain contains `preproc_function_def` or a macro invocation node).
  4. Check for NOLINT comment on same/preceding line.
  5. Numeric casts (int/float/double/char): "info".
- **Returns:** Findings with graduated severity.

#### Graph Enhancement Required
None -- this is inherently per-node.

### New Test Cases
1. **void_cast_skipped** -- `void f(int x) { (void)x; }` -> No finding or "info" -- Covers: language idiom ignorance (#13)
2. **pointer_cast_error** -- `void f(void* p) { int* ip = (int*)p; }` -> Finding with severity "error" -- Covers: no severity graduation (#15)
3. **numeric_cast_info** -- `void f() { int x = (int)3.14; }` -> Finding with severity "info" -- Covers: no severity graduation (#15)
4. **functional_cast** -- `void f() { int x = int(3.14); }` -> Finding (functional cast is also legacy-style) -- Covers: missing compound variants (#9)
5. **const_cast_via_c_style** -- `void f(const int* p) { int* q = (int*)p; }` -> Finding with "error" (casting away const) -- Covers: no severity graduation (#15)
6. **nolint_suppression** -- `void f() { int x = (int)3.14; // NOLINT }` -> No finding -- Covers: no suppression awareness (#11)
7. **cast_in_macro** -- `#define CAST(x) ((int)(x))` -> "info" (macro context) -- Covers: missing context (#4)

---

## large_object_by_value

### Current Implementation
- **File:** `src/audit/pipelines/cpp/large_object_by_value.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `large_object_by_value`
- **Detection method:** Uses `compile_parameter_declaration_query` to find all function parameters. Checks if the type matches a hardcoded list of `LARGE_TYPES` (string, vector, map, unordered_map, set, unordered_set, list, deque, array, shared_ptr, unique_ptr -- both with and without `std::` prefix). Skips parameters passed by reference (`&`) or pointer (`*`) by checking the declarator node kind and full declaration text. Reports "info" severity.

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric #12):** The `LARGE_TYPES` list is hardcoded and incomplete. Missing: `std::tuple`, `std::pair`, `std::any`, `std::optional`, `std::function`, `std::basic_string`, `std::multimap`, `std::multiset`, `std::unordered_multimap`, `std::unordered_multiset`, `std::stack`, `std::queue`, `std::priority_queue`, `std::bitset`, `std::valarray`, and any user-defined large types.
2. **High false negative rate (Rubric #3):** `shared_ptr` and `unique_ptr` are in `LARGE_TYPES` but they are small objects (8-16 bytes). Passing them by value is actually idiomatic: `unique_ptr` by value transfers ownership, `shared_ptr` by value increments refcount. These should NOT be flagged.
3. **Missing context (Rubric #4):** Does not check if the function is a "sink" (takes ownership). `void take_ownership(std::unique_ptr<Foo> p)` is the correct pattern for ownership transfer. Does not check if the parameter is used in a move context.
4. **No scope awareness (Rubric #7):** Checks all parameter declarations globally, including those in lambda expressions, operator overloads, and function templates where by-value might be intentional (perfect forwarding templates, `std::move` semantics).
5. **Language idiom ignorance (Rubric #13):** `unique_ptr` pass-by-value is THE idiomatic C++ way to express ownership transfer. `shared_ptr` pass-by-value is idiomatic when the callee needs to share ownership. Flagging these is actively harmful.
6. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
7. **Literal blindness (Rubric #8):** `is_large_type` uses `base.ends_with(t)` (line 54), which would match `my_custom_vector` as a "vector" type, or `my_shared_ptr` as a "shared_ptr" type.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** string by value, vector by value, const ref skipped, pointer skipped, primitive types skipped, map by value, metadata.
- **What's NOT tested:** `unique_ptr` by value (false positive), `shared_ptr` by value (false positive), user-defined large types, template parameters, rvalue reference parameters (`T&&`), `const T` by value (sometimes intentional), nested templates (`std::vector<std::string>`), typedef/using aliases (`using StringVec = std::vector<std::string>;`), function-style parameter lists in lambdas.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Graph query -- filter C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `compile_parameter_declaration_query` (existing).
- **Why not graph:** Parameter nodes are not indexed in the graph.
- **Returns:** All parameter declarations with type and declarator.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter AST inspection.
- **Query/Prompt:**
  1. Remove `unique_ptr` and `shared_ptr` from the large types list entirely.
  2. Use exact type name matching (not `ends_with`) -- match `std::string` and `string` but not `my_string`.
  3. Check for rvalue reference (`&&`) -- by-value-then-move is idiomatic, but the parameter type already implies by-value.
  4. Check for NOLINT suppression.
  5. Distinguish `const T` by value vs `T` by value -- `const T` by value is sometimes intentional for thread safety.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
None -- per-node analysis.

### New Test Cases
1. **unique_ptr_by_value_ok** -- `void take(std::unique_ptr<Foo> p) {}` -> No finding -- Covers: language idiom ignorance (#13)
2. **shared_ptr_by_value_ok** -- `void share(std::shared_ptr<Foo> p) {}` -> No finding -- Covers: language idiom ignorance (#13)
3. **rvalue_ref_skipped** -- `void f(std::string&& s) {}` -> No finding -- Covers: missing context (#4)
4. **typedef_alias** -- `using Strings = std::vector<std::string>; void f(Strings v) {}` -> Finding (type alias resolves to vector) -- Covers: literal blindness (#8), though hard without type resolution
5. **custom_type_not_matched** -- `void f(my_vector v) {}` -> No finding -- Covers: literal blindness (#8)
6. **template_parameter** -- `template<typename T> void f(T val) {}` -> No finding (cannot determine size) -- Covers: missing context (#4)
7. **const_by_value** -- `void f(const std::string s) {}` -> Finding -- Covers: missing edge cases

---

## endl_flush

### Current Implementation
- **File:** `src/audit/pipelines/cpp/endl_flush.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `endl_flush`
- **Detection method:** Uses `compile_qualified_identifier_query` (`(qualified_identifier) @qualified_id`) to find all qualified identifiers. Filters to those whose text equals exactly `"std::endl"`. Reports "info" severity.

### Problems Identified
1. **High false negative rate (Rubric #3):** Only matches `std::endl` as a qualified identifier. Misses: `using namespace std; ... endl` (bare `endl` after using directive), `using std::endl; ... endl`, `std::flush` (same performance issue). Also misses `endl` used with `using` declarations.
2. **Missing context (Rubric #4):** Does not check whether the `endl` is used in a performance-critical context (tight loop) vs. a one-time log message. Inside a loop, `endl` is a real performance problem; in a one-time `cout << "Error: " << msg << endl;` it's negligible.
3. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
4. **Single-node detection (Rubric #14):** Each `endl` is analyzed independently. No loop context detection.
5. **No severity graduation (Rubric #15):** All findings are "info" regardless of context. `endl` inside a hot loop should be "warning"; outside a loop should be "info".
6. **Missing compound variants (Rubric #9):** Does not detect `std::flush`, which has the same performance implication.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** std::endl detected, newline char no finding, multiple endl, other qualified identifiers skipped, metadata.
- **What's NOT tested:** bare `endl` after `using namespace std`, `std::flush`, `endl` inside a loop (should escalate severity), `endl` in `cerr`/`clog` (less concerning since cerr is unbuffered anyway), suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- filter C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `(qualified_identifier) @qualified_id` plus `(identifier) @id` for bare `endl` after `using` directives.
- **Why not graph:** `endl` is an identifier in expression context, not a symbol/callsite in the graph.
- **Query:** Two queries: qualified_identifier matching `std::endl` or `std::flush`, and bare `identifier` matching `endl` or `flush` (validated by checking for a `using namespace std` or `using std::endl` earlier in the file).
- **Returns:** All `endl`/`flush` usage locations.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph.
- **Query/Prompt:**
  1. For each finding, check if it's inside a loop body (walk parent chain for `for_statement`, `while_statement`, `do_statement`, `for_range_loop`). If yes, severity = "warning".
  2. Use `graph.find_symbol(file_path, line)` to find the enclosing function, then check if the function is called inside a loop via `graph.traverse_callers()` (depth 2) -- if any caller calls this function in a loop, escalate.
  3. If `endl` is used with `std::cerr` (parent is `<<` expression with `cerr` on the left side), downgrade or skip (cerr is unbuffered by default).
  4. Check for NOLINT suppression.
- **Returns:** Findings with graduated severity.

#### Graph Enhancement Required
- None strictly required, but loop detection via CFG would be more reliable. Check if `graph.function_cfgs` has back-edge detection for loops.

### New Test Cases
1. **bare_endl_after_using** -- `using namespace std; void f() { cout << endl; }` -> Finding -- Covers: high false negative (#3)
2. **std_flush_detected** -- `void f() { std::cout << std::flush; }` -> Finding -- Covers: missing compound variants (#9)
3. **endl_in_loop_warning** -- `void f() { for(int i=0;i<100;i++) std::cout << i << std::endl; }` -> Finding with severity "warning" -- Covers: no severity graduation (#15)
4. **endl_outside_loop_info** -- `void f() { std::cout << "done" << std::endl; }` -> Finding with severity "info" -- Covers: no severity graduation (#15)
5. **endl_with_cerr_skipped** -- `void f() { std::cerr << "error" << std::endl; }` -> No finding or "info" (cerr is unbuffered) -- Covers: missing context (#4)
6. **nolint_suppression** -- `std::cout << std::endl; // NOLINT` -> No finding -- Covers: no suppression awareness (#11)
7. **using_std_endl** -- `using std::endl; void f() { std::cout << endl; }` -> Finding -- Covers: high false negative (#3)

---

## missing_override

### Current Implementation
- **File:** `src/audit/pipelines/cpp/missing_override.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `missing_override`
- **Detection method:** Uses class/struct specifier queries. For classes/structs with a `base_class_clause`, walks the body's children looking for `function_definition` or `declaration` nodes. Detection of `virtual` keyword is done by: (1) checking if text `starts_with("virtual")` or `contains(" virtual ")`, (2) walking children looking for a node with `kind() == "virtual"` or text == `"virtual"`. Skips destructors (text contains `~`), overrides (text contains `"override"`), finals (text contains `"final"`), and pure virtuals (text contains `"= 0"`). Only flags virtual methods in derived classes (those with base_class_clause).

### Problems Identified
1. **High false positive rate (Rubric #2):** The virtual keyword detection via string matching (line 59: `text.starts_with("virtual")` or `contains(" virtual ")`) is fragile. A function whose name or parameter type contains "virtual" would also match. A comment `// virtual method` in the function body would also trigger.
2. **High false negative rate (Rubric #3):** Methods that override a base class virtual method but DON'T use the `virtual` keyword in the derived class are NOT flagged. In C++, a derived class can override without redeclaring `virtual` -- the method is implicitly virtual. This pipeline ONLY flags methods explicitly marked `virtual`, missing the main use case: methods that ARE overrides but lack the `override` specifier.
3. **Broken detection (Rubric #1):** The pipeline's fundamental logic is inverted. It finds `virtual` methods and checks for `override`. But the real problem is methods that override base class methods WITHOUT using `override`. Since the pipeline can't see the base class methods (single-file analysis), it uses `virtual` as a proxy, which is wrong -- a method can override without being explicitly `virtual`.
4. **No scope awareness (Rubric #7):** Only checks direct children of the `field_declaration_list`. Methods defined inside access specifiers or nested scopes may be missed.
5. **Literal blindness (Rubric #8):** Destructor skip uses `text.contains("~")` (line 75), which would match any text containing a tilde, not just destructors.
6. **Single-node detection (Rubric #14):** Cannot actually verify if a method overrides a base class method because base class definition may be in another file. The `has_base_class` check only verifies inheritance exists, not that specific methods are being overridden.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Missing override detected, override present (no finding), no base class (no finding), pure virtual skipped, final skipped, metadata.
- **What's NOT tested:** Method overriding without `virtual` keyword (the main real-world case), multiple inheritance, out-of-line method definitions, protected virtual methods, `virtual` keyword in a comment triggering false positive, method with "virtual" in parameter name, covariant return types.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all Class/Struct Symbol nodes.
- **Query:** Filter graph Symbol nodes for `kind == Class || kind == Struct`.
- **Returns:** Class/struct list with locations.

#### Step 2: Narrowing
- **Tool:** Tree-sitter on each class/struct.
- **Why not graph:** Graph does not encode inheritance hierarchy or virtual method information.
- **Query:** For each class with a `base_class_clause`: extract all method declarations/definitions. Use proper AST node kinds (not string matching) to identify:
  - `virtual_function_specifier` child node for `virtual` keyword
  - `virtual_specifier` child for `override`/`final`
  - `destructor_name` node for destructors
  - Check for `= 0` via `pure_virtual_clause` node
- **Returns:** Methods that are virtual (explicitly or inherited) but lack `override`.

#### Step 3: False Positive Removal
- **Tool:** Graph + AI prompt for cross-file analysis.
- **Query/Prompt:**
  1. Use `graph.find_symbols_by_name(base_class_name)` to locate the base class definition.
  2. Extract base class virtual methods via tree-sitter on the base class file.
  3. Match derived class methods against base class virtual methods by name and parameter types.
  4. Only flag methods that genuinely override a base class virtual method but lack `override`.
  5. Check for NOLINT suppression.
- **Returns:** Verified findings with base class reference in the message.

#### Graph Enhancement Required
- **Inheritance edges:** The graph currently has no edge type for class inheritance (no `InheritsFrom` or `Extends` edge). Adding this would allow traversal of the inheritance hierarchy to find all virtual methods in the base class chain.
- **Virtual method annotation:** Symbol nodes could carry a `is_virtual` flag to enable graph-level override checking.

### New Test Cases
1. **implicit_virtual_no_override** -- Derived method overrides base virtual without `virtual` keyword or `override` -> Finding -- Covers: high false negative (#3)
2. **virtual_in_comment** -- `class D : public B { /* virtual method */ void foo() {} };` -> No finding (comment should not trigger) -- Covers: high false positive (#2)
3. **multiple_inheritance** -- Class inheriting from two bases, overriding methods from both -> Findings for both -- Covers: missing edge cases
4. **cross_file_base_class** -- Base class in separate file, derived class overrides without `override` -> Finding (requires graph) -- Covers: single-node detection (#14)
5. **covariant_return_type** -- `class D : public B { virtual D* clone() {} };` where base has `virtual B* clone()` -> Finding (still needs override) -- Covers: missing edge cases
6. **tilde_in_parameter** -- Method with parameter type containing `~` -> No false positive -- Covers: literal blindness (#8)
7. **protected_virtual** -- Protected virtual method without override -> Finding -- Covers: no scope awareness (#7)

---

## raw_union

### Current Implementation
- **File:** `src/audit/pipelines/cpp/raw_union.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `raw_union`
- **Detection method:** Uses `compile_union_specifier_query` (`(union_specifier name: (type_identifier)? @union_name) @union_def`) to find all union declarations. Reports every union as "info" severity with a suggestion to use `std::variant`.

### Problems Identified
1. **High false positive rate (Rubric #2):** Flags ALL unions indiscriminately, including: anonymous unions inside classes (a valid C++ pattern for overlapping storage), unions used in low-level code (serialization, hardware register mapping), unions in C compatibility layers, and unions that are already part of a tagged union pattern (union + enum discriminator).
2. **Missing context (Rubric #4):** Does not check if the union is already part of a tagged union pattern (a struct containing both a union and an enum discriminator). This is the pre-C++17 pattern and is perfectly safe.
3. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
4. **Language idiom ignorance (Rubric #13):** Anonymous unions inside structs/classes are idiomatic C++ for overlapping member access. `std::variant` is not a replacement for anonymous unions.
5. **No severity graduation (Rubric #15):** All unions are "info" regardless of risk. A union containing non-trivial types (std::string, std::vector) is undefined behavior pre-C++17 and should be "error". A union of POD types with a discriminator is safe and should be at most "info".
6. **Single-node detection (Rubric #14):** Does not check if the union contains non-trivially-destructible types, which is the actual danger.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Named union, anonymous union, no union, multiple unions.
- **What's NOT tested:** Anonymous union inside a struct (should be info or skipped), tagged union pattern (union + enum), union with non-trivial members (should be error), union in `extern "C"` block (C interop, should be skipped), `std::variant` present in same file (already modernized), suppression comments.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Graph query -- filter C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `(union_specifier) @union_def` (existing query).
- **Why not graph:** Graph indexes Union as a Symbol node but doesn't carry details about members or context.
- **Returns:** All union definitions.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter AST inspection.
- **Query/Prompt:**
  1. Check if union is anonymous (no `name` child) AND inside a `class_specifier` or `struct_specifier` -> skip or "info" (idiomatic anonymous union).
  2. Check if union is inside `extern "C"` -> skip (C interop).
  3. Walk union body members: if any member type is a non-trivial type (contains `string`, `vector`, `shared_ptr`, or has a class/struct type), severity = "warning" (risk of UB).
  4. Check if the enclosing struct/class has an enum member alongside the union (tagged union pattern) -> "info".
  5. Check for NOLINT suppression.
- **Returns:** Findings with graduated severity.

#### Graph Enhancement Required
None.

### New Test Cases
1. **anonymous_union_in_struct_info** -- `struct Foo { union { int i; float f; }; };` -> No finding or "info" -- Covers: language idiom ignorance (#13)
2. **tagged_union_pattern** -- `struct Val { enum { INT, FLT } tag; union { int i; float f; }; };` -> "info" (tagged, safe) -- Covers: missing context (#4)
3. **union_with_string_error** -- `union Bad { std::string s; int i; };` -> "warning" or "error" (UB) -- Covers: no severity graduation (#15)
4. **extern_c_union_skipped** -- `extern "C" { union Data { int i; float f; }; }` -> No finding or "info" -- Covers: language idiom ignorance (#13)
5. **nolint_suppression** -- `union Data { int i; float f; }; // NOLINT` -> No finding -- Covers: no suppression awareness (#11)
6. **union_with_trivial_members_only** -- `union Reg { uint32_t raw; struct { uint16_t lo; uint16_t hi; }; };` -> "info" (hardware register mapping) -- Covers: missing context (#4)

---

## excessive_includes

### Current Implementation
- **File:** `src/audit/pipelines/cpp/excessive_includes.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `excessive_includes`
- **Detection method:** Uses `compile_preproc_include_query` (`(preproc_include) @include_dir`) to count all `#include` directives in a file. Compares against `INCLUDE_THRESHOLD_SOURCE` (20 for source files) and `INCLUDE_THRESHOLD_HEADER` (15 for header files). Reports "info" severity if count exceeds threshold.

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric #12):** 20 for source files and 15 for headers are arbitrary. A large source file implementing many features legitimately needs many includes. A small utility header might need 15+ includes if it uses many STL types. No justification or configurability.
2. **High false positive rate (Rubric #2):** Counts ALL includes, including standard library headers, project headers, and conditional includes (`#ifdef`-guarded includes). A file that includes `<algorithm>`, `<vector>`, `<string>`, `<map>`, `<iostream>`, `<fstream>`, `<sstream>`, `<memory>`, `<functional>`, `<utility>`, `<chrono>`, `<thread>`, `<mutex>`, `<condition_variable>`, `<atomic>`, `<numeric>`, `<iterator>`, `<stdexcept>`, `<cassert>`, `<cmath>`, `<cstdint>` (21 standard headers) is flagged, even though these are all legitimate.
3. **Missing context (Rubric #4):** Does not distinguish system includes (`<>`) from project includes (`""`). Does not check how many of the includes are actually used.
4. **No data flow tracking (Rubric #10):** Cannot determine unused includes (would require analyzing which symbols from each header are actually used in the file).
5. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
6. **No severity graduation (Rubric #15):** Always "info" regardless of how far over the threshold. A file with 100 includes is much worse than one with 21.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Under threshold (no finding), over threshold source, header lower threshold, header at threshold, empty file.
- **What's NOT tested:** File at exactly the source threshold (20), conditional includes (`#ifdef`), mix of system and project includes, very large include count (100+), `.h` files (treated as header by the `is_header` method -- correctly includes `.h`), `.hh` extension, suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- `graph.file_nodes` filtered to C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Graph query -- `graph.file_dependency_edges()` to get per-file import counts.
- **Why not tree-sitter first:** The graph already tracks import relationships via Imports edges.
- **Query:** For each C++ file node, count outgoing Imports edges. Compare against thresholds.
- **Returns:** Files with excessive import counts.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter for additional context.
- **Query/Prompt:**
  1. Separate system includes (tree-sitter `system_lib_string` path child) from project includes (`string_literal` path child). Apply different thresholds.
  2. Check file size (line count from graph `file_entries()`). Scale threshold with file size: a 2000-line file legitimately needs more includes than a 50-line file.
  3. Graduate severity: 1.5x threshold = "info", 2x threshold = "warning", 3x threshold = "error".
  4. Check for NOLINT suppression.
- **Returns:** Findings with graduated severity and breakdown of system vs. project includes.

#### Graph Enhancement Required
None -- Imports edges already exist.

### New Test Cases
1. **at_exactly_threshold** -- 20 includes in source file -> No finding -- Covers: boundary condition
2. **conditional_includes** -- `#ifdef WIN32 #include <windows.h> #endif` -> Should count or not based on policy -- Covers: missing context (#4)
3. **severity_graduation** -- 30 includes -> "info"; 40 includes -> "warning"; 60 includes -> "error" -- Covers: no severity graduation (#15)
4. **system_vs_project** -- 15 system + 6 project -> maybe only flag project includes -- Covers: missing context (#4)
5. **large_file_higher_threshold** -- 2000-line file with 25 includes -> No finding (proportional) -- Covers: hardcoded thresholds (#12)
6. **nolint_suppression** -- File with excessive includes but `// NOLINT` at top -> No finding -- Covers: no suppression awareness (#11)
7. **h_extension_as_header** -- `test.h` with 16 includes -> Finding (header threshold is 15) -- Covers: boundary condition

---

## exception_across_boundary

### Current Implementation
- **File:** `src/audit/pipelines/cpp/exception_across_boundary.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `exception_across_boundary`
- **Detection method:** Uses `compile_throw_statement_query` (`(throw_statement) @throw_stmt`) to find all `throw` statements. For each, walks the parent chain looking for a `linkage_specification` node containing a `string_literal` child with text `"\"C\""`. Reports "error" severity.

### Problems Identified
1. **High false negative rate (Rubric #3):** Only detects direct `throw` statements. Does not detect: functions that call other functions which may throw (no call graph analysis), `throw` in templates instantiated within `extern "C"`, standard library functions that throw (e.g., `std::vector::at()`, `std::stoi()`), or implicit exceptions from `new` (bad_alloc).
2. **Missing context (Rubric #4):** Does not check for `noexcept` specifier on the function. A function declared `noexcept` inside `extern "C"` that throws will terminate the program (different issue), but the pipeline doesn't differentiate.
3. **No data flow tracking (Rubric #10):** A `throw` inside a `try-catch` block within `extern "C"` is safe if the catch handles all exceptions. The pipeline does not check for enclosing try-catch.
4. **Missing compound variants (Rubric #9):** Does not detect other exception-generating patterns: `dynamic_cast` on references (throws `bad_cast`), `typeid` on null pointer (throws `bad_typeid`), container `.at()` (throws `out_of_range`).
5. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
6. **Single-node detection (Rubric #14):** Each throw is analyzed independently. Does not check if the throw is inside a try-catch block within the extern "C" function.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** throw in extern C, throw in regular C++ (no finding), extern C without throw, nested throw in extern C, metadata.
- **What's NOT tested:** throw inside try-catch within extern C (false positive), calling a throwing function from extern C, `noexcept` function with throw, `extern "C++"` block inside `extern "C"` block (should not flag), function pointer to throwing function called from extern C, dynamic_cast in extern C.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- filter C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter to find `extern "C"` blocks, then identify all functions within them.
- **Why not graph:** Graph does not encode linkage specification information.
- **Query:**
  1. Find all `linkage_specification` nodes with `"C"` string literal.
  2. Within those blocks, extract all `function_definition` nodes.
  3. For each function, find `throw_statement` nodes.
- **Returns:** Functions in extern "C" blocks that contain throw statements.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph.
- **Query/Prompt:**
  1. For each throw, walk parent chain for enclosing `try_statement`. If found, check if the catch clause is `catch(...)` or catches the specific exception type -> skip (exception is handled).
  2. Use `graph.find_symbol()` for the extern "C" function, then `graph.traverse_callees([fn_node], 3)` to find transitive callees. Check if any callee is known to throw (name-based heuristic: `at`, `stoi`, `stof`, `dynamic_cast`, or functions without `noexcept`).
  3. Check for NOLINT suppression.
- **Returns:** Findings including transitive throw risk.

#### Graph Enhancement Required
- **Linkage annotation on Symbol nodes:** Add an optional `linkage` field to Symbol nodes to indicate `extern "C"` vs C++ linkage.
- **noexcept annotation:** Track whether functions are declared `noexcept`.

### New Test Cases
1. **throw_in_try_catch_ok** -- `extern "C" { void f() { try { throw 42; } catch(...) {} } }` -> No finding -- Covers: no data flow tracking (#10)
2. **calling_throwing_function** -- `void thrower() { throw 42; } extern "C" { void f() { thrower(); } }` -> Finding (transitive throw) -- Covers: single-node detection (#14)
3. **extern_cpp_in_extern_c** -- `extern "C" { extern "C++" { void f() { throw 42; } } }` -> No finding (inner C++ linkage) -- Covers: missing context (#4)
4. **noexcept_with_throw** -- `extern "C" { void f() noexcept { throw 42; } }` -> Finding with different message (terminate, not UB) -- Covers: missing context (#4)
5. **dynamic_cast_in_extern_c** -- `extern "C" { void f(Base& b) { Derived& d = dynamic_cast<Derived&>(b); } }` -> Finding -- Covers: missing compound variants (#9)
6. **nolint_suppression** -- `extern "C" { void f() { throw 42; // NOLINT } }` -> No finding -- Covers: no suppression awareness (#11)

---

## uninitialized_member

### Current Implementation
- **File:** `src/audit/pipelines/cpp/uninitialized_member.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `uninitialized_member`
- **Detection method:** Uses `compile_field_declaration_query` to find all field declarations inside class/struct bodies. Checks if the type is a primitive (from `PRIMITIVE_TYPES` list). Checks for initializers via `text.contains('=')` or `text.contains('{')` on the full declaration text, and `default_value` field or `bitfield_clause`/`initializer_list` child on the declarator node. Skips non-primitive types (assumes they have default constructors). Reports "warning" severity.

### Problems Identified
1. **High false negative rate (Rubric #3):** Only checks for in-class member initializers (NSDMIs). Does not check constructor member initializer lists. A class with `Foo() : x(0) {}` initializes `x` in the constructor, but the pipeline still flags `int x;` as uninitialized.
2. **Literal blindness (Rubric #8):** `has_initializer` uses `text.contains('=')` (line 65), which would match a field like `bool operator=(const Foo&);` as "initialized" because the text contains `=`. Also, `text.contains('{')` would match if the field type has a brace in its template arguments.
3. **Missing compound variants (Rubric #9):** Does not check enum members (enum values stored as member variables), pointer members (raw pointers are primitives and often uninitialized), or C-style array members (`int arr[10];`).
4. **No scope awareness (Rubric #7):** Does not consider constructors. If ALL constructors initialize the member, it's not truly uninitialized.
5. **Hardcoded thresholds without justification (Rubric #12):** The `PRIMITIVE_TYPES` list includes `ptrdiff_t`, `intptr_t`, `uintptr_t` but misses `wchar_t`, `char16_t`, `char32_t`, `char8_t`, `ssize_t`, raw pointers, and enum types.
6. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
7. **Single-node detection (Rubric #14):** Does not check constructors for member initializer lists. Cannot see if the member is initialized in every constructor.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** Uninitialized int, initialized with `=`, brace-initialized, non-primitive skipped, struct member, multiple uninitialized, metadata.
- **What's NOT tested:** Member initialized in constructor initializer list (false positive), pointer member (`int* ptr;`), enum member, bitfield member (`int x : 3;`), `= default` constructor, `static` member (should be skipped -- different initialization rules), `mutable` member, array member, member with `volatile` qualifier, suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find Class/Struct Symbol nodes.
- **Returns:** Class/struct locations.

#### Step 2: Narrowing
- **Tool:** Tree-sitter on each class/struct body.
- **Why not graph:** Graph does not index individual field declarations.
- **Query:** `(field_declaration type: (_) @field_type declarator: (_) @field_declarator) @field_decl` inside class/struct specifiers. Filter to primitive types.
- **Returns:** Primitive member fields without in-class initializers.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph.
- **Query/Prompt:**
  1. For each class/struct, find all constructors (function_definition where declarator name matches class name, or `destructor_name` for destructors -- actually constructors don't have destructor_name).
  2. Parse each constructor's member initializer list (`field_initializer_list` node) to check if the flagged member is initialized.
  3. If ALL constructors initialize the member, skip the finding.
  4. Use `graph.find_symbols_by_name(class_name)` to find out-of-line constructor definitions.
  5. Add `static` member detection (walk children for `storage_class_specifier` == "static") and skip.
  6. Check for NOLINT suppression.
- **Returns:** Only members that are truly uninitialized (no NSDMI, no constructor init in ALL paths).

#### Graph Enhancement Required
- **Constructor analysis:** Would benefit from Symbol nodes carrying constructor information, or a dedicated edge type for "initializes member."

### New Test Cases
1. **initialized_in_constructor** -- `class Foo { int x; Foo() : x(0) {} };` -> No finding -- Covers: high false negative (#3), single-node detection (#14)
2. **static_member_skipped** -- `class Foo { static int count; };` -> No finding -- Covers: missing edge cases
3. **pointer_member** -- `class Foo { int* ptr; };` -> Finding -- Covers: missing compound variants (#9)
4. **enum_member** -- `enum Color { RED }; class Foo { Color c; };` -> Finding -- Covers: missing compound variants (#9)
5. **bitfield_member** -- `class Foo { int x : 3; };` -> No finding (bitfield) -- Covers: missing edge cases
6. **multiple_constructors_partial** -- `class Foo { int x; Foo() : x(0) {} Foo(int) {} };` -> Finding (second constructor doesn't init x) -- Covers: no scope awareness (#7)
7. **volatile_member** -- `class Foo { volatile int x; };` -> Finding -- Covers: missing compound variants (#9)
8. **array_member** -- `class Foo { int arr[10]; };` -> Finding -- Covers: missing compound variants (#9)
9. **nolint_suppression** -- `class Foo { int x; // NOLINT };` -> No finding -- Covers: no suppression awareness (#11)

---

## shared_ptr_cycle_risk

### Current Implementation
- **File:** `src/audit/pipelines/cpp/shared_ptr_cycle_risk.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `shared_ptr_cycle_risk`
- **Detection method:** Uses `compile_field_declaration_query` to find all field declarations inside class/struct bodies. Checks if the type text or full declaration text `contains("shared_ptr")`. Reports "info" severity for every `shared_ptr` class member.

### Problems Identified
1. **High false positive rate (Rubric #2):** Flags ALL `shared_ptr` members, not just those that could create cycles. A `shared_ptr<SomeResource>` that has no back-reference to the owning class is perfectly safe. Only `shared_ptr` members where the pointed-to type also holds a `shared_ptr` back to this class (or transitively) create cycles.
2. **Literal blindness (Rubric #8):** `is_shared_ptr_type` uses `type_text.contains("shared_ptr")` (line 27). This would match `my_shared_ptr_wrapper`, `not_a_shared_ptr`, or any type whose name contains the substring "shared_ptr".
3. **No data flow tracking (Rubric #10):** Cannot detect actual cycles. Would need cross-class analysis: if class A has `shared_ptr<B>` and class B has `shared_ptr<A>`, that's a cycle. But the pipeline has no cross-file or even cross-class awareness.
4. **Missing context (Rubric #4):** Does not check what type the `shared_ptr` points to. A `shared_ptr<int>` cannot create a cycle. Only `shared_ptr<SomeClass>` where `SomeClass` has reference back to the container matters.
5. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
6. **No severity graduation (Rubric #15):** All findings are "info" regardless of risk. Self-referential `shared_ptr<SameClass>` is high risk; `shared_ptr<int>` is zero risk.
7. **Single-node detection (Rubric #14):** Each `shared_ptr` member is analyzed in isolation. Cannot detect the mutual-reference pattern that actually causes cycles.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** shared_ptr member detected, weak_ptr not detected, unique_ptr not detected, multiple shared_ptrs, local variable not detected, metadata.
- **What's NOT tested:** `shared_ptr<int>` (false positive -- no cycle possible), self-referential `shared_ptr<SameClass>` (highest risk), mutual reference between two classes, `shared_ptr` in a comment or string, custom type containing "shared_ptr" in its name, `boost::shared_ptr`, `shared_ptr` as return type (not a member), suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query -- find all Class/Struct Symbol nodes.
- **Returns:** All classes/structs.

#### Step 2: Narrowing
- **Tool:** Tree-sitter on each class body.
- **Why not graph:** Graph does not index field types.
- **Query:** `field_declaration` inside class/struct where type contains `shared_ptr`. Extract the template argument type (the pointed-to type).
- **Returns:** List of (class_name, field_name, pointed_to_type) tuples.

#### Step 3: False Positive Removal
- **Tool:** Graph cross-class analysis.
- **Query/Prompt:**
  1. For `shared_ptr<T>` where T is a primitive type (int, float, etc.) -> skip entirely.
  2. For `shared_ptr<SameClass>` -> high risk, severity "warning" (self-referential).
  3. For `shared_ptr<OtherClass>`, use `graph.find_symbols_by_name("OtherClass")` to locate OtherClass definition. Parse its body for `shared_ptr<OriginalClass>` members. If found -> cycle confirmed, severity "warning". If OtherClass has `weak_ptr<OriginalClass>` -> skip (already using weak_ptr to break cycle).
  4. Transitive cycle detection: BFS through `shared_ptr` member relationships up to depth 3.
  5. If no cycle detected -> skip or "info" with low confidence.
  6. Check for NOLINT suppression.
- **Returns:** Only findings where cycle risk is confirmed or self-referential.

#### Graph Enhancement Required
- **Class member type edges:** A new edge type `HasMember { member_name, member_type }` from Class/Struct nodes to the pointed-to type's Symbol node would enable graph-level cycle detection without re-parsing.
- **Ownership graph:** A dedicated ownership relationship (owns/borrows) between classes via shared_ptr/weak_ptr would make cycle detection a simple graph cycle check.

### New Test Cases
1. **shared_ptr_int_no_finding** -- `class Foo { std::shared_ptr<int> p; };` -> No finding (no cycle possible) -- Covers: high false positive (#2)
2. **self_referential_warning** -- `class Node { std::shared_ptr<Node> next; };` -> Finding with severity "warning" -- Covers: no severity graduation (#15)
3. **mutual_reference_cycle** -- `class A { std::shared_ptr<B> b; }; class B { std::shared_ptr<A> a; };` -> Finding on both -- Covers: no data flow tracking (#10)
4. **weak_ptr_breaks_cycle** -- `class A { std::shared_ptr<B> b; }; class B { std::weak_ptr<A> a; };` -> No finding on B -- Covers: missing context (#4)
5. **custom_type_name** -- `class my_shared_ptr_wrapper {}; class Foo { my_shared_ptr_wrapper w; };` -> No finding -- Covers: literal blindness (#8)
6. **boost_shared_ptr** -- `class Foo { boost::shared_ptr<Foo> self; };` -> Finding -- Covers: missing compound variants (#9)
7. **nolint_suppression** -- `class Foo { std::shared_ptr<Foo> self; // NOLINT };` -> No finding -- Covers: no suppression awareness (#11)

---

## magic_numbers

### Current Implementation
- **File:** `src/audit/pipelines/cpp/magic_numbers.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `magic_number`
- **Detection method:** Uses `compile_numeric_literal_query` (`(number_literal) @number`) to find all numeric literals. Filters out values in `EXCLUDED_VALUES` (0, 1, 2, 0.0, 1.0, -1, 10, 100, 1000, powers of 2, hex masks) and `COMMON_ALLOWED_NUMBERS` (3-8, 16-128, HTTP codes, ports, timeouts). Exempt contexts: `preproc_def`, `preproc_function_def`, `enumerator`, `template_argument_list`, `bitfield_clause`, `field_declaration`, `array_declarator`, `initializer_list`, `declaration` with `const`/`constexpr`, and `subscript_argument_list`. Max 200 findings per file. Reports "info" severity.

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric #12):** The `EXCLUDED_VALUES` list is oddly specific -- includes `256`, `512`, `1024`, `2048`, `4096`, `8192` (powers of 2) but not `16384`, `32768`, `65536`. Includes `10`, `100`, `1000` but not `10000` (which IS in `COMMON_ALLOWED_NUMBERS`). The combined allowlist of ~80 values makes the pipeline extremely permissive for some numbers and strict for others.
2. **High false positive rate (Rubric #2):** Numbers in `static const` or `inline constexpr` declarations at namespace scope are exempt, but numbers in `static constexpr` member declarations might not be (depends on whether `is_exempt_context` walks far enough). Numbers in `switch` case labels are flagged. Numbers in `assert()` calls are flagged.
3. **Missing context (Rubric #4):** Does not check the semantic context. A number `42` in a test file as an expected value is different from `42` in production code as a buffer size. Does not check for test files or test functions.
4. **No suppression/annotation awareness (Rubric #11):** No NOLINT support.
5. **Overlapping detection (Rubric #16):** `field_declaration` is in the exempt list, so `int x = 42;` as a field is exempt, but `int x = 42;` as a local variable is not. This seems inconsistent.
6. **Literal blindness (Rubric #8):** String comparison on the literal text: `"0xFF"` is excluded but `"0Xff"` or `"0XFF"` would not be (case sensitivity). Similarly, `"1.0"` is excluded but `"1.0f"` or `"1.0L"` would not be.
7. **No severity graduation (Rubric #15):** All magic numbers are "info". A magic number used as a buffer size (potential security issue) should be more severe than a magic number in a debug message.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** Magic number detected, const skipped, constexpr skipped, enum skipped, define skipped, common values skipped, array index skipped, template argument skipped.
- **What's NOT tested:** `static constexpr` member, `switch` case labels, `assert()` arguments, numbers with suffixes (`42U`, `3.14f`, `1LL`), negative numbers (`-42` -- parsed as unary minus + literal), hex variants (`0XFF` vs `0xFF`), numbers in return statements, MAX_FINDINGS_PER_FILE cap, test files, suppression comments.

### Replacement Pipeline Design
**Target trait:** NodePipeline

#### Step 1: File Identification
- **Tool:** Graph query -- filter C++ files.
- **Returns:** C++ file paths.

#### Step 2: Narrowing
- **Tool:** Tree-sitter -- `(number_literal) @number` (existing query).
- **Why not graph:** Numeric literals are expression-level, not in the graph.
- **Returns:** All numeric literals with their values and locations.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter AST inspection.
- **Query/Prompt:**
  1. Normalize the literal value: parse to numeric, handle suffixes (U, L, LL, f), handle hex case insensitivity.
  2. Expand exempt contexts to include: `switch_statement` case labels, `static_assert` arguments, macro invocation arguments, return statements in small functions.
  3. Add test file detection: if file path contains `test`, `spec`, `_test`, `Test`, skip or reduce severity.
  4. Check for NOLINT suppression on same/preceding line.
  5. Graduate severity: magic number in array size/buffer allocation -> "warning"; magic number in arithmetic/comparison -> "info".
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
None.

### New Test Cases
1. **number_with_suffix** -- `void f() { unsigned x = 42U; }` -> Finding for `42U` -- Covers: literal blindness (#8)
2. **hex_case_insensitive** -- `void f() { int x = 0XFF; }` -> No finding (same as 0xFF) -- Covers: literal blindness (#8)
3. **switch_case_exempt** -- `void f(int x) { switch(x) { case 42: break; } }` -> No finding or "info" -- Covers: missing context (#4)
4. **assert_argument** -- `void f() { assert(x == 42); }` -> No finding or reduced severity -- Covers: missing context (#4)
5. **static_constexpr_member** -- `class Foo { static constexpr int N = 42; };` -> No finding -- Covers: high false positive (#2)
6. **negative_number** -- `void f() { int x = -42; }` -> Finding for `42` (unary minus is separate) -- Covers: literal blindness (#8)
7. **test_file_reduced** -- `test_math.cpp` with `int expected = 42;` -> No finding or reduced severity -- Covers: missing context (#4)
8. **nolint_suppression** -- `void f() { int x = 42; // NOLINT }` -> No finding -- Covers: no suppression awareness (#11)
9. **buffer_size_warning** -- `void f() { char buf[42]; }` -> Finding with severity "warning" -- Covers: no severity graduation (#15)
10. **float_suffix** -- `void f() { float x = 3.14f; }` -> Finding for `3.14f` -- Covers: literal blindness (#8)
