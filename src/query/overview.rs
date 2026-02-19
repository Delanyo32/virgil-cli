use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_section;

// ── Data structs ──

#[derive(Debug, Serialize)]
struct OverviewSummary {
    total_files: i64,
    total_lines: i64,
    total_bytes: i64,
    total_symbols: i64,
    exported_symbols: i64,
    languages: Vec<LanguageCount>,
}

#[derive(Debug, Serialize, Clone)]
struct LanguageCount {
    language: String,
    count: i64,
}

#[derive(Debug, Serialize)]
pub struct TopSymbol {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line_span: i64,
}

#[derive(Debug, Serialize, Clone)]
struct ExportedSymbol {
    name: String,
    kind: String,
}

#[derive(Debug, Serialize)]
struct FileExportRow {
    directory: String,
    file_name: String,
    file_path: String,
    symbol_name: String,
    kind: String,
}

#[derive(Debug, Serialize)]
struct DirStats {
    file_count: i64,
    total_lines: i64,
}

#[derive(Debug, Serialize)]
struct ApiSurfaceEntry {
    kind: String,
    count: i64,
    examples: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Insight {
    label: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct ModuleTreeNode {
    path: String,
    name: String,
    file_count: i64,
    total_lines: i64,
    files: Vec<ModuleFile>,
    children: Vec<ModuleTreeNode>,
}

#[derive(Debug, Serialize)]
struct ModuleFile {
    name: String,
    exports: Vec<ExportedSymbol>,
    total_exports: usize,
}

// ── Query functions ──

fn query_summary(engine: &QueryEngine) -> Result<OverviewSummary> {
    // File totals
    let mut stmt = engine
        .conn
        .prepare(
            "SELECT COUNT(*) AS total_files, \
             COALESCE(SUM(line_count),0) AS total_lines, \
             COALESCE(SUM(size_bytes),0) AS total_bytes \
             FROM files",
        )
        .context("failed to prepare summary file query")?;
    let (total_files, total_lines, total_bytes) = stmt
        .query_row([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .context("failed to query file summary")?;

    // Symbol totals
    let mut stmt = engine
        .conn
        .prepare(
            "SELECT COUNT(*) AS total_symbols, \
             COALESCE(SUM(CASE WHEN is_exported THEN 1 ELSE 0 END),0) AS exported \
             FROM symbols",
        )
        .context("failed to prepare summary symbol query")?;
    let (total_symbols, exported_symbols) = stmt
        .query_row([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .context("failed to query symbol summary")?;

    // Language breakdown
    let mut stmt = engine
        .conn
        .prepare(
            "SELECT language, COUNT(*) AS file_count \
             FROM files GROUP BY language ORDER BY file_count DESC",
        )
        .context("failed to prepare language query")?;
    let languages = stmt
        .query_map([], |row| {
            Ok(LanguageCount {
                language: row.get(0)?,
                count: row.get(1)?,
            })
        })
        .context("failed to query languages")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect languages")?;

    Ok(OverviewSummary {
        total_files,
        total_lines,
        total_bytes,
        total_symbols,
        exported_symbols,
        languages,
    })
}

fn query_file_exports(engine: &QueryEngine) -> Result<Vec<FileExportRow>> {
    let sql = "SELECT \
        CASE WHEN position('/' IN file_path)>0 \
             THEN regexp_replace(file_path,'/[^/]+$','') ELSE '.' END AS directory, \
        regexp_replace(file_path, '^.*/', '') AS file_name, \
        file_path, name AS symbol_name, kind \
        FROM symbols WHERE is_exported = true \
        ORDER BY directory, file_name, kind, name";

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare file exports query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FileExportRow {
                directory: row.get(0)?,
                file_name: row.get(1)?,
                file_path: row.get(2)?,
                symbol_name: row.get(3)?,
                kind: row.get(4)?,
            })
        })
        .context("failed to query file exports")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect file exports")
}

fn query_directory_stats(engine: &QueryEngine) -> Result<BTreeMap<String, DirStats>> {
    let sql = "SELECT \
        CASE WHEN position('/' IN path)>0 \
             THEN regexp_replace(path,'/[^/]+$','') ELSE '.' END AS directory, \
        COUNT(*) AS file_count, COALESCE(SUM(line_count),0) AS total_lines \
        FROM files GROUP BY directory ORDER BY directory";

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare directory stats query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                DirStats {
                    file_count: row.get(1)?,
                    total_lines: row.get(2)?,
                },
            ))
        })
        .context("failed to query directory stats")?;

    let mut map = BTreeMap::new();
    for row in rows {
        let (dir, stats) = row.context("failed to read directory stats row")?;
        map.insert(dir, stats);
    }
    Ok(map)
}

fn query_api_surface(engine: &QueryEngine) -> Result<Vec<ApiSurfaceEntry>> {
    let sql = "SELECT kind, COUNT(*) AS count, \
        STRING_AGG(name, ',' ORDER BY name) AS all_names \
        FROM symbols WHERE is_exported = true \
        GROUP BY kind ORDER BY count DESC";

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare API surface query")?;
    let rows = stmt
        .query_map([], |row| {
            let kind: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            let all_names: String = row.get::<_, String>(2).unwrap_or_default();
            let examples: Vec<String> = all_names
                .split(',')
                .take(5)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Ok(ApiSurfaceEntry {
                kind,
                count,
                examples,
            })
        })
        .context("failed to query API surface")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect API surface")
}

fn query_top_symbols(engine: &QueryEngine) -> Result<Vec<TopSymbol>> {
    let sql = "SELECT name, kind, file_path, \
         CAST(end_line AS INTEGER) - CAST(start_line AS INTEGER) as line_span \
         FROM symbols \
         ORDER BY line_span DESC LIMIT 5";

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare top symbols query")?;
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

fn query_insights(engine: &QueryEngine, summary: &OverviewSummary) -> Result<Vec<Insight>> {
    let mut insights = Vec::new();

    // Export ratio
    if summary.total_symbols > 0 {
        let pct = (summary.exported_symbols as f64 / summary.total_symbols as f64 * 100.0) as i64;
        insights.push(Insight {
            label: "Export ratio".to_string(),
            value: format!(
                "{}% of symbols exported ({}/{})",
                pct, summary.exported_symbols, summary.total_symbols
            ),
        });
    }

    // Largest file
    let mut stmt = engine
        .conn
        .prepare("SELECT path, line_count FROM files ORDER BY line_count DESC LIMIT 1")
        .context("failed to prepare largest file query")?;
    if let Ok((path, lines)) = stmt.query_row([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        insights.push(Insight {
            label: "Largest file".to_string(),
            value: format!("{} ({} lines)", path, lines),
        });
    }

    // Deepest path
    let mut stmt = engine
        .conn
        .prepare(
            "SELECT path, \
             LENGTH(path) - LENGTH(REPLACE(path, '/', '')) AS depth \
             FROM files ORDER BY depth DESC LIMIT 1",
        )
        .context("failed to prepare deepest path query")?;
    if let Ok((path, depth)) = stmt.query_row([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        let dir = if let Some(pos) = path.rfind('/') {
            &path[..pos]
        } else {
            "."
        };
        insights.push(Insight {
            label: "Deepest path".to_string(),
            value: format!("{} (depth {})", dir, depth),
        });
    }

    // Hotspot — directory with most symbols
    let mut stmt = engine
        .conn
        .prepare(
            "SELECT \
             CASE WHEN position('/' IN file_path)>0 \
                  THEN regexp_replace(file_path,'/[^/]+$','') ELSE '.' END AS directory, \
             COUNT(*) AS sym_count \
             FROM symbols GROUP BY directory ORDER BY sym_count DESC LIMIT 1",
        )
        .context("failed to prepare hotspot query")?;
    if let Ok((dir, sym_count)) = stmt.query_row([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        if summary.total_symbols > 0 {
            let pct = (sym_count as f64 / summary.total_symbols as f64 * 100.0) as i64;
            insights.push(Insight {
                label: "Hotspot".to_string(),
                value: format!("{} has {}% of all symbols", dir, pct),
            });
        }
    }

    Ok(insights)
}

// ── Tree building ──

/// Compute directory depth: "." is 0, "src" is 1, "src/db" is 2, etc.
fn dir_depth(path: &str) -> usize {
    if path == "." {
        0
    } else {
        path.matches('/').count() + 1
    }
}

fn build_module_tree(
    file_exports: &[FileExportRow],
    dir_stats: &BTreeMap<String, DirStats>,
    max_depth: usize,
) -> Vec<ModuleTreeNode> {
    // Group exports by file_path
    let mut file_export_map: BTreeMap<String, Vec<ExportedSymbol>> = BTreeMap::new();
    let mut file_dir_map: BTreeMap<String, String> = BTreeMap::new();
    let mut file_name_map: BTreeMap<String, String> = BTreeMap::new();

    for row in file_exports {
        file_export_map
            .entry(row.file_path.clone())
            .or_default()
            .push(ExportedSymbol {
                name: row.symbol_name.clone(),
                kind: row.kind.clone(),
            });
        file_dir_map.insert(row.file_path.clone(), row.directory.clone());
        file_name_map.insert(row.file_path.clone(), row.file_name.clone());
    }

    // Build nested tree structure from directory stats
    // Key: directory path, Value: tree node
    let mut tree: BTreeMap<String, ModuleTreeNode> = BTreeMap::new();

    // Initialize nodes for all directories that have files
    for (dir, stats) in dir_stats {
        tree.insert(
            dir.clone(),
            ModuleTreeNode {
                path: dir.clone(),
                name: dir.rsplit('/').next().unwrap_or(dir).to_string(),
                file_count: stats.file_count,
                total_lines: stats.total_lines,
                files: Vec::new(),
                children: Vec::new(),
            },
        );
    }

    // Synthesize intermediate directory nodes so every path segment has a node.
    // e.g. if "packages/parser/src" exists, ensure "packages" and "packages/parser"
    // also exist (with zeroed stats — they'll aggregate from children in display).
    let existing_dirs: Vec<String> = tree.keys().cloned().collect();
    for dir in &existing_dirs {
        let mut current = dir.as_str();
        while let Some(pos) = current.rfind('/') {
            let parent = &current[..pos];
            if tree.contains_key(parent) {
                break; // already exists, and all its ancestors will too
            }
            tree.insert(
                parent.to_string(),
                ModuleTreeNode {
                    path: parent.to_string(),
                    name: parent.rsplit('/').next().unwrap_or(parent).to_string(),
                    file_count: 0,
                    total_lines: 0,
                    files: Vec::new(),
                    children: Vec::new(),
                },
            );
            current = parent;
        }
    }

    // Attach files with exports to their directories
    for (file_path, exports) in &file_export_map {
        let dir = file_dir_map.get(file_path).unwrap();
        let file_name = file_name_map.get(file_path).unwrap();
        let total_exports = exports.len();
        if let Some(node) = tree.get_mut(dir.as_str()) {
            node.files.push(ModuleFile {
                name: file_name.clone(),
                exports: exports.clone(),
                total_exports,
            });
        }
    }

    // Build parent-child relationships: each dir's parent is the longest
    // prefix that also exists in the tree (always the immediate parent now
    // that we've synthesized intermediates).
    let dirs: Vec<String> = tree.keys().cloned().collect();
    let mut children_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut has_parent: std::collections::HashSet<String> = std::collections::HashSet::new();

    for dir in &dirs {
        if let Some(pos) = dir.rfind('/') {
            let parent = &dir[..pos];
            if tree.contains_key(parent) {
                children_map
                    .entry(parent.to_string())
                    .or_default()
                    .push(dir.clone());
                has_parent.insert(dir.clone());
            }
        }
    }

    // Assemble tree recursively, using actual directory depth for the limit.
    fn assemble(
        dir: &str,
        tree: &mut BTreeMap<String, ModuleTreeNode>,
        children_map: &BTreeMap<String, Vec<String>>,
        max_depth: usize,
    ) -> Option<ModuleTreeNode> {
        let mut node = tree.remove(dir)?;

        if let Some(child_dirs) = children_map.get(dir) {
            for child_dir in child_dirs {
                // Use actual directory depth to decide whether to recurse
                if dir_depth(child_dir) <= max_depth {
                    if let Some(child) = assemble(child_dir, tree, children_map, max_depth) {
                        node.children.push(child);
                    }
                }
            }
        }

        // Roll up stats from children into synthetic intermediate nodes
        if node.file_count == 0 && node.total_lines == 0 {
            for child in &node.children {
                node.file_count += aggregate_files(child);
                node.total_lines += aggregate_lines(child);
            }
        }

        Some(node)
    }

    fn aggregate_files(node: &ModuleTreeNode) -> i64 {
        let mut total = node.file_count;
        for child in &node.children {
            total += aggregate_files(child);
        }
        total
    }

    fn aggregate_lines(node: &ModuleTreeNode) -> i64 {
        let mut total = node.total_lines;
        for child in &node.children {
            total += aggregate_lines(child);
        }
        total
    }

    // Find root directories (those without parents in our set)
    let roots: Vec<String> = dirs
        .into_iter()
        .filter(|d| !has_parent.contains(d))
        .collect();

    let mut result = Vec::new();
    for root in &roots {
        if let Some(node) = assemble(root, &mut tree, &children_map, max_depth) {
            result.push(node);
        }
    }

    result
}

// ── Format functions ──

fn kind_abbrev(kind: &str) -> &str {
    match kind {
        "function" => "F",
        "class" => "C",
        "method" => "M",
        "variable" => "V",
        "interface" => "I",
        "type_alias" => "T",
        "enum" => "E",
        "arrow_function" => "A",
        _ => "?",
    }
}

fn format_summary_line(summary: &OverviewSummary) -> String {
    let lang_parts: Vec<String> = summary
        .languages
        .iter()
        .take(3)
        .map(|l| format!("{} ({})", l.language, l.count))
        .collect();
    let lang_str = if summary.languages.len() > 3 {
        format!(
            "{}, +{} more",
            lang_parts.join(", "),
            summary.languages.len() - 3
        )
    } else {
        lang_parts.join(", ")
    };

    format!(
        "{} files | {} lines | {} | {} symbols ({} exported)\n",
        summary.total_files,
        format_number(summary.total_lines),
        lang_str,
        summary.total_symbols,
        summary.exported_symbols,
    )
}

fn format_number(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_module_tree(nodes: &[ModuleTreeNode], indent: usize) -> String {
    let mut out = String::new();
    let prefix = "  ".repeat(indent);

    for node in nodes {
        // Directory line
        out.push_str(&format!(
            "{}{:<42} {:>3} files  {:>6} lines\n",
            prefix,
            format!("{}/", node.path),
            node.file_count,
            format_number(node.total_lines),
        ));

        // Files with exports (max 20 files shown per directory)
        let file_indent = format!("{}  ", prefix);
        let files_to_show = if node.files.len() > 20 {
            &node.files[..20]
        } else {
            &node.files
        };

        for file in files_to_show {
            let export_strs: Vec<String> = file
                .exports
                .iter()
                .take(5)
                .map(|e| format!("[{}] {}", kind_abbrev(&e.kind), e.name))
                .collect();
            let mut export_line = export_strs.join(", ");
            if file.total_exports > 5 {
                export_line.push_str(&format!(", +{}", file.total_exports - 5));
            }
            out.push_str(&format!(
                "{}{:<40} {}\n",
                file_indent, file.name, export_line
            ));
        }
        if node.files.len() > 20 {
            out.push_str(&format!(
                "{}({} more files)\n",
                file_indent,
                node.files.len() - 20
            ));
        }

        // Recurse into children
        if !node.children.is_empty() {
            out.push_str(&format_module_tree(&node.children, indent + 1));
        }
    }

    out
}

fn format_api_surface(entries: &[ApiSurfaceEntry], total_exported: i64) -> String {
    let mut out = String::new();
    for entry in entries {
        let examples_str = entry.examples.join(", ");
        let suffix = if entry.count as usize > entry.examples.len() {
            ", ..."
        } else {
            ""
        };
        out.push_str(&format!(
            "  {:<14} {:>4}  {}{}\n",
            entry.kind, entry.count, examples_str, suffix
        ));
    }
    let _ = total_exported; // used in section header
    out
}

fn format_top_symbols(symbols: &[TopSymbol]) -> String {
    if symbols.is_empty() {
        return "(no symbols)\n".to_string();
    }
    let mut out = String::new();
    for s in symbols {
        out.push_str(&format!(
            "  {:<30} {:<16} {:<40} {} lines\n",
            s.name, s.kind, s.file_path, s.line_span
        ));
    }
    out
}

fn format_insights(insights: &[Insight]) -> String {
    let mut out = String::new();
    for insight in insights {
        out.push_str(&format!("  {}: {}\n", insight.label, insight.value));
    }
    out
}

// ── Entry point ──

pub fn run_overview(engine: &QueryEngine, format: &OutputFormat, depth: usize) -> Result<String> {
    let summary = query_summary(engine)?;
    let file_exports = query_file_exports(engine)?;
    let dir_stats = query_directory_stats(engine)?;
    let api_surface = query_api_surface(engine)?;
    let top_symbols = query_top_symbols(engine)?;
    let insights = query_insights(engine, &summary)?;
    let module_tree = build_module_tree(&file_exports, &dir_stats, depth);

    match format {
        OutputFormat::Json => {
            let combined = serde_json::json!({
                "summary": summary,
                "module_tree": module_tree,
                "api_surface": api_surface,
                "largest_symbols": top_symbols,
                "insights": insights,
            });
            Ok(serde_json::to_string_pretty(&combined)?)
        }
        OutputFormat::Csv => {
            // Flat file-level rows for CSV
            let mut out = String::new();
            out.push_str("directory,file,language,lines,exported_count,exported_symbols\n");

            let sql = "SELECT \
                CASE WHEN position('/' IN f.path)>0 \
                     THEN regexp_replace(f.path,'/[^/]+$','') ELSE '.' END AS directory, \
                f.name AS file, f.language, f.line_count AS lines, \
                COUNT(CASE WHEN s.is_exported THEN 1 END) AS exported_count, \
                COALESCE(STRING_AGG(CASE WHEN s.is_exported THEN s.name END, ',' ORDER BY s.name),'') AS exported_symbols \
                FROM files f LEFT JOIN symbols s ON f.path = s.file_path \
                GROUP BY f.path, f.name, f.language, f.line_count \
                ORDER BY directory, file";

            let mut stmt = engine
                .conn
                .prepare(sql)
                .context("failed to prepare CSV query")?;
            let mut rows = stmt.query([]).context("failed to execute CSV query")?;

            while let Some(row) = rows.next().context("failed to fetch CSV row")? {
                let dir: String = row.get(0)?;
                let file: String = row.get(1)?;
                let lang: String = row.get(2)?;
                let lines: i64 = row.get(3)?;
                let exp_count: i64 = row.get(4)?;
                let exp_syms: String = row.get(5)?;

                // CSV-safe quoting
                let exp_syms_safe = if exp_syms.contains(',') || exp_syms.contains('"') {
                    format!("\"{}\"", exp_syms.replace('"', "\"\""))
                } else {
                    exp_syms
                };
                out.push_str(&format!(
                    "{},{},{},{},{},{}\n",
                    dir, file, lang, lines, exp_count, exp_syms_safe
                ));
            }

            Ok(out)
        }
        OutputFormat::Table => {
            let mut out = String::new();

            // Section 1: Summary
            out.push_str(&format_section("Summary", &format_summary_line(&summary)));

            // Section 2: Module Tree
            if !module_tree.is_empty() {
                out.push_str(&format_section(
                    "Module Tree",
                    &format_module_tree(&module_tree, 0),
                ));
            }

            // Section 3: API Surface
            if !api_surface.is_empty() {
                let header = format!("API Surface ({} exported)", summary.exported_symbols);
                out.push_str(&format_section(
                    &header,
                    &format_api_surface(&api_surface, summary.exported_symbols),
                ));
            }

            // Section 4: Largest Symbols
            if !top_symbols.is_empty() {
                out.push_str(&format_section(
                    "Largest Symbols",
                    &format_top_symbols(&top_symbols),
                ));
            }

            // Section 5: Insights
            if !insights.is_empty() {
                out.push_str(&format_section("Insights", &format_insights(&insights)));
            }

            Ok(out)
        }
    }
}
