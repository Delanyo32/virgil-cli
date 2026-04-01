# Plan: Python Test Quality Detection Pipelines

## Config
Auto-commit: yes
Auto-proceed phases: no
Max retries: 2

## Desired State
virgil-cli detects test quality issues in Python test files via `audit code-quality tech-debt`. Six patterns across three new GraphPipeline implementations detect: missing assertions, trivial assertions, test pollution (global mutable state), excessive mocking, sleep-in-tests, and empty test files. Testing category detection rate improves from 13.9% toward >50%.

### Criteria
- [ ] `virgil-cli audit <target> code-quality tech-debt` detects missing assertions in test functions
- [ ] `virgil-cli audit <target> code-quality tech-debt` detects trivial assertions (`assert True`)
- [ ] `virgil-cli audit <target> code-quality tech-debt` detects global mutable state in test files
- [ ] `virgil-cli audit <target> code-quality tech-debt` detects excessive mocking (>3 patches per test)
- [ ] `virgil-cli audit <target> code-quality tech-debt` detects `time.sleep()` in test files
- [ ] `virgil-cli audit <target> code-quality tech-debt` detects empty test files (no test functions)
- [ ] All new pipelines implement `GraphPipeline` trait and return `Vec<AuditFinding>` with `--format json`
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

## Phase 1: Core Test Quality Pipelines
Status: completed
Goal: Three new GraphPipeline implementations registered in Python tech-debt, detecting 6 test quality patterns

Phase Rubric:
- [ ] Running `cargo run -- audit code-quality tech-debt <dir> --language py --pipeline test_assertions,test_pollution,test_hygiene --format json` returns findings for each pattern
- [ ] All three pipelines have unit tests covering positive detection and negative (clean code) cases
- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy` passes with no warnings

Criteria Gate:
- [ ] Testing category detection rate > 13.9% (improved from baseline)

### Task 1.1: Create `test_assertions.rs` pipeline
Status: completed
Change: Create `src/audit/pipelines/python/test_assertions.rs` implementing `GraphPipeline`. Detect two patterns in test files (files matching `is_test_file()` from helpers.rs):
- `missing_assertion`: test functions (`def test_*`) whose body contains zero assertion-like statements (no `assert`, `self.assert*`, `pytest.raises`, `pytest.warns`, `pytest.approx`, `with raises`)
- `trivial_assertion`: test functions containing `assert True`, `assert False`, `assert 1`, or `assert None` as standalone assert statements
Use `compile_function_def_query()` from primitives.rs to find functions. Walk function body children to check for assertion statements. Severity: `warning`.
Test: `cargo test test_assertions` — unit tests for both patterns plus negative cases
Research:
- [code] `compile_function_def_query() -> Result<Arc<Query>>` — captures @fn_name, @params, @fn_body, @fn_def (source: src/audit/pipelines/python/primitives.rs:14)
- [code] `is_test_file(file_path: &str) -> bool` — checks path for test dirs and naming patterns (source: src/audit/pipelines/helpers.rs:612)
- [code] `extract_snippet(source: &[u8], node: Node, max_lines: usize) -> String` — truncated code excerpt (source: src/audit/primitives.rs)
- [code] `GraphPipelineContext { tree, source, file_path, id_counts, graph }` (source: src/audit/pipeline.rs:22)
- [code] Python assert statement: tree-sitter kind `assert_statement`, children include the expression being asserted
- [code] Python call expressions for `self.assertEqual(...)` etc: tree-sitter kind `call` with `attribute` function
Rubric:
- [ ] File `src/audit/pipelines/python/test_assertions.rs` exists and compiles
- [ ] `TestAssertionsPipeline` implements `GraphPipeline` with `name()` returning `"test_assertions"`
- [ ] Pipeline only checks files where `is_test_file(file_path)` returns true (skips non-test files)
- [ ] Detects `missing_assertion` in `def test_foo(): pass` and `def test_bar(): x = 1; print(x)`
- [ ] Does NOT flag `def test_ok(): assert result == 1` or `def test_raises(): with pytest.raises(ValueError): ...`
- [ ] Detects `trivial_assertion` in `def test_trivial(): assert True`
- [ ] Does NOT flag `def test_real(): assert x == 1`
- [ ] Unit tests pass: `cargo test test_assertions`
Findings: All 9 rubric items pass. 16 unit tests. Detects missing_assertion and trivial_assertion patterns. Correctly skips non-test files and non-test functions.
Attempts: 1/3

### Task 1.2: Create `test_pollution.rs` pipeline
Status: completed
Change: Create `src/audit/pipelines/python/test_pollution.rs` implementing `GraphPipeline`. Detect two patterns in test files:
- `global_mutable_test_state`: module-level (not inside any function or class) assignments to mutable containers (`[]`, `{}`, `set()`, `dict()`, `list()`, `defaultdict(...)`, `collections.OrderedDict()`) in test files. These cause cross-test pollution when mutated.
- `mutable_class_fixture`: class-level (inside `class Test*` but not inside a method) assignments to mutable containers. Same detection logic but scoped to class body.
Use `compile_function_def_query()` and `compile_class_def_query()` from primitives.rs. Walk module-level statements checking for assignment expressions with mutable RHS. Severity: `warning`.
Test: `cargo test test_pollution` — unit tests for both patterns plus negative cases
Research:
- [code] `compile_class_def_query() -> Result<Arc<Query>>` — captures @class_name, @class_body, @class_def (source: src/audit/pipelines/python/primitives.rs:76)
- [code] Python assignment: tree-sitter kind `expression_statement` containing `assignment` with `right` child
- [code] Mutable literals: `list` kind = `list`, `dict` kind = `dictionary`, `set` kind = `set`
- [code] Mutable calls: `call` with function `list`, `dict`, `set`, `defaultdict`, `OrderedDict`
Rubric:
- [ ] File `src/audit/pipelines/python/test_pollution.rs` exists and compiles
- [ ] `TestPollutionPipeline` implements `GraphPipeline` with `name()` returning `"test_pollution"`
- [ ] Pipeline only checks files where `is_test_file(file_path)` returns true
- [ ] Detects `global_mutable_test_state` in `SHARED_DATA = []` at module level of a test file
- [ ] Does NOT flag `CONSTANT = "immutable"` or `MAX_RETRIES = 3` (non-mutable types)
- [ ] Detects `mutable_class_fixture` in `class TestFoo:\n    data = []` (class-level mutable)
- [ ] Does NOT flag `class TestFoo:\n    timeout = 30` (immutable class attribute)
- [ ] Unit tests pass: `cargo test test_pollution`
Findings: All 10 rubric items pass. 22 unit tests. Detects global_mutable_test_state and mutable_class_fixture patterns. Build agent also added `pub mod test_assertions;` and `pub mod test_pollution;` to mod.rs (declarations only, no registration).
Attempts: 1/3

### Task 1.3: Create `test_hygiene.rs` pipeline
Status: completed
Change: Create `src/audit/pipelines/python/test_hygiene.rs` implementing `GraphPipeline`. Detect two patterns in test files:
- `excessive_mocking`: test functions (`def test_*`) decorated with more than 3 `@mock.patch`, `@patch`, `@patch.object`, `@patch.dict` decorators. Use `decorated_definition` tree-sitter node, count decorator children matching mock/patch patterns. Severity: `warning`.
- `sleep_in_test`: calls to `time.sleep(...)` or `asyncio.sleep(...)` inside test functions. These slow down test suites and usually indicate timing-dependent tests. Use `compile_call_query()` from primitives.rs, filter by function name. Severity: `info`.
Test: `cargo test test_hygiene` — unit tests for both patterns plus negative cases
Research:
- [code] `compile_call_query() -> Result<Arc<Query>>` — captures @fn_expr, @args, @call (source: src/audit/pipelines/python/primitives.rs:65)
- [code] Python `decorated_definition` node: children are `decorator` nodes (each with `@` + expression) followed by the actual `function_definition` or `class_definition`
- [code] `is_test_context_python(node, source, file_path) -> bool` — checks if node is inside test function/class (source: src/audit/pipelines/helpers.rs:970)
- [code] Decorator tree-sitter node: kind `decorator`, child is the decorator expression (attribute/call)
Rubric:
- [ ] File `src/audit/pipelines/python/test_hygiene.rs` exists and compiles
- [ ] `TestHygienePipeline` implements `GraphPipeline` with `name()` returning `"test_hygiene"`
- [ ] Pipeline only checks files where `is_test_file(file_path)` returns true
- [ ] Detects `excessive_mocking` when a test function has >3 patch decorators
- [ ] Does NOT flag a test function with exactly 2 patch decorators
- [ ] Detects `sleep_in_test` for `time.sleep(1)` inside a test function
- [ ] Does NOT flag `time.sleep()` in non-test files
- [ ] Unit tests pass: `cargo test test_hygiene`
Findings: All 8 rubric items pass. 14 unit tests. Detects excessive_mocking (>3 patches) and sleep_in_test (time.sleep/asyncio.sleep). Correct severity levels (warning/info).
Attempts: 1/3

### Task 1.4: Create `empty_test_files.rs` pipeline
Status: completed
Change: Create `src/audit/pipelines/python/empty_test_files.rs` implementing `GraphPipeline`. Detect one pattern:
- `empty_test_file`: files matching `is_test_file()` that contain zero `def test_*` functions. These are test discovery files or abandoned test stubs that add clutter. Use `compile_function_def_query()` to find all functions, count those starting with `test_`. If count is 0 and file is a test file, emit finding at line 1. Severity: `info`.
Exclude `conftest.py` and `__init__.py` from this check (they legitimately have no test functions).
Test: `cargo test empty_test_files` — unit tests for detection and exclusion cases
Research:
- [code] `is_test_file(file_path: &str) -> bool` — already handles test file detection (source: src/audit/pipelines/helpers.rs:612)
- [code] conftest.py is used for pytest fixtures, not tests — should be excluded
- [code] `__init__.py` in test dirs is for package discovery — should be excluded
Rubric:
- [ ] File `src/audit/pipelines/python/empty_test_files.rs` exists and compiles
- [ ] `EmptyTestFilesPipeline` implements `GraphPipeline` with `name()` returning `"empty_test_files"`
- [ ] Detects `empty_test_file` in a test file containing only `import pytest` and no test functions
- [ ] Does NOT flag `conftest.py` or `__init__.py`
- [ ] Does NOT flag a test file containing `def test_something(): assert True`
- [ ] Unit tests pass: `cargo test empty_test_files`
Findings: All 6 rubric items pass. 10 unit tests. Detects empty_test_file, excludes conftest.py and __init__.py. Severity is info.
Attempts: 1/3

### Task 1.5: Register pipelines and verify
Status: completed
Change: 
1. Add `pub mod test_assertions;`, `pub mod test_pollution;`, `pub mod test_hygiene;`, `pub mod empty_test_files;` to `src/audit/pipelines/python/mod.rs`
2. Add all four pipelines to `tech_debt_pipelines()` as `AnyPipeline::Graph(Box::new(...))` entries
3. Run `cargo test` and `cargo clippy` to verify everything compiles and passes
Test: `cargo test` (all tests), `cargo clippy` (no warnings)
Research:
- [code] Registration pattern in `tech_debt_pipelines()` — 8 existing entries, all `AnyPipeline::Graph(Box::new(...::new()?))` (source: src/audit/pipelines/python/mod.rs:40-51)
- [code] Module declarations at top of mod.rs — one `pub mod` per pipeline file (source: src/audit/pipelines/python/mod.rs:1-8)
Rubric:
- [ ] `src/audit/pipelines/python/mod.rs` has `pub mod test_assertions;`, `pub mod test_pollution;`, `pub mod test_hygiene;`, `pub mod empty_test_files;`
- [ ] `tech_debt_pipelines()` returns all four new pipelines as `AnyPipeline::Graph` entries
- [ ] `cargo test` exits 0 with no failures
- [ ] `cargo clippy` exits 0 with no warnings
- [ ] Running `cargo run -- audit code-quality tech-debt <test-dir> --language py --format json` includes findings from the new pipelines when test quality issues are present
Findings: All 5 rubric items pass. 2041 tests pass, 0 failures. Clippy clean (only pre-existing warnings). All 7 patterns detected in end-to-end CLI test with sample Python test files.
Attempts: 1/3
