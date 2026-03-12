use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::{format_output, format_section};

#[derive(Debug, Serialize)]
pub struct SymbolDefinition {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub is_exported: bool,
}

#[derive(Debug, Serialize)]
pub struct SymbolCaller {
    pub file_path: String,
    pub module_specifier: String,
}

#[derive(Debug, Serialize)]
pub struct SymbolDep {
    pub module_specifier: String,
    pub imported_name: String,
}

#[derive(Debug, Serialize)]
pub struct SymbolDetail {
    pub definitions: Vec<SymbolDefinition>,
    pub callers: Vec<SymbolCaller>,
    pub dependencies: Vec<SymbolDep>,
    pub import_count: i64,
    pub documentation: Vec<String>,
}

pub fn run_symbol_get(
    engine: &QueryEngine,
    symbol_name: &str,
    format: &OutputFormat,
) -> Result<String> {
    let escaped = symbol_name.replace('\'', "''");

    // 1. Symbol definitions
    let def_sql = format!(
        "SELECT name, kind, file_path, \
         CAST(start_line AS INTEGER) as start_line, \
         CAST(end_line AS INTEGER) as end_line, \
         is_exported \
         FROM symbols WHERE name = '{}' \
         ORDER BY file_path, start_line",
        escaped
    );

    let mut stmt = engine
        .conn
        .prepare(&def_sql)
        .context("failed to prepare symbol definition query")?;
    let definitions: Vec<SymbolDefinition> = stmt
        .query_map([], |row| {
            Ok(SymbolDefinition {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                is_exported: row.get(5)?,
            })
        })
        .context("failed to execute symbol definition query")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect symbol definitions")?;

    if definitions.is_empty() {
        anyhow::bail!("symbol '{}' not found", symbol_name);
    }

    // 2. Callers (files that import this symbol)
    let callers = if engine.has_imports() {
        let caller_sql = format!(
            "SELECT source_file AS file_path, module_specifier \
             FROM imports WHERE imported_name = '{}' \
             ORDER BY file_path LIMIT 20",
            escaped
        );
        let mut stmt = engine
            .conn
            .prepare(&caller_sql)
            .context("failed to prepare callers query")?;
        stmt.query_map([], |row| {
            Ok(SymbolCaller {
                file_path: row.get(0)?,
                module_specifier: row.get(1)?,
            })
        })
        .context("failed to execute callers query")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect callers")?
    } else {
        Vec::new()
    };

    // 3. Import count
    let import_count = if engine.has_imports() {
        let count_sql = format!(
            "SELECT COUNT(*) FROM imports WHERE imported_name = '{}'",
            escaped
        );
        engine
            .conn
            .query_row(&count_sql, [], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
    } else {
        0
    };

    // 4. Dependencies of the symbol's file(s)
    let file_paths: Vec<String> = definitions.iter().map(|d| d.file_path.clone()).collect();
    let dependencies = if engine.has_imports() && !file_paths.is_empty() {
        let file_list = file_paths
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        let dep_sql = format!(
            "SELECT DISTINCT module_specifier, imported_name \
             FROM imports WHERE source_file IN ({}) \
             ORDER BY module_specifier, imported_name",
            file_list
        );
        let mut stmt = engine
            .conn
            .prepare(&dep_sql)
            .context("failed to prepare dependencies query")?;
        stmt.query_map([], |row| {
            Ok(SymbolDep {
                module_specifier: row.get(0)?,
                imported_name: row.get(1)?,
            })
        })
        .context("failed to execute dependencies query")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect dependencies")?
    } else {
        Vec::new()
    };

    // 5. Documentation
    let documentation = if engine.has_comments() {
        let doc_sql = format!(
            "SELECT text FROM comments \
             WHERE associated_symbol = '{}' AND kind = 'doc' \
             ORDER BY file_path, start_line",
            escaped
        );
        let mut stmt = engine
            .conn
            .prepare(&doc_sql)
            .context("failed to prepare doc query")?;
        stmt.query_map([], |row| row.get::<_, String>(0))
            .context("failed to execute doc query")?
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let detail = SymbolDetail {
        definitions,
        callers,
        dependencies,
        import_count,
        documentation,
    };

    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&detail)?),
        OutputFormat::Csv => format_output(
            &detail.definitions,
            &[
                "name",
                "kind",
                "file_path",
                "start_line",
                "end_line",
                "is_exported",
            ],
            format,
        ),
        OutputFormat::Table => format_symbol_table(&detail),
    }
}

fn format_symbol_table(detail: &SymbolDetail) -> Result<String> {
    let mut out = String::new();

    // Definition section
    let def_content = format_output(
        &detail.definitions,
        &[
            "name",
            "kind",
            "file_path",
            "start_line",
            "end_line",
            "is_exported",
        ],
        &OutputFormat::Table,
    )?;
    out.push_str(&format_section("Definition", &def_content));

    // Usage stats
    out.push_str(&format_section(
        "Usage",
        &format!("Import references: {}", detail.import_count),
    ));

    // Callers
    if !detail.callers.is_empty() {
        let caller_content = format_output(
            &detail.callers,
            &["file_path", "module_specifier"],
            &OutputFormat::Table,
        )?;
        out.push_str(&format_section("Callers", &caller_content));
    }

    // Dependencies
    if !detail.dependencies.is_empty() {
        let dep_content = format_output(
            &detail.dependencies,
            &["module_specifier", "imported_name"],
            &OutputFormat::Table,
        )?;
        out.push_str(&format_section("File Dependencies", &dep_content));
    }

    // Documentation
    if !detail.documentation.is_empty() {
        let doc_text = detail.documentation.join("\n\n");
        out.push_str(&format_section("Documentation", &doc_text));
    }

    Ok(out)
}
