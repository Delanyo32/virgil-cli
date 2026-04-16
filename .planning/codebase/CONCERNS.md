# Codebase Concerns

**Analysis Date:** 2026-04-16

## Tech Debt

**Name-Based Call Graph Resolution (Heuristic Limitation):**
- Issue: Call graph traversal relies on name-based symbol lookup via `symbols_by_name` HashMap instead of type-aware resolution. This is a heuristic approach that can incorrectly match function calls across different scopes or modules.
- Files: `src/query_engine.rs` (lines 544-611, `traverse_via_graph`), `src/graph/mod.rs` (lines 105-147, `traverse_callees`/`traverse_callers`)
- Impact: False positives in `--calls` query results when multiple symbols have the same name across different files or modules. Query results may include unrelated functions.
- Fix approach: Implement type-aware call resolution by tracking scope information during symbol extraction. Store function signatures and type information in the CodeGraph. Consider adding a scope index to disambiguate symbols with identical names.

**File Discovery Does Not Implement Streaming or Chunking:**
- Issue: `Workspace::load()` loads ALL files into memory at once via rayon parallel iteration. For very large codebases (10,000+ files), this may cause OOM.
- Files: `src/workspace.rs` (lines 20-76, `load`), `src/s3.rs` (lines 200-286, `download_objects`)
- Impact: Large projects (500MB+ total size) may exhaust available memory. S3 codebase loading loads all files concurrently without memory budgeting.
- Fix approach: Implement chunked loading with a configurable memory budget. Add support for lazy file loading on-demand per query. For S3, implement progressive download with batch sizing limits.

**Taint Analysis Source/Sink Tables Are Hardcoded:**
- Issue: Taint source/sink/sanitizer patterns are static arrays hardcoded in `src/graph/taint.rs`. Adding new patterns requires code recompilation.
- Files: `src/graph/taint.rs` (lines 17-160+, SOURCES/SINKS/SANITIZERS const arrays)
- Impact: Security patterns cannot be updated without a new release. Difficult to customize detection for specific projects.
- Fix approach: Load taint patterns from JSON config files (project-local → user-global → built-in, similar to audit discovery). Add CLI flag to specify custom pattern files.

**Audit Pipeline Name Collision Resolution is First-Match:**
- Issue: When multiple JSON audit files declare the same pipeline name, only the first match wins via `discover_json_audits()`. Silently ignores subsequent definitions without user awareness.
- Files: `src/audit/json_audit.rs` (lines 55-80, `discover_json_audits`)
- Impact: User may expect custom audit to override built-in, but has no feedback that it's ignored. Leads to confusion and incorrect results.
- Fix approach: Warn on pipeline name collisions in discovery. Provide explicit priority levels (--audit-priority project,user,builtin). Allow users to disable specific built-in audits.

**Parser Stack Size Reduced to 4MB Without Safety Margin:**
- Issue: Rayon thread pool configured with 4MB stack size (reduced from 16MB). Deep AST recursion on complex files may overflow.
- Files: `src/audit/engine.rs` (line 153), `src/graph/builder.rs` (line 80)
- Impact: Stack overflow panic on deeply nested code (e.g., nested object literals in JavaScript). No graceful error handling.
- Fix approach: Monitor stack usage during AST traversal. Add stack depth checks and warn before overflow risk. Revert to 8MB as safety baseline.

**Graph Builder Uses Per-File Parallelism Without Scalability Bounds:**
- Issue: `GraphBuilder::build()` uses rayon with no limit on concurrent file parsing. With 1000+ files, memory usage can spike unpredictably.
- Files: `src/graph/builder.rs` (lines 78-130)
- Impact: OOM on very large projects. Unbounded parallelism defeats memory budgeting.
- Fix approach: Implement bounded thread pool (e.g., 8-16 concurrent parsers). Track memory usage per thread. Add progress reporting for long-running builds.

## Known Bugs

**S3 Download Does Not Handle Non-UTF-8 Files Gracefully:**
- Symptoms: Binary or non-UTF-8 files in S3 bucket are skipped with stderr warning but silently excluded from codebase. Users may not realize files were dropped.
- Files: `src/s3.rs` (lines 250-257, `download_objects`)
- Trigger: S3 bucket contains any non-text files (compiled objects, images, binaries)
- Workaround: Pre-filter S3 bucket contents before querying. Use exclude patterns to skip binary directories.

**Call Graph Traversal Does Not Handle Recursive Functions:**
- Symptoms: `traverse_callees()` and `traverse_callers()` BFS may visit the same node multiple times if there are cycles (recursive calls). Though `visited` set prevents infinite loops, it may not report all transitive calls correctly.
- Files: `src/graph/mod.rs` (lines 106-147, `traverse_calls`)
- Trigger: Query any recursive function with `--calls down`
- Workaround: Inspect results manually; cycles are detected but traversal may be incomplete.

**Server Mode Timeout is Global 120-Second Hard Limit:**
- Symptoms: Any query or audit taking >120 seconds returns HTTP 504 with no partial results. Complex audits on large codebases will fail.
- Files: `src/server.rs` (line 26, REQUEST_TIMEOUT)
- Trigger: Run audit on 10,000+ file codebase via HTTP server
- Workaround: Increase timeout manually in code. No CLI flag to adjust per-request.

**Circular Dependency Detection is File-Level Only:**
- Symptoms: `circular_dependencies` audit reports file-level cycles but misses module-level or symbol-level cycles (circular function calls).
- Files: `src/audit/pipelines/*/` circular_dependencies.rs files
- Trigger: Code with mutually recursive functions in same file
- Workaround: Manual inspection of call graph results.

## Security Considerations

**S3 Credentials Are Passed Via Environment Variables:**
- Risk: `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` in plaintext environment vars. If process memory is dumped or logged, credentials leak.
- Files: `src/s3.rs` (lines 56-62, credential loading)
- Current mitigation: Standard AWS SDK environment variable handling. No additional encryption or obfuscation.
- Recommendations: 
  - Support AWS IAM role assumption (for EC2/ECS/Lambda deployments)
  - Clear credentials from memory after client initialization
  - Add warning if credentials are passed in plaintext
  - Support `.aws/credentials` file-based auth as safer alternative

**Taint Analysis Does Not Cover All Source/Sink Patterns:**
- Risk: Custom security sinks (e.g., specific logging functions, custom validators) are not detected. Attackers can use undocumented patterns that bypass checks.
- Files: `src/graph/taint.rs` (SOURCES/SINKS/SANITIZERS arrays)
- Current mitigation: Hardcoded patterns for common OWASP sources (request.body, process.env, etc.)
- Recommendations:
  - Allow user-defined source/sink patterns via JSON config
  - Implement semantic versioning of pattern databases
  - Document how to add custom patterns

**No Validation of Graph Pipeline Inputs:**
- Risk: Graph pipelines accept arbitrary node selections and edge traversals. Malformed JSON pipelines could cause DoS (infinite loops, memory exhaustion).
- Files: `src/graph/executor.rs`, `src/graph/pipeline.rs`
- Current mitigation: Basic BFS/DFS depth limits in traversal
- Recommendations:
  - Validate pipeline DAG (directed acyclic graph structure)
  - Add execution timeout per pipeline stage
  - Limit result set size before returning to client

## Performance Bottlenecks

**Large File Parsing Is Single-Threaded Per File:**
- Problem: Each file is parsed sequentially by one rayon thread. Parsing large files (>100KB) dominates runtime.
- Files: `src/audit/engine.rs` (lines 160-181, per-file parsing loop), `src/graph/builder.rs` (lines 140-180)
- Cause: Tree-sitter `Parser` is not `Sync`, must be created per-thread. No opportunity for intra-file parallelism.
- Improvement path: Profile parsing times. Consider caching parsed ASTs across multiple audits. Use memoization for repeated symbol extraction on same file.

**Query Engine Rebuilds Name Matcher for Each File:**
- Problem: `compile_name_matcher()` is called once per query but builds new regex/globset per file in filter loop.
- Files: `src/query_engine.rs` (lines 473-497, `compile_name_matcher`; lines 152-175, filter application)
- Cause: Matchers are created lazily but not cached across file iterations.
- Improvement path: Pre-compile all matchers before file loop. Store in `struct PerQueryState`.

**Graph Traversal Uses Linear Search for Symbol Lookup:**
- Problem: `traverse_via_graph` calls `graph.find_symbol()` for each seed QueryResult, which does HashMap lookup for each seed. With 1000+ symbols, this is inefficient.
- Files: `src/query_engine.rs` (lines 552-555), `src/graph/mod.rs` (lines 149-154)
- Cause: No index for (file_path, start_line) → NodeIndex reverse lookup beyond HashMap.
- Improvement path: The HashMap `symbol_nodes` already optimizes this, so the current approach is O(1). No action needed, but verify no repeated hash collisions on large graphs.

**Circular Dependency Detection Uses Proxy File Approach:**
- Problem: Each file is checked independently for circular dependencies instead of full graph traversal. Scales O(n²) for n files.
- Files: `src/audit/pipelines/*/circular_dependencies.rs` files
- Cause: Pipeline::check() trait operates on single files. Cross-file analysis deferred to future engine-level pass.
- Improvement path: Implement true SCC (strongly connected components) detection on CodeGraph once file-level API evolves. Precompute SCCs at graph build time.

## Fragile Areas

**Language Detection via File Extension Only:**
- Files: `src/language.rs` (extension mapping)
- Why fragile: Files with wrong extension (.ts file named .js, .h file that's actually C++) will be parsed incorrectly. No magic number or content-based detection.
- Safe modification: Add optional content-based language detection as fallback (check shebang, file headers). Document extension requirements clearly.
- Test coverage: Unit tests for `Language::from_extension()` exist but no integration tests for misnamed files.

**Import Classification Relies on String Prefix Matching:**
- Files: `src/languages/typescript.rs`, `src/languages/python.rs`, etc. (import classification logic)
- Why fragile: Relative imports detected via `startswith("./")`, `startswith("..")`. Complex module paths (Node.js subpath imports, package aliases) may be misclassified.
- Safe modification: Add unit tests for edge cases (package.json exports, TypeScript paths, Python namespace packages). Document classification rules.
- Test coverage: Limited integration testing of import classification. Main test is query engine functional test.

**Signature Extraction Stops at First `{` Without Validation:**
- Files: `src/signature.rs` (signature extraction)
- Why fragile: For JavaScript arrow functions, constructors with comments in signature, or template literals, extraction may be incorrect.
- Safe modification: Add parser-based validation to confirm extracted text is valid syntax. Add tests for multi-line signatures and edge cases.
- Test coverage: Basic tests exist; no tests for template literals, JSDoc spanning signature, or malformed code.

**CodeGraph Node Indices Are Unstable Across Builds:**
- Files: `src/graph/builder.rs` (node creation order)
- Why fragile: `NodeIndex` is assigned sequentially during graph construction. File discover order or language ordering changes will reassign all indices, breaking any hardcoded references.
- Safe modification: Document that NodeIndex is not stable. Add API to lookup nodes by (file_path, start_line) instead of relying on index.
- Test coverage: No tests for graph stability across builds.

**Audit Category to Pipeline Mapping is Implicit:**
- Files: `src/audit/pipeline.rs` (category selector functions), `src/audit/engine.rs` (PipelineSelector enum)
- Why fragile: Adding new audit category requires changes in multiple places (enum, dispatcher, CLI, help text). Easy to forget and cause inconsistency.
- Safe modification: Create centralized audit category registry (HashMap<String, Vec<Pipeline>>). Use data-driven approach.
- Test coverage: No tests for category completeness or missing implementations.

**S3 Workspace Uses Synthetic Root Path:**
- Files: `src/workspace.rs` (load_from_s3), `src/s3.rs`, `src/query_engine.rs` (execute_read)
- Why fragile: S3 workspace root is `s3://bucket/prefix`, not a real filesystem path. `workspace.root()` returns non-existent path. Disk fallback in `execute_read` guards with `root.exists()` but other code paths may not.
- Safe modification: Create separate `WorkspaceSource` enum (Local(PathBuf), S3(S3Location)). Explicitly handle S3 vs local in code paths that access root.
- Test coverage: Limited testing of S3 workspace edge cases. No tests for S3 + read query, S3 + execute_read fallback.

## Scaling Limits

**All Files Loaded into Memory Upfront:**
- Current capacity: ~10,000 files of average 10KB = 100MB+ memory for file contents alone
- Limit: Typical system has 2-8GB available. Hits memory ceiling around 50,000 files or 5GB total codebase.
- Scaling path: Implement lazy loading or memory-mapped files. Use disk-based index for symbol lookups. Consider SQLite backend for large projects.

**Parser Creation Cost is O(n) for n Threads:**
- Current capacity: Rayon uses system CPU count threads (typically 8-16). Creating parser per thread is fast (<1ms each).
- Limit: Scaling to 1000+ concurrent threads would create 1000 parsers (parser memory ~50KB each = 50MB overhead).
- Scaling path: Parser pool with caching. Reuse parsers across files within same thread.

**CodeGraph Node Count is Unbounded:**
- Current capacity: ~100 symbols per file × 10,000 files = 1,000,000 nodes (petgraph handles this)
- Limit: Symbol lookup via HashMap scales fine, but graph traversal (BFS for call graphs) is O(n+e) where e is edges. With ~3x edges, very expensive on million-node graphs.
- Scaling path: Implement lazy traversal with pagination. Cache computed transitive closures. Use approximate algorithms for large call graphs.

**S3 Pagination Loads All Keys into Memory:**
- Current capacity: List up to 1,000 keys per request, paginate automatically
- Limit: Buckets with millions of objects will accumulate all keys in `Vec<String>` before downloading (millions of 1-500 byte strings = significant memory)
- Scaling path: Stream S3 pagination. Download files as they're listed, not after full enumeration.

## Dependencies at Risk

**tree-sitter 0.25 Streaming Iterator API:**
- Risk: `QueryMatches` uses `streaming_iterator::StreamingIterator` instead of standard `std::iter::Iterator`. This is a lower-level API that may be removed in future versions.
- Impact: All language modules (`src/languages/*.rs`) depend on this API. A breaking change would require rewriting all symbol extraction.
- Migration plan: Monitor tree-sitter releases. If streaming API changes, switch to tree-sitter `Query::captures()` API (standard iterator, slower but stable).

**aws-sdk-s3 1.x Blocking API:**
- Risk: Uses `tokio::task::block_on()` to run async S3 operations in sync context. This is brittle and not designed for long-term use.
- Impact: S3 support may break if tokio runtime is already active (e.g., called from async context).
- Migration plan: Refactor S3 operations to be async throughout. Add async version of `Workspace::load_from_s3()`. Update server mode and CLI to support async paths.

**petgraph 0.7 DiGraph is Not Sync:**
- Risk: CodeGraph uses `DiGraph<NodeWeight, EdgeWeight>` which is not `Sync` (requires `Arc<Mutex<>>` for thread safety). Current code shares via `Arc<CodeGraph>` for read-only access.
- Impact: If graph becomes mutable during queries, data races are possible. Current design is safe but constrains future evolution.
- Migration plan: Document that graph is immutable after construction. Add compile-time enforcement via newtypes. Consider switching to thread-safe graph library if mutation is needed.

## Missing Critical Features

**No Support for Language-Specific Type Information:**
- Problem: Call graph uses name-based resolution only. Cannot distinguish overloaded functions or type-polymorphic calls (generics, interfaces).
- Blocks: Accurate cross-module call graph, type-safe refactoring tools, IDE-like code navigation.

**No Caching of Parsed ASTs Between Queries:**
- Problem: Every query parses all matching files again (no persistent index). For repeated queries or audits, this is wasteful.
- Blocks: Server mode performance. Interactive CLI tools. Large batch jobs.

**No Support for Project Configuration Files:**
- Problem: Language filters, exclusion patterns, and audit rules must be specified via CLI. No `.virgilrc` or similar config file.
- Blocks: Team-based consistency. CI/CD integration without complex shell scripts.

**No Incremental/Delta Analysis:**
- Problem: Every query analyzes entire workspace. No support for "only changed files" or "since commit X".
- Blocks: Git hook integration. Fast CI check feedback.

## Test Coverage Gaps

**S3 Module Has No End-to-End Tests:**
- What's not tested: S3 listing with large result sets, download failure recovery, non-UTF-8 file handling, custom endpoints (R2, MinIO)
- Files: `src/s3.rs` (test module exists but only checks parsing logic)
- Risk: S3 feature may be broken without detection until user reports
- Priority: Medium - S3 is new and critical for server mode

**Graph Builder Parallel Execution Has Race Condition Potential:**
- What's not tested: Rayon thread pool with many files, memory pressure during parallel parsing
- Files: `src/graph/builder.rs` (no parallel stress tests)
- Risk: Deadlock or memory corruption in production at scale
- Priority: High - affects all audit/query operations

**Audit Engine JSON Pipeline Discovery Has No Coverage:**
- What's not tested: Project-local audit overrides, priority conflicts, malformed JSON handling
- Files: `src/audit/json_audit.rs` (unit tests for parsing, no integration tests)
- Risk: Custom audits silently fail or conflict with built-ins
- Priority: Medium - feature is recent, edge cases not exercised

**Server Mode Request Timeout Has No Tests:**
- What's not tested: 120-second timeout behavior, partial response on timeout, concurrent request handling
- Files: `src/server.rs` (no timeout tests)
- Risk: Timeout edge cases cause hung requests or inconsistent state
- Priority: Low - timeout behavior is standard, but integration test would be valuable

**Language Export Detection Has No Cross-Language Comparison Tests:**
- What's not tested: Whether export detection is consistent across languages (public in C# vs export in Rust vs uppercase in Go)
- Files: `src/languages/` directory (unit tests per language, no cross-language tests)
- Risk: Inconsistent behavior confuses users, makes cross-language audits unreliable
- Priority: Medium - affects audit accuracy

---

*Concerns audit: 2026-04-16*
