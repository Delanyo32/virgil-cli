# Coding Conventions

**Analysis Date:** 2026-04-16

## Naming Patterns

**Files:**
- Snake case for module files: `query_engine.rs`, `file_source.rs`, `query_lang.rs`
- Enum variants use PascalCase: `Language::TypeScript`, `SymbolKind::Function`
- Descriptive module names reflect functionality: `parser.rs`, `discovery.rs`, `signature.rs`

**Functions:**
- Snake case throughout: `create_parser()`, `parse_file()`, `extract_symbols()`, `discover_files()`
- Public functions use lowercase: `execute()`, `format_results()`, `load_registry()`
- Internal helpers start with underscores if needed, but mostly follow public convention
- Test functions use clear descriptive names: `full_pipeline_typescript()`, `discover_fixtures()`

**Variables:**
- Snake case for all variables: `file_count`, `symbol_info`, `is_exported`, `per_file_results`
- Boolean prefixes: `is_exported`, `is_external`, `is_terminal()` pattern respected
- Destructured bindings from matches use clear names: `name_cap`, `def_cap`, `value_cap`
- Loop variables: `p` for projects, `sym` for symbols, `lang` for languages, `entry` for registry entries

**Types:**
- Structs use PascalCase: `FileMetadata`, `SymbolInfo`, `ProjectEntry`, `QueryResult`
- Enum variants use PascalCase: `Language::TypeScript`, `SymbolKind::Function`, `QueryOutputFormat::Outline`
- Trait implementations are inferred from context (no Trait suffix pattern, use `impl Display for SymbolKind`)

## Code Style

**Formatting:**
- Rust standard formatting (via implicit `rustfmt` conventions)
- 4-space indentation (Rust standard)
- Lines organized for readability with clear blank lines between logical sections
- Comments on their own line above code blocks (e.g., `// ── Symbol queries ──`)

**Linting:**
- `#[allow(clippy::should_implement_trait)]` used sparingly when trait conversion is intentionally omitted (`SymbolKind::from_str()`)
- All unused imports are removed
- Pattern matching preferred over if-let chains in most cases
- Guard clauses used to exit early: `if let Some(ref kinds) = find_kinds && !kinds.contains(&sym.kind) { continue; }`

## Import Organization

**Order:**
1. Standard library imports (`use std::*`)
2. External crate imports (`use anyhow::*`, `use serde::*`, `use tree_sitter::*`)
3. Internal crate imports (`use crate::*`)
4. Nested imports organized by functionality

**Path Aliases:**
- No path aliases used; full module paths preferred for clarity
- Imports grouped logically by domain: `use crate::query_lang::{FindFilter, HasFilter, NameFilter, TsQuery}`
- Common patterns: `use crate::language::Language`, `use crate::models::{SymbolInfo, SymbolKind}`

**Example from query_engine.rs:**
```rust
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use regex::Regex;

use crate::graph::{CodeGraph, NodeWeight};
use crate::language::{self, Language};
```

## Error Handling

**Patterns:**
- `anyhow::Result<T>` used universally for fallible operations
- `.context()` for adding contextual messages: `.context("failed to read projects.json")?`
- `.with_context(|| ...)` for formatted context: `.with_context(|| format!("failed to read {}", path.display()))?`
- Early exits with `bail!()` for validation errors: `bail!("project '{}' already exists", name)`
- `ok_or_else()` with context for conversions: `.ok_or_else(|| anyhow::anyhow!("..."))?`
- Match expressions for handling variants (no panic on common failures)

**Error Recovery:**
- `.unwrap_or_else()` for JSON serialization fallbacks: `serde_json::to_string(&wrapper).unwrap_or_else(|_| "{}".to_string())`
- `.filter_map()` for graceful skipping of problematic files during parsing
- Warnings printed to stderr via `eprintln!()`, not panics
- `.ok()?` chains in parallel iterators to skip files with parsing issues

**Example from registry.rs:**
```rust
pub fn create_project(
    name: &str,
    path: PathBuf,
    exclude: Vec<String>,
    lang_filter: Option<&str>,
) -> Result<ProjectEntry> {
    let mut reg = load_registry()?;

    if reg.projects.iter().any(|p| p.name == name) {
        bail!("project '{}' already exists", name);
    }

    let canonical = fs::canonicalize(&path)
        .with_context(|| format!("path does not exist: {}", path.display()))?;
    // ...
}
```

## Logging

**Framework:** `eprintln!()` for diagnostic output (no logger framework)

**Patterns:**
- Status messages to stderr: `eprintln!("Created project '{}'", entry.name);`
- User feedback always goes to stderr; JSON results to stdout only
- Progress printed when meaningful (file counts, language breakdown)
- No debug-level logging in final code (tests use `.expect()` liberally)

## Comments

**When to Comment:**
- Section headers use ASCII art: `// ── Symbol queries ──`, `// ── Query compilation ──`
- Public struct documentation via doc comments (rarely used, most is self-explanatory)
- Complex tree-sitter queries explained near the constant definitions
- No inline comments for obvious code; only for non-obvious logic

**Intra-comment Documentation:**
- Tree-sitter S-expression queries documented at point of definition (see `TS_SYMBOL_QUERY`, `COMMENT_QUERY`)
- Language-specific rules noted in module docs (see CLAUDE.md integration)

## Function Design

**Size:** 
- Most functions 15-50 lines
- Extraction functions (`extract_symbols`, `extract_imports`) are deterministic and focused
- Parallel processing functions leverage rayon without inline closures > 30 lines

**Parameters:** 
- Functions accept required inputs as positional arguments
- Optional parameters use `Option<T>` or defaults
- File paths as `&Path` or `&str`, never `String` unless ownership needed
- References preferred: `&Workspace`, `&Query`, `&ProjectEntry`

**Return Values:** 
- `Result<T>` for any fallible operation
- `Option<T>` for optional results (e.g., `extract_signature()` returns `Option<String>`)
- Tuples for related return values: `parse_file()` returns `(FileMetadata, Tree)`
- Vec for collections: `Vec<SymbolInfo>`, `Vec<QueryResult>`

**Example from parser.rs:**
```rust
pub fn parse_file(
    parser: &mut tree_sitter::Parser,
    path: &Path,
    root: &Path,
    language: Language,
) -> Result<(FileMetadata, tree_sitter::Tree)> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    // ...
    Ok((metadata, tree))
}
```

## Module Design

**Exports:**
- `pub fn` for public API; `fn` for internal
- `pub struct` for domain models; derive `Debug`, `Clone` where applicable
- Language-specific modules (e.g., `languages/typescript.rs`) export compilation and extraction functions
- `lib.rs` re-exports all public modules for library consumers: `pub use graph::*;`

**Barrel Files:**
- `src/languages/mod.rs` re-exports all language module functions (no barrel pattern for individual symbols)
- `src/graph/mod.rs` exports primary types: `CodeGraph`, `NodeWeight`, `EdgeWeight`
- Top-level `lib.rs` uses explicit re-exports: `pub use graph::builder;`

**Visibility Patterns:**
```rust
// Private module
mod languages {
    pub mod typescript;
    pub mod rust_lang;
    // All extraction functions pub
}

// Public re-export from lib.rs
pub use languages::compile_symbol_query;
pub use languages::extract_symbols;
```

---

*Convention analysis: 2026-04-16*
