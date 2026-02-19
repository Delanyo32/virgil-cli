mod typescript;

use std::sync::Arc;

use anyhow::Result;
use tree_sitter::{Query, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo};

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    typescript::compile_symbol_query(language)
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    typescript::compile_import_query(language)
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    typescript::compile_comment_query(language)
}

pub fn extract_symbols(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<SymbolInfo> {
    typescript::extract_symbols(tree, source, query, file_path)
}

pub fn extract_imports(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<ImportInfo> {
    typescript::extract_imports(tree, source, query, file_path)
}

pub fn extract_comments(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<CommentInfo> {
    typescript::extract_comments(tree, source, query, file_path)
}
