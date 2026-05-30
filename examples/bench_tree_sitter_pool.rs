//! Variant of `bench_tree_sitter` that drops + recreates the parser every
//! N files, releasing the arena. Compare RSS vs the single-parser version
//! to measure the per-Parser arena retention.
//!
//! Usage:
//!   cargo run --release --example bench_tree_sitter_pool -- <dir> [<ext>] [<reset_every>]

use std::env;
use std::path::PathBuf;

use virgil_cli::language::Language;
use virgil_cli::parser::create_parser;

fn main() {
    let mut args = env::args().skip(1);
    let dir = args
        .next()
        .map(PathBuf::from)
        .expect("usage: bench_tree_sitter_pool <dir> [<ext>] [<reset_every>]");
    let ext = args.next().unwrap_or_else(|| "ts".to_string());
    let reset_every: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(50);
    let lang = Language::from_extension(&ext).expect("unknown extension");

    let mut files: Vec<PathBuf> = Vec::new();
    walk(&dir, &ext, &mut files);
    files.sort();
    eprintln!(
        "[bench_tree_sitter_pool] {} files, reset_every={}",
        files.len(),
        reset_every
    );

    let mut parser = create_parser(lang).expect("create parser");
    let mut total_bytes = 0usize;
    for (i, f) in files.iter().enumerate() {
        if i > 0 && i.is_multiple_of(reset_every) {
            drop(parser);
            parser = create_parser(lang).expect("recreate parser");
        }
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        total_bytes += src.len();
        let _ = parser.parse(&src, None);
        if i.is_multiple_of(500) {
            eprintln!(
                "[bench_tree_sitter_pool] parsed {}/{} bytes={}",
                i,
                files.len(),
                total_bytes
            );
        }
    }
    eprintln!("[bench_tree_sitter_pool] done bytes={}", total_bytes);
}

fn walk(dir: &std::path::Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "node_modules" || name == ".git" || name.starts_with(".") {
            continue;
        }
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
            out.push(p);
        }
    }
}
