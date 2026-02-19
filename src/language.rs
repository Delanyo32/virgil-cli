use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    C,
    Cpp,
    CSharp,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "js" => Some(Language::JavaScript),
            "jsx" => Some(Language::Jsx),
            "c" | "h" => Some(Language::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some(Language::Cpp),
            "cs" => Some(Language::CSharp),
            _ => None,
        }
    }

    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx | Language::Jsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Jsx => "jsx",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Language::TypeScript => "ts",
            Language::Tsx => "tsx",
            Language::JavaScript => "js",
            Language::Jsx => "jsx",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "cs",
        }
    }

    pub fn all_extensions(&self) -> &'static [&'static str] {
        match self {
            Language::TypeScript => &["ts"],
            Language::Tsx => &["tsx"],
            Language::JavaScript => &["js"],
            Language::Jsx => &["jsx"],
            Language::C => &["c", "h"],
            Language::Cpp => &["cpp", "cc", "cxx", "hpp", "hxx", "hh"],
            Language::CSharp => &["cs"],
        }
    }

    pub fn all() -> &'static [Language] {
        &[
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Jsx,
            Language::C,
            Language::Cpp,
            Language::CSharp,
        ]
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn parse_language_filter(filter: &str) -> Vec<Language> {
    filter
        .split(',')
        .filter_map(|s| Language::from_extension(s.trim()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_extension_valid() {
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("jsx"), Some(Language::Jsx));
        assert_eq!(Language::from_extension("c"), Some(Language::C));
        assert_eq!(Language::from_extension("h"), Some(Language::C));
        assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cc"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cxx"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hxx"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hh"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cs"), Some(Language::CSharp));
    }

    #[test]
    fn from_extension_invalid() {
        assert_eq!(Language::from_extension("py"), None);
        assert_eq!(Language::from_extension("rs"), None);
        assert_eq!(Language::from_extension(""), None);
    }

    #[test]
    fn extension_round_trip() {
        for lang in Language::all() {
            let ext = lang.extension();
            assert_eq!(Language::from_extension(ext), Some(*lang));
        }
    }

    #[test]
    fn all_returns_seven_variants() {
        assert_eq!(Language::all().len(), 7);
    }

    #[test]
    fn all_extensions_covers_all() {
        // C should have both .c and .h
        assert_eq!(Language::C.all_extensions(), &["c", "h"]);
        // C++ should have 6 extensions
        assert_eq!(Language::Cpp.all_extensions().len(), 6);
        // Single-extension languages
        assert_eq!(Language::TypeScript.all_extensions(), &["ts"]);
        assert_eq!(Language::CSharp.all_extensions(), &["cs"]);
    }

    #[test]
    fn parse_language_filter_single() {
        let result = parse_language_filter("ts");
        assert_eq!(result, vec![Language::TypeScript]);
    }

    #[test]
    fn parse_language_filter_multiple() {
        let result = parse_language_filter("ts,js,tsx");
        assert_eq!(result, vec![Language::TypeScript, Language::JavaScript, Language::Tsx]);
    }

    #[test]
    fn parse_language_filter_with_spaces() {
        let result = parse_language_filter("ts , js");
        assert_eq!(result, vec![Language::TypeScript, Language::JavaScript]);
    }

    #[test]
    fn parse_language_filter_invalid_ignored() {
        let result = parse_language_filter("ts,py,js");
        assert_eq!(result, vec![Language::TypeScript, Language::JavaScript]);
    }

    #[test]
    fn parse_language_filter_all_invalid() {
        let result = parse_language_filter("py,rs");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_language_filter_new_languages() {
        let result = parse_language_filter("c,cpp,cs");
        assert_eq!(result, vec![Language::C, Language::Cpp, Language::CSharp]);
    }

    #[test]
    fn parse_language_filter_cpp_extensions() {
        let result = parse_language_filter("cpp,hpp,cc");
        assert_eq!(result, vec![Language::Cpp, Language::Cpp, Language::Cpp]);
    }

    #[test]
    fn display_matches_as_str() {
        for lang in Language::all() {
            assert_eq!(lang.to_string(), lang.as_str());
        }
    }
}
