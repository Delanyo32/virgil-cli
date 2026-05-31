//! Cost of loading every language grammar (a `tree_sitter::Parser` per
//! `Language` × pre-compiled `Query` for symbol/import/comment).
//! Mirrors what the real pipeline pays before a single file is parsed.
//!
//! Usage:
//!   cargo run --release --example bench_grammars
//!   cargo run --release --example bench_grammars -- N    # also build N parsers per lang

use std::env;

use virgil_cli::language::Language;
use virgil_cli::languages;
use virgil_cli::parser::create_parser;

fn main() {
    let parsers_per_lang: usize = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    let langs = Language::all();
    eprintln!(
        "[bench_grammars] {} languages, {} parsers each",
        langs.len(),
        parsers_per_lang
    );

    let mut parsers = Vec::new();
    let mut queries = Vec::new();
    for &lang in langs {
        for _ in 0..parsers_per_lang {
            parsers.push(create_parser(lang).expect("parser"));
        }
        queries.push(languages::compile_symbol_query(lang).expect("symbol query"));
        queries.push(languages::compile_import_query(lang).expect("import query"));
        if let Ok(q) = languages::compile_comment_query(lang) {
            queries.push(q);
        }
    }
    eprintln!(
        "[bench_grammars] held {} parsers + {} queries",
        parsers.len(),
        queries.len()
    );
    // Hold everything alive until exit.
    drop(parsers);
    drop(queries);
}
