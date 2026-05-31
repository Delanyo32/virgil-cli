//! Workspace + DiskFileSource only — no parsing, no Cozo.
//! Shows the baseline for "discover files + read bytes into LRU".
//! Pair with `bench_tree_sitter` to isolate parser overhead from
//! file-IO overhead.
//!
//! Usage:
//!   cargo run --release --example bench_workspace -- <dir>

use std::env;
use std::path::PathBuf;

use virgil_cli::language::Language;
use virgil_cli::storage::workspace::Workspace;

fn main() {
    let dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: bench_workspace <dir>");
    let ws = Workspace::load(&dir, Language::all(), None).expect("load");
    eprintln!("[bench_workspace] {} files discovered", ws.file_count());

    let mut bytes = 0usize;
    for path in ws.files().to_vec() {
        if let Some(src) = ws.read_file(&path) {
            bytes += src.len();
        }
    }
    eprintln!("[bench_workspace] read total bytes={bytes}");
}
