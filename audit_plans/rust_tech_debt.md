# Rust Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 10
- **Trait types used:** Pipeline (all 10 are legacy `Pipeline` trait, wrapped via `AnyPipeline::Legacy`)
- **Overall assessment:** The pipelines are functional but uniformly use the legacy `Pipeline` trait, ignoring the pre-built CodeGraph entirely. Detection is almost exclusively name-based string matching via tree-sitter, resulting in high false positive rates across several pipelines (clone_detection, magic_numbers, must_use_ignored) and limited ability to distinguish intentional patterns from genuine tech debt. No pipeline has suppression/annotation awareness, and most lack severity graduation.

---

## panic_detection

### Current Implementation
- **File:** `src/audit/pipelines/rust/panic_detection.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `unwrap`, `expect`, `panic`, `todo`, `unimplemented`, `unreachable`
- **Detection method:** Uses two tree-sitter queries: `compile_method_call_query` (matches `.unwrap()` and `.expect()` via `field_expression > field_identifier`) and `compile_macro_invocation_query` (matches `panic!()`, `todo!()`, `unimplemented!()`, `unreachable!()` via `macro_invocation > identifier`). Filters out test files (`is_test_file`), test contexts (`is_test_context_rust`), and `.lock().unwrap()` chains. Downgrades severity to "info" inside `main()`.

### Problems Identified
1. **High false positive rate (Rubric #2):** Flags `.unwrap()` after `.is_some()` / `.is_ok()` guards or in `if let` exhaustive branches. Also flags `.expect()` with meaningful messages on infallible operations (e.g., regex compilation with a known-good pattern).
2. **Language idiom ignorance (Rubric #13):** Does not exempt `unwrap()` on infallible conversions like `TryFrom` on known-good values, `Mutex::lock().unwrap()` is partially handled but `RwLock::read().unwrap()` / `RwLock::write().unwrap()` are not suppressed. Also, `.expect()` on `env::var()` at startup is idiomatic.
3. **No suppression/annotation awareness (Rubric #11):** No way to suppress findings via `// SAFETY:` comments, `#[allow(clippy::unwrap_used)]`, or inline `// virgil-ignore` markers.
4. **Missing context (Rubric #4):** Uses tree-sitter positional lookup to check test context, but does not use the CodeGraph to determine if the enclosing function is reachable from production entry points or if the value being unwrapped is guaranteed `Some`/`Ok` by prior control flow.
5. **No severity graduation (Rubric #15):** `unwrap` and `expect` get the same "warning" severity, but `expect` with a descriptive message is far lower risk than bare `unwrap`. `todo!()` and `unimplemented!()` are the same severity as `unreachable!()` even though the latter is often correct.
6. **Single-node detection (Rubric #14):** For methods, detects the call node but does not inspect the receiver type or preceding control flow to determine if the unwrap is guarded.
7. **Missing compound variants (Rubric #9):** Does not detect `unwrap_or_else(|| panic!(...))` which is semantically equivalent to `expect()`.
8. **No scope awareness (Rubric #7):** Skips test files but does not skip build scripts (`build.rs`) or proc-macro crates where `unwrap` is normal.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** All 6 pattern names detected, clean code produces no findings, correct metadata (file, pipeline, pattern, severity, message), `unwrap` in `main()` downgraded to info, snippet captures full expression.
- **What's NOT tested:** `lock().unwrap()` suppression, `is_test_context_rust` filtering (test files vs `#[test]` fns vs `#[cfg(test)]` mods), chained unwrap, `expect` with long message, macro patterns in different positions (nested in closures), `RwLock::read().unwrap()`.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate `graph.file_nodes` to get all Rust file paths. Filter out test files using `is_test_file()` and build scripts (path ends with `build.rs`).
- **Returns:** List of `(file_path, NodeIndex)` pairs for production Rust files.

#### Step 2: Symbol-Level Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph does not store call-site method names at granularity needed (e.g., `.unwrap()` vs `.expect()` are both just `CallSite` nodes named "unwrap"/"expect", but we need receiver context from AST). However, the graph's `CallSite` nodes could be used for initial candidate identification.
- **Query:** Same `compile_method_call_query` and `compile_macro_invocation_query` as current. Collect all candidate call sites with line numbers.
- **Returns:** List of `(file_path, line, pattern_name, snippet)` candidates.

#### Step 3: False Positive Removal
- **Tool:** Graph query + Tree-sitter
- **Why not graph alone:** Graph CFGs have `Guard` statements that can express prior `is_some()`/`is_ok()` checks, but the CFG does not currently track type narrowing or variable state. Tree-sitter parent inspection is still needed for `lock()` receiver chain detection.
- **Query:** For each candidate:
  1. Graph: Look up the enclosing function via `graph.find_symbol(file_path, fn_start_line)`. Check if function has `#[test]` attribute via tree-sitter sibling inspection.
  2. Graph CFG: Check `function_cfgs` for the enclosing function. Look for a `Guard` statement on the same variable before the `unwrap` call (e.g., `if x.is_some()` → Guard with condition_vars containing `x`).
  3. Tree-sitter: Check if receiver chain contains `.lock()`, `.read()`, or `.write()` (Mutex/RwLock pattern).
  4. Tree-sitter: Check for preceding `// SAFETY:` comment on the previous line.
  5. Graduated severity: `todo!`/`unimplemented!` → "warning", `unreachable!` → "info", `expect` with message → "info", bare `unwrap` → "warning", `panic!` → "warning".
- **Returns:** Filtered findings with graduated severity.

#### Graph Enhancement Required
- **Missing:** `CallSite` nodes do not store the receiver expression text or the method's receiver type.
- **Why needed:** To distinguish `.lock().unwrap()` from `option.unwrap()` at graph level without falling back to tree-sitter.
- **Proposed change:** Add `receiver: Option<String>` field to `CallSite` node weight, storing the receiver expression text (e.g., "mutex.lock()").

### New Test Cases
1. **guarded_unwrap_not_flagged** — Input: `if x.is_some() { x.unwrap() }` → Expected: not flagged — Covers: #2 false positive
2. **rwlock_read_unwrap_not_flagged** — Input: `let guard = rwlock.read().unwrap();` → Expected: not flagged — Covers: #13 idiom ignorance
3. **expect_with_message_lower_severity** — Input: `let re = Regex::new(r"^\d+$").expect("valid regex");` → Expected: flagged as "info" not "warning" — Covers: #15 severity graduation
4. **unwrap_or_else_panic_detected** — Input: `x.unwrap_or_else(|| panic!("gone"))` → Expected: flagged — Covers: #9 compound variants
5. **safety_comment_suppression** — Input: `// SAFETY: guaranteed Some\n x.unwrap()` → Expected: not flagged — Covers: #11 suppression
6. **build_rs_not_flagged** — Input: `unwrap()` in file path `build.rs` → Expected: not flagged — Covers: #7 scope
7. **cfg_test_module_not_flagged** — Input: `#[cfg(test)] mod tests { fn t() { x.unwrap(); } }` → Expected: not flagged — Covers: #7 test context
8. **unreachable_lower_severity** — Input: `unreachable!()` → Expected: flagged as "info" — Covers: #15 severity graduation

---

## clone_detection

### Current Implementation
- **File:** `src/audit/pipelines/rust/clone_detection.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `clone`, `to_owned`, `to_string`
- **Detection method:** Uses `compile_method_call_query` to find all `.clone()`, `.to_owned()`, and `.to_string()` calls. Every match is flagged as "info" severity with no filtering whatsoever.

### Problems Identified
1. **High false positive rate (Rubric #2):** Every single `.clone()`, `.to_owned()`, `.to_string()` call is flagged. This includes: `Clone` on `Copy` types (no-op), `.clone()` on `Arc`/`Rc` (cheap, idiomatic), `.to_string()` for display/logging, `.to_owned()` converting `&str` to `String` for return values (necessary). This pipeline will generate enormous noise on any real codebase.
2. **Language idiom ignorance (Rubric #13):** `.clone()` on `Arc`, `Rc`, `Cow`, and `Copy` types is idiomatic and zero/cheap-copy. `.to_string()` implementing `Display` is the standard pattern. Flagging all of these is anti-idiomatic.
3. **No data flow tracking (Rubric #10):** Does not check if the cloned value is actually used (clone might be necessary for ownership transfer), or if there's an alternative borrow path.
4. **No scope awareness (Rubric #7):** Does not skip test files or test contexts where clone usage is acceptable for test setup.
5. **No suppression/annotation awareness (Rubric #11):** No `#[allow(clippy::clone_on_copy)]` or `// virgil-ignore` support.
6. **Single-node detection (Rubric #14):** Only checks the method name, not the receiver type. Cannot distinguish `Arc::clone(&x)` (idiomatic) from `big_struct.clone()` (potentially expensive).
7. **No severity graduation (Rubric #15):** All findings are "info" regardless of context. A `.clone()` in a hot loop is far worse than one in initialization code.
8. **Missing context (Rubric #4):** The graph's `CallSite` nodes and `Calls` edges could identify which functions are calling clone, and whether those functions are in hot paths (called from loops).
9. **Overlapping detection across pipelines (Rubric #16):** `.to_string()` could overlap with `stringly_typed` if a string field is populated via `.to_string()`.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Detects `.clone()`, `.to_owned()`, `.to_string()`, clean code with `.len()` and `.is_empty()` not flagged, correct metadata, snippet content.
- **What's NOT tested:** `Arc::clone()`, `Copy` type clone, clone in test context, clone in loop vs initialization, `Clone` trait method call syntax (not `.clone()`), `.to_string()` on integers/display types.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph query
- **Query:** Iterate `graph.file_nodes` for all Rust files. Exclude test files.
- **Returns:** List of production Rust file paths.

#### Step 2: Candidate Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph `CallSite` nodes store call name but not the receiver type, which is essential for distinguishing cheap vs expensive clones.
- **Query:** Same `compile_method_call_query` with targets `["clone", "to_owned", "to_string"]`. Additionally, inspect the receiver node of each call to extract the receiver expression text.
- **Returns:** List of `(file_path, line, pattern, receiver_text, snippet)`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + AI prompt
- **Why not graph alone:** The graph does not have type information. Distinguishing `Arc::clone` from `BigStruct::clone` requires either type inference (not available) or heuristic receiver text inspection.
- **Query/Prompt (tree-sitter heuristic):**
  1. If receiver text matches `Arc::clone(`, `Rc::clone(`, or method-call form where receiver was assigned from `Arc::new`/`Rc::new` in same scope — suppress.
  2. If the call is `<type>::clone(&x)` form (scoped call, not method call) — this is UFCS, likely intentional `Arc::clone` pattern — suppress.
  3. If the `.clone()` is on a literal or a simple identifier that appears as a function parameter of a `Copy`-hinted type (i32, u64, bool, char, &str, etc.) — suppress.
  4. Skip test contexts via `is_test_context_rust`.
  5. Graduated severity: `.clone()` inside a loop body → "warning", `.clone()` in function body → "info".
- **Returns:** Filtered, severity-graduated findings.

#### Graph Enhancement Required
- **Missing:** Type information for symbols and variables (the graph has `SymbolKind` but not the actual Rust type).
- **Why needed:** To definitively distinguish `Arc<T>::clone` (cheap) from `Vec<T>::clone` (expensive) without heuristics.
- **Proposed change:** Add optional `type_annotation: Option<String>` to `Symbol` and `Parameter` node weights, populated from tree-sitter type nodes during graph construction.

### New Test Cases
1. **arc_clone_not_flagged** — Input: `let b = Arc::clone(&a);` → Expected: not flagged — Covers: #13 idiom
2. **rc_clone_not_flagged** — Input: `let b = a.clone(); // a: Rc<T>` (where a is assigned from `Rc::new(...)`) → Expected: not flagged — Covers: #13 idiom
3. **clone_in_loop_higher_severity** — Input: `for x in items { let y = x.clone(); }` → Expected: flagged as "warning" — Covers: #15 severity
4. **to_string_on_display_not_flagged** — Input: `let s = 42.to_string();` → Expected: not flagged (integer Display is standard) — Covers: #2 false positive
5. **test_context_clone_not_flagged** — Input: `#[test] fn t() { let b = a.clone(); }` → Expected: not flagged — Covers: #7 scope
6. **clone_on_copy_type_not_flagged** — Input: `let x: i32 = 5; let y = x.clone();` → Expected: not flagged — Covers: #13 idiom

---

## god_object_detection

### Current Implementation
- **File:** `src/audit/pipelines/rust/god_object_detection.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `large_impl_block` (>=10 methods), `large_struct` (>=15 fields)
- **Detection method:** Uses `compile_impl_block_query` to find impl blocks and count `function_item` children; uses `compile_struct_fields_query` to find structs and count `field_declaration` children. Filters out trait impls via `is_trait_impl` helper (text-based "for" keyword detection). Reports with truncated snippets (3 lines).

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric #12):** `LARGE_IMPL_THRESHOLD = 10` and `LARGE_STRUCT_THRESHOLD = 15` are magic numbers. 10 methods is low — many well-designed types have 10+ methods when implementing builder patterns, iterators, or comprehensive APIs. No citation or rationale provided.
2. **No scope awareness (Rubric #7):** Does not distinguish generated code (e.g., `derive` macro expansions, protobuf-generated structs) from hand-written code. Also does not skip test helper types.
3. **Missing context (Rubric #4):** The graph has `Symbol` nodes with `kind: Struct/Method` and `DefinedIn`/`Contains` edges. Could use graph to count methods per struct across multiple impl blocks (same struct, different impl blocks in different files) for a true total count.
4. **No suppression/annotation awareness (Rubric #11):** No `#[allow(...)]` or comment-based suppression.
5. **Missing compound variants (Rubric #9):** Counts only `function_item` children in the impl block's `declaration_list`. Does not count associated types, constants, or other items that also contribute to complexity. Multiple impl blocks for the same type in different files are counted separately, missing the total.
6. **No severity graduation (Rubric #15):** All findings are "warning" regardless of how far over the threshold. An impl with 11 methods and one with 50 methods get the same severity.
7. **High false negative rate (Rubric #3):** If a type has 5 methods in one impl block and 8 in another (both in the same file), neither is flagged even though the type has 13 total methods.
8. **Language idiom ignorance (Rubric #13):** Builder pattern types naturally have many methods (one per field). Does not detect or exempt builder pattern structs.

### Test Coverage
- **Existing tests:** 9 tests
- **What's tested:** Large impl (12 methods) flagged, small impl (3 methods) not flagged, large struct (16 fields) flagged, small struct (5 fields) not flagged, both in same file, trait impl skipped, generic struct detected, clean code, correct metadata, snippet truncation, empty impl, multiple impl blocks counted separately.
- **What's NOT tested:** Threshold boundary (exactly 10 methods), builder pattern impl, generated code (`#[derive]` expansions), multiple impl blocks for same type across files, associated types/constants impact.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Per-Type Method Aggregation
- **Tool:** Graph query
- **Query:** For each `Symbol` node where `kind == Struct`, use `graph.find_symbols_by_name(struct_name)` to find all related symbols. Then traverse `Contains` edges from the struct's file node to find all `Method` symbols in the same file that belong to impl blocks for this struct. Alternatively, iterate all `Symbol` nodes with `kind == Method`, group by enclosing impl type name (requires tree-sitter to extract impl type name).
- **Returns:** Map of `struct_name -> (total_method_count, total_field_count, file_paths)`.

#### Step 2: Threshold Check with Graduation
- **Tool:** Graph query (data from step 1)
- **Query:** For each struct exceeding thresholds:
  - 10-19 methods → "info"
  - 20-29 methods → "warning"  
  - 30+ methods → "error"
  - 15-24 fields → "info"
  - 25+ fields → "warning"
- **Returns:** Graduated findings.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph
- **Query:**
  1. Tree-sitter: Check if struct has `#[derive(...)]` with code-generation derives (serde, protobuf, etc.) — exempt from field count check.
  2. Tree-sitter: Check if impl block methods follow builder pattern (all return `Self` or `&mut Self`) — exempt from method count.
  3. Graph: Check if the impl is a trait impl by inspecting `is_trait_impl` on the AST node (already done, keep).
- **Returns:** Filtered findings.

#### Graph Enhancement Required
- **Missing:** Association between `Symbol` (Struct) and its impl blocks' method counts. The graph stores methods as separate symbols with `DefinedIn` edges to files, but does not store which impl block they belong to or what type the impl is for.
- **Why needed:** To aggregate method counts across multiple impl blocks for the same type.
- **Proposed change:** Add `impl_target: Option<String>` field to `Symbol` nodes of kind `Method`, storing the type name from the enclosing `impl` block.

### New Test Cases
1. **multiple_impl_blocks_aggregated** — Input: `impl Foo { fn a() {} fn b() {} ... }` and `impl Foo { fn c() {} fn d() {} ... }` totaling 12 methods → Expected: flagged — Covers: #3 false negative
2. **builder_pattern_exempt** — Input: impl with 15 methods all returning `Self` → Expected: not flagged — Covers: #13 idiom
3. **severity_graduation_20_methods** — Input: impl with 22 methods → Expected: flagged as "warning" — Covers: #15 severity
4. **derived_struct_exempt** — Input: `#[derive(serde::Deserialize)] struct Big { ... 20 fields }` → Expected: not flagged for fields — Covers: #7 generated code
5. **threshold_boundary_exactly_10** — Input: impl with exactly 10 methods → Expected: flagged as "info" — Covers: #12 threshold

---

## stringly_typed

### Current Implementation
- **File:** `src/audit/pipelines/rust/stringly_typed.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `stringly_typed_field`, `stringly_typed_param`
- **Detection method:** Two checks: (1) struct fields where field name is in `SUSPICIOUS_NAMES` (kind, type, status, mode, state, level, role, variant, phase, stage) AND type is in `STRING_TYPES` (String, &str, Option<String>, Option<&str>); (2) function parameters with same name+type criteria. Skips fields in structs with `#[derive(Deserialize)]`.

### Problems Identified
1. **Literal blindness (Rubric #8):** The suspicious name list is hardcoded and may miss domain-specific names like "category", "priority", "severity", "action", "direction", "encoding". Conversely, `type` is a keyword in Rust and can't be used as a field name directly (must be `r#type`), so the `type` check may not match tree-sitter output depending on how the identifier is parsed.
2. **High false positive rate (Rubric #2):** Fields named "status" that legitimately hold free-form strings (HTTP status text, error messages) are flagged. Also, parameters named "mode" that accept user-provided strings (e.g., file modes "r", "w", "rw") are flagged.
3. **Missing context (Rubric #4):** Does not check if an enum with a similar name already exists in the codebase. If `enum Status` exists in the same file/module, the finding is more actionable; if not, it's speculative.
4. **No suppression/annotation awareness (Rubric #11):** Only `Deserialize` derive is checked for fields. No `Serialize` derive check for params, no comment-based suppression.
5. **Missing edge cases in tests (Rubric #6):** No test for `r#type` field name, no test for `Cow<str>` type, no test for nested types like `Vec<String>` in suspicious name fields.
6. **No severity graduation (Rubric #15):** All findings are "info" regardless of whether there are 2 or 20 string-typed suspicious fields in the same struct.
7. **High false negative rate (Rubric #3):** Does not detect `Cow<'_, str>`, `Box<str>`, `Arc<str>`, or `Rc<str>` as string types. Does not detect `Into<String>` parameter types.
8. **Missing compound variants (Rubric #9):** Does not check for `Vec<String>` fields with suspicious names like `tags`, `labels`, `categories` which suggest an enum set.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** String status field detected, enum-typed status not flagged, &str mode param detected, non-suspicious name not flagged, Option<String> kind field detected.
- **What's NOT tested:** `r#type` field, `Cow<str>` type, Deserialize-derived struct param, multiple suspicious fields in one struct, param with same name in trait impl.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Candidate Identification
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph `Symbol` nodes store name and kind but not field-level details (individual struct fields are not graph nodes). Parameter nodes exist in graph but don't store type text.
- **Query:** Same `compile_field_declaration_query` and `compile_parameter_query`. Expand `STRING_TYPES` to include `Cow<str>`, `Cow<'_, str>`, `Box<str>`, `Arc<str>`, `Rc<str>`.
- **Returns:** List of `(file_path, line, name, type_text, context: "field"|"param")`.

#### Step 2: Context Enrichment
- **Tool:** Graph query
- **Query:** For each finding, use `graph.find_symbols_by_name(suspicious_name)` to check if an enum with that name (or PascalCase variant, e.g., "status" → "Status") exists anywhere in the codebase. If yes, escalate severity to "warning" since the enum already exists and should be used.
- **Returns:** Enriched findings with `has_existing_enum: bool`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:**
  1. Check if struct has `#[derive(Deserialize)]` OR `#[derive(Serialize)]` — skip.
  2. Check if the field/param has a `// virgil-ignore` comment on the previous line — skip.
  3. Check if the enclosing function is a `new()` constructor or builder method — lower severity since constructors often accept raw strings.
- **Returns:** Filtered findings.

### New Test Cases
1. **cow_str_type_detected** — Input: `struct Config { status: Cow<'_, str> }` → Expected: flagged — Covers: #3 false negative
2. **existing_enum_higher_severity** — Input: file with `enum Status { Active, Inactive }` and `struct Config { status: String }` → Expected: flagged as "warning" — Covers: #4 missing context
3. **rtype_field_detected** — Input: `struct Config { r#type: String }` → Expected: flagged — Covers: #6 edge case
4. **serialize_derive_exempt** — Input: `#[derive(Serialize)] struct Dto { status: String }` → Expected: not flagged — Covers: #2 false positive
5. **vec_string_tags_detected** — Input: `struct Config { tags: Vec<String> }` → Expected: flagged (with expanded suspicious names) — Covers: #9 compound variants

---

## must_use_ignored

### Current Implementation
- **File:** `src/audit/pipelines/rust/must_use_ignored.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `ignored_result` (expression statement), `discarded_result` (`let _ = ...`)
- **Detection method:** Two tree-sitter queries: (1) expression statements containing method calls where the method name is in `TARGET_METHODS` (lock, send, write, read, flush, recv, try_lock, try_send, try_recv); (2) `let _ = ...` patterns with same target methods. Only checks method calls via `field_expression`, not scoped function calls.

### Problems Identified
1. **High false positive rate (Rubric #2):** `.read()` and `.write()` on `BufReader`/`BufWriter` return `Result` but `.read()` on a `HashMap` / `RwLock` returns a guard — the same method name has very different semantics. `.flush()` on `stdout` in a CLI tool where the process exits immediately is harmless.
2. **High false negative rate (Rubric #3):** Does not detect ignored results from free functions (e.g., `std::fs::write(...)` as expression statement). Does not detect `let _ = std::fs::remove_file(...)`. Does not detect ignored results from `.map()`, `.and_then()` on Results. Does not detect `drop(mutex.lock())` which immediately drops the guard.
3. **Missing context (Rubric #4):** Does not use the graph to check if the called method is actually `#[must_use]` annotated. Currently relies on a hardcoded list of method names.
4. **No suppression/annotation awareness (Rubric #11):** `let _ = ...` is the Rust idiom for intentionally discarding a result. Flagging it is arguably a false positive since the user has explicitly acknowledged the discard. Should at least check for a `// intentionally discarded` comment.
5. **Overlapping detection across pipelines (Rubric #16):** `let _ = mutex.lock()` overlaps with the panic_detection pipeline if the user "fixes" by adding `.unwrap()`.
6. **No scope awareness (Rubric #7):** Does not skip test files or test contexts.
7. **No severity graduation (Rubric #15):** Ignoring `lock()` (can cause deadlock from dropped guard) is far more severe than ignoring `flush()`.
8. **Language idiom ignorance (Rubric #13):** `let _ = tx.send(...)` in channel teardown is idiomatic Rust when the receiver may have been dropped. Flagging it adds noise.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Ignored lock detected, assigned lock not flagged, non-target method not flagged, `let _ = sender.send(42)` detected as discarded, named let binding not flagged.
- **What's NOT tested:** `let _ = ...` with comment, free function ignored results, `.read()` on different types, `.write()` as expression statement, test file exclusion, `drop(lock.lock())`.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Candidate Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph `CallSite` nodes don't distinguish between expression-statement calls and assigned calls (both are just `Calls` edges).
- **Query:** Current two queries plus a new query for scoped function calls as expression statements: `(expression_statement (call_expression function: (scoped_identifier) @fn_name) @call) @stmt`. Expand target list to include scoped functions: `std::fs::write`, `std::fs::remove_file`, `std::fs::create_dir`, etc.
- **Returns:** Candidate findings with call site info.

#### Step 2: Contextual Filtering
- **Tool:** Tree-sitter + Graph
- **Query:**
  1. Tree-sitter: For `let _ = ...` patterns, check if there's a comment on the same or previous line containing "intentional", "ignore", "ok to discard" — downgrade to "info".
  2. Graph: For `lock()` dropped as expression statement, escalate to "error" severity (potential deadlock from immediately-dropped guard).
  3. Tree-sitter: Check if inside test context — skip.
  4. Tree-sitter: For `send()` calls, check if the enclosing block has a `drop` or scope-end that suggests teardown — downgrade.
  5. Severity: `lock()` ignored → "error", `send()`/`recv()` ignored → "warning", `flush()` ignored → "info", `let _ = send()` → "info" (explicit acknowledgment).
- **Returns:** Filtered, graduated findings.

#### Graph Enhancement Required
- **Missing:** `#[must_use]` attribute information on function/method symbols.
- **Why needed:** To detect ignored results of any `#[must_use]` function, not just a hardcoded list.
- **Proposed change:** Add `must_use: bool` field to `Symbol` node weight, populated during graph construction by checking for `#[must_use]` attribute on function definitions.

### New Test Cases
1. **let_underscore_with_comment_downgraded** — Input: `// intentionally ignoring\nlet _ = tx.send(42);` → Expected: "info" not "warning" — Covers: #11 suppression
2. **scoped_fn_ignored** — Input: `std::fs::write("f.txt", data);` → Expected: flagged — Covers: #3 false negative
3. **lock_ignored_error_severity** — Input: `mutex.lock();` (expression statement) → Expected: "error" severity — Covers: #15 graduation
4. **test_context_not_flagged** — Input: `#[test] fn t() { tx.send(1); }` → Expected: not flagged — Covers: #7 scope
5. **drop_lock_detected** — Input: `drop(mutex.lock().unwrap());` → Expected: flagged — Covers: #3 false negative
6. **send_in_teardown_info** — Input: `let _ = tx.send(Shutdown);` in a function named `shutdown`/`cleanup` → Expected: "info" — Covers: #13 idiom

---

## mutex_overuse

### Current Implementation
- **File:** `src/audit/pipelines/rust/mutex_overuse.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `arc_mutex`, `arc_rwlock`
- **Detection method:** Uses `compile_generic_type_query` to match nested generic types `Outer<Inner<T>>`. Checks if outer is `Arc` and inner ends with `Mutex` or `RwLock`. Provides graduated messages based on inner type: `Mutex<bool>` → suggest AtomicBool, `Mutex<usize/u64/u32/i64/i32>` → suggest Atomic*, `Mutex<HashMap/BTreeMap>` → suggest DashMap, otherwise generic suggestion.

### Problems Identified
1. **High false negative rate (Rubric #3):** The generic type query only matches one level of nesting (`Outer<Inner<T>>`). Does not detect `Arc<Mutex<T>>` when the `Mutex` is fully qualified (`std::sync::Mutex`) or when there are intermediate wrappers. Also does not detect `Arc<parking_lot::Mutex<T>>` or `Arc<tokio::sync::Mutex<T>>`.
2. **Missing context (Rubric #4):** Does not check how the `Arc<Mutex<T>>` is used. If it's only locked once in a sequential context, it's not overuse. If it's shared across many threads with contention, the suggestion is more relevant. The graph's `Acquires`/`ReleasedBy` edges could identify lock patterns.
3. **No scope awareness (Rubric #7):** Does not skip test files, where `Arc<Mutex<T>>` for test synchronization is normal.
4. **No suppression/annotation awareness (Rubric #11):** No way to mark intentional `Arc<Mutex<T>>` usage.
5. **High false positive rate (Rubric #2):** `Arc<Mutex<Vec<T>>>` is flagged as generic "consider concurrent data structure" but there's no standard concurrent Vec in the ecosystem. `Arc<Mutex<T>>` where T is a custom type has no atomic alternative.
6. **No severity graduation (Rubric #15):** `Arc<Mutex<bool>>` (clear AtomicBool replacement) and `Arc<Mutex<CustomStruct>>` (no alternative exists) both get flagged, one as "warning" and one as "info", but the atomic cases deserve "warning" while custom types should be "info" at most.
7. **Hardcoded thresholds without justification (Rubric #12):** The list of atomic-replaceable types is incomplete. Missing `Mutex<isize>`, `Mutex<u8>`, `Mutex<i8>`, `Mutex<u16>`, `Mutex<i16>`, `Mutex<f32>`, `Mutex<f64>` (no atomic float, should not be suggested).
8. **Single-node detection (Rubric #14):** Only checks type annotations, not how the value is used. A `Arc<Mutex<bool>>` that needs to be combined atomically with other state under the same lock should not be flagged.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** `Arc<Mutex<Vec<i32>>>` detected, `Arc<RwLock<HashMap<String, i32>>>` detected, `Mutex<i32>` without Arc not flagged, `Arc<Vec<i32>>` without Mutex not flagged.
- **What's NOT tested:** `Arc<Mutex<bool>>` with AtomicBool suggestion, `Arc<Mutex<usize>>` with AtomicUsize suggestion, `Arc<Mutex<HashMap>>` with DashMap suggestion, fully qualified paths, test file exclusion, `parking_lot::Mutex`.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Candidate Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph does not store type annotation text for variables/fields. Generic type nesting is an AST-level concern.
- **Query:** Same `compile_generic_type_query` but also add a text-search fallback: scan source for patterns matching `Arc<.*Mutex` and `Arc<.*RwLock` to catch fully qualified and aliased variants.
- **Returns:** List of `(file_path, line, outer, inner, full_type_text)`.

#### Step 2: Contextual Analysis
- **Tool:** Graph query
- **Query:** For each candidate, find the enclosing function symbol via `graph.find_symbol(file, line)`. Use `graph.function_cfgs` to check how many `Call` statements reference `.lock()` / `.read()` / `.write()` on the same variable. If only one lock site exists, this is less likely to be overuse.
- **Returns:** Enriched findings with `lock_site_count`.

#### Step 3: Graduated Severity + False Positive Removal
- **Tool:** Tree-sitter
- **Query:**
  1. If inner type is `bool`, `u8`-`u64`, `i8`-`i64`, `usize`, `isize` → "warning" with specific atomic suggestion.
  2. If inner type is `HashMap`/`BTreeMap`/`HashSet`/`BTreeSet` → "warning" with DashMap/DashSet suggestion.
  3. If inner type is `f32`/`f64` → "info" (no atomic float; suggest `AtomicU32`/`AtomicU64` with transmute only if user is comfortable).
  4. If inner type is custom → "info" only if lock_site_count > 1.
  5. Skip test files and test contexts.
- **Returns:** Filtered, graduated findings.

### New Test Cases
1. **arc_mutex_bool_suggests_atomic** — Input: `let x: Arc<Mutex<bool>> = ...;` → Expected: flagged as "warning" with AtomicBool message — Covers: #7 graduation already works but not tested
2. **parking_lot_mutex_detected** — Input: `let x: Arc<parking_lot::Mutex<Vec<i32>>> = ...;` → Expected: flagged — Covers: #3 false negative
3. **test_file_not_flagged** — Input: `Arc<Mutex<i32>>` in `tests/test_sync.rs` → Expected: not flagged — Covers: #7 scope
4. **single_lock_site_lower_severity** — Input: `Arc<Mutex<CustomStruct>>` locked only once → Expected: "info" or not flagged — Covers: #14 context
5. **arc_mutex_hashmap_dashmap** — Input: `let m: Arc<Mutex<HashMap<String, i32>>> = ...;` → Expected: flagged with DashMap suggestion — Covers: test coverage gap

---

## pub_field_leakage

### Current Implementation
- **File:** `src/audit/pipelines/rust/pub_field_leakage.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `all_fields_public`
- **Detection method:** Uses `compile_struct_fields_query` to find structs with named fields. Only flags `pub` structs where ALL fields have `visibility_modifier` AND the struct has >2 fields. Skips `#[derive(Deserialize)]`/`#[derive(Serialize)]` and `#[repr(C)]` structs.

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric #12):** The `total <= 2` exemption is arbitrary. A 2-field struct with both fields pub can still leak invariants (e.g., `pub struct Range { pub start: usize, pub end: usize }` where `start <= end` should be enforced).
2. **High false positive rate (Rubric #2):** Configuration structs (`Config`, `Settings`, `Options`) that are deliberately all-pub for ergonomic construction are flagged. Data-transfer structs without invariants are flagged. Unit-like accessor structs (all fields are independent values) are flagged.
3. **Missing context (Rubric #4):** Does not check if the struct has any methods that enforce invariants. If there are no methods at all (pure data), pub fields are fine. If there are methods with assertions/validations, the pub fields are a genuine leak.
4. **No suppression/annotation awareness (Rubric #11):** Only checks `Deserialize`/`Serialize` and `repr(C)`. Does not check for `// virgil-ignore` or a custom attribute.
5. **No severity graduation (Rubric #15):** All findings are "info". A 4-field struct and a 20-field struct get the same severity, despite the 20-field case being much more concerning.
6. **Language idiom ignorance (Rubric #13):** In Rust, fully-public structs are idiomatic for POD types, configuration, and builder input. The pattern is so common that flagging all instances generates excessive noise.
7. **Missing edge cases in tests (Rubric #6):** No test for `pub(crate)` visibility modifier (treated same as `pub`), no test for tuple struct fields.
8. **No scope awareness (Rubric #7):** Does not skip generated code or test helper structs.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** Small struct (<=2 fields) exempt, mixed visibility not flagged, all private not flagged, non-pub struct not flagged, 4-field all-pub flagged, correct metadata, and an explicit test for the 2-field exemption.
- **What's NOT tested:** `pub(crate)` visibility, serde Serialize skip, `repr(C)` skip, struct with validation methods, configuration-named struct, builder pattern struct.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Candidate Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph does not store individual struct fields as nodes, only the struct symbol itself.
- **Query:** Same `compile_struct_fields_query`. Collect all pub structs with all-pub fields and >2 fields.
- **Returns:** List of `(file_path, line, struct_name, field_count)`.

#### Step 2: Method Presence Check
- **Tool:** Graph query
- **Query:** For each candidate struct, search `graph.find_symbols_by_name(struct_name)` to find associated Method symbols. Iterate graph edges to find methods that belong to impl blocks of this struct. If the struct has methods that perform validation (name contains "validate", "check", "assert", "verify", body contains `assert!`/`panic!`/`return Err`), escalate severity — the pub fields genuinely leak invariants.
- **Returns:** Enriched findings with `has_validation_methods: bool`.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:**
  1. If struct name ends with `Config`, `Settings`, `Options`, `Params`, `Args`, `Props` → downgrade to "info" (likely intentional).
  2. If struct has `#[derive(Builder)]` or builder-pattern methods → skip.
  3. If struct has zero methods across all impl blocks → skip (pure data).
  4. If `has_validation_methods` → "warning".
  5. Otherwise → "info".
- **Returns:** Filtered, graduated findings.

### New Test Cases
1. **config_struct_downgraded** — Input: `pub struct Config { pub host: String, pub port: u16, pub timeout: u64 }` → Expected: "info" or not flagged — Covers: #2 false positive
2. **struct_with_validation_escalated** — Input: pub struct with all-pub fields AND an `impl` with a `validate()` method → Expected: "warning" — Covers: #4 context
3. **struct_with_no_methods_not_flagged** — Input: `pub struct Point { pub x: f64, pub y: f64, pub z: f64 }` with no impl block → Expected: not flagged (pure data) — Covers: #13 idiom
4. **pub_crate_visibility_counted** — Input: `pub struct Foo { pub(crate) a: i32, pub(crate) b: i32, pub(crate) c: i32 }` → Expected: flagged — Covers: #6 edge case
5. **two_field_with_invariant_flagged** — Input: `pub struct Range { pub start: usize, pub end: usize }` with `fn new() { assert!(start <= end) }` → Expected: flagged — Covers: #12 threshold

---

## missing_trait_abstraction

### Current Implementation
- **File:** `src/audit/pipelines/rust/missing_trait_abstraction.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `concrete_infra_type`
- **Detection method:** Uses `compile_parameter_query` to find function parameters. Extracts the leaf type name (strips `&`, `mut`, and path segments). Checks if the leaf type is in `CONCRETE_INFRA_TYPES` (File, TcpStream, TcpListener, UdpSocket, BufReader, BufWriter, Stdin, Stdout, Stderr). Exempts functions named `main`, `new`, or starting with `open`.

### Problems Identified
1. **High false positive rate (Rubric #2):** Functions that specifically need a `File` (e.g., to call `file.metadata()`, `file.set_permissions()`) cannot use `impl Read`. Also, `TcpStream` is needed when you need both read and write on the same handle. The pipeline does not distinguish "needs Read" from "needs File-specific APIs".
2. **High false negative rate (Rubric #3):** Does not detect `PathBuf` parameters (should suggest `impl AsRef<Path>`), `String` parameters (could suggest `impl AsRef<str>` or `&str`), `Vec<u8>` parameters (could suggest `impl AsRef<[u8]>`), `Box<dyn Read>` (already abstracted, skip). Missing `UnixStream`, `ChildStdout`, `ChildStdin`.
3. **Missing context (Rubric #4):** Does not check what methods are called on the parameter inside the function body. If only `read()` is called, `impl Read` is appropriate. If `File`-specific methods like `metadata()` are called, the concrete type is necessary.
4. **No suppression/annotation awareness (Rubric #11):** Only exempts `main`, `new`, `open*` functions. No comment-based suppression.
5. **Single-node detection (Rubric #14):** Only checks the parameter type declaration, not how the parameter is used in the function body.
6. **No scope awareness (Rubric #7):** Does not skip test files or test helper functions.
7. **Language idiom ignorance (Rubric #13):** In trait impl methods, the parameter type is dictated by the trait definition — the implementor cannot change it to `impl Read`. The pipeline does not check if the enclosing function is part of a trait impl.
8. **No severity graduation (Rubric #15):** All findings are "info" regardless of whether the function is exported (API boundary, higher impact) or private.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** `File` param flagged, `&File` param flagged, `impl Read` param not flagged, non-infra type not flagged, `TcpStream` flagged.
- **What's NOT tested:** `main()` exemption, `new()` exemption, trait impl method, `PathBuf` param, usage-based filtering, test file.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Candidate Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph `Parameter` nodes store name and position but not type text.
- **Query:** Same `compile_parameter_query`. Expand `CONCRETE_INFRA_TYPES` to include `PathBuf`, `UnixStream`, `ChildStdout`, `ChildStdin`, `ChildStderr`. Extract the function name and whether it's in a trait impl.
- **Returns:** List of `(file_path, line, param_name, leaf_type, fn_name, is_trait_impl)`.

#### Step 2: Usage Analysis
- **Tool:** Tree-sitter
- **Why not higher-ranked tool:** Graph CFGs store `Call` statements with names but don't link them to specific receiver variables with enough fidelity to determine if only `Read` trait methods are called.
- **Query:** Within the function body, find all method calls on the parameter variable. If ALL calls are from `Read` trait (`read`, `read_to_string`, `read_to_end`, `read_exact`) → suggest `impl Read`. If ALL calls are from `Write` trait → suggest `impl Write`. If any call is type-specific → suppress finding.
- **Returns:** Filtered findings with specific trait suggestion.

#### Step 3: Scope Filtering
- **Tool:** Graph query + Tree-sitter
- **Query:**
  1. Skip if `is_trait_impl` (cannot change parameter type).
  2. Skip test files and test contexts.
  3. Graph: If function symbol is `exported: true` → "warning" (public API should be generic). If private → "info".
- **Returns:** Graduated findings.

#### Graph Enhancement Required
- **Missing:** Parameter type text in `Parameter` node weight.
- **Why needed:** To detect concrete infra types at graph level without tree-sitter fallback.
- **Proposed change:** Add `type_text: Option<String>` field to `Parameter` node weight.

### New Test Cases
1. **trait_impl_method_not_flagged** — Input: `impl Handler for MyHandler { fn handle(&self, stream: TcpStream) {} }` → Expected: not flagged — Covers: #13 idiom
2. **file_specific_method_used_not_flagged** — Input: `fn process(file: File) { file.metadata(); }` → Expected: not flagged — Covers: #5 single-node
3. **only_read_methods_suggest_trait** — Input: `fn process(file: File) { file.read_to_string(&mut s); }` → Expected: flagged with "impl Read" suggestion — Covers: #4 context
4. **exported_fn_higher_severity** — Input: `pub fn process(file: File) {}` → Expected: "warning" — Covers: #15 graduation
5. **pathbuf_param_detected** — Input: `fn load(path: PathBuf) {}` → Expected: flagged with "impl AsRef<Path>" suggestion — Covers: #3 false negative
6. **test_file_not_flagged** — Input: `File` param in test file → Expected: not flagged — Covers: #7 scope

---

## async_blocking

### Current Implementation
- **File:** `src/audit/pipelines/rust/async_blocking.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `blocking_in_async`
- **Detection method:** Three-phase: (1) Find all `async fn` bodies via `compile_function_item_query`, checking if function text starts with "async"; (2) Within those body byte ranges, find scoped calls matching `BLOCKING_SCOPED_PREFIXES` (std::fs::, fs::, std::thread::sleep, thread::sleep); (3) Find method calls `.join()` (without arguments, to distinguish from `str::join`). Filters out calls inside `spawn_blocking`/`block_in_place` closures.

### Problems Identified
1. **High false negative rate (Rubric #3):** Does not detect blocking in `async` closures/blocks (only checks `async fn`). Misses: `std::net::TcpStream::connect()`, `std::io::stdin().read_line()`, `std::process::Command::output()`, `std::thread::spawn().join()` (the `spawn` is fine but `join` blocks), `reqwest::blocking::get()`. Does not detect `std::time::Instant::elapsed()` in a spin loop.
2. **Missing context (Rubric #4):** Does not use the graph's CFG to trace whether a blocking call is reached from an async context transitively (e.g., `async fn a()` calls `sync fn b()` which calls `std::fs::read()` — the blocking call in `b` is not inside an async body but blocks the runtime).
3. **No suppression/annotation awareness (Rubric #11):** The `spawn_blocking` check is hardcoded. No support for custom blocking wrappers or `// virgil-ignore` comments. Does not recognize `#[tokio::main]` on `main()` as an async context.
4. **Single-node detection (Rubric #14):** Only checks if the call text starts with the blocking prefix. `fs::read` could be `tokio::fs::read` if `use tokio::fs` is in scope — the pipeline would miss the tokio import resolution.
5. **No severity graduation (Rubric #15):** `std::fs::read` (may block for milliseconds on SSD) and `std::thread::sleep(Duration::from_secs(60))` (blocks for 60 seconds) get the same "warning" severity.
6. **High false positive rate (Rubric #2):** `fs::metadata()` is fast and often acceptable in async code. `fs::read` on small known files may be intentional when tokio::fs overhead is not warranted.
7. **Missing compound variants (Rubric #9):** Does not detect `File::open()` + `file.read_to_string()` chains where the `open` is blocking but `read` uses the opened handle.
8. **Language idiom ignorance (Rubric #13):** In `#[tokio::test]` async tests, blocking calls are often acceptable for test setup.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** `std::fs::read` in async flagged, `std::fs::read` in sync not flagged, `tokio::fs::read` in async not flagged, `thread::sleep` in async flagged, `.join()` in async flagged, `parts.join(",")` (string join with args) not flagged.
- **What's NOT tested:** `spawn_blocking` wrapper exemption, `async` block (not fn), `std::net::TcpStream::connect`, `std::process::Command::output`, `#[tokio::main]` on main, import aliasing (`use std::fs; fs::read`), nested async/sync call chains.

### Replacement Pipeline Design

**Target trait:** GraphPipeline

#### Step 1: Identify Async Contexts
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph `Symbol` nodes don't store whether a function is async.
- **Query:** Current `compile_function_item_query` approach. Also detect async blocks: `(async_block) @async_block` and `#[tokio::main]`/`#[async_std::main]` annotated functions via attribute inspection.
- **Returns:** List of async body byte ranges + enclosing function NodeIndex (from graph).

#### Step 2: Direct Blocking Call Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph `CallSite` nodes store call names but not whether they are blocking (no type info).
- **Query:** Expand blocking call list: add `std::net::TcpStream::connect`, `std::net::TcpListener::bind`, `std::process::Command::output`, `std::process::Command::status`, `std::io::stdin`, `reqwest::blocking::`. Keep current scoped prefix matching plus method call matching.
- **Returns:** Candidate blocking calls within async ranges.

#### Step 3: Transitive Blocking Detection
- **Tool:** Graph query
- **Query:** For each async function, use `graph.traverse_callees([fn_node], 2)` to find callees up to depth 2. For each callee, check its CFG or tree-sitter body for blocking calls. This catches `async fn a() -> b()` where `b` has `std::fs::read`.
- **Returns:** Additional findings for transitively-blocked async functions.

#### Step 4: False Positive Removal
- **Tool:** Tree-sitter
- **Query:**
  1. Check if call is inside `spawn_blocking`, `block_in_place`, or `block_on` closure — skip.
  2. Check for `// virgil-ignore` or `// blocking ok` comment — skip.
  3. Skip test contexts (`#[tokio::test]`, `#[test]`).
  4. Severity: `thread::sleep` → "error" (always wrong in async), `std::fs::*` → "warning", `metadata()`/`exists()` → "info".
- **Returns:** Filtered, graduated findings.

#### Graph Enhancement Required
- **Missing:** `is_async: bool` on `Symbol` node weight for functions/methods.
- **Why needed:** To identify async functions at graph level without re-parsing AST.
- **Proposed change:** Add `is_async: bool` field to `Symbol` nodes of kind `Function`/`Method`.

### New Test Cases
1. **async_block_blocking_detected** — Input: `let fut = async { std::fs::read("f").unwrap() };` → Expected: flagged — Covers: #3 false negative
2. **transitive_blocking_detected** — Input: `async fn a() { b(); } fn b() { std::fs::read("f"); }` → Expected: flagged (in `a`) — Covers: #4 missing context
3. **tokio_test_not_flagged** — Input: `#[tokio::test] async fn t() { std::fs::write("f", "data"); }` → Expected: not flagged — Covers: #13 idiom
4. **thread_sleep_error_severity** — Input: `async fn f() { std::thread::sleep(Duration::from_secs(1)); }` → Expected: "error" — Covers: #15 graduation
5. **metadata_info_severity** — Input: `async fn f() { std::fs::metadata("f"); }` → Expected: "info" — Covers: #2 false positive
6. **tcp_connect_detected** — Input: `async fn f() { std::net::TcpStream::connect("addr"); }` → Expected: flagged — Covers: #3 false negative
7. **spawn_blocking_wrapper_not_flagged** — Input: `async fn f() { tokio::task::spawn_blocking(|| std::fs::read("f")).await; }` → Expected: not flagged — Covers: existing but untested

---

## magic_numbers

### Current Implementation
- **File:** `src/audit/pipelines/rust/magic_numbers.rs`
- **Trait type:** Pipeline (legacy)
- **Patterns detected:** `magic_number`
- **Detection method:** Uses `compile_numeric_literal_query` to find all `integer_literal` and `float_literal` nodes. Filters out: values in `EXCLUDED_VALUES` (0, 1, 2, powers of 2, hex masks) and `COMMON_ALLOWED_NUMBERS` (3-8, 16-128, HTTP codes, ports, timeouts), exempt ancestor kinds (const_item, static_item, enum_variant, attribute_item, match_arm, range_expression, macro_invocation), index expressions (`arr[0]`), test files, and test contexts.

### Problems Identified
1. **Hardcoded thresholds without justification (Rubric #12):** The `EXCLUDED_VALUES` and `COMMON_ALLOWED_NUMBERS` lists are extensive but arbitrary. Why is `10` excluded but `20` not? Why are HTTP status codes excluded globally rather than only in HTTP-related code? The combined allowlist is ~80 values, which makes the pipeline very permissive.
2. **High false positive rate (Rubric #2):** Numbers in `assert_eq!` comparisons are flagged (since `macro_invocation` exemption only covers the macro name node, not the arguments within it). Numbers in array/vec literal initializers (e.g., `vec![0u8; 1024]`) where 1024 is in the excluded list but `vec![0u8; 2000]` would be flagged despite being a reasonable buffer size.
3. **High false negative rate (Rubric #3):** Does not detect magic strings (e.g., hardcoded URLs, file paths, regex patterns). Only numeric literals are checked.
4. **No data flow tracking (Rubric #10):** If a magic number is assigned to a well-named variable (`let timeout_ms = 5000;`), the number is still magic even though the variable provides context. Conversely, `let x = 42;` looks magic but if `42` is the answer to everything in the domain, it may be intentional.
5. **Missing context (Rubric #4):** Does not check if the number appears in a function that has many magic numbers (higher severity) vs. a function with just one (lower priority).
6. **No suppression/annotation awareness (Rubric #11):** No way to mark a number as intentionally magic (`// magic: answer to life`).
7. **Overlapping detection across pipelines (Rubric #16):** The number 10 in `LARGE_IMPL_THRESHOLD` would be in a `const_item` and thus exempt, but any pipeline-internal threshold not in a const would be flagged.
8. **No severity graduation (Rubric #15):** All findings are "info". A number appearing once is less concerning than the same magic number appearing 5 times across different functions.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** `9999` detected, `const` context skipped, common values (0, 1, 2) skipped, index expression skipped, `static` context skipped, float `3.14159` detected.
- **What's NOT tested:** Numbers inside `assert_eq!` macro arguments, numbers in vec/array size expressions, hex literals not in allowed list, negative numbers (e.g., `-1`), numbers with type suffix (e.g., `42u32`), numbers in `match` arm guards vs patterns, `COMMON_ALLOWED_NUMBERS` specific values (HTTP codes).

### Replacement Pipeline Design

**Target trait:** GraphPipeline (for file-level context only; detection remains tree-sitter)

#### Step 1: Candidate Detection
- **Tool:** Tree-sitter query
- **Why not higher-ranked tool:** Graph does not index individual numeric literals.
- **Query:** Same `compile_numeric_literal_query`. Keep current filtering but add: exempt numbers inside macro arguments (walk parent chain for `token_tree` inside `macro_invocation`), exempt numbers with type suffixes that indicate intent (e.g., `1024usize`).
- **Returns:** List of `(file_path, line, value)`.

#### Step 2: Frequency Analysis
- **Tool:** Post-processing on candidates
- **Query:** Group candidates by value. If the same magic number appears 3+ times across the codebase, escalate all instances to "warning" (strong signal it should be a constant).
- **Returns:** Frequency-annotated findings.

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query:**
  1. Check if the number is assigned to a descriptively-named variable (name contains the number's semantic meaning) — downgrade.
  2. Check for `// magic:` or `// virgil-ignore` comment on same/previous line — skip.
  3. Skip test files and test contexts.
  4. Severity: 3+ occurrences of same value → "warning", single occurrence → "info".
- **Returns:** Filtered, graduated findings.

### New Test Cases
1. **number_in_assert_not_flagged** — Input: `assert_eq!(result, 42);` → Expected: not flagged (inside macro) — Covers: #2 false positive
2. **same_number_repeated_higher_severity** — Input: `9999` appears in 3 different functions → Expected: "warning" — Covers: #15 graduation
3. **typed_suffix_number_exempt** — Input: `let buf = vec![0u8; 2000usize];` → Expected: `2000usize` not flagged (typed suffix shows intent) — Covers: #2 false positive
4. **hex_not_in_allowlist** — Input: `let mask = 0xDEADBEEF;` → Expected: flagged — Covers: #6 edge case
5. **negative_number_detected** — Input: `let threshold = -42;` → Expected: flagged (the unary_expression wrapping -42) — Covers: #6 edge case
6. **comment_suppression** — Input: `// virgil-ignore\nlet x = 9999;` → Expected: not flagged — Covers: #11 suppression

---

## Overall Cross-Pipeline Issues

### Overlapping Detection (Rubric #16)
1. **panic_detection + must_use_ignored:** If a user adds `.unwrap()` to fix a must_use_ignored finding on `.lock()`, the panic_detection pipeline now flags the `.unwrap()`. The `.lock().unwrap()` suppression in panic_detection partially addresses this but it's fragile.
2. **clone_detection + stringly_typed:** A `.to_string()` call that creates a stringly-typed field value could be flagged by both pipelines (clone_detection for the `.to_string()` call, stringly_typed for the field it populates).
3. **god_object_detection + pub_field_leakage:** A large struct with many pub fields triggers both pipelines. These findings are complementary but should cross-reference each other.

### Universal Missing Features
1. **No pipeline uses GraphPipeline trait:** All 10 pipelines use the legacy `Pipeline` trait and are wrapped via `AnyPipeline::Legacy`. None leverage the pre-built `CodeGraph`, `function_cfgs`, taint analysis, or call graph traversal.
2. **No suppression mechanism:** Zero pipelines support `// virgil-ignore`, `#[allow(...)]`, or any user-controlled suppression.
3. **No cross-file analysis:** Every pipeline operates on a single file's tree-sitter AST. None use the graph's import edges or call graph for cross-file context.
4. **Inconsistent test file handling:** panic_detection and magic_numbers skip test files; clone_detection, god_object_detection, must_use_ignored, mutex_overuse, pub_field_leakage, missing_trait_abstraction, and async_blocking do not.

### Priority Ranking for Migration
1. **clone_detection** — Highest noise, most false positives, most impactful to fix
2. **must_use_ignored** — Hardcoded method list misses most cases, needs graph
3. **async_blocking** — Missing transitive blocking detection, graph enables it
4. **panic_detection** — Mature but needs guard-awareness from CFG
5. **magic_numbers** — High noise, needs macro argument exemption
6. **god_object_detection** — Needs cross-impl-block aggregation from graph
7. **missing_trait_abstraction** — Needs usage analysis, graph callees help
8. **stringly_typed** — Needs enum existence check from graph
9. **pub_field_leakage** — Needs method presence check from graph
10. **mutex_overuse** — Smallest issue count, already has good severity graduation
