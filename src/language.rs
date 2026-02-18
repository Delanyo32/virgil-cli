use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "js" => Some(Language::JavaScript),
            "jsx" => Some(Language::Jsx),
            _ => None,
        }
    }

    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx | Language::Jsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Jsx => "jsx",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Language::TypeScript => "ts",
            Language::Tsx => "tsx",
            Language::JavaScript => "js",
            Language::Jsx => "jsx",
        }
    }

    pub fn all() -> &'static [Language] {
        &[Language::TypeScript, Language::Tsx, Language::JavaScript, Language::Jsx]
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
    fn all_returns_four_variants() {
        assert_eq!(Language::all().len(), 4);
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
    fn display_matches_as_str() {
        for lang in Language::all() {
            assert_eq!(lang.to_string(), lang.as_str());
        }
    }
}
