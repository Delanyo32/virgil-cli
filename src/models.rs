use std::fmt;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub language: String,
    pub size_bytes: u64,
    pub line_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Variable,
    Interface,
    TypeAlias,
    Enum,
    ArrowFunction,
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            SymbolKind::Variable => "variable",
            SymbolKind::Interface => "interface",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Enum => "enum",
            SymbolKind::ArrowFunction => "arrow_function",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub is_exported: bool,
}

#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub source_file: String,
    pub module_specifier: String,
    pub imported_name: String,
    pub local_name: String,
    pub kind: String,
    pub is_type_only: bool,
    pub line: u32,
    pub is_external: bool,
}

impl ImportInfo {
    /// Classify a module specifier as external (library) or internal (user code).
    /// Internal: starts with `.` (relative path) or `#` (Node.js subpath import).
    /// External: everything else (bare specifiers like `react`, `@scope/pkg`, builtins).
    pub fn is_external_specifier(module_specifier: &str) -> bool {
        !(module_specifier.starts_with('.') || module_specifier.starts_with('#'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_external_specifier_classifies_correctly() {
        // External: bare specifiers, scoped packages, builtins
        assert!(ImportInfo::is_external_specifier("react"));
        assert!(ImportInfo::is_external_specifier("@scope/pkg"));
        assert!(ImportInfo::is_external_specifier("fs"));
        assert!(ImportInfo::is_external_specifier("lodash/merge"));

        // Internal: relative paths and subpath imports
        assert!(!ImportInfo::is_external_specifier("./utils"));
        assert!(!ImportInfo::is_external_specifier("../components/Button"));
        assert!(!ImportInfo::is_external_specifier("."));
        assert!(!ImportInfo::is_external_specifier("#internal/utils"));
    }

    #[test]
    fn symbol_kind_display() {
        assert_eq!(SymbolKind::Function.to_string(), "function");
        assert_eq!(SymbolKind::Class.to_string(), "class");
        assert_eq!(SymbolKind::Method.to_string(), "method");
        assert_eq!(SymbolKind::Variable.to_string(), "variable");
        assert_eq!(SymbolKind::Interface.to_string(), "interface");
        assert_eq!(SymbolKind::TypeAlias.to_string(), "type_alias");
        assert_eq!(SymbolKind::Enum.to_string(), "enum");
        assert_eq!(SymbolKind::ArrowFunction.to_string(), "arrow_function");
    }
}
