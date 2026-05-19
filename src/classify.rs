//! File-path classification helpers used at graph-build time.
//!
//! Moved out of the deleted `src/pipeline/helpers.rs` so the remaining
//! consumers (`src/graph/builder.rs`, `src/cozo/from_code_graph.rs`) don't
//! have to keep a vestigial pipeline module alive.

pub fn is_test_file(file_path: &str) -> bool {
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    if file_name.ends_with("_test.rs") || file_name.ends_with("_test.go") {
        return true;
    }
    if (file_name.starts_with("test_") && file_name.ends_with(".py"))
        || file_name.ends_with("_test.py")
        || file_name == "conftest.py"
    {
        return true;
    }
    if file_name.ends_with("Test.java")
        || file_name.ends_with("Tests.java")
        || file_name.ends_with("Spec.java")
    {
        return true;
    }
    if file_name.ends_with("Tests.cs")
        || file_name.ends_with("Test.cs")
        || file_name.ends_with("Spec.cs")
    {
        return true;
    }
    if file_name.ends_with("Test.php") {
        return true;
    }
    if file_name.ends_with("_test.cpp")
        || file_name.ends_with("_test.cc")
        || file_name.ends_with("_unittest.cpp")
    {
        return true;
    }
    if file_name.ends_with("Test.cpp") && file_name.len() > "Test.cpp".len() {
        return true;
    }
    if (file_name.starts_with("test_") && file_name.ends_with(".cpp"))
        || (file_name.starts_with("test_") && file_name.ends_with(".cc"))
    {
        return true;
    }
    let lower = file_name.to_lowercase();
    if lower.contains(".test.") || lower.contains(".spec.") {
        return true;
    }
    let path = file_path.replace('\\', "/");
    path.contains("/tests/")
        || path.starts_with("tests/")
        || path.contains("/test/")
        || path.starts_with("test/")
        || path.contains("/__tests__/")
        || path.starts_with("__tests__/")
        || path.contains("/testing/")
        || path.starts_with("testing/")
        || path.contains("/testdata/")
        || path.starts_with("testdata/")
}

pub fn is_barrel_file(path: &str) -> bool {
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    matches!(
        file_name,
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" | "__init__.py" | "mod.rs"
    )
}
