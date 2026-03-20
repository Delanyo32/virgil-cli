use tree_sitter::Query;

pub fn find_capture_index(query: &Query, name: &str) -> usize {
    query
        .capture_names()
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("query must have @{name} capture"))
}

pub fn node_text<'a>(node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

pub fn extract_snippet(source: &[u8], node: tree_sitter::Node, max_lines: usize) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        text.to_string()
    } else {
        let mut snippet: String = lines[..max_lines].join("\n");
        snippet.push_str("\n...");
        snippet
    }
}
