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

use crate::audit::project_index::GraphNode;
use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo};

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

pub fn extract_symbols(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
    language: Language,
) -> Vec<SymbolInfo> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::extract_symbols(tree, source, query, file_path)
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

/// Resolve an internal import to a graph node within the project.
/// Returns None for external imports, unresolvable imports, or C# (no file mapping).
pub fn resolve_import(
    source_file: &str,
    import: &ImportInfo,
    language: Language,
    known_files: &HashSet<String>,
) -> Option<GraphNode> {
    if import.is_external {
        return None;
    }
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            typescript::resolve_import(source_file, &import.module_specifier, known_files)
                .map(GraphNode::File)
        }
        Language::Rust => rust_lang::resolve_import(source_file, &import.module_specifier, known_files)
            .map(GraphNode::File),
        Language::Python => {
            python::resolve_import(source_file, &import.module_specifier, known_files)
                .map(GraphNode::File)
        }
        Language::Go => {
            go::resolve_import(&import.module_specifier, known_files).map(GraphNode::Package)
        }
        Language::Java => {
            java::resolve_import(&import.module_specifier, known_files).map(GraphNode::File)
        }
        Language::Php => {
            php::resolve_import(source_file, &import.module_specifier, &import.kind, known_files)
                .map(GraphNode::File)
        }
        Language::C => c_lang::resolve_import(source_file, &import.module_specifier, known_files)
            .map(GraphNode::File),
        Language::Cpp => {
            cpp::resolve_import(source_file, &import.module_specifier, known_files)
                .map(GraphNode::File)
        }
        Language::CSharp => None, // No file-level mapping without .csproj
    }
}
