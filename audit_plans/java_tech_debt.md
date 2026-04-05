# Java Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 11
- **Trait types used:** All 11 use `Pipeline` (legacy trait). None use `GraphPipeline` or `NodePipeline`.
- **Overall assessment:** The pipelines are competent single-file, tree-sitter-based detectors with reasonable false-positive mitigation (Lombok skip, accessor filtering, logging detection, test-file skip). However, they universally suffer from **no graph usage**, **no suppression/annotation awareness** (except god_class which checks Lombok), **no severity graduation**, and **heuristic-heavy detection** that produces both false positives and false negatives. Several pipelines rely on name-matching heuristics (string_concat_in_loops) or hardcoded type lists (resource_leaks, raw_types) without justification. None leverage the CodeGraph's symbol resolution, call graph, CFG, taint analysis, or resource lifecycle tracking that is already built and available.

---

## god_class

### Current Implementation
- **File:** `src/audit/pipelines/java/god_class.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `god_class` -- classes with >10 non-accessor methods
- **Detection method:** Tree-sitter query for `class_declaration`, then counts children of `class_body` that are `method_declaration` and are not accessor methods (get*/set*/is* with <=2 statements). Skips Lombok-annotated classes (@Data, @Getter, @Setter, @Builder) and classes whose name ends with "Builder".

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `METHOD_THRESHOLD = 10` (line 13) is arbitrary. No documentation on why 10 was chosen. Spring `@RestController` classes with many endpoints, or test helper classes, may legitimately have >10 methods. There is no graduated severity (e.g., 10 = info, 20 = warning, 30+ = error).
2. **[No severity graduation]:** All findings are "warning" regardless of whether a class has 11 or 50 methods. A class with 50 methods is far worse than one with 11.
3. **[High false positive rate]:** Does not skip `enum` declarations with many methods (enum constants can have method bodies). Does not skip `interface` declarations (Java 8+ default methods). Does not skip classes annotated with `@Configuration`, `@Component`, or framework annotations that often have many small bean methods.
4. **[No suppression/annotation awareness]:** Only checks Lombok annotations. Does not respect `@SuppressWarnings("god-class")` or any custom suppression annotation. Does not skip `@Generated` classes.
5. **[Missing compound variants]:** Does not consider field count as a contributing factor to "god class" detection. A class with 10 methods and 30 fields is worse than one with 11 methods and 2 fields.
6. **[Single-node detection]:** Only counts methods. Does not consider inner class count, nested complexity, or lines of code. The "god class" concept from the LCOM4 metric includes cohesion analysis (methods that share fields), which requires graph-level data.
7. **[High false negative rate]:** The accessor filter (line 113-133) checks method name prefixes (get/set/is) and body statement count <= 2. But a method named `getData()` with 50 lines of complex logic is still filtered as an accessor. The heuristic is too coarse.
8. **[Language idiom ignorance]:** Does not account for Java records (`record_declaration`) which should never trigger god_class. Does not account for sealed classes.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Detects a class with 12 methods; clean class with 3 methods; exactly-at-threshold (10) is clean.
- **What's NOT tested:** Accessor method filtering, Lombok annotation skipping, Builder name skipping, enum classes, interface declarations, nested classes, inner classes, classes with both static and instance methods, classes with high field count, `@Generated` annotation.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` filtered to `language == Java`. For each file, collect all Symbol nodes with `kind == Class` via graph edges from File node.
- **Returns:** List of `(file_path, class_symbol_node_index, class_name, start_line, end_line)` tuples.

#### Step 2: Narrowing
- **Tool:** Graph query + Tree-sitter
- **Why not graph only:** Graph Symbol nodes lack method-count detail; need tree-sitter to inspect class body children and count methods, fields, inner classes. However, graph `Contains` edges from class symbol to child symbols could provide method count if the graph captures them.
- **Query:** For each class symbol, use tree-sitter to: (a) count non-accessor `method_declaration` children, (b) count `field_declaration` children, (c) check for suppression annotations. Compute a composite score: `method_count * 1.0 + field_count * 0.5`.
- **Returns:** `(class_name, method_count, field_count, composite_score, has_suppression)` per class.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** For each candidate, check: (a) `@SuppressWarnings` or `@Generated` annotation, (b) `record_declaration` kind (skip), (c) framework annotations (@Configuration, @Component, @RestController), (d) enum/interface type (skip unless truly excessive). Also validate accessor detection: only filter methods where body contains exactly 1 return statement or 1 assignment.
- **Returns:** Filtered list with graduated severity: composite_score 15-25 = info, 25-40 = warning, 40+ = error.

#### Graph Enhancement Required
- **Contains edges for class-to-method relationships:** If graph built `Contains` edges from class Symbol nodes to child method Symbol nodes, method counting could be done entirely via graph traversal without tree-sitter re-parsing.

### New Test Cases
1. **test_accessor_with_complex_body** -- `public List<User> getUsers() { /* 20 lines of logic */ }` should NOT be filtered as accessor -- Covers: high false negative rate
2. **test_enum_with_methods** -- `enum Color { RED; void m1(){}...void m12(){} }` should NOT trigger -- Covers: language idiom ignorance
3. **test_generated_annotation** -- `@Generated class Foo { /* 15 methods */ }` should be skipped -- Covers: no suppression/annotation awareness
4. **test_severity_graduation** -- Class with 11 methods vs 50 methods should produce different severity levels -- Covers: no severity graduation
5. **test_record_declaration** -- `record Point(int x, int y) { /* methods */ }` should not trigger -- Covers: language idiom ignorance
6. **test_composite_score** -- Class with 8 methods + 20 fields should trigger (high composite) -- Covers: missing compound variants
7. **test_framework_controller** -- `@RestController class UserController { /* 12 endpoints */ }` should be info, not warning -- Covers: high false positive rate
8. **test_interface_default_methods** -- `interface Foo { default void m1(){} ... }` should not trigger -- Covers: high false positive rate

---

## null_returns

### Current Implementation
- **File:** `src/audit/pipelines/java/null_returns.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `null_return` -- methods that `return null`
- **Detection method:** Tree-sitter query for `return_statement` containing `null_literal`. Walks parent chain to find enclosing `method_declaration`. Skips constructors, test files (via `is_test_file`), and methods annotated with `@Test` or `@Deprecated`.

### Problems Identified
1. **[High false positive rate]:** Reports every `return null` regardless of context. Methods returning `null` as a legitimate sentinel in private helpers, factory methods returning `null` for "not found" (where `Optional` is overkill), or null returns in `catch` blocks (already caught by exception_swallowing) all trigger. No distinction between public API methods (where `Optional<T>` is important) and private internal methods.
2. **[No data flow tracking]:** Does not check if the return type is already `Optional<T>` (in which case `return null` is a bug, not tech debt -- different severity). Does not check if the method's return type is a primitive (where `return null` is impossible and the tree-sitter match is on autoboxed code).
3. **[No suppression/annotation awareness]:** Only skips `@Test` and `@Deprecated`. Does not respect `@Nullable`, `@SuppressWarnings`, or `@CheckForNull` annotations which explicitly document the null contract.
4. **[No severity graduation]:** All findings are "info" severity. A public API method returning null is much worse than a private helper. A method with multiple null return paths is worse than one with a single early return.
5. **[Overlapping detection]:** Overlaps with `exception_swallowing` pipeline's `catch_return_null` pattern. A `return null` inside a catch block will be reported by both pipelines.
6. **[Missing context]:** The message says "consider returning Optional<T>" but does not check the return type. If the return type is `void`, `int`, or `boolean`, Optional is not applicable.
7. **[High false negative rate]:** Does not detect `return (String) null` (cast null), `return condition ? null : value` (ternary null), or `Optional.ofNullable(null)` (misuse of Optional).

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Detects null return in method; skips constructor; clean non-null return.
- **What's NOT tested:** Test file skipping, @Test annotation skipping, @Deprecated skipping, @Nullable annotation, public vs private methods, multiple null returns in one method, ternary null, Optional return type, return type validation, overlap with exception_swallowing.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` filtered to Java. For each file, collect Symbol nodes with `kind == Method` and `exported == true` (public API methods are higher priority).
- **Returns:** `(file_path, method_name, exported, start_line, end_line)` tuples.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not capture return statements or return types; tree-sitter is needed for AST inspection.
- **Query:** For each method, use tree-sitter to: (a) find all `return_statement` nodes containing `null_literal`, (b) extract the method's return type node text, (c) count null return paths.
- **Returns:** `(method_name, return_type, null_return_count, is_exported, has_nullable_annotation)` per method.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Filter out: (a) methods annotated with `@Nullable`, `@CheckForNull`, `@SuppressWarnings`, (b) private methods with single null return as early exit, (c) methods whose return type is `Optional<T>` (flag as bug instead, different pattern), (d) methods where null return is inside a catch block (defer to exception_swallowing). Graduate severity: public method with null return = warning, private method = info, multiple null returns = bump severity.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- None strictly required, but adding return-type information to Symbol nodes would allow graph-only detection of public-API null returns.

### New Test Cases
1. **test_nullable_annotation_skipped** -- `@Nullable public String find() { return null; }` should be skipped -- Covers: no suppression/annotation awareness
2. **test_optional_return_type_is_bug** -- `Optional<String> find() { return null; }` should flag as error, not info -- Covers: no data flow tracking
3. **test_public_vs_private_severity** -- public method should be warning, private should be info -- Covers: no severity graduation
4. **test_ternary_null** -- `return x != null ? x : null;` should be detected -- Covers: high false negative rate
5. **test_overlap_with_catch** -- `catch(Exception e) { return null; }` should NOT also be reported here -- Covers: overlapping detection
6. **test_void_return_type** -- method with `void` return type should not suggest Optional -- Covers: missing context
7. **test_suppress_warnings** -- `@SuppressWarnings("null") String m() { return null; }` should be skipped -- Covers: no suppression/annotation awareness

---

## exception_swallowing

### Current Implementation
- **File:** `src/audit/pipelines/java/exception_swallowing.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `empty_catch`, `printstacktrace_only`, `catch_return_null`
- **Detection method:** Tree-sitter query for `catch_clause`. Checks body: 0 named children = empty_catch, 1 named child that is `printStackTrace()` = printstacktrace_only, 1 named child that is `return null` = catch_return_null. Skips if body contains logging calls (checks method name for log/warn/error/info/debug, or receiver text containing "log"/"logger").

### Problems Identified
1. **[High false negative rate]:** Only detects catch blocks with exactly 0 or 1 named children. A catch block with `e.printStackTrace(); return null;` (2 statements) is missed entirely. A catch block with `System.out.println(e);` is missed (not recognized as a print statement, only `printStackTrace` is checked).
2. **[Missing compound variants]:** Does not detect `catch (Exception e) { /* comment only */ }` -- a comment node may make `named_child_count > 0` but the block is still effectively empty. Does not detect `catch (Exception e) { e.getMessage(); }` which just reads the message without doing anything.
3. **[No scope awareness]:** Does not differentiate between catch blocks at different scopes. A catch block in a `finally` clause, or in a lambda, is treated the same as a top-level try-catch.
4. **[No suppression/annotation awareness]:** Does not check for `@SuppressWarnings("empty-catch")` or intentional empty catches marked with a comment like `// intentionally swallowed`.
5. **[Literal blindness]:** The logging detection (line 108-141) checks method name and receiver text, but does not handle `System.err.println()`, `System.out.println()`, or SLF4J's `LoggerFactory.getLogger()`. The receiver check is case-insensitive substring (`contains("log")`) which could match `dialog.error()` (false negative avoidance) but also `catalog.warn()` (false positive in skipping).
6. **[No severity graduation]:** All three patterns are "warning". An empty catch block is more dangerous than a `printStackTrace`-only block. A `catch(Exception e)` that catches broad exceptions is worse than `catch(FileNotFoundException e)`.
7. **[Missing edge cases in tests]:** Does not test multi-catch (`catch (IOException | SQLException e)`), nested try-catch, catch blocks with only a comment, or the logging detection heuristic with different logging frameworks.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Empty catch, printStackTrace-only, return-null catch, logging catch (clean), rethrow catch (clean).
- **What's NOT tested:** Two-statement catch blocks, System.out.println in catch, catch with only a comment, multi-catch syntax, nested try-catch, broad exception type (Exception/Throwable) vs specific, `@SuppressWarnings`, intentional empty catch comment.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java files. All Java files are candidates.
- **Returns:** List of `(file_path)`.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not model try-catch structures; catch clauses are not graph nodes.
- **Query:** For each file, find all `catch_clause` nodes. For each, inspect: (a) body named_child_count, (b) whether body contains only print/logging/no-op statements, (c) exception type caught (broad vs specific), (d) whether the catch rethrows.
- **Returns:** `(file_path, line, catch_type, body_analysis: {empty, print_only, return_null, comment_only, swallowed}, exception_broadness)` per catch clause.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Check: (a) comments in empty catch blocks that indicate intentional swallowing (e.g., "// expected", "// ignore", "// intentionally"), (b) `@SuppressWarnings` on enclosing method, (c) test file context. Graduate severity: empty catch with broad exception (Exception/Throwable) = error, empty catch with specific exception = warning, printStackTrace-only = info.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- None. Catch clause analysis is inherently AST-local.

### New Test Cases
1. **test_two_statement_catch** -- `catch(Exception e) { e.printStackTrace(); return null; }` should still be detected -- Covers: high false negative rate
2. **test_system_out_println** -- `catch(Exception e) { System.out.println(e); }` should be detected as weak handling -- Covers: literal blindness
3. **test_comment_only_catch** -- `catch(Exception e) { /* intentionally empty */ }` should be skipped if comment indicates intent -- Covers: no suppression/annotation awareness
4. **test_broad_vs_specific_exception** -- `catch(Throwable t) {}` should be error, `catch(FileNotFoundException e) {}` should be warning -- Covers: no severity graduation
5. **test_multi_catch** -- `catch(IOException | SQLException e) {}` should be detected -- Covers: missing edge cases in tests
6. **test_nested_try_catch** -- Inner catch block should be flagged independently -- Covers: no scope awareness
7. **test_catch_get_message_noop** -- `catch(Exception e) { e.getMessage(); }` should be detected -- Covers: missing compound variants

---

## mutable_public_fields

### Current Implementation
- **File:** `src/audit/pipelines/java/mutable_public_fields.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `mutable_public_field` -- public non-final fields
- **Detection method:** Tree-sitter query for `field_declaration`, then checks for `public` modifier and absence of `final` modifier using `has_modifier()`.

### Problems Identified
1. **[High false positive rate]:** Flags all public non-final fields, but many are legitimate: (a) fields in DTOs/POJOs that are intentionally mutable for serialization frameworks (Jackson, JAXB), (b) fields in `@Entity` JPA classes, (c) fields annotated with `@Inject`, `@Autowired`, or `@Value` (Spring), (d) public `static` fields (which have different semantics than instance fields).
2. **[No suppression/annotation awareness]:** Does not check for `@SuppressWarnings`, `@Data` (Lombok generates getters/setters), `@Entity`, `@Component`, or serialization annotations.
3. **[No severity graduation]:** All findings are "warning". A public mutable `List<String>` field is much more dangerous than a public mutable `int` field (mutable collection vs primitive).
4. **[Missing compound variants]:** Does not detect `protected` non-final fields (also encapsulation leak in inheritance hierarchies). Does not detect package-private non-final fields in public classes.
5. **[Overlapping detection]:** Partially overlaps with `missing_final` pipeline. A `private` non-final field triggers `missing_final`, and a `public` non-final field triggers `mutable_public_fields`. But `protected` non-final fields trigger neither.
6. **[Language idiom ignorance]:** Does not skip fields in `record_declaration` (records are inherently immutable in Java, their components are final). Does not skip `volatile` fields (intentionally mutable for concurrency). Does not skip `transient` fields.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Detects public non-final field; clean public final field; clean private field.
- **What's NOT tested:** Protected fields, static fields, volatile fields, Lombok @Data annotation, JPA @Entity annotation, record fields, public mutable collection types, serialization annotations.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java. Collect Symbol nodes with `kind == Variable` and `exported == true` (public fields).
- **Returns:** `(file_path, field_name, start_line)` tuples.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph Symbol nodes do not capture modifiers (final/volatile/static); tree-sitter needed for modifier inspection.
- **Query:** For each public field, inspect modifiers for: (a) `final` (skip), (b) `static` (flag differently), (c) `volatile` (skip), (d) `transient` (skip). Check enclosing class for framework annotations.
- **Returns:** `(field_name, field_type, is_static, is_collection_type, enclosing_class_annotations)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Skip if: (a) enclosing class has @Entity, @Data, @Component, @Configuration, (b) field has @Inject/@Autowired/@Value, (c) field is `volatile` or `transient`, (d) enclosing type is `record_declaration`. Graduate: public mutable collection = error, public mutable object = warning, public mutable primitive = info.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- Adding modifier information (final, static, volatile) to Symbol node metadata would allow graph-only filtering.

### New Test Cases
1. **test_entity_class_skipped** -- `@Entity class User { public String name; }` should be skipped -- Covers: no suppression/annotation awareness
2. **test_lombok_data_skipped** -- `@Data class Dto { public int x; }` should be skipped -- Covers: no suppression/annotation awareness
3. **test_static_field** -- `public static int count;` should be flagged with different message -- Covers: missing compound variants
4. **test_volatile_field** -- `public volatile boolean running;` should be skipped -- Covers: language idiom ignorance
5. **test_collection_field_severity** -- `public List<String> items;` should be error, `public int x;` should be warning -- Covers: no severity graduation
6. **test_protected_field** -- `protected String name;` should also be detected -- Covers: overlapping detection / missing compound variants
7. **test_record_fields** -- `record Point(int x, int y) {}` should not trigger -- Covers: language idiom ignorance

---

## string_concat_in_loops

### Current Implementation
- **File:** `src/audit/pipelines/java/string_concat_in_loops.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `string_concat_in_loop` -- `+=` assignments inside loops that look like string concatenation
- **Detection method:** Tree-sitter query for `assignment_expression`. Checks: (a) operator contains `+=`, (b) node is inside a loop (for/enhanced_for/while/do), (c) LHS variable name is not clearly numeric (hardcoded NUMERIC_NAMES list), (d) LHS contains a string-like pattern (STRING_LIKE_PATTERNS list) OR RHS contains a string literal or is a binary expression.

### Problems Identified
1. **[Literal blindness]:** The detection is entirely heuristic-based on variable naming (line 13-21). Variables named `data += something` would be flagged even if `data` is a numeric accumulator. Variables named `x += "hello"` would be missed because `x` is not in STRING_LIKE_PATTERNS and would rely on RHS heuristic.
2. **[No data flow tracking]:** Does not resolve the type of the LHS variable. A `String` type declaration could be checked via tree-sitter by walking backward to the `local_variable_declaration`. This would eliminate the entire name-heuristic approach.
3. **[High false positive rate]:** `rhs_node.kind() == "binary_expression"` (line 96) matches ANY binary expression on the RHS, including `count += a + b` where both sides are numeric. The check should verify the binary expression involves string literals or string-typed variables.
4. **[High false negative rate]:** Does not detect `str = str + "item"` (plain concatenation, not `+=`). Does not detect `String.concat()` in loops. Does not detect `"" + var` concatenation patterns.
5. **[Missing compound variants]:** Does not flag `StringBuilder` misuse inside loops (creating a new `StringBuilder` inside each iteration). Does not detect stream-based string building in loops.
6. **[Hardcoded thresholds without justification]:** The NUMERIC_NAMES and STRING_LIKE_PATTERNS lists (lines 13-21) are arbitrary. No justification for why these specific names were chosen. Missing common string-like names: "url", "uri", "header", "body", "content", "template", "format".
7. **[No suppression/annotation awareness]:** No way to suppress findings via annotation or comment.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** String concat in for loop; clean outside loop; detects in while loop.
- **What's NOT tested:** Enhanced for loop, do-while loop, numeric += (false positive), plain concatenation (`str = str + "x"`), StringBuilder misuse, variable type resolution, non-string-named variables with string literals, nested loops.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java files. All are candidates.
- **Returns:** `(file_path)` list.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not model assignment expressions or loop structures. CFG could theoretically identify loop back-edges, but assignment type resolution needs AST.
- **Query:** For each file: (a) find all `assignment_expression` nodes with `+=` operator inside loop constructs, (b) for each, walk backward from LHS identifier to its `local_variable_declaration` to extract the declared type, (c) if type is `String` or unresolvable but RHS contains string literal, flag it. Also find `assignment_expression` with `=` operator where RHS is a `binary_expression` containing the same LHS identifier and a string literal (plain concatenation pattern).
- **Returns:** `(file_path, line, lhs_name, lhs_type, pattern: {plus_equals, plain_concat, string_concat_method})`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Only flag if LHS type resolves to `String` or if RHS definitely contains string literals. Skip if `@SuppressWarnings` present. Skip if the variable is used with `StringBuilder` later in the same method.
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- None required, but per-function CFGs could be used to identify loop back-edges more accurately than parent-chain walking.

### New Test Cases
1. **test_type_resolution** -- `int count = 0; for(...) { count += 1; }` should NOT trigger -- Covers: no data flow tracking
2. **test_plain_concatenation** -- `for(...) { str = str + "x"; }` should trigger -- Covers: high false negative rate
3. **test_enhanced_for** -- `for (String s : list) { result += s; }` should trigger -- Covers: missing edge cases in tests
4. **test_numeric_binary_expression** -- `for(...) { total += a + b; }` (numeric) should NOT trigger -- Covers: high false positive rate
5. **test_stringbuilder_misuse** -- `for(...) { StringBuilder sb = new StringBuilder(); sb.append(x); result += sb.toString(); }` should trigger -- Covers: missing compound variants
6. **test_string_concat_method** -- `for(...) { str = str.concat("x"); }` should trigger -- Covers: high false negative rate
7. **test_suppress_warnings** -- `@SuppressWarnings("string-concat") void m() { ... }` should be skipped -- Covers: no suppression/annotation awareness

---

## instanceof_chains

### Current Implementation
- **File:** `src/audit/pipelines/java/instanceof_chains.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `instanceof_chain` -- if/else-if chains with >=3 instanceof checks
- **Detection method:** Tree-sitter query for `if_statement` with `condition`. Checks if condition contains `instanceof_expression`. Follows `alternative` field to count the chain length. Deduplicates via `reported` HashSet of node IDs.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `CHAIN_THRESHOLD = 3` (line 13) is arbitrary. Two instanceof checks could already indicate a missing polymorphic design. No justification for 3.
2. **[No severity graduation]:** All findings are "warning". A chain of 3 is much less severe than a chain of 10+.
3. **[High false negative rate]:** The chain-following logic (line 133-143) only follows the `alternative` field when it directly is an `if_statement`. It does NOT handle `else { if (...) {...} }` patterns (where the alternative is a block containing an if_statement). The comment on line 137-139 explicitly acknowledges this gap. This is a significant false negative.
4. **[Language idiom ignorance]:** Java 16+ has pattern matching for instanceof (`if (obj instanceof String s)`). This is a modern idiom and the pipeline should either skip it or treat it differently than the old-style instanceof.
5. **[No suppression/annotation awareness]:** No way to suppress findings. In some code (e.g., AST visitors), instanceof chains are idiomatic and intentional.
6. **[Missing context]:** Does not report which types are being checked in the chain, making the finding less actionable. Does not suggest using a visitor pattern or sealed class hierarchy.
7. **[Single-node detection]:** Only counts instanceof occurrences. Does not check if the checked types share a common supertype (which would make polymorphism feasible). Graph-level type hierarchy analysis would improve this.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** 3-chain detection, 5-chain detection, 2-chain is clean, separate (non-chained) ifs are clean.
- **What's NOT tested:** `else { if (...) }` pattern (the known false negative), pattern matching instanceof (Java 16+), mixed instanceof and non-instanceof conditions, deduplication across multiple chains in same method, nested instanceof chains.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java files.
- **Returns:** `(file_path)` list.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not model if/else chains or instanceof expressions. These are AST-local constructs.
- **Query:** For each file, find all `if_statement` nodes. For each, recursively follow both `alternative` (direct if_statement) AND `alternative` (block containing single if_statement) to properly count chains. Extract the list of types being checked.
- **Returns:** `(file_path, line, chain_length, types_checked: Vec<String>, uses_pattern_matching: bool)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + AI prompt
- **Why AI prompt:** Determining if a chain is idiomatic (e.g., AST visitor pattern) requires semantic understanding beyond tree-sitter.
- **Context gathering query:** Tree-sitter extracts: enclosing class name, method name, chain snippet (first 10 lines of chain).
- **Context shape:** `{ class_name, method_name, chain_snippet, chain_length, types_checked }`
- **Prompt:** "Given this instanceof chain in class `{class_name}`, method `{method_name}`, checking types `{types_checked}`: Is this an idiomatic visitor/dispatcher pattern, or tech debt that should be refactored to polymorphism? Return: {is_idiomatic: bool, reason: string, suggested_refactoring: string}"
- **Expected return:** `{ is_idiomatic: bool, reason: string, suggested_refactoring: string }`
- Graduate severity: 3-4 = info, 5-7 = warning, 8+ = error.
- **Returns:** Filtered findings with severity and actionable refactoring suggestion.

#### Graph Enhancement Required
- Type hierarchy edges (Extends, Implements) would allow checking if the instanceof-checked types share a common supertype, making the polymorphism suggestion more targeted.

### New Test Cases
1. **test_else_block_if_pattern** -- `if (x instanceof A) {} else { if (x instanceof B) {} else { if (x instanceof C) {} } }` should detect chain of 3 -- Covers: high false negative rate
2. **test_pattern_matching_instanceof** -- `if (obj instanceof String s) {}` should be handled differently -- Covers: language idiom ignorance
3. **test_severity_graduation** -- Chain of 3 vs chain of 10 should have different severity -- Covers: no severity graduation
4. **test_mixed_conditions** -- `if (x instanceof A && x.isActive())` should still count -- Covers: missing edge cases in tests
5. **test_types_reported** -- Finding message should list the types being checked -- Covers: missing context
6. **test_visitor_pattern** -- `class NodeVisitor { void visit(Node n) { if (n instanceof Add) ... } }` should be noted as potentially idiomatic -- Covers: high false positive rate

---

## resource_leaks

### Current Implementation
- **File:** `src/audit/pipelines/java/resource_leaks.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `resource_leak` -- resource types created outside try-with-resources
- **Detection method:** Tree-sitter query for `local_variable_declaration` with `object_creation_expression`. Checks if the declared type is in RESOURCE_TYPES list (16 hardcoded types). Skips if inside `try_with_resources_statement`. Skips wrapped creations (e.g., `new BufferedReader(new FileReader("f"))`).

### Problems Identified
1. **[Hardcoded thresholds without justification]:** RESOURCE_TYPES (line 16-32) is a hardcoded list of 16 types. Missing many common resource types: `Channel`, `DataSource`, `HttpClient`, `CloseableHttpResponse`, `ZipInputStream`, `ZipOutputStream`, `ObjectInputStream`, `ObjectOutputStream`, `RandomAccessFile`, `DatagramSocket`, `SSLSocket`, `JarFile`, `SQLiteDatabase`, `Cursor` (Android), `EntityManager` (JPA), `Session` (Hibernate). Any class implementing `AutoCloseable`/`Closeable` should be flagged, not just hardcoded types.
2. **[No data flow tracking]:** Does not check if the resource is closed later in the same method (via `.close()` call in a finally block). Only checks for try-with-resources. Traditional try-finally-close patterns are legitimate and should not be flagged.
3. **[High false positive rate]:** The `is_in_try_with_resources` check (line 165-186) returns true for ANY node inside a `try_with_resources_statement`, including the try body. So `try (FileInputStream a = ...) { FileInputStream b = new FileInputStream("x"); }` would incorrectly skip the inner `b` creation because it is inside the try-with-resources statement body, not the resource specification.
4. **[High false negative rate]:** Only checks `local_variable_declaration`. Does not detect: (a) resources assigned to fields (`this.conn = new Connection()`), (b) resources created inline without assignment (`new FileInputStream("f").read()`), (c) resources returned from factory methods (`DriverManager.getConnection(url)`).
5. **[Single-node detection]:** A graph-level approach using `Acquires`/`ReleasedBy` edges from the resource lifecycle analyzer would be much more accurate. The `CodeGraph` already has `resource::ResourceAnalyzer` that computes these edges.
6. **[No suppression/annotation awareness]:** Does not check for `@SuppressWarnings("resource")` or `@WillClose` / `@WillNotClose` annotations.
7. **[Missing compound variants]:** Does not detect resource leaks through exception paths (resource created, exception thrown before close, no finally block).

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Detects resource leak (Connection), clean try-with-resources, clean non-resource type, detects FileInputStream leak.
- **What's NOT tested:** Resources with finally-close (false positive), wrapped creation pattern, field assignments, factory method resources, the try-body false positive bug, custom AutoCloseable types, multiple resources in one method, `@SuppressWarnings("resource")`.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool. The CodeGraph already has `Acquires`/`ReleasedBy` edges built by `ResourceAnalyzer`.
- **Query:** Iterate graph edges of type `Acquires`. For each `Acquires` edge, check if there is a corresponding `ReleasedBy` edge from the same node. Nodes with `Acquires` but no `ReleasedBy` are potential leaks.
- **Returns:** `(file_path, line, resource_type, symbol_name)` for unreleased resources.

#### Step 2: Narrowing
- **Tool:** Tree-sitter (only if graph `Acquires`/`ReleasedBy` edges are insufficient)
- **Why not graph only:** Graph resource analysis may not capture all patterns; tree-sitter as fallback for local_variable_declaration patterns not in graph.
- **Query:** For resources not caught by graph, use tree-sitter to find `local_variable_declaration` with types implementing AutoCloseable (check via import-resolved type hierarchy if available, else fall back to expanded known-types list).
- **Returns:** Combined list of potential leaks.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** For each candidate: (a) check if there is a `.close()` call on the variable in a finally block in the same method, (b) check `@SuppressWarnings("resource")`, (c) check if variable is returned (resource transferred to caller, not a leak). Graduate severity: Connection/Statement = error (database resource), stream = warning, other = info.
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- Verify that `ResourceAnalyzer` produces `Acquires`/`ReleasedBy` edges for Java code. If not, this is the primary enhancement needed. The analyzer should recognize `new <AutoCloseable subtype>(...)` as acquire and `.close()` / try-with-resources as release.

### New Test Cases
1. **test_finally_close_is_clean** -- `Connection c = new Connection(); try { ... } finally { c.close(); }` should NOT trigger -- Covers: no data flow tracking
2. **test_try_body_resource_not_skipped** -- `try (A a = ...) { B b = new B(); }` inner `b` should be flagged -- Covers: high false positive rate (bug)
3. **test_factory_method_resource** -- `Connection c = DriverManager.getConnection(url);` should be detected -- Covers: high false negative rate
4. **test_custom_autocloseable** -- `class MyResource implements AutoCloseable { ... }; MyResource r = new MyResource();` should be detected -- Covers: hardcoded thresholds
5. **test_resource_returned** -- `FileInputStream create() { return new FileInputStream("f"); }` should NOT trigger (transferred) -- Covers: high false positive rate
6. **test_suppress_warnings** -- `@SuppressWarnings("resource") void m() { Connection c = new Connection(); }` should be skipped -- Covers: no suppression/annotation awareness
7. **test_field_assignment_resource** -- `this.conn = new Connection();` should be detected -- Covers: high false negative rate
8. **test_severity_graduation** -- Connection leak = error, InputStream leak = warning -- Covers: no severity graduation

---

## static_utility_sprawl

### Current Implementation
- **File:** `src/audit/pipelines/java/static_utility_sprawl.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `static_utility_class` -- classes where ALL methods are static and count > 3
- **Detection method:** Tree-sitter query for `class_declaration`. Counts methods, counts static methods. Flags if all methods are static and total > STATIC_METHOD_THRESHOLD (3).

### Problems Identified
1. **[Hardcoded thresholds without justification]:** `STATIC_METHOD_THRESHOLD = 3` (line 13) is low. Classes like `Math`, `Collections`, `Arrays`, and `Objects` are all static utility classes with many methods and are not tech debt. The threshold should be higher (e.g., 8-10) and consider the cohesion of the methods.
2. **[No severity graduation]:** All findings are "info". A class with 4 static methods is very different from one with 40.
3. **[High false positive rate]:** Does not skip: (a) classes annotated with `@UtilityClass` (Lombok), which explicitly declare intent, (b) classes with a private constructor (standard Java pattern for utility classes), (c) classes named `*Utils`, `*Helper`, `*Constants` (naming convention indicates intent), (d) `abstract` classes (cannot be instantiated anyway), (e) classes that are explicitly designed as functional entry points.
4. **[Missing compound variants]:** Only checks methods. Does not consider static fields (constants). A class with 4 static methods and 20 static constants is a different pattern (constants holder vs utility class).
5. **[No suppression/annotation awareness]:** Does not check for `@SuppressWarnings` or `@UtilityClass`.
6. **[Language idiom ignorance]:** Static utility classes with a private constructor are the standard Java idiom for utility classes. These should be recognized as intentional, not flagged.
7. **[Single-node detection]:** Does not check if the static methods are cohesive (related functionality) or if they should be split. Graph-level analysis of method callers could reveal if the utility class serves too many different concerns.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Detects class with 5 static methods; clean class with mixed methods; clean below threshold.
- **What's NOT tested:** Private constructor pattern, @UtilityClass annotation, abstract classes, classes named *Utils/*Helper, static fields alongside methods, enums with static methods, interfaces with static methods.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java. Collect class Symbol nodes. For each class, check if all child methods (via `Contains` edges or tree-sitter) are static.
- **Returns:** `(file_path, class_name, static_method_count)` for all-static classes.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph only:** Modifier inspection (static, abstract) and constructor analysis require AST access.
- **Query:** For each all-static class: (a) check for private constructor (skip -- intentional utility class), (b) check for @UtilityClass annotation (skip), (c) check class name pattern (*Utils, *Helper, *Constants), (d) count static fields.
- **Returns:** `(class_name, static_method_count, static_field_count, has_private_ctor, has_utility_annotation, name_pattern)`.

#### Step 3: False Positive Removal
- **Tool:** Graph query
- **Why not tree-sitter:** Graph can show how many different callers use the utility class, indicating if it truly serves multiple concerns.
- **Query:** For each candidate, use `graph.find_symbols_by_name(class_method_name)` and `traverse_callers()` to count distinct calling classes. If callers are diverse (>5 different classes), suggest splitting. If callers are focused (1-3 classes), suggest inlining.
- Graduate severity: >15 methods and diverse callers = warning, >8 methods = info, else skip.
- **Returns:** Filtered findings with severity and caller analysis.

#### Graph Enhancement Required
- None strictly required. Caller traversal is already available via `traverse_callers()`.

### New Test Cases
1. **test_private_constructor_skipped** -- `class Utils { private Utils() {} static void a(){} static void b(){} static void c(){} static void d(){} }` should be skipped -- Covers: language idiom ignorance
2. **test_utility_class_annotation** -- `@UtilityClass class Helpers { ... }` should be skipped -- Covers: no suppression/annotation awareness
3. **test_abstract_class_skipped** -- `abstract class Base { static void a(){} ... }` should be skipped -- Covers: high false positive rate
4. **test_severity_graduation** -- 4 static methods = info, 20 static methods = warning -- Covers: no severity graduation
5. **test_constants_holder** -- Class with 4 static methods + 15 static final fields should be flagged as constants holder pattern -- Covers: missing compound variants
6. **test_utils_naming_convention** -- `class StringUtils { ... }` should note the naming convention in the message -- Covers: missing context

---

## magic_strings

### Current Implementation
- **File:** `src/audit/pipelines/java/magic_strings.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `magic_string` -- `.equals()` and `.equalsIgnoreCase()` calls with string literals
- **Detection method:** Tree-sitter query for `method_invocation`. Filters to method names `equals` or `equalsIgnoreCase`. Checks if arguments contain a non-empty `string_literal`.

### Problems Identified
1. **[High false positive rate]:** Flags ALL `.equals("literal")` calls, but many are legitimate: (a) single-use comparisons (`if (action.equals("submit"))` in a controller), (b) comparisons against well-known values ("true", "false", "yes", "no"), (c) comparisons in test code (test file not filtered), (d) comparisons against configuration keys that are only used once.
2. **[High false negative rate]:** Only detects `.equals()` and `.equalsIgnoreCase()`. Does not detect: (a) `switch(str) { case "ADMIN": ... }` (magic strings in switch cases), (b) `str.contains("magic")`, `str.startsWith("prefix")`, `str.endsWith("suffix")`, (c) `"literal".equals(obj)` (reversed equals pattern -- literal is the receiver, not the argument), (d) string comparisons via `==` operator (though less common, still occurs), (e) `String.valueOf()` comparisons.
3. **[No suppression/annotation awareness]:** No way to suppress findings.
4. **[No severity graduation]:** All findings are "info". A magic string used in 5 different places is much worse than one used once. Graph-level analysis of how many times the same literal appears across the codebase would enable severity graduation.
5. **[Literal blindness]:** The empty string check (`text.len() > 2`, line 76) is correct but does not skip single-character strings which are often legitimate delimiters (",", ":", "/", " ").
6. **[Missing context]:** Does not report how many times the same literal appears in the codebase. A string used once is low priority; one used in 10 places is high priority.
7. **[Missing compound variants]:** Does not detect magic numbers (`if (status == 200)`, `if (timeout == 3000)`) which are the numeric equivalent of magic strings.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Detects equals with magic string, detects equalsIgnoreCase, clean equals with variable, clean equals with empty string.
- **What's NOT tested:** Reversed equals pattern (`"ADMIN".equals(role)`), switch case strings, contains/startsWith/endsWith, test file context, single-character strings, @SuppressWarnings, repeated use of same literal.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java files. Skip test files.
- **Returns:** `(file_path)` list.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not model string literals or method invocations at this granularity.
- **Query:** For each file: (a) find all `method_invocation` nodes where method name is `equals`, `equalsIgnoreCase`, `contains`, `startsWith`, `endsWith` AND arguments contain a `string_literal` longer than 1 character, (b) find all `switch_expression`/`switch_statement` case labels with string literals, (c) find reversed equals pattern where receiver is a string literal. Collect all literal values with their locations.
- **Returns:** `(file_path, line, literal_value, context: {equals, switch_case, contains, starts_with})` per finding.

#### Step 3: False Positive Removal
- **Tool:** Graph query (cross-file literal frequency analysis)
- **Why not tree-sitter:** Cross-file analysis requires graph or aggregation across files.
- **Query:** Aggregate all collected literals across all files. Group by literal value. Literals appearing >=3 times across the codebase get elevated severity. Single-use literals in private methods = skip. Skip well-known values ("true", "false", "null", "yes", "no", common HTTP methods). Skip if method has `@SuppressWarnings`. Graduate: literal used 5+ times = warning, 3-4 times = info, 1-2 times = skip (unless in public API).
- **Returns:** Filtered, deduplicated findings with frequency information.

#### Graph Enhancement Required
- String literal frequency index: a map of `literal_value -> Vec<(file_path, line)>` built during graph construction would enable cross-file magic string detection.

### New Test Cases
1. **test_reversed_equals** -- `"ADMIN".equals(role)` should be detected -- Covers: high false negative rate
2. **test_switch_case_strings** -- `switch(s) { case "A": case "B": case "C": }` should be detected -- Covers: high false negative rate
3. **test_contains_magic_string** -- `str.contains("SECRET_PREFIX")` should be detected -- Covers: high false negative rate
4. **test_single_char_skipped** -- `s.equals(",")` should be skipped -- Covers: literal blindness
5. **test_well_known_values** -- `s.equals("true")` should be skipped -- Covers: high false positive rate
6. **test_test_file_skipped** -- Findings in test files should be skipped -- Covers: high false positive rate
7. **test_repeated_literal_severity** -- Same string in 5 places should be warning, not info -- Covers: no severity graduation

---

## raw_types

### Current Implementation
- **File:** `src/audit/pipelines/java/raw_types.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `raw_generic_type` -- use of known generic types without type parameters
- **Detection method:** Three separate tree-sitter queries for field declarations, local variable declarations, and formal parameters. Each checks if the type is a `type_identifier` (not `generic_type`) and is in the KNOWN_GENERICS list (21 types).

### Problems Identified
1. **[Hardcoded thresholds without justification]:** KNOWN_GENERICS (line 16-38) lists 21 types. Missing many common generics: `Supplier`, `Function`, `Consumer`, `Predicate`, `BiFunction`, `Callable`, `Reference`, `WeakReference`, `SoftReference`, `AtomicReference`, `ConcurrentHashMap`, `ConcurrentLinkedQueue`, `BlockingQueue`, `ThreadLocal`, `Pair`, `Entry`. Any type that takes type parameters could be raw.
2. **[High false negative rate]:** Only checks `type_identifier` node kind. A raw type used in a method return type is not detected (`List getData()` method signature). Raw types in generic bounds (`<T extends Comparable>`) are not detected. Raw types in cast expressions (`(List) obj`) are not detected. Raw types in `instanceof` expressions are not detected.
3. **[No suppression/annotation awareness]:** Does not check for `@SuppressWarnings("rawtypes")` which is the standard Java annotation for intentionally using raw types.
4. **[No severity graduation]:** All findings are "warning". Using `List` raw in a public API is much worse than in a private helper method or a legacy compatibility layer.
5. **[Missing context]:** Does not suggest what type parameter to use. If the variable is initialized with a parameterized constructor (e.g., `List items = new ArrayList<String>()`), the type parameter from the RHS could be suggested.
6. **[Overlapping detection]:** Field, local, and param queries could produce duplicate findings if a variable declaration matches multiple patterns (unlikely but possible with complex AST).
7. **[Language idiom ignorance]:** Does not recognize that raw types are sometimes required for backward compatibility with pre-generics code (Java 1.4), reflection APIs, or certain framework patterns. These cases should be info, not warning.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Raw field, raw local, raw param, clean parameterized type, clean non-generic type, clean primitive.
- **What's NOT tested:** Raw return type, raw in cast expression, raw in instanceof, `@SuppressWarnings("rawtypes")`, raw with initialization hint, raw in method signature generics, custom generic types not in the list.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java files.
- **Returns:** `(file_path)` list.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph:** Graph does not capture type information at the variable level. Tree-sitter is needed to inspect type nodes.
- **Query:** For each file, find all `type_identifier` nodes that are children of: `field_declaration.type`, `local_variable_declaration.type`, `formal_parameter.type`, `method_declaration.type` (return type), `cast_expression.type`. Check if the identifier matches any import that resolves to a generic class. Alternatively, expand the known-generics list significantly or check if the same identifier appears elsewhere in the file with type arguments.
- **Returns:** `(file_path, line, raw_type_name, context: {field, local, param, return, cast}, is_public_api)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Check: (a) `@SuppressWarnings("rawtypes")` on enclosing declaration, (b) if RHS initialization has type parameters (suggest those), (c) if the raw type is in a pre-generics compatibility context. Graduate severity: public API raw type = error, private = warning, suppressed = skip.
- **Returns:** Filtered findings with type parameter suggestions where possible.

#### Graph Enhancement Required
- Import resolution to type definitions would allow detecting custom generic types (any class with `type_parameters` in its declaration) instead of relying on a hardcoded list.

### New Test Cases
1. **test_raw_return_type** -- `class Foo { List getData() { return null; } }` should be detected -- Covers: high false negative rate
2. **test_raw_cast** -- `List items = (List) obj;` should be detected -- Covers: high false negative rate
3. **test_suppress_rawtypes** -- `@SuppressWarnings("rawtypes") List items;` should be skipped -- Covers: no suppression/annotation awareness
4. **test_type_parameter_suggestion** -- `List items = new ArrayList<String>()` should suggest `List<String>` -- Covers: missing context
5. **test_public_vs_private_severity** -- `public List getItems()` = error, `private List items` = warning -- Covers: no severity graduation
6. **test_custom_generic** -- `class Box<T> {} Box items;` should be detected -- Covers: hardcoded thresholds
7. **test_raw_in_generic_bound** -- `<T extends Comparable>` should be detected -- Covers: high false negative rate

---

## missing_final

### Current Implementation
- **File:** `src/audit/pipelines/java/missing_final.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `missing_final_field` -- private non-final fields
- **Detection method:** Tree-sitter query for `field_declaration`. Checks for `private` modifier and absence of `final` modifier.

### Problems Identified
1. **[High false positive rate]:** Flags ALL private non-final fields. But many private fields are intentionally mutable: (a) fields set via setter methods (JavaBean pattern), (b) fields set via dependency injection (`@Autowired`, `@Inject`, `@Value`), (c) mutable state in stateful objects (counters, caches, accumulators), (d) fields initialized in a non-constructor `init()` method or `@PostConstruct`, (e) fields in `@Entity` JPA classes (managed by ORM).
2. **[No data flow tracking]:** Does not check if the field is actually mutated after construction. If a field is assigned only in the constructor, it should be final. If it is reassigned in other methods, the finding is a false positive. This requires intra-class data flow analysis.
3. **[No suppression/annotation awareness]:** Does not check for `@SuppressWarnings`, `@Inject`, `@Autowired`, `@Value`, `@Setter` (Lombok), or `@Entity`.
4. **[No severity graduation]:** All findings are "info". A private non-final field that is never reassigned (should definitely be final) is more important than one that is reassigned in a setter.
5. **[Overlapping detection]:** Does not flag `protected` or package-private non-final fields (gap with `mutable_public_fields`). Together, the two pipelines leave `protected` fields uncovered.
6. **[Language idiom ignorance]:** Does not recognize that `volatile` fields cannot be final (they need to be mutable for concurrency). Does not recognize `transient` fields used in serialization. Does not skip fields in inner classes that are modified by lambdas.
7. **[Missing context]:** The message says "consider making it final" but does not check if that is actually possible (i.e., is the field ever reassigned?). This requires at minimum a tree-sitter walk of the enclosing class to find assignments to the field.

### Test Coverage
- **Existing tests:** 3 tests
- **What's tested:** Detects private non-final field; clean private final field; clean public field (not this pipeline's concern).
- **What's NOT tested:** @Autowired/@Inject fields, volatile fields, setter-assigned fields, constructor-only assignment (true positive), @Entity classes, Lombok @Setter, field reassignment detection, protected/package-private fields.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Why not higher-ranked tool:** Graph is the highest-ranked tool.
- **Query:** Iterate `graph.file_entries()` for Java. Collect Symbol nodes with `kind == Variable` and `exported == false` (private fields).
- **Returns:** `(file_path, field_name, start_line)` tuples.

#### Step 2: Narrowing
- **Tool:** Tree-sitter
- **Why not graph only:** Graph does not capture modifier details. Need tree-sitter to check for `final`, `volatile`, and annotations.
- **Query:** For each private non-final field: (a) check for `volatile` modifier (skip), (b) check for injection annotations (@Autowired, @Inject, @Value, @Setter), (c) search the enclosing class body for assignment expressions targeting this field name outside of constructors. If field is only assigned in constructor(s), it is a true candidate for `final`.
- **Returns:** `(field_name, is_volatile, has_injection_annotation, assigned_only_in_constructor, assignment_locations)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:** Only flag fields that: (a) are assigned only in constructor(s) and could be `final`, or (b) are never assigned at all after declaration (could be `final` with initializer). Skip volatile, injection-annotated, and setter-assigned fields. Graduate: constructor-only = warning (should be final), never-assigned = info (potentially unused or set via reflection), setter-assigned = skip.
- **Returns:** Filtered findings with graduated severity and rationale.

#### Graph Enhancement Required
- Intra-class field assignment tracking: edges from methods to fields they write would enable graph-only detection of "constructor-only assignment" pattern.

### New Test Cases
1. **test_autowired_field_skipped** -- `@Autowired private UserService userService;` should be skipped -- Covers: no suppression/annotation awareness
2. **test_volatile_field_skipped** -- `private volatile boolean running;` should be skipped -- Covers: language idiom ignorance
3. **test_constructor_only_assignment** -- Field assigned only in constructor should be flagged as "should be final" -- Covers: no data flow tracking
4. **test_setter_assigned_field** -- `private String name; void setName(String n) { this.name = n; }` should be skipped -- Covers: high false positive rate
5. **test_entity_class** -- `@Entity class User { private String name; }` should be skipped -- Covers: no suppression/annotation awareness
6. **test_never_assigned** -- `private int unused;` (never assigned after declaration, no initializer) should be flagged as potentially unused -- Covers: missing context
7. **test_lombok_setter** -- `@Setter private String name;` should be skipped -- Covers: no suppression/annotation awareness

---

## Cross-Pipeline Issues

### 1. No Pipeline Uses GraphPipeline Trait
All 11 pipelines use the legacy `Pipeline` trait. None leverage the `CodeGraph` which is already built during audit runs. The graph provides symbol resolution, call graph traversal, CFG analysis, taint propagation, and resource lifecycle tracking -- all of which would dramatically improve detection accuracy.

### 2. No Cross-File Analysis
All pipelines operate on individual files. Patterns like magic strings (same literal in multiple files), god classes (classes used by too many other classes), and resource leaks (resource passed to another method that closes it) require cross-file analysis that the graph enables.

### 3. Inconsistent Test File Handling
Only `null_returns` calls `is_test_file()`. Other pipelines that should skip test files (magic_strings, missing_final, mutable_public_fields) do not. Test code has different quality standards and should not trigger the same findings.

### 4. No Annotation Framework
There is no shared utility for checking `@SuppressWarnings` with pipeline-specific tags. Each pipeline would need to implement this independently. A shared helper like `is_suppressed(node, source, pipeline_name)` would benefit all pipelines.

### 5. Protected Field Gap
`mutable_public_fields` checks public fields. `missing_final` checks private fields. Neither checks `protected` or package-private fields, leaving a gap in encapsulation analysis.

### Migration Priority (by impact)
1. **resource_leaks** -- Highest impact. Graph already has `Acquires`/`ReleasedBy` edges. Current pipeline has a false-positive bug and misses many patterns.
2. **god_class** -- High impact. Graph `Contains` edges and caller analysis would enable cohesion-based detection.
3. **null_returns** -- High impact. Graph could enable cross-method null propagation tracking.
4. **string_concat_in_loops** -- Medium impact. Type resolution via tree-sitter would eliminate name heuristics.
5. **magic_strings** -- Medium impact. Cross-file literal frequency requires graph-level aggregation.
6. **exception_swallowing** -- Medium impact. Mostly tree-sitter-local, but graph could help with exception type hierarchy.
7. **instanceof_chains** -- Low-medium impact. Mostly tree-sitter-local, but type hierarchy in graph would improve suggestions.
8. **raw_types** -- Low-medium impact. Import resolution in graph would detect custom generic types.
9. **static_utility_sprawl** -- Low impact. Caller analysis in graph would add cohesion insight.
10. **mutable_public_fields** -- Low impact. Mostly correct, needs annotation awareness.
11. **missing_final** -- Low impact. Needs intra-class data flow, which is tree-sitter-achievable.
