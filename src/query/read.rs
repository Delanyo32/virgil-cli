use std::path::Path;

use anyhow::{Context, Result, bail};

pub fn run_read(
    file_path: &str,
    root: &Path,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<String> {
    let full_path = root.join(file_path);

    if !full_path.exists() {
        bail!("file not found: {}", full_path.display());
    }

    let content = std::fs::read_to_string(&full_path)
        .with_context(|| format!("failed to read {}", full_path.display()))?;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = start_line.map(|s| s.saturating_sub(1)).unwrap_or(0);
    let end = end_line.map(|e| e.min(total)).unwrap_or(total);

    if start >= total {
        return Ok(String::new());
    }

    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let line_num = start + i + 1;
        out.push_str(&format!("{:>4}  {}\n", line_num, line));
    }

    Ok(out)
}
