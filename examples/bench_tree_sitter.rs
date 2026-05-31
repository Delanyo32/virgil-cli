//! Tree-sitter arena footprint.
//!
//! Parses every file in the given directory once, then drops the parser.
//! Run under `/usr/bin/time -l` to compare peak RSS vs the full pipeline.
//!
//! Usage:
//!   cargo run --release --example bench_tree_sitter -- <dir> [<ext>]
//!
//! With and without a parser-pool reset to expose how much of the arena
//! is per-Parser leak.

use std::env;
use std::path::PathBuf;

use virgil_cli::language::Language;
use virgil_cli::parser::create_parser;

fn main() {
    let mut args = env::args().skip(1);
    let dir = args
        .next()
        .map(PathBuf::from)
        .expect("usage: bench_tree_sitter <dir> [<ext>]");
    let ext = args.next().unwrap_or_else(|| "ts".to_string());
    let lang = Language::from_extension(&ext).expect("unknown extension");

    let mut files: Vec<PathBuf> = Vec::new();
    walk(&dir, &ext, &mut files);
    files.sort();
    eprintln!("[bench_tree_sitter] {} files", files.len());

    let mut parser = create_parser(lang).expect("create parser");
    let mut total_bytes = 0usize;
    let mut total_nodes = 0u64;
    for (i, f) in files.iter().enumerate() {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        total_bytes += src.len();
        if let Some(tree) = parser.parse(&src, None) {
            total_nodes += count_nodes(tree.root_node());
        }
        if i.is_multiple_of(500) {
            eprintln!(
                "[bench_tree_sitter] parsed {}/{} bytes={} nodes={}",
                i,
                files.len(),
                total_bytes,
                total_nodes
            );
        }
    }
    eprintln!(
        "[bench_tree_sitter] done bytes={} nodes={}",
        total_bytes, total_nodes
    );
    // Hold parser alive until end so the arena shows up in peak RSS.
    drop(parser);
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

fn count_nodes(n: tree_sitter::Node) -> u64 {
    let mut c = 1u64;
    let mut cur = n.walk();
    for child in n.children(&mut cur) {
        c += count_nodes(child);
    }
    c
}
