use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::s3::S3Client;

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

    format_lines(&content, start_line, end_line)
}

pub fn run_read_s3(
    file_path: &str,
    root: &str,
    client: &S3Client,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<String> {
    let key = format!("{}/{}", root.trim_end_matches('/'), file_path);
    let content = client.get_file_string(&key)?;
    format_lines(&content, start_line, end_line)
}

fn format_lines(
    content: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<String> {
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
