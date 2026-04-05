# TypeScript Tech Debt Pipeline Audit

## Summary
- **Total pipelines:** 11
- **Trait types used:** All 11 use the legacy `Pipeline` trait (none use `NodePipeline` or `GraphPipeline`)
- **Overall assessment:** The pipelines are competently built with clean tree-sitter queries and reasonable heuristics, but they collectively suffer from (a) zero graph utilization despite the CodeGraph being available, (b) no suppression/annotation awareness across any pipeline, (c) no scope awareness distinguishing test/generated code (except `type_assertions`), and (d) several pipelines with overlapping detection domains (`any_escape_hatch` vs `record_string_any`, `type_assertions` vs `leaking_impl_types`). Most pipelines would benefit from upgrading to `GraphPipeline` to leverage cross-file context, export/import relationships, and call graph traversal for false positive reduction.

---

## any_escape_hatch

### Current Implementation
- **File:** `src/audit/pipelines/typescript/any_escape_hatch.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `any_annotation`, `any_in_generics`, `any_return`
- **Detection method:** Tree-sitter query for `predefined_type` nodes, then filters by text == "any". Walks parent chain to classify usage context (generic type argument, return type, or plain annotation).

### Problems Identified
1. **[High false positive rate]:** Flags `any` in type-only positions like `type X = any` which may be deliberate re-export aliasing or compatibility shims. Also flags `any` inside `catch (e: any)` which is the only valid way to type a catch parameter in strict mode (lines 39-44, `text != "any"` filter is the only gate).
2. **[No suppression/annotation awareness]:** No mechanism to skip findings preceded by `// @ts-ignore`, `// @ts-expect-error`, `// eslint-disable-next-line`, or `// virgil-ignore`. A developer who intentionally uses `any` with an explanation has no way to suppress the finding.
3. **[No scope awareness]:** Does not distinguish test files from production code. `any` in test fixtures/mocks is far less concerning than `any` in production API boundaries. The `is_test_file` helper exists in primitives but is not used here.
4. **[Overlapping detection across pipelines]:** `Record<string, any>` is flagged by both this pipeline (`any_in_generics` pattern) and the `record_string_any` pipeline (`record_any` pattern). Two findings for the same line, same root cause.
5. **[No severity graduation]:** All findings are severity `"warning"`. An `any` in a function parameter annotation is less dangerous than `any` in a public API return type of an exported function. Exported-function `any` should be `warning`, while local variable `any` could be `info`.
6. **[Missing compound variants]:** Does not detect `as any` usage (separate from `as_expression` — but the user sees two different pipelines reporting the same `any` problem from different angles). Does not detect `Function` type (equivalent to `any` for function types) or `object` (less strict than `Record<string, unknown>`).
7. **[Missing context — uses tree-sitter when graph would eliminate noise]:** The graph has `Symbol.exported` which could be used to graduate severity: exported functions returning `any` are far worse than private ones. Currently, the pipeline cannot determine export status.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** Basic `any` annotation, `any` in generics, `any` return type, skips `string`, skips `unknown`, multiple `any`, TSX compilation.
- **What's NOT tested:** `any` in catch clause, `any` in type alias, `any` in conditional type (`T extends any ? X : Y`), `any` in intersection/union types, `any` in function parameter position (covered by implicit_any but not here), `any` in mapped types, suppression comments, test file behavior.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — iterate `graph.file_nodes` filtered to `.ts`/`.tsx` files
- **Why not higher-ranked tool:** Graph is the highest-ranked tool. Used directly.
- **Query:** `graph.file_nodes.keys().filter(|p| p.ends_with(".ts") || p.ends_with(".tsx"))`
- **Returns:** List of file paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — `(predefined_type) @predefined` query, filter `text == "any"`
- **Why not higher-ranked tool:** Graph nodes do not capture type annotation details at the AST level; this is inherently a syntax-level check.
- **Query:** Same `compile_predefined_type_query` as current, post-filter on text.
- **Returns:** List of `(file_path, line, column, parent_context)` tuples

#### Step 3: False Positive Removal
- **Tool:** Graph + tree-sitter combined
- **Query/Prompt:**
  1. Check `is_test_file(file_path)` — downgrade severity to `info` for test files
  2. Check if node is inside a catch clause (`catch_clause` parent) — downgrade to `info` with distinct pattern `any_in_catch`
  3. Check preceding sibling for suppression comment (`// @ts-ignore`, `// @ts-expect-error`, `// virgil-ignore`) — skip entirely
  4. Use `graph.find_symbol(file_path, line)` to check `exported` flag — if exported function returns `any`, severity = `warning`; if non-exported, severity = `info`
  5. Deduplicate with `record_string_any` by checking if the `any` node is inside a `generic_type` whose name is `Record` — if so, skip (let the specialized pipeline handle it)
- **Returns:** Filtered findings with graduated severity

#### Graph Enhancement Required
- No new graph data needed. Existing `Symbol.exported` and `file_nodes` are sufficient.

### New Test Cases
1. **any_in_catch_clause** — `try {} catch (e: any) {}` -> pattern `any_in_catch`, severity `info` — Covers: [High false positive rate]
2. **any_in_test_file** — `let x: any = mock();` in `foo.test.ts` -> severity `info` — Covers: [No scope awareness]
3. **any_with_suppression_comment** — `// @ts-expect-error\nlet x: any = 1;` -> no finding — Covers: [No suppression/annotation awareness]
4. **any_in_record_dedup** — `let x: Record<string, any> = {};` -> 0 findings (deferred to record_string_any) — Covers: [Overlapping detection across pipelines]
5. **any_in_exported_return** — `export function foo(): any {}` -> severity `warning` — Covers: [No severity graduation]
6. **any_in_private_return** — `function foo(): any {}` -> severity `info` — Covers: [No severity graduation]
7. **any_in_conditional_type** — `type X = T extends any ? Y : Z;` -> finding detected — Covers: [Missing edge cases in tests]

---

## type_assertions

### Current Implementation
- **File:** `src/audit/pipelines/typescript/type_assertions.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `double_assertion`, `as_any`, `test_type_assertion`, `type_assertion`
- **Detection method:** Tree-sitter query for `as_expression` nodes. Checks for double assertion (child is also `as_expression`), `as any` (target type text == "any"), test file context, and generic assertion. Skips inner `as_expression` when outer is double.

### Problems Identified
1. **[High false positive rate]:** Flags `as const` which is a standard TypeScript pattern for creating literal types and is explicitly recommended by the TS team. Line 49: `target_type_text` is checked but there's no exclusion for `"const"`. Every `as const` usage produces a finding.
2. **[Missing compound variants]:** Does not detect angle-bracket syntax assertions (`<string>value`) which are equivalent to `as string` in `.ts` files (disabled in `.tsx` by default but valid in `.ts`). The tree-sitter node for this is `type_assertion` (different from `as_expression`).
3. **[No suppression/annotation awareness]:** No mechanism to skip findings with `// @ts-ignore` or `// @ts-expect-error` annotations.
4. **[Overlapping detection across pipelines]:** `x as any` is flagged as both `as_any` here (severity `warning`) and as `any_annotation`/`any_in_generics` by `any_escape_hatch` (since the `any` keyword itself is a `predefined_type` node). Two findings for the same construct.
5. **[Language idiom ignorance]:** `as const` is idiomatic TypeScript for creating readonly tuple types and literal types. Flagging it as tech debt is incorrect. Similarly, `as unknown` is the safe alternative to `as any` and should not be flagged the same way.
6. **[No severity graduation]:** `as string` and `as SomeInterface` are treated identically. Widening assertions (asserting a more specific type to a broader one) are safe; narrowing assertions (asserting `unknown` to a specific type) are the actual risk.
7. **[Missing context — uses tree-sitter when graph would eliminate noise]:** Could use graph to check if the assertion is in an exported function boundary (public API assertion is worse than internal utility).

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** `as any`, generic `as string`, double assertion, test file pattern, clean code, TSX compilation.
- **What's NOT tested:** `as const` (false positive), `as unknown` (should be lower severity), angle-bracket assertion (`<string>value`), `as` in ternary expression, assertion inside generic type argument, suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node iteration
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — query both `(as_expression) @as_expr` and `(type_assertion) @angle_bracket` to catch both assertion syntaxes
- **Why not higher-ranked tool:** Graph does not model type assertion AST nodes.
- **Query:** Combined query matching both forms
- **Returns:** List of assertion nodes with target type text

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph
- **Query/Prompt:**
  1. Skip `as const` assertions entirely — idiomatic TypeScript
  2. Skip `as unknown` — this is the safe escape hatch
  3. Check preceding line for suppression comments
  4. Use `graph.find_symbol` to determine if assertion is in an exported function — graduate severity
  5. Deduplicate: if target type is `any`, do NOT also emit from `any_escape_hatch` (coordinate via pattern naming)
- **Returns:** Filtered, severity-graduated findings

#### Graph Enhancement Required
- None needed beyond existing `Symbol.exported`.

### New Test Cases
1. **skips_as_const** — `const x = [1, 2, 3] as const;` -> 0 findings — Covers: [Language idiom ignorance], [High false positive rate]
2. **skips_as_unknown** — `let x = y as unknown;` -> 0 findings or severity `info` — Covers: [Language idiom ignorance]
3. **detects_angle_bracket_assertion** — `let x = <string>y;` -> 1 finding — Covers: [Missing compound variants]
4. **suppression_comment_skips** — `// @ts-expect-error\nlet x = y as any;` -> 0 findings — Covers: [No suppression/annotation awareness]
5. **dedup_with_any_escape_hatch** — `let x = y as any;` -> only `as_any` pattern, not `any_annotation` — Covers: [Overlapping detection across pipelines]
6. **severity_graduation_exported** — `export function f() { return x as Foo; }` -> severity `warning` vs non-exported `info` — Covers: [No severity graduation]

---

## optional_everything

### Current Implementation
- **File:** `src/audit/pipelines/typescript/optional_everything.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `optional_overload`
- **Detection method:** Tree-sitter query for `interface_declaration` nodes. Iterates `property_signature` children in the interface body, counts total vs optional (has `?` token). Flags if `total >= 5` and `optional/total > 0.6`.

### Problems Identified
1. **[Hardcoded thresholds without justification]:** The threshold of `>= 5 total properties AND > 60% optional` (line 83) is arbitrary. No documentation justifies why 5 and 60% were chosen. A 4-property interface with all optional (`100%`) is not flagged. The threshold should be configurable or at least documented.
2. **[High false negative rate]:** Only checks `interface_declaration` — misses `type` aliases with optional properties in object literal types: `type Config = { a?: string; b?: number; ... }`. The tree-sitter node for this is `object_type` inside a `type_alias_declaration`.
3. **[No scope awareness]:** Does not skip test files, generated files (`.d.ts`), or third-party type declarations.
4. **[No suppression/annotation awareness]:** No way to mark an interface as intentionally having many optionals (e.g., configuration objects, builder patterns).
5. **[Language idiom ignorance]:** Configuration/options objects (e.g., `interface RequestOptions`) are idiomatically optional-heavy in TypeScript. The pipeline should check naming patterns (`*Options`, `*Config`, `*Props`, `*Settings`) and reduce severity.
6. **[Missing context — uses tree-sitter when graph would eliminate noise]:** Could use graph to check if the interface is exported — internal optional-heavy interfaces are less concerning. Could also check if the interface is used as a function parameter type (options pattern) vs standalone data model.
7. **[Single-node detection]:** Only looks at the interface body — does not check if the interface `extends` another interface that already defines required properties (making the optional properties an overlay, which is a valid pattern like `Partial<T>`).

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** Mostly-optional interface (5/6), small interface (3 props), mostly-required, boundary at exactly 60%, TSX compilation.
- **What's NOT tested:** Type alias with object type, interface extending another interface, `.d.ts` files, configuration/options naming patterns, interface with methods (not just properties), suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX, excluding `.d.ts` and test files
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node filtering
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — query both `(interface_declaration)` and `(type_alias_declaration)` with object type bodies
- **Why not higher-ranked tool:** Graph `Symbol` nodes do not contain property-level detail.
- **Query:** Two queries — existing interface query plus new `(type_alias_declaration name: (type_identifier) @name body: (object_type) @body) @decl`
- **Returns:** Interface/type names with property counts

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + heuristics
- **Query/Prompt:**
  1. Check interface name against `*Options`, `*Config`, `*Props`, `*Settings`, `*Params` — downgrade to `info` (these are idiomatically optional-heavy)
  2. Check if interface has `extends` clause — if so, optional properties may be an overlay pattern, downgrade severity
  3. Check file path for `.d.ts` — skip declaration files
  4. Check for suppression comments above the interface declaration
- **Returns:** Filtered findings with graduated severity

#### Graph Enhancement Required
- None needed.

### New Test Cases
1. **detects_optional_type_alias** — `type Config = { a?: string; b?: number; c?: boolean; d?: string; e?: number; f: string; }` -> 1 finding — Covers: [High false negative rate]
2. **skips_options_naming_pattern** — `interface RequestOptions { a?: string; ... }` -> downgraded severity — Covers: [Language idiom ignorance]
3. **skips_dts_files** — Same interface in `types.d.ts` -> 0 findings — Covers: [No scope awareness]
4. **interface_extending_base** — `interface Extended extends Base { a?: string; ... }` -> check considers overlay pattern — Covers: [Single-node detection]
5. **boundary_four_all_optional** — `interface X { a?: string; b?: number; c?: boolean; d?: string; }` -> currently 0 findings (4 < 5 threshold) — Covers: [Hardcoded thresholds without justification]
6. **suppression_skips** — `// virgil-ignore\ninterface Big { ... }` -> 0 findings — Covers: [No suppression/annotation awareness]

---

## type_duplication

### Current Implementation
- **File:** `src/audit/pipelines/typescript/type_duplication.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `duplicate_shape`
- **Detection method:** Two strategies: (1) Jaccard similarity of property name sets between all interface pairs in the same file — flags if overlap > 70% and shared >= 3. (2) Suffix pattern detection — finds interfaces sharing a base name with different suffixes (`Row`, `DTO`, `Response`, etc.).

### Problems Identified
1. **[High false negative rate — single-file only]:** The O(n^2) comparison only runs within a single file (line 99-168). Type duplication across files (e.g., `UserDTO` in `user.dto.ts` and `UserResponse` in `user.response.ts`) is completely missed. This is the most common form of type duplication in real codebases.
2. **[Missing context — uses tree-sitter when graph would eliminate noise]:** The graph's `file_entries()` and `symbols_by_name` could enable cross-file duplicate detection. The graph already indexes all symbols by name and file.
3. **[High false positive rate]:** The suffix pattern detection (lines 172-214) flags `UserRow` and `UserDTO` even if they have completely different property sets. Two interfaces sharing a base name is not necessarily duplication — it could be intentional layered architecture (entity vs DTO pattern).
4. **[Language idiom ignorance]:** The entity/DTO/response separation is a common architectural pattern in TypeScript backend codebases (NestJS, TypeORM). Flagging it as tech debt contradicts standard practice. The suffix check should at minimum also verify Jaccard similarity.
5. **[Hardcoded thresholds without justification]:** Jaccard > 0.7 and intersection >= 3 (line 111-115) are arbitrary. No justification for why 70% overlap is the threshold.
6. **[No suppression/annotation awareness]:** No way to mark intentional shape duplication.
7. **[High false negative rate — only checks interfaces]:** Misses `type` aliases with object literal types, which are equally common in TypeScript.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** High overlap detection, low overlap skipped, suffix pattern detection, single interface skipped, TSX compilation.
- **What's NOT tested:** Cross-file duplication (impossible with current design), suffix pattern with no actual overlap, type alias objects, interfaces with method signatures (not just properties), three-way duplication.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.symbols_by_name` to collect all Interface/TypeAlias symbols across all files
- **Why not higher-ranked tool:** Graph is highest-ranked and uniquely enables cross-file comparison.
- **Query:** Iterate `graph.graph` nodes where `NodeWeight::Symbol { kind: Interface | TypeAlias, .. }`, group by file
- **Returns:** Map of `file_path -> [(symbol_name, start_line, end_line)]`

#### Step 2: Narrowing
- **Tool:** Tree-sitter — for each interface/type-alias, extract property name sets from the AST body
- **Why not higher-ranked tool:** Graph `Symbol` nodes do not contain property-level detail; tree-sitter is needed for structural comparison.
- **Query:** `compile_interface_declaration_query` + new type alias query, extract property names per type
- **Returns:** Map of `(file_path, type_name) -> HashSet<property_names>`

#### Step 3: False Positive Removal
- **Tool:** Graph + heuristics
- **Query/Prompt:**
  1. For suffix pattern matches, also require Jaccard > 0.5 (lower threshold since naming already suggests relationship)
  2. Skip comparison pairs where both types are in different architectural layers (check file path for `/dto/`, `/entity/`, `/model/` — if in different layers, this may be intentional)
  3. Cross-file comparison: use `graph.symbols_by_name` to find types with similar names across files, then compare property sets
  4. Check for suppression comments
- **Returns:** Deduplicated cross-file + intra-file findings

#### Graph Enhancement Required
- **Property-level symbol data:** The graph's `Symbol` node currently only stores `name`, `kind`, `start_line`, `end_line`, `exported`. To enable cross-file property comparison without re-parsing, the graph would need to store property name sets for interface/type-alias symbols. This could be a `HashMap<NodeIndex, Vec<String>>` field on `CodeGraph` for interface property names.

### New Test Cases
1. **cross_file_duplication** — Two files with similar interfaces -> finding detected — Covers: [High false negative rate]
2. **suffix_pattern_no_overlap** — `UserRow { id }` and `UserDTO { name, email }` -> no finding (no field overlap) — Covers: [High false positive rate]
3. **type_alias_objects** — `type A = { x: string; y: number; }; type B = { x: string; y: number; z: boolean; }` -> finding detected — Covers: [High false negative rate]
4. **intentional_layer_separation** — Interfaces in `/dto/` and `/entity/` directories -> downgraded or skipped — Covers: [Language idiom ignorance]
5. **three_way_duplication** — Three interfaces with high overlap -> all pairs reported — Covers: [Missing edge cases in tests]
6. **suppression_skips** — `// virgil-ignore` above interface -> skip — Covers: [No suppression/annotation awareness]

---

## record_string_any

### Current Implementation
- **File:** `src/audit/pipelines/typescript/record_string_any.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `record_any`
- **Detection method:** Tree-sitter query for `generic_type` nodes. Filters where the type name is `Record`, then checks if any type argument is a `predefined_type` with text `"any"`.

### Problems Identified
1. **[High false negative rate]:** Only detects `Record<string, any>` — misses equivalent patterns: `{ [key: string]: any }` (index signature), `Map<string, any>`, `Record<number, any>`, `Record<string, any[]>` (nested any in array). The index signature form is syntactically different (`index_signature` node in tree-sitter) but semantically identical.
2. **[Overlapping detection across pipelines]:** The `any` in `Record<string, any>` is also detected by `any_escape_hatch` as `any_in_generics`. Users get two findings for the same line.
3. **[No suppression/annotation awareness]:** No suppression mechanism.
4. **[No scope awareness]:** Does not distinguish test files.
5. **[Literal blindness]:** Detects `Record<string, any>` but not `Record<string, never>` (useless type) or `Record<string, object>` (almost-as-bad catch-all).
6. **[No severity graduation]:** Always severity `warning`. `Record<string, any>` in a local variable is less dangerous than in an exported type alias or function parameter.
7. **[Missing context — uses tree-sitter when graph would eliminate noise]:** Could check if the Record type appears in an exported function's parameter/return type (public API surface) vs local usage.

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** `Record<string, any>`, skips `Record<string, number>`, skips `Map<string, any>`, skips `Record<string, unknown>`, TSX compilation.
- **What's NOT tested:** Index signature `{ [key: string]: any }`, `Record<number, any>`, nested `Record<string, any[]>`, `Record<any, string>` (any as key), test file behavior, suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node iteration
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — expanded query to match both `generic_type` (for `Record<...>`) and `index_signature` (for `{ [key: string]: any }`)
- **Why not higher-ranked tool:** Graph does not model type annotation AST nodes.
- **Query:** Combined query: `[(generic_type name: (type_identifier) @name type_arguments: (type_arguments) @args) @generic (index_signature) @idx_sig]`
- **Returns:** All Record-like patterns with their type arguments

#### Step 3: False Positive Removal
- **Tool:** Graph + tree-sitter
- **Query/Prompt:**
  1. Skip if in test file — downgrade to `info`
  2. Check for suppression comments
  3. Use `graph.find_symbol` to determine if the type is used in an exported boundary — graduate severity
  4. Coordinate with `any_escape_hatch` to avoid duplicate findings (the `any_escape_hatch` pipeline should skip `any` nodes inside `Record` generic arguments)
- **Returns:** Deduplicated, severity-graduated findings

#### Graph Enhancement Required
- None needed.

### New Test Cases
1. **detects_index_signature_any** — `let x: { [key: string]: any } = {};` -> 1 finding — Covers: [High false negative rate]
2. **detects_record_number_any** — `let x: Record<number, any> = {};` -> 1 finding — Covers: [High false negative rate]
3. **dedup_with_any_escape_hatch** — `let x: Record<string, any>` -> only `record_any` finding, not `any_in_generics` — Covers: [Overlapping detection across pipelines]
4. **test_file_downgrade** — `let x: Record<string, any>` in `foo.test.ts` -> severity `info` — Covers: [No scope awareness]
5. **suppression_skips** — `// virgil-ignore\nlet x: Record<string, any>` -> 0 findings — Covers: [No suppression/annotation awareness]
6. **exported_severity** — `export type Cache = Record<string, any>` -> severity `warning` — Covers: [No severity graduation]

---

## enum_usage

### Current Implementation
- **File:** `src/audit/pipelines/typescript/enum_usage.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `string_enum`, `numeric_enum`
- **Detection method:** Tree-sitter query for `enum_declaration` nodes. Iterates enum body children, checks if any `enum_assignment` child has a `string` or `template_string` value. If yes, `string_enum` (info). Otherwise, `numeric_enum` (warning).

### Problems Identified
1. **[Language idiom ignorance]:** Flags ALL enums unconditionally. Numeric enums used for bitflags (`enum Flags { Read = 1 << 0, Write = 1 << 1 }`) are idiomatic and cannot be replaced with union types. `const enum` declarations are tree-shaken and do not have the runtime cost objection — flagging them is misleading.
2. **[High false positive rate]:** Every single enum in the codebase produces a finding. In projects that deliberately use enums (e.g., following Angular conventions), this pipeline produces massive noise with 100% false positive rate for intentional usage.
3. **[No suppression/annotation awareness]:** No way to mark intentional enum usage.
4. **[No scope awareness]:** Flags enums in `.d.ts` files and test files.
5. **[Missing compound variants]:** Does not detect `const enum` which is syntactically different (has `const` keyword before `enum`). `const enum` should either be skipped entirely or have a distinct, lower-severity pattern since it compiles to inline constants.
6. **[No severity graduation]:** `numeric_enum` is always `warning`, `string_enum` is always `info`. Should consider: exported enum in public API (higher severity) vs internal enum (lower severity). Enum with single member (likely unnecessary) vs large enum (likely intentional).
7. **[Hardcoded thresholds without justification]:** N/A for thresholds, but the blanket "all enums are bad" heuristic is itself a hardcoded opinion without nuance.

### Test Coverage
- **Existing tests:** 4 tests
- **What's tested:** Numeric enum, string enum, no enum (union type), TSX compilation.
- **What's NOT tested:** `const enum`, enum with computed values, enum with mixed string/numeric values, bitflag pattern, single-member enum, exported vs non-exported enum, enum in `.d.ts` file, suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX, excluding `.d.ts`
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node iteration
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — existing `compile_enum_declaration_query`
- **Why not higher-ranked tool:** Graph `Symbol` nodes identify enums by `kind: Enum` but do not contain member details.
- **Query:** Same enum query, plus check for `const` keyword in parent
- **Returns:** Enum declarations with member details and const status

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph
- **Query/Prompt:**
  1. Skip `const enum` — they compile away and don't have the runtime cost objection
  2. Check for bitflag pattern: if any member value contains `<<`, `|`, `&` operators, skip (bitflags are idiomatic)
  3. Check `graph.find_symbol` for `exported` — graduate severity (exported enums are more concerning since they affect public API)
  4. Single-member enums — higher severity (likely unnecessary complexity)
  5. Check for suppression comments
- **Returns:** Filtered, severity-graduated findings

#### Graph Enhancement Required
- None needed.

### New Test Cases
1. **skips_const_enum** — `const enum Color { Red, Green, Blue }` -> 0 findings or distinct `info` pattern — Covers: [Language idiom ignorance]
2. **detects_bitflag_pattern** — `enum Flags { Read = 1 << 0, Write = 1 << 1 }` -> skip or `info` — Covers: [Language idiom ignorance]
3. **exported_enum_higher_severity** — `export enum Status { Active, Inactive }` -> higher severity than non-exported — Covers: [No severity graduation]
4. **single_member_enum** — `enum Singleton { Value = "VALUE" }` -> flagged — Covers: [Missing edge cases in tests]
5. **mixed_values_enum** — `enum Mixed { A = 0, B = "hello" }` -> appropriate pattern — Covers: [Missing compound variants]
6. **suppression_skips** — `// virgil-ignore\nenum Color { ... }` -> 0 findings — Covers: [No suppression/annotation awareness]
7. **dts_file_skips** — Same enum in `types.d.ts` -> 0 findings — Covers: [No scope awareness]

---

## implicit_any

### Current Implementation
- **File:** `src/audit/pipelines/typescript/implicit_any.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `implicit_any_param`
- **Detection method:** Tree-sitter query for function/arrow/method declarations. Iterates `required_parameter` and `optional_parameter` children. Skips `this` parameter, destructuring patterns, and parameters with a `type` field. Flags untyped parameters.

### Problems Identified
1. **[High false positive rate]:** Flags callback parameters in `.forEach`, `.map`, `.filter`, `.reduce` where TypeScript infers types from the array element type. Example: `[1, 2, 3].map(x => x + 1)` — `x` has no annotation but TypeScript infers it as `number`. This is idiomatic and correct TypeScript. The pipeline has no way to detect contextual typing.
2. **[No scope awareness]:** Does not skip test files where untyped parameters in test helpers are common and harmless.
3. **[No suppression/annotation awareness]:** No suppression mechanism.
4. **[Language idiom ignorance]:** Simple arrow function callbacks (`arr.sort((a, b) => a - b)`) are universally written without type annotations in TypeScript because contextual typing handles them. Flagging these is noise.
5. **[Overlapping detection across pipelines]:** An untyped parameter effectively has type `any` when `noImplicitAny` is off. This overlaps conceptually with `any_escape_hatch` — though they detect different AST patterns, the user sees "you have any problems" from multiple angles.
6. **[Missing context — uses tree-sitter when graph would eliminate noise]:** Could check if the function is a callback argument to a known typed function (e.g., `Array.prototype.map`) using call graph or parent expression context.
7. **[No severity graduation]:** All findings are `info`. An untyped parameter in an exported function is much worse than in a local callback.
8. **[High false negative rate]:** Does not detect untyped variable declarations (`let x;` with no type or initializer), untyped function return types, or untyped class properties.

### Test Coverage
- **Existing tests:** 7 tests
- **What's tested:** Untyped parameter, typed parameter, multiple untyped, mixed typed/untyped, arrow with types, arrow without types, TSX compilation.
- **What's NOT tested:** Callback parameters in `.map`/`.filter`, `this` parameter skip verification, destructuring pattern skip, default parameter values (which provide implicit typing), rest parameters, test file behavior, suppression comments, exported function severity.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX, skip test files and `.d.ts`
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node iteration
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — existing `compile_function_query`, iterate parameters
- **Why not higher-ranked tool:** Graph `Parameter` nodes exist but do not capture type annotation presence.
- **Query:** Same query, filter parameters without `type` field
- **Returns:** Untyped parameter nodes with context

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter parent context analysis
- **Query/Prompt:**
  1. Check if the function/arrow is an argument to a call expression (callback position) — if so, TypeScript contextual typing applies, downgrade to `info` or skip
  2. Check if the parameter has a default value (`value` field) — default values provide type inference, skip
  3. Use `graph.find_symbol` for `exported` — exported function untyped params = `warning`, internal = `info`
  4. Skip test files
  5. Check for suppression comments
- **Returns:** Filtered findings with graduated severity

#### Graph Enhancement Required
- **Parameter type annotation flag:** The graph's `Parameter` node has `is_taint_source` but not `has_type_annotation`. Adding this boolean would enable graph-only detection without re-parsing. However, the tree-sitter approach is sufficient for Step 2.

### New Test Cases
1. **skips_callback_parameter** — `[1,2,3].map(x => x + 1);` -> 0 findings (contextual typing) — Covers: [High false positive rate], [Language idiom ignorance]
2. **skips_default_value_param** — `function foo(x = 5) {}` -> 0 findings (type inferred from default) — Covers: [High false positive rate]
3. **test_file_skips** — Untyped params in `foo.test.ts` -> 0 findings or `info` — Covers: [No scope awareness]
4. **exported_function_severity** — `export function foo(x) {}` -> severity `warning` — Covers: [No severity graduation]
5. **rest_parameter** — `function foo(...args) {}` -> finding detected — Covers: [Missing edge cases in tests]
6. **suppression_skips** — `// @ts-expect-error\nfunction foo(x) {}` -> 0 findings — Covers: [No suppression/annotation awareness]

---

## unchecked_index_access

### Current Implementation
- **File:** `src/audit/pipelines/typescript/unchecked_index_access.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `unchecked_index`
- **Detection method:** Tree-sitter query for `subscript_expression` nodes. Skips: (1) nodes inside `if_statement` conditions, (2) nodes inside `optional_chain_expression`, (3) assignment targets (`arr[i] = value`). Flags everything else.

### Problems Identified
1. **[High false positive rate]:** Flags constant-index access on tuples (`const [a, b] = [1, 2]; const x = tuple[0];`) where TypeScript knows the type at compile time. Also flags `Map.get` result used with bracket notation, and string character access (`str[0]`).
2. **[Literal blindness]:** Does not distinguish between constant index access (`arr[0]`) and dynamic index access (`arr[i]`). Constant index access on known-length arrays is safe. The index child is available but its kind/value is not checked.
3. **[High false negative rate]:** Only checks the direct `if_statement` condition — misses other guard patterns: ternary conditions (`arr[0] ? ... : ...` outside `if`), nullish coalescing (`arr[0] ?? default`), `Array.isArray()` + length check, and `in` operator checks.
4. **[No suppression/annotation awareness]:** No suppression mechanism.
5. **[No scope awareness]:** Does not skip test files.
6. **[Missing context — uses tree-sitter when graph would eliminate noise]:** Could use graph to check if the variable being indexed was bounds-checked earlier in the same function (data flow: was `length` checked before the access?).
7. **[No data flow tracking]:** Does not check if the result of the index access is immediately checked for `undefined` (e.g., `const x = arr[0]; if (x !== undefined) { ... }` — the access on the first line is flagged but the check on the second line makes it safe).
8. **[No severity graduation]:** All findings are `info`. Dynamic index access in a loop without bounds checking is far more dangerous than `arr[0]` in controlled code.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Array index, object index, assignment target skip, if condition skip, no subscript, TSX compilation.
- **What's NOT tested:** Constant vs dynamic index, string character access, tuple access, `Map` bracket access, ternary guard, nullish coalescing guard, optional chaining skip verification, nested subscript (`arr[0][1]`), test file behavior, suppression comments.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node iteration
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — existing `compile_subscript_expression_query`
- **Why not higher-ranked tool:** Graph does not model subscript expression AST nodes.
- **Query:** Same query
- **Returns:** Subscript expression nodes with index child details

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter parent context + Graph CFG
- **Query/Prompt:**
  1. Check if index is a `number` literal — if constant index (0, 1, 2...), downgrade severity to `info`
  2. Check parent chain for ternary expression condition, nullish coalescing operator (`??`), logical AND (`&&`)
  3. Check if the subscript result is compared to `undefined` or `null` in subsequent statements (simple same-block lookahead)
  4. Use `graph.function_cfgs` to check if a bounds check (`.length` comparison) precedes the index access in the same function
  5. Skip test files, check for suppression comments
- **Returns:** Filtered findings with severity graduation

#### Graph Enhancement Required
- **CFG-based bounds check detection:** The existing per-function CFGs (`function_cfgs`) could be leveraged to trace whether a `Guard` statement checking `.length` dominates the index access point. This requires walking the CFG from the index access block backward to find a dominating guard. The CFG infrastructure exists but a dominator analysis utility would need to be added.

### New Test Cases
1. **constant_index_downgrade** — `let x = arr[0];` -> severity `info` (known constant index) — Covers: [Literal blindness]
2. **dynamic_index_no_guard** — `let x = arr[i];` -> severity `warning` — Covers: [No severity graduation]
3. **ternary_guard_skips** — `let x = arr[0] ? arr[0] : default;` -> skip (guarded) — Covers: [High false negative rate]
4. **nullish_coalescing_guard** — `let x = arr[0] ?? default;` -> skip or downgrade — Covers: [High false negative rate]
5. **string_character_access** — `let c = str[0];` -> downgrade (string indexing always returns string) — Covers: [High false positive rate]
6. **subsequent_undefined_check** — `const x = arr[0]; if (x !== undefined) { use(x); }` -> skip — Covers: [No data flow tracking]
7. **suppression_skips** — `// virgil-ignore\nlet x = arr[0];` -> 0 findings — Covers: [No suppression/annotation awareness]

---

## mutable_types

### Current Implementation
- **File:** `src/audit/pipelines/typescript/mutable_types.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `mutable_interface`
- **Detection method:** Tree-sitter query for `interface_declaration` nodes. Counts `property_signature` children, checks each for `readonly` modifier (via anonymous `readonly` token or `accessibility_modifier` node). Flags if `total > 3` and ALL properties are mutable.

### Problems Identified
1. **[High false positive rate]:** Flags ALL interfaces with >3 mutable properties, including those that are intentionally mutable (e.g., form state interfaces, mutable data models, ORM entities, React state). Most real-world interfaces are intentionally mutable.
2. **[Hardcoded thresholds without justification]:** The `> 3 properties` threshold (line 80) is arbitrary. No justification for why 3 is the cutoff. An interface with exactly 3 mutable properties is fine but 4 is flagged.
3. **[Language idiom ignorance]:** In React, component props interfaces are typically mutable (React handles immutability through its own mechanisms). ORM entity interfaces must be mutable for persistence. Flagging these as tech debt is misleading.
4. **[High false negative rate]:** Only checks `interface_declaration` — misses `type` aliases with object types, class declarations with mutable properties, and `Readonly<T>` utility type usage (which indicates the developer IS thinking about immutability for some types but not others).
5. **[No scope awareness]:** Does not distinguish test files or `.d.ts` files.
6. **[No suppression/annotation awareness]:** No suppression mechanism.
7. **[No severity graduation]:** Always `info`. An exported data model interface with all mutable properties is more concerning than an internal state interface.
8. **[Single-node detection]:** Does not check if the interface is already wrapped in `Readonly<>` at usage sites, or if a `Readonly` version exists (`ReadonlyUser` alongside `User`).

### Test Coverage
- **Existing tests:** 5 tests
- **What's tested:** All mutable (4 props), one readonly property skips, small interface (2 props) skips, boundary at exactly 3 props, TSX compilation.
- **What's NOT tested:** Type alias objects, class properties, `Readonly<T>` usage elsewhere in the file, React props/state patterns, ORM entity patterns, test file behavior, suppression comments, exported vs non-exported.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — `graph.file_nodes` filtered to TS/TSX, excluding `.d.ts` and test files
- **Why not higher-ranked tool:** Graph is highest-ranked.
- **Query:** File node iteration
- **Returns:** File paths

#### Step 2: Narrowing
- **Tool:** Tree-sitter — existing `compile_interface_declaration_query` + check for `readonly` on each property
- **Why not higher-ranked tool:** Graph does not model property-level `readonly` modifiers.
- **Query:** Same query, count mutable/readonly properties
- **Returns:** Interfaces with all-mutable property sets

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter + Graph + heuristics
- **Query/Prompt:**
  1. Check interface name against `*Props`, `*State`, `*Entity`, `*Model`, `*Input`, `*Form` — these are idiomatically mutable, skip
  2. Check if a `Readonly<InterfaceName>` usage exists elsewhere in the file (search for `Readonly` generic with the interface name) — if so, developer is already handling immutability, skip
  3. Use `graph.find_symbol` for `exported` — exported interfaces = `info`, non-exported = skip (internal types are less concerning)
  4. Check for suppression comments
- **Returns:** Filtered findings

#### Graph Enhancement Required
- None needed.

### New Test Cases
1. **skips_props_pattern** — `interface ButtonProps { label: string; onClick: () => void; disabled: boolean; size: string; }` -> 0 findings — Covers: [Language idiom ignorance]
2. **skips_entity_pattern** — `interface UserEntity { id: string; name: string; email: string; role: string; }` -> 0 findings — Covers: [Language idiom ignorance]
3. **detects_type_alias_mutable** — `type Config = { a: string; b: number; c: boolean; d: string; }` -> 1 finding — Covers: [High false negative rate]
4. **skips_readonly_usage_elsewhere** — Interface + `Readonly<InterfaceName>` in same file -> 0 findings — Covers: [Single-node detection]
5. **test_file_skips** — Same interface in `foo.test.ts` -> 0 findings — Covers: [No scope awareness]
6. **suppression_skips** — `// virgil-ignore\ninterface Foo { ... }` -> 0 findings — Covers: [No suppression/annotation awareness]

---

## unconstrained_generics

### Current Implementation
- **File:** `src/audit/pipelines/typescript/unconstrained_generics.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `unconstrained_generic`
- **Detection method:** Tree-sitter query for `type_parameter` nodes. Checks for `constraint` field (the `extends` clause). Skips class/interface-level generics (only flags function/method-level). Uses parent chain walk to determine if the generic is in a function/method context.

### Problems Identified
1. **[High false positive rate]:** Flags `function identity<T>(x: T): T` which is the canonical TypeScript generic function. Unconstrained generics are frequently intentional and correct — they represent true polymorphism. The mere absence of `extends` is not a bug.
2. **[Language idiom ignorance]:** Generic utility functions (`identity`, `pipe`, `compose`, `map`, `filter`, `reduce`), factory functions (`createInstance<T>`), and container types are all idiomatically unconstrained. Flagging them as tech debt contradicts TypeScript best practices.
3. **[No suppression/annotation awareness]:** No suppression mechanism.
4. **[No scope awareness]:** Does not skip test files, `.d.ts` files, or utility/helper files.
5. **[No severity graduation]:** All findings are `info`. A generic with multiple unconstrained type parameters is more concerning than a single `<T>`. Also, unused type parameters (declared but never referenced in the signature) are the actual problem — not unconstrained ones.
6. **[Missing context — uses tree-sitter when graph would eliminate noise]:** Could use graph to check how many callers use the generic function — if it's called with many different types, unconstrained is correct. If always called with the same type, the generic is unnecessary (different problem).
7. **[Broken detection (borderline)]:** The `is_in_function_or_method` check (lines 85-98) stops at `class_declaration` or `interface_declaration`, which means a generic method inside a generic class `class Box<T> { get<U>() {} }` correctly detects `U` but not `T`. However, the function also stops at `function_declaration` which means nested generics in function bodies could be missed if the parent chain hits a class before a function.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Unconstrained function generic, constrained generic skip, class-level generic skip, method generic, arrow function generic, TSX compilation.
- **What's NOT tested:** Multiple type parameters, generic inside generic class, function expression generic, generic with default type (`<T = string>`), unused type parameter, suppression comments, test file behavior.

### Replacement Pipeline Design
**Target trait:** NodePipeline (justification: this check is inherently per-function — it does not need cross-file or graph data)

#### Step 1: File Identification
- **Tool:** Tree-sitter file parsing (NodePipeline runs per-file)
- **Why not higher-ranked tool:** This is a per-function syntactic check. Graph data does not help determine if a type parameter should be constrained.
- **Query:** Standard file iteration
- **Returns:** Per-file AST

#### Step 2: Narrowing
- **Tool:** Tree-sitter — existing `compile_type_parameter_query`, filter to function/method context
- **Why not higher-ranked tool:** Constraint presence is purely syntactic.
- **Query:** Same query with parent context filtering
- **Returns:** Unconstrained type parameters in function/method signatures

#### Step 3: False Positive Removal
- **Tool:** Tree-sitter
- **Query/Prompt:**
  1. Skip type parameters with default types (`<T = string>`) — these provide a fallback and are less concerning
  2. Check if the type parameter is actually used in the function signature (referenced in parameters or return type) — unused type parameters are a different, more severe problem
  3. If the function has a single type parameter used in both parameter and return type (`<T>(x: T): T`), this is the identity pattern — downgrade to `info` or skip
  4. Skip test files and `.d.ts` files
  5. Check for suppression comments
- **Returns:** Filtered findings

#### Graph Enhancement Required
- None needed. This is inherently a per-function check.

### New Test Cases
1. **skips_generic_with_default** — `function foo<T = string>(x: T): T {}` -> 0 findings — Covers: [High false positive rate]
2. **detects_unused_type_param** — `function foo<T>(x: number): number {}` (T never used) -> higher severity — Covers: [No severity graduation]
3. **multiple_unconstrained** — `function foo<T, U, V>(x: T, y: U): V {}` -> 3 findings or severity escalation — Covers: [No severity graduation]
4. **identity_pattern_skips** — `function identity<T>(x: T): T { return x; }` -> skip or lowest severity — Covers: [Language idiom ignorance]
5. **test_file_skips** — Same in `foo.test.ts` -> 0 findings — Covers: [No scope awareness]
6. **suppression_skips** — `// virgil-ignore\nfunction foo<T>() {}` -> 0 findings — Covers: [No suppression/annotation awareness]

---

## leaking_impl_types

### Current Implementation
- **File:** `src/audit/pipelines/typescript/leaking_impl_types.rs`
- **Trait type:** Legacy `Pipeline`
- **Patterns detected:** `leaking_orm_type`
- **Detection method:** Tree-sitter query for function declarations. Checks if function is exported (parent is `export_statement`). Extracts return type text and checks if it contains any string from a hardcoded list of 18 ORM type names (`PrismaClient`, `Repository`, `EntityManager`, etc.). Uses substring match.

### Problems Identified
1. **[High false positive rate]:** Uses substring matching (`return_type_text.contains(pattern)`, line 78). A return type like `ConnectionConfig` or `PoolOptions` would match `Connection` and `Pool` respectively. The type name `Model` is extremely generic — any return type containing "Model" (e.g., `ViewModel`, `DataModel`, `ModelResult`) triggers a false positive.
2. **[High false negative rate]:** Only checks `function_declaration` — misses arrow functions (`export const getDB = (): PrismaClient => ...`), method definitions in exported classes, and variable declarations with type annotations. The `compile_function_query` matches all three, but `is_exported` (line 101-108) only checks for direct parent `export_statement`, which doesn't work for arrow functions (where the parent chain is `variable_declarator -> lexical_declaration -> export_statement`).
3. **[Hardcoded thresholds without justification]:** The 18 ORM type names (lines 13-32) are hardcoded. There's no way for users to add their own implementation types (e.g., `RedisClient`, `KafkaProducer`, `ElasticSearchClient`).
4. **[Missing context — uses tree-sitter when graph would eliminate noise]:** The graph's `Symbol.exported` flag already computes export status correctly for all symbol types. Using the graph would fix the arrow function export detection bug. Additionally, the graph's import edges could verify that the matched type name actually comes from an ORM library (not a user-defined `Model` type).
5. **[No suppression/annotation awareness]:** No suppression mechanism.
6. **[No scope awareness]:** Does not skip test files (exporting test helpers that return ORM types is common and not a concern).
7. **[Language idiom ignorance]:** In repository/service patterns, the repository layer is expected to expose ORM types. Only the controller/API layer should abstract them away. The pipeline cannot distinguish architectural layers.
8. **[Overlapping detection across pipelines]:** If the return type is `any` (e.g., `export function getDB(): any`), both this pipeline (no match, since "any" is not in ORM_PATTERNS) and `any_escape_hatch` would fire, but on different aspects. If the return type is `Record<string, any>`, `record_string_any` fires separately.

### Test Coverage
- **Existing tests:** 6 tests
- **What's tested:** Leaking PrismaClient, leaking Repository generic, non-exported skip, safe return type, no return type, TSX compilation.
- **What's NOT tested:** Arrow function export (bug — will miss it), substring false positive (e.g., `ConnectionConfig`), `Model` false positive, class method export, test file behavior, suppression comments, re-exported functions.

### Replacement Pipeline Design
**Target trait:** GraphPipeline

#### Step 1: File Identification
- **Tool:** Graph — iterate `Symbol` nodes where `exported == true` and `kind` is Function/ArrowFunction/Method
- **Why not higher-ranked tool:** Graph is highest-ranked. This directly leverages `Symbol.exported` to find all exported functions regardless of syntax.
- **Query:** `graph.graph.node_indices().filter(|idx| matches!(graph.graph[idx], NodeWeight::Symbol { exported: true, kind: Function | ArrowFunction | Method, .. }))`
- **Returns:** List of exported function symbol nodes with file paths and line numbers

#### Step 2: Narrowing
- **Tool:** Tree-sitter — for each exported function, extract the return type annotation text
- **Why not higher-ranked tool:** Graph `Symbol` does not store return type information.
- **Query:** Parse the function node at the known line, extract `return_type` field text
- **Returns:** `(file_path, line, return_type_text)` tuples

#### Step 3: False Positive Removal
- **Tool:** Graph import analysis + word-boundary matching
- **Query/Prompt:**
  1. Use word-boundary matching instead of substring: check if the ORM type name appears as a complete identifier (not as substring). Regex: `\bPrismaClient\b`, `\bRepository\b`, etc.
  2. Use `graph.file_dependency_edges()` to check if the file imports from known ORM libraries (`@prisma/client`, `typeorm`, `sequelize`, `knex`). Only flag if the leaking type actually comes from an ORM import.
  3. Skip test files
  4. Check for suppression comments
  5. Check file path for repository/service layer patterns (`/repository/`, `/dal/`, `/persistence/`) — downgrade severity for expected ORM-facing layers
- **Returns:** Filtered findings with contextual severity

#### Graph Enhancement Required
- **Import source tracking on Symbol nodes:** To determine if a return type like `Repository` comes from `typeorm` vs a user-defined type, the graph would need to resolve type references to their import sources. Currently, `Imports` edges connect files, not types. Adding per-symbol import resolution would enable precise ORM type identification. In the interim, checking file-level imports for ORM library presence is a reasonable heuristic.

### New Test Cases
1. **detects_arrow_function_export** — `export const getDB = (): PrismaClient => prisma;` -> 1 finding — Covers: [High false negative rate] (currently broken)
2. **no_substring_false_positive** — `export function getConfig(): ConnectionConfig {}` -> 0 findings — Covers: [High false positive rate]
3. **no_model_false_positive** — `export function getModel(): ViewModel {}` -> 0 findings — Covers: [High false positive rate]
4. **test_file_skips** — `export function getDB(): PrismaClient {}` in `foo.test.ts` -> 0 findings — Covers: [No scope awareness]
5. **suppression_skips** — `// virgil-ignore\nexport function getDB(): PrismaClient {}` -> 0 findings — Covers: [No suppression/annotation awareness]
6. **orm_import_verification** — File with `import { Repository } from "typeorm"` + leaking return type -> finding. File without typeorm import + `Repository` return type -> no finding — Covers: [Missing context]
7. **class_method_export** — `export class UserService { getRepo(): Repository<User> {} }` -> 1 finding — Covers: [High false negative rate]

---

## Cross-Pipeline Issues

### Overlapping Detection Matrix

| Code Pattern | Pipelines That Fire | Duplicate? |
|---|---|---|
| `let x: any = 1;` | any_escape_hatch | No |
| `let x = y as any;` | type_assertions (`as_any`) + any_escape_hatch (`any_annotation` or `any_in_generics`) | YES |
| `Record<string, any>` | record_string_any (`record_any`) + any_escape_hatch (`any_in_generics`) | YES |
| `function foo(x) {}` | implicit_any (`implicit_any_param`) | No |
| `enum Color { Red }` | enum_usage (`numeric_enum`) | No |
| All interfaces >3 mutable props | mutable_types (`mutable_interface`) | No |
| Mostly-optional interfaces | optional_everything (`optional_overload`) + mutable_types (if also all mutable) | PARTIAL |

### Systemic Issues (All Pipelines)

1. **No suppression/annotation awareness:** Zero out of 11 pipelines check for `// @ts-ignore`, `// @ts-expect-error`, `// eslint-disable`, `// virgil-ignore`, or JSDoc `@suppress` annotations. This is the single most impactful gap across the entire suite.

2. **No scope awareness (except type_assertions):** Only `type_assertions` uses `is_test_file()`. The remaining 10 pipelines treat test code identically to production code, generating significant noise in real-world audits.

3. **All use Legacy Pipeline trait:** All 11 pipelines implement the legacy `Pipeline` trait, meaning they receive only `(tree, source, file_path)` — no access to the CodeGraph. This prevents cross-file analysis and graph-based false positive reduction.

4. **No `.d.ts` awareness:** No pipeline skips TypeScript declaration files, which are third-party type definitions where findings are not actionable.

5. **No export-aware severity graduation:** The graph's `Symbol.exported` flag is available but unused. Every pipeline could benefit from treating exported (public API) findings as more severe than internal ones.
