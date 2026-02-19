use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::{format_output, format_section};

#[derive(Debug, Serialize)]
pub struct LanguageBreakdown {
    pub language: String,
    pub file_count: i64,
    pub total_bytes: i64,
    pub total_lines: i64,
}

#[derive(Debug, Serialize)]
pub struct TopSymbol {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line_span: i64,
}

#[derive(Debug, Serialize)]
pub struct DirectoryBreakdown {
    pub directory: String,
    pub file_count: i64,
}

pub fn run_overview(engine: &QueryEngine, format: &OutputFormat) -> Result<String> {
    let languages = query_language_breakdown(engine)?;
    let top_symbols = query_top_symbols(engine)?;
    let directories = query_directory_breakdown(engine)?;

    let lang_section = format_output(
        &languages,
        &["language", "file_count", "total_bytes", "total_lines"],
        format,
    )?;

    let sym_section = format_output(
        &top_symbols,
        &["name", "kind", "file_path", "line_span"],
        format,
    )?;

    let dir_section = format_output(
        &directories,
        &["directory", "file_count"],
        format,
    )?;

    match format {
        OutputFormat::Json => {
            let combined = serde_json::json!({
                "languages": languages,
                "top_symbols": top_symbols,
                "directories": directories,
            });
            Ok(serde_json::to_string_pretty(&combined)?)
        }
        _ => {
            let mut out = String::new();
            out.push_str(&format_section("Languages", &lang_section));
            out.push_str(&format_section("Top Symbols (by line span)", &sym_section));
            out.push_str(&format_section("Directories", &dir_section));
            Ok(out)
        }
    }
}

fn query_language_breakdown(engine: &QueryEngine) -> Result<Vec<LanguageBreakdown>> {
    let sql = format!(
        "SELECT language, COUNT(*) as file_count, \
         SUM(size_bytes) as total_bytes, SUM(line_count) as total_lines \
         FROM read_parquet('{}') GROUP BY language ORDER BY file_count DESC",
        engine.files_parquet()
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare language breakdown query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LanguageBreakdown {
                language: row.get(0)?,
                file_count: row.get(1)?,
                total_bytes: row.get::<_, i64>(2).unwrap_or(0),
                total_lines: row.get::<_, i64>(3).unwrap_or(0),
            })
        })
        .context("failed to query language breakdown")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect language breakdown rows")
}

fn query_top_symbols(engine: &QueryEngine) -> Result<Vec<TopSymbol>> {
    let sql = format!(
        "SELECT name, kind, file_path, \
         CAST(end_line AS INTEGER) - CAST(start_line AS INTEGER) as line_span \
         FROM read_parquet('{}') \
         ORDER BY line_span DESC LIMIT 10",
        engine.symbols_parquet()
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare top symbols query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TopSymbol {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                line_span: row.get(3)?,
            })
        })
        .context("failed to query top symbols")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect top symbol rows")
}

fn query_directory_breakdown(engine: &QueryEngine) -> Result<Vec<DirectoryBreakdown>> {
    let sql = format!(
        "SELECT CASE WHEN position('/' IN path) > 0 \
         THEN regexp_replace(path, '/[^/]+$', '') \
         ELSE '.' END as directory, \
         COUNT(*) as file_count \
         FROM read_parquet('{}') \
         GROUP BY directory ORDER BY file_count DESC",
        engine.files_parquet()
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare directory breakdown query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DirectoryBreakdown {
                directory: row.get(0)?,
                file_count: row.get(1)?,
            })
        })
        .context("failed to query directory breakdown")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect directory breakdown rows")
}
