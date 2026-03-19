use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TsQuery {
    /// File glob filter: string or array of strings
    #[serde(default)]
    pub files: Option<FileFilter>,

    /// Glob patterns to exclude files
    #[serde(default)]
    pub files_exclude: Option<Vec<String>>,

    /// Symbol kind filter: string or array of strings
    #[serde(default)]
    pub find: Option<FindFilter>,

    /// Name filter: string (glob), or {contains, regex}
    #[serde(default)]
    pub name: Option<NameFilter>,

    /// Only return symbols inside a parent symbol with this name
    #[serde(default)]
    pub inside: Option<String>,

    /// Visibility filter: exported, public, private, protected, internal
    #[serde(default)]
    pub visibility: Option<String>,

    /// Has filter: string, [strings], or {not: "docstring"}
    #[serde(default)]
    pub has: Option<HasFilter>,

    /// Line count filter: {min, max}
    #[serde(default)]
    pub lines: Option<LineRange>,

    /// Include full body in results
    #[serde(default)]
    pub body: Option<bool>,

    /// Number of preview lines to include
    #[serde(default)]
    pub preview: Option<usize>,

    /// Call graph direction: down, up, both
    #[serde(default)]
    pub calls: Option<String>,

    /// Call graph traversal depth
    #[serde(default)]
    pub depth: Option<usize>,

    /// Override output format
    #[serde(default)]
    pub format: Option<String>,

    /// Read a file by path with optional line range (uses `lines` for range)
    #[serde(default)]
    pub read: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FileFilter {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FindFilter {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum NameFilter {
    Glob(String),
    Complex {
        #[serde(default)]
        contains: Option<String>,
        #[serde(default)]
        regex: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum HasFilter {
    Single(String),
    Multiple(Vec<String>),
    Not { not: String },
}

#[derive(Debug, Deserialize)]
pub struct LineRange {
    #[serde(default)]
    pub min: Option<u32>,
    #[serde(default)]
    pub max: Option<u32>,
}

impl FileFilter {
    pub fn patterns(&self) -> Vec<&str> {
        match self {
            FileFilter::Single(s) => vec![s.as_str()],
            FileFilter::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

impl FindFilter {
    pub fn kinds(&self) -> Vec<&str> {
        match self {
            FindFilter::Single(s) => vec![s.as_str()],
            FindFilter::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_query() {
        let q: TsQuery = serde_json::from_str("{}").unwrap();
        assert!(q.find.is_none());
        assert!(q.name.is_none());
    }

    #[test]
    fn parse_find_single() {
        let q: TsQuery = serde_json::from_str(r#"{"find": "function"}"#).unwrap();
        assert_eq!(q.find.unwrap().kinds(), vec!["function"]);
    }

    #[test]
    fn parse_find_multiple() {
        let q: TsQuery = serde_json::from_str(r#"{"find": ["function", "method"]}"#).unwrap();
        assert_eq!(q.find.unwrap().kinds(), vec!["function", "method"]);
    }

    #[test]
    fn parse_name_glob() {
        let q: TsQuery = serde_json::from_str(r#"{"name": "handle*"}"#).unwrap();
        match q.name.unwrap() {
            NameFilter::Glob(s) => assert_eq!(s, "handle*"),
            _ => panic!("expected glob"),
        }
    }

    #[test]
    fn parse_name_contains() {
        let q: TsQuery = serde_json::from_str(r#"{"name": {"contains": "auth"}}"#).unwrap();
        match q.name.unwrap() {
            NameFilter::Complex { contains, .. } => assert_eq!(contains.unwrap(), "auth"),
            _ => panic!("expected complex"),
        }
    }

    #[test]
    fn parse_name_regex() {
        let q: TsQuery = serde_json::from_str(r#"{"name": {"regex": "^get[A-Z]"}}"#).unwrap();
        match q.name.unwrap() {
            NameFilter::Complex { regex, .. } => assert_eq!(regex.unwrap(), "^get[A-Z]"),
            _ => panic!("expected complex"),
        }
    }

    #[test]
    fn parse_files_single() {
        let q: TsQuery = serde_json::from_str(r#"{"files": "src/**/*.ts"}"#).unwrap();
        assert_eq!(q.files.unwrap().patterns(), vec!["src/**/*.ts"]);
    }

    #[test]
    fn parse_files_multiple() {
        let q: TsQuery = serde_json::from_str(r#"{"files": ["src/**", "lib/**"]}"#).unwrap();
        assert_eq!(q.files.unwrap().patterns(), vec!["src/**", "lib/**"]);
    }

    #[test]
    fn parse_has_not() {
        let q: TsQuery = serde_json::from_str(r#"{"has": {"not": "docstring"}}"#).unwrap();
        match q.has.unwrap() {
            HasFilter::Not { not } => assert_eq!(not, "docstring"),
            _ => panic!("expected not"),
        }
    }

    #[test]
    fn parse_lines() {
        let q: TsQuery = serde_json::from_str(r#"{"lines": {"min": 10, "max": 50}}"#).unwrap();
        let lr = q.lines.unwrap();
        assert_eq!(lr.min, Some(10));
        assert_eq!(lr.max, Some(50));
    }

    #[test]
    fn parse_full_query() {
        let q: TsQuery = serde_json::from_str(
            r#"{
                "files": "src/api/**",
                "find": "function",
                "name": "handle*",
                "visibility": "exported",
                "lines": {"min": 5},
                "preview": 3
            }"#,
        )
        .unwrap();
        assert!(q.files.is_some());
        assert!(q.find.is_some());
        assert!(q.name.is_some());
        assert_eq!(q.visibility.as_deref(), Some("exported"));
        assert_eq!(q.lines.unwrap().min, Some(5));
        assert_eq!(q.preview, Some(3));
    }
}
