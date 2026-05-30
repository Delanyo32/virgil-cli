mod c_lang;
mod cpp;
mod csharp;
mod go;
mod java;
mod php;
mod python;
mod rust_lang;
mod typescript;

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tree_sitter::{Query, Tree};

use crate::graph::GraphNode;
use crate::language::Language;
use crate::models::{
    AttrsBucket, CommentInfo, ExtractedTypes, ImportInfo, ReferencesBucket, SymbolInfo, ThrowsRow,
};

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::compile_symbol_query(language)
        }
        Language::C => c_lang::compile_symbol_query(language),
        Language::Cpp => cpp::compile_symbol_query(language),
        Language::CSharp => csharp::compile_symbol_query(language),
        Language::Rust => rust_lang::compile_symbol_query(language),
        Language::Python => python::compile_symbol_query(language),
        Language::Go => go::compile_symbol_query(language),
        Language::Java => java::compile_symbol_query(language),
        Language::Php => php::compile_symbol_query(language),
    }
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::compile_import_query(language)
        }
        Language::C => c_lang::compile_import_query(language),
        Language::Cpp => cpp::compile_import_query(language),
        Language::CSharp => csharp::compile_import_query(language),
        Language::Rust => rust_lang::compile_import_query(language),
        Language::Python => python::compile_import_query(language),
        Language::Go => go::compile_import_query(language),
        Language::Java => java::compile_import_query(language),
        Language::Php => php::compile_import_query(language),
    }
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::compile_comment_query(language)
        }
        Language::C => c_lang::compile_comment_query(language),
        Language::Cpp => cpp::compile_comment_query(language),
        Language::CSharp => csharp::compile_comment_query(language),
        Language::Rust => rust_lang::compile_comment_query(language),
        Language::Python => python::compile_comment_query(language),
        Language::Go => go::compile_comment_query(language),
        Language::Java => java::compile_comment_query(language),
        Language::Php => php::compile_comment_query(language),
    }
}

/// Per-language separator used to join parent-chain segments in
/// `symbol.qualified_name`. Rust / C / C++ use the scope-resolution
/// operator `::`; PHP class-internal qualification also uses `::`. All
/// other supported languages use the dotted convention.
pub fn qname_separator(language: Language) -> &'static str {
    match language {
        Language::Rust | Language::C | Language::Cpp | Language::Php => "::",
        Language::TypeScript
        | Language::Tsx
        | Language::JavaScript
        | Language::Jsx
        | Language::CSharp
        | Language::Go
        | Language::Java
        | Language::Python => ".",
    }
}

pub fn extract_symbols(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
    language: Language,
) -> Vec<SymbolInfo> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::extract_symbols(tree, source, query, file_path, language)
        }
        Language::C => c_lang::extract_symbols(tree, source, query, file_path),
        Language::Cpp => cpp::extract_symbols(tree, source, query, file_path),
        Language::CSharp => csharp::extract_symbols(tree, source, query, file_path),
        Language::Rust => rust_lang::extract_symbols(tree, source, query, file_path),
        Language::Python => python::extract_symbols(tree, source, query, file_path),
        Language::Go => go::extract_symbols(tree, source, query, file_path),
        Language::Java => java::extract_symbols(tree, source, query, file_path),
        Language::Php => php::extract_symbols(tree, source, query, file_path),
    }
}

pub fn extract_imports(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
    language: Language,
) -> Vec<ImportInfo> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::extract_imports(tree, source, query, file_path)
        }
        Language::C => c_lang::extract_imports(tree, source, query, file_path),
        Language::Cpp => cpp::extract_imports(tree, source, query, file_path),
        Language::CSharp => csharp::extract_imports(tree, source, query, file_path),
        Language::Rust => rust_lang::extract_imports(tree, source, query, file_path),
        Language::Python => python::extract_imports(tree, source, query, file_path),
        Language::Go => go::extract_imports(tree, source, query, file_path),
        Language::Java => java::extract_imports(tree, source, query, file_path),
        Language::Php => php::extract_imports(tree, source, query, file_path),
    }
}

pub fn extract_comments(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
    language: Language,
) -> Vec<CommentInfo> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::extract_comments(tree, source, query, file_path)
        }
        Language::C => c_lang::extract_comments(tree, source, query, file_path),
        Language::Cpp => cpp::extract_comments(tree, source, query, file_path),
        Language::CSharp => csharp::extract_comments(tree, source, query, file_path),
        Language::Rust => rust_lang::extract_comments(tree, source, query, file_path),
        Language::Python => python::extract_comments(tree, source, query, file_path),
        Language::Go => go::extract_comments(tree, source, query, file_path),
        Language::Java => java::extract_comments(tree, source, query, file_path),
        Language::Php => php::extract_comments(tree, source, query, file_path),
    }
}

/// Issue #13 + #14 type-extraction facade. Each language returns:
/// - `Vec<TypeRow>` (one per type-position node, pre-dedup),
/// - `Vec<ParameterTypeRow>` (one per function parameter),
/// - `Vec<ReturnsTypeRow>` (one per annotated function return),
/// - `Vec<InheritanceRow>` (one per `extends`/`implements` edge),
/// - `Vec<FieldTypeRow>` (one per typed field/property declaration; #14).
pub fn extract_types(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    language: Language,
) -> ExtractedTypes {
    match language {
        Language::Rust => rust_lang::extract_types(tree, source, file_path),
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::extract_types(tree, source, file_path)
        }
        Language::Python => python::extract_types(tree, source, file_path),
        Language::Go => go::extract_types(tree, source, file_path),
        Language::Java => java::extract_types(tree, source, file_path),
        Language::Php => php::extract_types(tree, source, file_path),
        Language::C => c_lang::extract_types(tree, source, file_path),
        Language::Cpp => cpp::extract_types(tree, source, file_path),
        Language::CSharp => csharp::extract_types(tree, source, file_path),
    }
}

/// Issue #13 (followup): per-language `throws`/`@throws` extraction.
/// Only Java, C#, and PHP currently emit rows; the other languages
/// return an empty vec.
pub fn extract_throws(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    language: Language,
) -> Vec<ThrowsRow> {
    match language {
        Language::Java => java::extract_throws(tree, source, file_path),
        Language::CSharp => csharp::extract_throws(tree, source, file_path),
        Language::Php => php::extract_throws(tree, source, file_path),
        _ => Vec::new(),
    }
}

/// Issue #15 per-language attribute facade. Each language returns an
/// `AttrsBucket` with only its own variant populated. Symbols are
/// passed in so the extractor can synthesize stable symbol_ids per
/// ADR-0002 (`path|line|col|name|kind`).
pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    language: Language,
    symbols: &[SymbolInfo],
) -> AttrsBucket {
    let mut bucket = AttrsBucket::default();
    match language {
        Language::Rust => {
            bucket.rust = rust_lang::extract_attrs(tree, source, file_path, symbols);
        }
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            bucket.typescript = typescript::extract_attrs(tree, source, file_path, symbols);
        }
        Language::Python => {
            bucket.python = python::extract_attrs(tree, source, file_path, symbols);
        }
        Language::Go => {
            bucket.go = go::extract_attrs(tree, source, file_path, symbols);
        }
        Language::Java => {
            bucket.java = java::extract_attrs(tree, source, file_path, symbols);
        }
        Language::Php => {
            bucket.php = php::extract_attrs(tree, source, file_path, symbols);
        }
        Language::C => {
            bucket.c = c_lang::extract_attrs(tree, source, file_path, symbols);
        }
        Language::Cpp => {
            bucket.cpp = cpp::extract_attrs(tree, source, file_path, symbols);
        }
        Language::CSharp => {
            bucket.csharp = csharp::extract_attrs(tree, source, file_path, symbols);
        }
    }
    bucket
}

/// Issue #16 references-fact emission facade per ADR-0005. Each
/// language emits `occurrence` / `scope` / `binding` rows; the
/// Cozoscript resolver consumes them to materialise `references`.
pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    language: Language,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    match language {
        Language::Rust => rust_lang::extract_references(tree, source, file_path, symbols),
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::extract_references(tree, source, file_path, symbols)
        }
        Language::Python => python::extract_references(tree, source, file_path, symbols),
        Language::Go => go::extract_references(tree, source, file_path, symbols),
        Language::Java => java::extract_references(tree, source, file_path, symbols),
        Language::Php => php::extract_references(tree, source, file_path, symbols),
        Language::C => c_lang::extract_references(tree, source, file_path, symbols),
        Language::Cpp => cpp::extract_references(tree, source, file_path, symbols),
        Language::CSharp => csharp::extract_references(tree, source, file_path, symbols),
    }
}

/// Resolve an internal import to a graph node within the project.
/// Returns None for external imports, unresolvable imports, or C# (no file mapping).
pub fn resolve_import(
    source_file: &str,
    import: &ImportInfo,
    language: Language,
    known_files: &HashSet<String>,
) -> Option<GraphNode> {
    // These languages can't classify internal-vs-external syntactically — it
    // depends on the workspace file set (e.g. Rust's bare/crate-name-qualified
    // `use` paths). Skip the `is_external` short-circuit and let the
    // per-language resolver decide by matching files.
    if import.is_external
        && !matches!(
            language,
            Language::Go
                | Language::Java
                | Language::Python
                | Language::Php
                | Language::Rust
        )
    {
        return None;
    }
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::resolve_import(source_file, &import.module_specifier, known_files)
                .map(GraphNode::File)
        }
        Language::Rust => {
            rust_lang::resolve_import(source_file, &import.module_specifier, known_files)
                .map(GraphNode::File)
        }
        Language::Python => {
            python::resolve_import(source_file, &import.module_specifier, known_files)
                .map(GraphNode::File)
        }
        Language::Go => {
            go::resolve_import(&import.module_specifier, known_files).map(GraphNode::Package)
        }
        Language::Java => java::resolve_import(&import.module_specifier, known_files),
        Language::Php => php::resolve_import(
            source_file,
            &import.module_specifier,
            &import.kind,
            known_files,
        )
        .map(GraphNode::File),
        Language::C => c_lang::resolve_import(source_file, &import.module_specifier, known_files)
            .map(GraphNode::File),
        Language::Cpp => cpp::resolve_import(source_file, &import.module_specifier, known_files)
            .map(GraphNode::File),
        Language::CSharp => None, // No file-level mapping without .csproj
    }
}
