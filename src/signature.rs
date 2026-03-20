use crate::language::Language;

/// Extract a one-line signature from an AST node's source text.
/// Strategy: take text from start up to the first `{` or end of first line, trim.
pub fn extract_signature(source: &str, start_line: u32, language: Language) -> Option<String> {
    if start_line == 0 {
        return None;
    }
    let lines: Vec<&str> = source.lines().collect();
    let idx = (start_line - 1) as usize;
    if idx >= lines.len() {
        return None;
    }

    // For most languages, the signature is the first line up to `{`
    let first_line = lines[idx].trim();

    // Some declarations span multiple lines before the body opens.
    // Collect lines until we find `{` or run out of a reasonable window.
    let mut sig = String::new();
    for line in lines
        .iter()
        .take(std::cmp::min(idx + 5, lines.len()))
        .skip(idx)
    {
        let line = line.trim();
        if let Some(pos) = line.find('{') {
            let before_brace = line[..pos].trim();
            if !before_brace.is_empty() {
                if !sig.is_empty() {
                    sig.push(' ');
                }
                sig.push_str(before_brace);
            }
            break;
        }
        if !sig.is_empty() {
            sig.push(' ');
        }
        sig.push_str(line);

        // Python/Go: stop at `:` for Python, stop at first line for Go if no `{`
        if language == Language::Python && line.ends_with(':') {
            break;
        }
    }

    // Fallback: just use the first line if sig is empty
    if sig.is_empty() {
        sig = first_line.to_string();
    }

    // Trim trailing colons (Python) and normalize whitespace
    let sig = sig.trim().trim_end_matches('{').trim().to_string();

    if sig.is_empty() { None } else { Some(sig) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_function() {
        let source = "export function greet(name: string): string {\n  return `Hello ${name}`;\n}";
        let sig = extract_signature(source, 1, Language::TypeScript).unwrap();
        assert_eq!(sig, "export function greet(name: string): string");
    }

    #[test]
    fn rust_function() {
        let source =
            "pub fn process(input: &str) -> Result<String> {\n    Ok(input.to_string())\n}";
        let sig = extract_signature(source, 1, Language::Rust).unwrap();
        assert_eq!(sig, "pub fn process(input: &str) -> Result<String>");
    }

    #[test]
    fn python_function() {
        let source = "def authenticate(username: str, password: str) -> bool:\n    pass";
        let sig = extract_signature(source, 1, Language::Python).unwrap();
        assert_eq!(
            sig,
            "def authenticate(username: str, password: str) -> bool:"
        );
    }

    #[test]
    fn multiline_signature() {
        let source = "pub fn complex(\n    a: i32,\n    b: i32,\n) -> i32 {\n    a + b\n}";
        let sig = extract_signature(source, 1, Language::Rust).unwrap();
        assert!(sig.contains("pub fn complex("));
        assert!(sig.contains(") -> i32"));
    }

    #[test]
    fn class_declaration() {
        let source = "export class UserService {\n  private db: Database;\n}";
        let sig = extract_signature(source, 1, Language::TypeScript).unwrap();
        assert_eq!(sig, "export class UserService");
    }
}
