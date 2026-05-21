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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Variable,
    Interface,
    TypeAlias,
    Enum,
    ArrowFunction,
    Struct,
    Union,
    Namespace,
    Macro,
    Property,
    Typedef,
    Trait,
    Constant,
    Module,
    Parameter,
    /// Struct / class / interface field. Distinct from `Property` (which
    /// carries getter/setter semantics in TS/C#); plain data members go
    /// here. Used as the `kind` segment of the synthesized symbol_id in
    /// `field_type` rows (issue #14).
    Field,
}

impl SymbolKind {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(SymbolKind::Function),
            "class" => Some(SymbolKind::Class),
            "method" => Some(SymbolKind::Method),
            "variable" => Some(SymbolKind::Variable),
            "interface" => Some(SymbolKind::Interface),
            "type_alias" => Some(SymbolKind::TypeAlias),
            "enum" => Some(SymbolKind::Enum),
            "arrow_function" => Some(SymbolKind::ArrowFunction),
            "struct" => Some(SymbolKind::Struct),
            "union" => Some(SymbolKind::Union),
            "namespace" => Some(SymbolKind::Namespace),
            "macro" => Some(SymbolKind::Macro),
            "property" => Some(SymbolKind::Property),
            "typedef" => Some(SymbolKind::Typedef),
            "trait" => Some(SymbolKind::Trait),
            "constant" => Some(SymbolKind::Constant),
            "module" => Some(SymbolKind::Module),
            "parameter" => Some(SymbolKind::Parameter),
            "field" => Some(SymbolKind::Field),
            _ => None,
        }
    }
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
            SymbolKind::Struct => "struct",
            SymbolKind::Union => "union",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Macro => "macro",
            SymbolKind::Property => "property",
            SymbolKind::Typedef => "typedef",
            SymbolKind::Trait => "trait",
            SymbolKind::Constant => "constant",
            SymbolKind::Module => "module",
            SymbolKind::Parameter => "parameter",
            SymbolKind::Field => "field",
        };
        f.write_str(s)
    }
}

/// Coarse-grained visibility classifier shared across all 9 languages.
///
/// Cross-language mapping (per docs/attrs-<lang>.md):
/// - Rust: `pub` → Public; `pub(crate)` / `pub(super)` / `pub(in …)` → Internal;
///   absent or `pub(self)` → Private.
/// - TypeScript: `export` or `public` modifier → Public; `protected` → Protected;
///   no modifier on top-level → Public; no modifier on class member → Private.
/// - Python: all symbols → Public (no language-level access control).
/// - Go: capitalised first rune → Public, else Private.
/// - Java / C#: `public` / `private` / `protected` keywords map directly;
///   absent → Internal (package-private) for Java; Private for C# class members.
/// - PHP: `public` / `private` / `protected` keywords map directly; absent → Public.
/// - C: `static` at file scope → Private; otherwise Public.
/// - C++: explicit class-scope keywords map directly; `static` at file scope → Private;
///   otherwise Public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolVisibility {
    Public,
    Private,
    Protected,
    Internal,
}

impl SymbolVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolVisibility::Public => "public",
            SymbolVisibility::Private => "private",
            SymbolVisibility::Protected => "protected",
            SymbolVisibility::Internal => "internal",
        }
    }
}

impl fmt::Display for SymbolVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub is_exported: bool,
    pub visibility: SymbolVisibility,
    pub is_async: bool,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_mutable: bool,
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

#[derive(Debug, Clone)]
pub struct CommentInfo {
    pub file_path: String,
    pub text: String,
    pub kind: String,
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub associated_symbol: Option<String>,
    pub associated_symbol_kind: Option<String>,
}

/// A type-expression occurrence extracted from one file. Maps to one row
/// in the Cozo `type` relation; rows dedupe per `(file_path, display_name)`
/// per ADR-0003.
#[derive(Debug, Clone)]
pub struct TypeRow {
    pub file_path: String,
    /// One of the 7 schema variants: primitive | named | generic |
    /// union | intersection | function | tuple | array.
    pub kind: String,
    pub display_name: String,
    pub canonical_name: Option<String>,
}

/// One per function parameter. `type_display_name` is `None` for untyped
/// parameters (Python, JS, dynamic PHP). The emitter joins this row to a
/// `TypeRow` of the same `(file_path, display_name)` to fill in
/// `parameter.type_id`.
#[derive(Debug, Clone)]
pub struct ParameterTypeRow {
    pub file_path: String,
    pub function_start_line: u32,
    pub function_start_col: u32,
    pub function_name: String,
    pub function_kind: SymbolKind,
    pub parameter_start_line: u32,
    pub parameter_start_col: u32,
    pub parameter_name: String,
    pub position: i64,
    pub type_display_name: Option<String>,
    pub is_optional: bool,
    pub has_default: bool,
}

/// One per annotated function return. Languages without explicit return
/// annotations (Python without `-> T`, JS without TS, etc.) emit no row
/// for that function.
#[derive(Debug, Clone)]
pub struct ReturnsTypeRow {
    pub file_path: String,
    pub function_start_line: u32,
    pub function_start_col: u32,
    pub function_name: String,
    pub function_kind: SymbolKind,
    pub type_display_name: String,
}

/// Issue #14: links a struct/class/interface field symbol to its
/// declared type. Untyped fields (e.g. JS class fields, dynamic PHP
/// properties, Python attributes without PEP 526 annotations) emit no
/// row. The emitter computes `symbol_id` from `(file_path,
/// field_start_line, field_start_col, field_name, field_kind)` per
/// ADR-0002 and `type_id` by joining `type_display_name` against the
/// per-file `TypeRow`s produced by the same extractor.
#[derive(Debug, Clone)]
pub struct FieldTypeRow {
    pub file_path: String,
    pub field_start_line: u32,
    pub field_start_col: u32,
    pub field_name: String,
    pub field_kind: SymbolKind,
    pub type_display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InheritanceKind {
    Extends,
    Implements,
}

/// One per inheritance edge. `parent` is identified by the parent type's
/// `display_name` (joined to a `TypeRow` for `canonical_name`). The
/// emitter resolves both endpoints to symbol IDs when possible.
#[derive(Debug, Clone)]
pub struct InheritanceRow {
    pub file_path: String,
    pub child_start_line: u32,
    pub child_start_col: u32,
    pub child_name: String,
    pub child_kind: SymbolKind,
    pub parent_display_name: String,
    pub parent_canonical_name: Option<String>,
    pub kind: InheritanceKind,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub file_path: String,
    pub file_name: String,
    pub extension: String,
    pub language: String,
    pub error_type: String,
    pub error_message: String,
    pub size_bytes: u64,
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
        assert_eq!(SymbolKind::Struct.to_string(), "struct");
        assert_eq!(SymbolKind::Union.to_string(), "union");
        assert_eq!(SymbolKind::Namespace.to_string(), "namespace");
        assert_eq!(SymbolKind::Macro.to_string(), "macro");
        assert_eq!(SymbolKind::Property.to_string(), "property");
        assert_eq!(SymbolKind::Typedef.to_string(), "typedef");
        assert_eq!(SymbolKind::Trait.to_string(), "trait");
        assert_eq!(SymbolKind::Constant.to_string(), "constant");
        assert_eq!(SymbolKind::Module.to_string(), "module");
        assert_eq!(SymbolKind::Parameter.to_string(), "parameter");
        assert_eq!(SymbolKind::Field.to_string(), "field");
    }
}
