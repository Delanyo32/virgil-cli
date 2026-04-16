---
phase: 04-security-per-language-scalability-migration
reviewed: 2026-04-16T00:00:00Z
depth: standard
files_reviewed: 86
files_reviewed_list:
  - src/audit/builtin/c_buffer_overflow_security_c.json
  - src/audit/builtin/c_command_injection_c.json
  - src/audit/builtin/c_integer_overflow_c.json
  - src/audit/builtin/c_memory_mismanagement_c.json
  - src/audit/builtin/c_path_traversal_c.json
  - src/audit/builtin/c_toctou_c.json
  - src/audit/builtin/c_uninitialized_memory_c.json
  - src/audit/builtin/c_weak_randomness_c.json
  - src/audit/builtin/code_injection_javascript.json
  - src/audit/builtin/code_injection_python.json
  - src/audit/builtin/command_injection_csharp.json
  - src/audit/builtin/command_injection_go.json
  - src/audit/builtin/command_injection_java.json
  - src/audit/builtin/command_injection_javascript.json
  - src/audit/builtin/command_injection_php.json
  - src/audit/builtin/command_injection_python.json
  - src/audit/builtin/cpp_buffer_overflow_cpp.json
  - src/audit/builtin/cpp_exception_safety_cpp.json
  - src/audit/builtin/cpp_injection_cpp.json
  - src/audit/builtin/cpp_integer_overflow_cpp.json
  - src/audit/builtin/cpp_memory_mismanagement_cpp.json
  - src/audit/builtin/cpp_path_traversal_cpp.json
  - src/audit/builtin/cpp_race_conditions_cpp.json
  - src/audit/builtin/cpp_type_confusion_cpp.json
  - src/audit/builtin/cpp_weak_randomness_cpp.json
  - src/audit/builtin/csharp_path_traversal_csharp.json
  - src/audit/builtin/csharp_race_conditions_csharp.json
  - src/audit/builtin/format_string_c.json
  - src/audit/builtin/go_integer_overflow_go.json
  - src/audit/builtin/go_path_traversal_go.json
  - src/audit/builtin/go_type_confusion_go.json
  - src/audit/builtin/insecure_deserialization_csharp.json
  - src/audit/builtin/insecure_deserialization_java.json
  - src/audit/builtin/insecure_deserialization_javascript.json
  - src/audit/builtin/insecure_deserialization_php.json
  - src/audit/builtin/insecure_deserialization_python.json
  - src/audit/builtin/integer_overflow_rust.json
  - src/audit/builtin/java_path_traversal_java.json
  - src/audit/builtin/java_race_conditions_java.json
  - src/audit/builtin/memory_leak_indicators_c.json
  - src/audit/builtin/memory_leak_indicators_cpp.json
  - src/audit/builtin/memory_leak_indicators_csharp.json
  - src/audit/builtin/memory_leak_indicators_go.json
  - src/audit/builtin/memory_leak_indicators_java.json
  - src/audit/builtin/memory_leak_indicators_javascript.json
  - src/audit/builtin/memory_leak_indicators_php.json
  - src/audit/builtin/memory_leak_indicators_python.json
  - src/audit/builtin/memory_leak_indicators_rust.json
  - src/audit/builtin/memory_leak_indicators_typescript.json
  - src/audit/builtin/panic_dos_rust.json
  - src/audit/builtin/path_traversal_javascript.json
  - src/audit/builtin/path_traversal_python.json
  - src/audit/builtin/path_traversal_rust.json
  - src/audit/builtin/prototype_pollution_javascript.json
  - src/audit/builtin/race_conditions_go.json
  - src/audit/builtin/race_conditions_rust.json
  - src/audit/builtin/redos_resource_exhaustion_javascript.json
  - src/audit/builtin/reflection_injection_java.json
  - src/audit/builtin/reflection_unsafe_csharp.json
  - src/audit/builtin/resource_exhaustion_go.json
  - src/audit/builtin/resource_exhaustion_python.json
  - src/audit/builtin/resource_exhaustion_rust.json
  - src/audit/builtin/session_auth_php.json
  - src/audit/builtin/timing_weak_crypto_javascript.json
  - src/audit/builtin/toctou_rust.json
  - src/audit/builtin/type_confusion_rust.json
  - src/audit/builtin/type_juggling_php.json
  - src/audit/builtin/type_system_bypass_typescript.json
  - src/audit/builtin/unescaped_output_php.json
  - src/audit/builtin/unsafe_include_php.json
  - src/audit/builtin/unsafe_memory_rust.json
  - src/audit/builtin/unsafe_type_assertions_security_typescript.json
  - src/audit/builtin/weak_cryptography_csharp.json
  - src/audit/builtin/weak_cryptography_java.json
  - src/audit/builtin/xxe_format_string_python.json
  - src/audit/pipelines/c/mod.rs
  - src/audit/pipelines/cpp/mod.rs
  - src/audit/pipelines/csharp/mod.rs
  - src/audit/pipelines/go/mod.rs
  - src/audit/pipelines/java/mod.rs
  - src/audit/pipelines/javascript/mod.rs
  - src/audit/pipelines/php/mod.rs
  - src/audit/pipelines/python/mod.rs
  - src/audit/pipelines/rust/mod.rs
  - src/audit/pipelines/typescript/mod.rs
  - tests/audit_json_integration.rs
findings:
  critical: 2
  warning: 6
  info: 5
  total: 13
status: issues_found
---

# Phase 4: Code Review Report

**Reviewed:** 2026-04-16T00:00:00Z
**Depth:** standard
**Files Reviewed:** 86
**Status:** issues_found

## Summary

This phase migrates security and scalability audit pipelines for C, C++, C#, Go, Java, JavaScript/TypeScript, PHP, Python, and Rust from Rust trait implementations to the JSON-driven audit engine. The JSON definitions are embedded at compile time via `include_dir!` and the engine dispatches them alongside the Rust pipelines.

The implementation is architecturally consistent and the integration tests provide solid coverage across all nine language families. However, the review uncovered two critical issues: (1) the `security_pipelines()` function in every Rust language `mod.rs` returns an empty `Vec`, meaning the 75+ new JSON security pipelines are served exclusively through the JSON engine path and the Rust `security_pipelines()` stub never participates -- this is correct by design, but several `mod.rs` files still have non-empty Rust security pipelines that now coexist with JSON duplicates for csharp, go, java, javascript, and php, creating a risk of double-reporting; and (2) several JSON pipelines for C and C++ use the exact same `match_pattern` (`call_expression` for all C security rules, `new_expression` for four independent C++ rules), making every pattern in a file functionally identical -- the audit findings are indistinguishable from one another since they differ only in the `message` field but fire on the same AST node.

Key structural issue: the C security pipeline family (`c_buffer_overflow_security_c.json`, `c_command_injection_c.json`, `c_path_traversal_c.json`, `c_toctou_c.json`, `c_uninitialized_memory_c.json`, `c_weak_randomness_c.json`, and `format_string_c.json`) all use an identical `match_pattern`. This means every C function call will produce seven separate findings across seven pipelines with different pattern names but all triggered from the same AST node. This creates a systematic false-positive flood for any C codebase.

## Critical Issues

### CR-01: C and C++ security pipelines use identical match_pattern, causing systematic finding collisions

**File:** `src/audit/builtin/c_buffer_overflow_security_c.json:8`, `src/audit/builtin/c_command_injection_c.json:8`, `src/audit/builtin/c_path_traversal_c.json:8`, `src/audit/builtin/c_toctou_c.json:8`, `src/audit/builtin/c_uninitialized_memory_c.json:8`, `src/audit/builtin/c_weak_randomness_c.json:8`, `src/audit/builtin/format_string_c.json:8`

**Issue:** All seven C security pipelines share the identical `match_pattern`:
```
"(call_expression function: (identifier) @fn_name arguments: (argument_list) @args) @call"
```
Every C function call will trigger all seven pipelines simultaneously. A single `malloc()` call will produce `buffer_overflow_risk`, `command_injection_call`, `path_traversal_risk`, `toctou_check`, `uninitialized_memory`, `weak_randomness`, and `format_string_vulnerability` findings -- all from the same source line. In addition, four C++ pipelines (`cpp_buffer_overflow_cpp.json`, `cpp_injection_cpp.json`, `cpp_path_traversal_cpp.json`, `cpp_weak_randomness_cpp.json`) share the identical `call_expression` pattern, and four others (`cpp_integer_overflow_cpp.json`, `cpp_memory_mismanagement_cpp.json`, `cpp_type_confusion_cpp.json`, `cpp_exception_safety_cpp.json`) share the identical `new_expression` pattern. The result is a massive false-positive flood that degrades signal quality to the point that security audits on any C/C++ codebase become unusable.

**Fix:** The D-07 decision to simplify is acknowledged in descriptions, but the multiple-pipeline-one-pattern approach defeats the purpose of distinct pipelines. Either (a) merge all C security checks into a single `c_security` pipeline with one `flag` stage whose message enumerates all risks, or (b) accept the current design but document it prominently with a warning in descriptions and ensure the test suite verifies that findings can be disambiguated by pipeline name. Option (a) example:
```json
{
  "pipeline": "c_security",
  "category": "security",
  "languages": ["c"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (identifier) @fn_name arguments: (argument_list) @args) @call"
    },
    {
      "flag": {
        "pattern": "unsafe_function_call",
        "message": "Verify: no gets/strcpy/strcat/sprintf (buffer overflow), no system/popen with dynamic args (command injection), no fopen with unvalidated paths (path traversal), no access/stat before open (TOCTOU), no uninitialized malloc buffers sent to network (data disclosure), no rand()/strcmp on secrets (weak randomness), no printf with variable format (format string).",
        "severity": "error"
      }
    }
  ]
}
```

### CR-02: Several language mod.rs files still contain non-empty Rust security pipelines that overlap with new JSON pipelines, risking double-reporting

**File:** `src/audit/pipelines/csharp/mod.rs:56-62`, `src/audit/pipelines/go/mod.rs:51-56`, `src/audit/pipelines/java/mod.rs:54-60`, `src/audit/pipelines/javascript/mod.rs:56-61`, `src/audit/pipelines/php/mod.rs:44-49`

**Issue:** The following language `security_pipelines()` functions return non-empty Rust pipelines:
- C#: `sql_injection`, `xxe`, `csharp_ssrf`
- Go: `sql_injection`, `ssrf_open_redirect`
- Java: `sql_injection`, `xxe`, `java_ssrf`
- JavaScript/TypeScript: `xss_dom_injection`, `ssrf`
- PHP: `sql_injection`, `ssrf`

Meanwhile, new JSON files have been added for these same languages covering overlapping security topics (e.g., `command_injection_csharp.json`, `insecure_deserialization_java.json`, `command_injection_php.json`). The audit engine (`engine.rs`) runs both Rust pipelines and JSON pipelines in the same pass. If any of the Rust pipeline names match a JSON pipeline name with the same language, the JSON dedup logic in `json_audit.rs` (`dedup_key`) uses `pipeline_name:language` as the key. However, the new JSON pipelines have unique names (e.g., `command_injection`, `insecure_deserialization`) that do NOT conflict with the existing Rust pipelines (`sql_injection`, `xxe`), so there is no actual deduplication conflict for this phase's additions. This is not a current regression but is a latent risk: if future JSON files are added for `sql_injection` or `xxe` for these languages, the dedup logic may silently suppress either the Rust or JSON version without a compile-time warning.

**Fix:** Add a comment to each non-empty `security_pipelines()` function listing which pipeline names are still owned by Rust to alert future contributors:
```rust
pub fn security_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    // NOTE: sql_injection, xxe, csharp_ssrf are Rust-owned.
    // Do NOT add JSON builtins with pipeline names matching these
    // for the csharp language -- the JSON engine dedup would silently
    // suppress one of them.
    Ok(vec![
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        ...
    ])
}
```

## Warnings

### WR-01: `unsafe_type_assertions_security` TypeScript pipeline is a functional duplicate of `type_system_bypass`

**File:** `src/audit/builtin/unsafe_type_assertions_security_typescript.json:7`, `src/audit/builtin/type_system_bypass_typescript.json:7`

**Issue:** Both pipelines use the identical match_pattern `(as_expression) @as_expr` and both target `["typescript", "tsx"]`. They differ only in their pipeline name and flag message. Every TypeScript `as` expression will produce two findings: one for `type_system_bypass` and one for `unsafe_type_assertion`. The dedup logic in `json_audit.rs` only deduplicates on `pipeline_name:language`, not on match_pattern identity, so both will run and both will fire on every `as_expression` node.
```
// type_system_bypass_typescript.json line 7
"match_pattern": "(as_expression) @as_expr"

// unsafe_type_assertions_security_typescript.json line 7
"match_pattern": "(as_expression) @as_expr"
```

**Fix:** Either remove `unsafe_type_assertions_security_typescript.json` and fold its specific security message into `type_system_bypass_typescript.json`, or distinguish the two by using different match_patterns (e.g., target `as any` specifically for the security variant if the tree-sitter grammar supports it).

### WR-02: `resource_exhaustion` (Python) and `xxe_format_string` (Python) share the identical match_pattern, causing double-firing on every attribute call

**File:** `src/audit/builtin/resource_exhaustion_python.json:8`, `src/audit/builtin/xxe_format_string_python.json:8`, `src/audit/builtin/command_injection_python.json:8`, `src/audit/builtin/insecure_deserialization_python.json:8`, `src/audit/builtin/path_traversal_python.json:8`

**Issue:** All five Python security pipelines that use attribute calls share the identical pattern:
```
"(call function: (attribute object: (identifier) @obj attribute: (identifier) @method) @call)"
```
Every `obj.method()` call in Python code fires all five pipelines simultaneously. The descriptions acknowledge individual simplifications per D-07, but the compounding effect is that a single `pickle.loads(data)` call generates five simultaneous findings: `insecure_deserialization`, `command_injection_call`, `unvalidated_path_join`, `redos_pattern`, and `xxe_format_string`. This degrades audit signal quality for Python codebases.

**Fix:** Consolidate all five attribute-call Python security pipelines into a single pipeline with a combined message, matching the approach used for C (or accept the trade-off and document it clearly).

### WR-03: `code_injection` (Python) and `memory_leak_indicators` (Python) share the identical match_pattern for direct identifier calls

**File:** `src/audit/builtin/code_injection_python.json:7`, `src/audit/builtin/memory_leak_indicators_python.json:7`

**Issue:** Both use `(call function: (identifier) @fn)`. Every direct Python function call -- including `print()`, `len()`, `range()` -- fires both `code_injection_call` (severity: error) and `potential_memory_leak` (severity: warning). The two pipelines belong to different categories (`security` vs `scalability`) and this cross-category collision means a simple `print()` call raises an error-severity security finding.

**Fix:** Either add a filter stage (if the JSON engine supports `filter` to narrow by function name) or document that these pipelines are designed as broad signal generators whose findings must be triaged by pipeline name, not treated as precise matches.

### WR-04: `memory_leak_indicators` (JavaScript) vs `code_injection` (JavaScript) share the identical direct-call match_pattern under different languages array values

**File:** `src/audit/builtin/memory_leak_indicators_javascript.json:7`, `src/audit/builtin/code_injection_javascript.json:7`

**Issue:** `memory_leak_indicators_javascript.json` uses `languages: ["javascript", "jsx"]` and `code_injection_javascript.json` uses `languages: ["javascript", "jsx", "typescript", "tsx"]`. Both have the identical pattern `(call_expression function: (identifier) @fn)`. For `.js` and `.jsx` files, both fire on every direct function call. A `setInterval()` call produces both `potential_memory_leak` (warning) and `code_injection_call` (error). The severity mismatch is particularly confusing: `setInterval` would be flagged as a code injection error.

**Fix:** Remove `code_injection_javascript.json`'s `javascript` and `jsx` entries from the languages list (it should only target TypeScript/TSX where `eval` concerns are higher), or adjust to differentiate patterns.

### WR-05: `c_memory_mismanagement` uses `warning` severity for potential double-free and use-after-free, but these are memory corruption bugs deserving `error`

**File:** `src/audit/builtin/c_memory_mismanagement_c.json:14`

**Issue:** Double-free and use-after-free vulnerabilities in C are exploitable memory corruption bugs (CVE-class issues) that can lead to arbitrary code execution. The pipeline assigns `severity: "warning"` while the analogous `c_buffer_overflow_security_c.json` (which detects `gets`, `strcpy` etc.) uses `severity: "error"`. The inconsistency means a double-free -- which is at least as severe as a `strcpy` -- is surfaced at lower priority.

**Fix:**
```json
"flag": {
  "pattern": "memory_mismanagement",
  "severity": "error"
}
```

### WR-06: `unsafe_include` PHP pipeline uses an alternative tree-sitter group syntax that may not be supported by the JSON audit executor

**File:** `src/audit/builtin/unsafe_include_php.json:8`

**Issue:** The `match_pattern` uses a tree-sitter alternation group syntax:
```
"[(include_expression) (require_expression)] @include_expr"
```
All other JSON pipelines in this codebase use single-node patterns. The `GraphStage` executor in `graph/pipeline.rs` may or may not support the tree-sitter alternation `[...]` group syntax in the JSON `match_pattern` field. If the executor compiles this as a raw tree-sitter query string, tree-sitter does support alternation groups and this will work. But if the executor wraps it in an enclosing expression or applies special processing, the brackets may cause a parse error that silently skips the pipeline. This is a latent breakage risk with no test coverage for PHP `include`/`require` detection in the integration tests.

**Fix:** Add an integration test for `unsafe_include` PHP that verifies `include("$_GET['file']")` is flagged, and verify the tree-sitter alternation syntax parses correctly. If the executor does not support `[...]` syntax, split into two pipelines (`unsafe_include_php` and `unsafe_require_php`) each with a single-node pattern.

## Info

### IN-01: `redos_resource_exhaustion` JavaScript pipeline name mismatch with other ReDoS pipelines

**File:** `src/audit/builtin/redos_resource_exhaustion_javascript.json:2`

**Issue:** The pipeline name is `redos_resource_exhaustion` but the Python equivalent is named `resource_exhaustion`. The CLI `--pipeline` flag and API responses will expose different names for what is functionally the same check across languages. Users filtering by `--pipeline resource_exhaustion` will miss the JavaScript version, and vice versa.

**Fix:** Rename to `resource_exhaustion` with `languages: ["javascript", "jsx", "typescript", "tsx"]` to match the Python and Go counterparts. Verify no existing tests or CLI references hardcode `redos_resource_exhaustion`.

### IN-02: Missing integration tests for C, C++, C#, PHP, and Java security/scalability pipelines from this phase

**File:** `tests/audit_json_integration.rs:584`

**Issue:** The integration test file covers Rust, Go, JavaScript, TypeScript, Python, and Java pipelines from this phase, but has zero test coverage for:
- Any C pipeline (c_buffer_overflow_security, c_command_injection, c_toctou, etc.)
- Any C++ pipeline (cpp_buffer_overflow, cpp_race_conditions, etc.)
- Any C# pipeline (csharp_path_traversal, csharp_race_conditions, insecure_deserialization_csharp, etc.)
- PHP pipelines (session_auth, unescaped_output, unsafe_include, type_juggling, etc.)
- `memory_leak_indicators` for C#, Java, PHP, TypeScript

**Fix:** Add at minimum one positive and one negative test for each uncovered language/pipeline pair, following the existing test structure.

### IN-03: `c_integer_overflow` uses `binary_expression` which matches comparison operators, string concatenation and boolean ops in addition to arithmetic

**File:** `src/audit/builtin/c_integer_overflow_c.json:8`

**Issue:** The pattern `(binary_expression left: (_) @left right: (_) @right) @expr` matches all binary operators in C including `==`, `!=`, `&&`, `||`, `|`, `&`, `<<`, `>>`. The message says to verify arithmetic expressions used as `malloc` size arguments, but bitwise and comparison expressions unrelated to allocation sizes will fire the `unchecked_arithmetic` warning. While the description acknowledges D-07 imprecision, the pattern could be narrowed to arithmetic-only operators if the JSON engine supports a filter by operator type.

**Fix:** Document in the description that comparison operators are included, or add a note to the message: "Note: this fires on all binary expressions including non-arithmetic operators; focus review on expressions used as allocation size arguments."

### IN-04: `csharp_race_conditions` uses `variable_declarator` in its pattern which may not match the actual C# grammar node name

**File:** `src/audit/builtin/csharp_race_conditions_csharp.json:8`

**Issue:** The match_pattern is:
```
"(field_declaration (variable_declaration (variable_declarator (identifier) @field_name))) @field_decl"
```
The C# tree-sitter grammar (`tree-sitter-c-sharp`) uses `variable_declaration` directly under `field_declaration`, but the intermediate `variable_declarator` node may not be a named child in the actual grammar -- it could be a `variable_declarator` or an anonymous node. If this pattern fails to compile (tree-sitter query error), the pipeline silently skips per the engine's `eprintln!` + continue behavior. There are no integration tests for this pipeline to catch a silent failure.

**Fix:** Verify the pattern against the actual C# grammar (`tree-sitter-c-sharp` grammar file) and add an integration test that checks `csharp_race_conditions` fires for a class with a `Dictionary<K,V>` field.

### IN-05: `java_race_conditions` field_declaration pattern includes `declarator:` named field which may not exist in Java's tree-sitter grammar

**File:** `src/audit/builtin/java_race_conditions_java.json:8`

**Issue:** The match_pattern is:
```
"(field_declaration declarator: (variable_declarator name: (identifier) @name)) @field"
```
Java's tree-sitter grammar may use a different field name than `declarator:` for the child of `field_declaration`. If the field name does not match, the pattern silently matches nothing. This is particularly risky because the integration tests for Java cover `command_injection`, `weak_cryptography`, `insecure_deserialization`, `java_path_traversal`, and `reflection_injection` -- but NOT `java_race_conditions`. A silent pattern failure would go undetected.

**Fix:** Verify the field name against `tree-sitter-java` grammar and add an integration test:
```rust
#[test]
fn java_race_conditions_java_finds_field() {
    // class with HashMap field should trigger thread_unsafe_collection
    let content = "class Service { private java.util.HashMap<String, String> cache = new java.util.HashMap<>(); }";
    // ...
}
```

---

_Reviewed: 2026-04-16T00:00:00Z_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
