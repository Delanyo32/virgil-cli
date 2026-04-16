# Testing Patterns

**Analysis Date:** 2026-04-16

## Test Framework

**Runner:**
- Cargo test (built-in Rust test framework)
- No external test runner needed
- Config: Uses default test configuration in `Cargo.toml` (no special `[test]` sections)

**Assertion Library:**
- `assert!()`, `assert_eq!()`, `assert_ne!()` from standard library
- No custom assertion macros

**Run Commands:**
```bash
cargo test                    # Run all tests
cargo test --lib             # Run library tests only
cargo test --test integration_test  # Run specific integration test
cargo test -- --nocapture    # Show println! output during tests
cargo test -- --test-threads=1  # Run sequentially
```

## Test File Organization

**Location:**
- Unit tests co-located with implementation: `#[cfg(test)] mod tests { }` blocks at end of each module
- Integration tests in separate `tests/` directory: `tests/integration_test.rs`
- Fixture files in `tests/fixtures/` (sample source files: `sample.ts`, `imports_sample.js`, `component.tsx`, etc.)

**Naming:**
- Test functions use descriptive names: `full_pipeline_typescript()`, `parse_file_metadata()`, `discover_single_language()`
- Fixture files match language: `sample.ts`, `sample.js`, `imports_sample.ts`, `imports_sample.js`, `component.tsx`
- Test modules use consistent suffix: `#[cfg(test)] mod tests { }` in every file

**Structure:**
```
src/
├── parser.rs
│   └── #[cfg(test)] mod tests { }
├── registry.rs
│   └── #[cfg(test)] mod tests { }
├── query_lang.rs
│   └── #[cfg(test)] mod tests { }
└── ...
tests/
├── integration_test.rs       # Multi-language parsing pipeline tests
└── fixtures/
    ├── sample.ts
    ├── sample.js
    ├── imports_sample.ts
    ├── imports_sample.js
    ├── component.tsx
    ├── component.jsx
    └── empty.ts
```

## Test Structure

**Suite Organization:**
- Each module has one `#[cfg(test)] mod tests { }` block
- Shared test utilities defined at top of `tests` block
- Helper functions extracted for repeated patterns

**Example from integration_test.rs:**
```rust
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Parse a single fixture file and return (metadata, symbols, imports).
fn parse_fixture_full(
    filename: &str,
    language: Language,
) -> (
    virgil_cli::models::FileMetadata,
    Vec<SymbolInfo>,
    Vec<ImportInfo>,
) {
    let dir = fixtures_dir();
    let path = dir.join(filename);
    let mut ts_parser = parser::create_parser(language).expect("create parser");
    let (metadata, tree) =
        parser::parse_file(&mut ts_parser, &path, &dir, language).expect("parse_file");
    let source = std::fs::read_to_string(&path).expect("read source");
    let sym_query = languages::compile_symbol_query(language).expect("compile query");
    let syms = languages::extract_symbols(
        &tree,
        source.as_bytes(),
        &sym_query,
        &metadata.path,
        language,
    );
    let imp_query = languages::compile_import_query(language).expect("compile import query");
    let imps = languages::extract_imports(
        &tree,
        source.as_bytes(),
        &imp_query,
        &metadata.path,
        language,
    );
    (metadata, syms, imps)
}
```

**Patterns:**
- Setup via helper functions: `fixtures_dir()`, `parse_fixture()`, `create_test_dir()`
- No cleanup needed for tempdir (automatically dropped)
- Assertions check multiple properties in sequence
- Early assertions on counts, then detailed checks on specific items

## Mocking

**Framework:** None (no mocking library used)

**Patterns:**
- In-memory file structures created explicitly: `HashMap::new()` for `MemoryFileSource`
- Temporary directories via `tempfile::tempdir()` for filesystem tests
- No mock objects; actual parsing and extraction tested end-to-end

**Example from file_source.rs tests:**
```rust
#[test]
fn memory_file_source_basic() {
    let mut files = HashMap::new();
    files.insert("src/main.rs".to_string(), Arc::from("fn main() {}"));
    files.insert("src/lib.rs".to_string(), Arc::from("pub mod foo;"));
    
    let source = MemoryFileSource::new(files);
    assert_eq!(source.read_file("src/main.rs"), Some(Arc::from("fn main() {}")));
}
```

**What to Mock:**
- File I/O: Use `tempfile::tempdir()` to create isolated test directories
- Standard input: Not tested (handled by CLI parsing, not core logic)

**What NOT to Mock:**
- Tree-sitter parsing: Always test with real parsers
- Symbol extraction: Always use real extraction logic and fixture files
- Registry operations: Always test with real registry files in temp directories

## Fixtures and Factories

**Test Data:**
- Fixture files stored as actual TypeScript/JavaScript/Rust source files in `tests/fixtures/`
- Each language has dedicated fixture: `sample.ts`, `imports_sample.ts`, `component.tsx`
- Minimal fixtures: `empty.ts` for edge cases

**Location:**
- `tests/fixtures/` directory
- Files committed to repo; not generated during tests
- Contain realistic code patterns that match real-world expectations

**Example fixture: sample.ts**
```typescript
// A realistic fixture with functions, classes, types, interfaces
export function greet(name: string) { ... }
export class UserService { ... }
export const API_URL = "...";
// etc.
```

## Coverage

**Requirements:** None enforced (no coverage tool configured)

**Observation:**
- ~335 test functions exist across codebase (`#[cfg(test)]` count)
- Each module has tests covering main paths and edge cases
- Integration tests verify multi-language parsing pipelines
- Parser, language extraction, and registry modules heavily tested

## Test Types

**Unit Tests:**
- Scope: Individual functions and data structures
- Approach: Test single module in isolation using helper functions
- Examples: `parser::create_parser()` tests all 12 languages, `Language::from_extension()` tests each variant
- Located in `src/**/*.rs` with `#[cfg(test)] mod tests`

**Integration Tests:**
- Scope: End-to-end parsing pipelines across languages
- Approach: Parse fixture files, extract symbols/imports, verify counts and specific items
- Examples: `full_pipeline_typescript()`, `import_extraction_javascript()`
- Located in `tests/integration_test.rs`
- Use actual fixture files in `tests/fixtures/`

**E2E Tests:**
- Not used (CLI testing would require spawning subprocesses; not worthwhile given architecture)
- Confidence gained via integration tests + parser correctness tests

## Common Patterns

**Async Testing:**
- Not applicable (no async code in core parsing logic)
- Server code in `src/server.rs` uses tokio, but no async tests (not tested via cargo test)

**Error Testing:**
```rust
#[test]
fn parse_file_invalid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("invalid.ts");
    std::fs::write(&file_path, "const x: invalid type;").unwrap();

    let mut parser = create_parser(Language::TypeScript).unwrap();
    let result = parse_file(&mut parser, &file_path, dir.path(), Language::TypeScript);
    
    // Some parse errors are recoverable; test behavior
    // Most failures tested via `.expect()` in integration tests
}
```

**Fixture-based Testing:**
```rust
#[test]
fn full_pipeline_typescript() {
    let (meta, syms) = parse_fixture("sample.ts", Language::TypeScript);
    assert_eq!(meta.name, "sample.ts");
    assert_eq!(meta.extension, "ts");
    assert_eq!(meta.language, "typescript");

    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(syms.len(), 10, "expected 10 symbols, got: {names:?}");

    let expected = ["greet", "UserService", "API_URL", "fetchData", "User"];
    for name in &expected {
        assert!(names.contains(name), "missing symbol: {name}");
    }
}
```

**Parallel Testing (rayon):**
- Tests don't test parallelism directly (rayon used internally)
- Integration tests call `execute()` which uses rayon internally
- Single-threaded tests via `cargo test -- --test-threads=1` if needed

## Test Fixtures

**Available Fixtures:**
- `tests/fixtures/sample.ts` - TypeScript with functions, classes, types
- `tests/fixtures/sample.js` - JavaScript with functions and classes
- `tests/fixtures/imports_sample.ts` - TypeScript with various import styles
- `tests/fixtures/imports_sample.js` - JavaScript with require/import/dynamic
- `tests/fixtures/component.tsx` - React component
- `tests/fixtures/component.jsx` - JSX component
- `tests/fixtures/empty.ts` - Empty file for edge cases

**How to Add New Fixture:**
1. Create file in `tests/fixtures/` with actual source code
2. Update `integration_test.rs` to add test function using `parse_fixture()`
3. Verify symbol/import count assertions match actual content
4. Run `cargo test --test integration_test` to verify

## Running Tests

**All Tests:**
```bash
cargo test
```

**Specific Module:**
```bash
cargo test parser::
cargo test query_engine::
```

**Integration Tests Only:**
```bash
cargo test --test integration_test
```

**With Output:**
```bash
cargo test -- --nocapture
```

**Single Test:**
```bash
cargo test parse_file_metadata -- --exact
```

---

*Testing analysis: 2026-04-16*
