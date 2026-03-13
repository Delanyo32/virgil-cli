use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::{CouplingSortField, OutputFormat};
use crate::query::db::QueryEngine;
use crate::query::format::{format_output, format_section};

// --- Dead Code ---

#[derive(Debug, Serialize)]
struct DeadCodeRow {
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
    end_line: u32,
}

pub fn run_dead_code(
    engine: &QueryEngine,
    file: Option<&str>,
    kind: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_imports() {
        return Ok("No import data available. Dead code analysis requires imports.\n".to_string());
    }

    let mut conditions = vec!["s.is_exported = true".to_string(), "i.imported_name IS NULL".to_string()];

    if let Some(f) = file {
        conditions.push(format!("s.file_path LIKE '{}%'", f.replace('\'', "''")));
    }
    if let Some(k) = kind {
        conditions.push(format!("s.kind = '{}'", k.replace('\'', "''")));
    }

    let sql = format!(
        "SELECT s.name, s.kind, s.file_path, s.start_line, s.end_line \
         FROM symbols s \
         LEFT JOIN ( \
             SELECT DISTINCT imported_name FROM imports WHERE is_external = false \
         ) i ON i.imported_name = s.name \
         WHERE {} \
         ORDER BY s.file_path, s.start_line \
         LIMIT {}",
        conditions.join(" AND "),
        limit
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare dead code query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DeadCodeRow {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
            })
        })
        .context("failed to query dead code")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect dead code rows")?;

    let headers = &["name", "kind", "file_path", "start_line", "end_line"];
    format_output(&rows, headers, format)
}

// --- Coupling & Cohesion ---

#[derive(Debug, Serialize)]
struct CouplingRow {
    file_path: String,
    fan_in: i64,
    fan_out: i64,
    instability: f64,
}

pub fn run_coupling(
    engine: &QueryEngine,
    file: Option<&str>,
    sort: &CouplingSortField,
    limit: usize,
    cycles: bool,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_imports() {
        return Ok("No import data available. Coupling analysis requires imports.\n".to_string());
    }

    if cycles {
        return run_coupling_cycles(engine, format);
    }

    let file_filter = if let Some(f) = file {
        format!(
            " WHERE file_path LIKE '{}%'",
            f.replace('\'', "''")
        )
    } else {
        String::new()
    };

    let order = match sort {
        CouplingSortField::Instability => "instability DESC NULLS LAST",
        CouplingSortField::FanIn => "fan_in DESC",
        CouplingSortField::FanOut => "fan_out DESC",
        CouplingSortField::File => "file_path ASC",
    };

    let sql = format!(
        "WITH fan_out AS ( \
             SELECT source_file AS file_path, COUNT(DISTINCT module_specifier) AS fan_out \
             FROM imports WHERE is_external = false GROUP BY source_file \
         ), \
         fan_in AS ( \
             SELECT module_specifier AS file_path, COUNT(DISTINCT source_file) AS fan_in \
             FROM imports WHERE is_external = false GROUP BY module_specifier \
         ), \
         combined AS ( \
             SELECT COALESCE(fo.file_path, fi.file_path) AS file_path, \
                    COALESCE(fi.fan_in, 0) AS fan_in, \
                    COALESCE(fo.fan_out, 0) AS fan_out, \
                    ROUND(COALESCE(fo.fan_out, 0)::DOUBLE / \
                          NULLIF(COALESCE(fi.fan_in, 0) + COALESCE(fo.fan_out, 0), 0), 2) AS instability \
             FROM fan_out fo FULL OUTER JOIN fan_in fi ON fo.file_path = fi.file_path \
         ) \
         SELECT file_path, fan_in, fan_out, instability \
         FROM combined{} \
         ORDER BY {} \
         LIMIT {}",
        file_filter, order, limit
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare coupling query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CouplingRow {
                file_path: row.get(0)?,
                fan_in: row.get(1)?,
                fan_out: row.get(2)?,
                instability: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
            })
        })
        .context("failed to query coupling")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect coupling rows")?;

    let headers = &["file_path", "fan_in", "fan_out", "instability"];
    format_output(&rows, headers, format)
}

// --- Circular Dependencies (Tarjan's SCC) ---

fn run_coupling_cycles(engine: &QueryEngine, format: &OutputFormat) -> Result<String> {
    let sql = "SELECT DISTINCT source_file, module_specifier \
               FROM imports WHERE is_external = false";

    let mut stmt = engine.conn.prepare(sql).context("failed to prepare cycle query")?;
    let edges = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to query edges")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect edges")?;

    // Build adjacency list
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut all_nodes: Vec<String> = Vec::new();
    for (from, to) in &edges {
        adj.entry(from.clone()).or_default().push(to.clone());
        if !all_nodes.contains(from) {
            all_nodes.push(from.clone());
        }
        if !all_nodes.contains(to) {
            all_nodes.push(to.clone());
        }
    }

    // Tarjan's SCC
    let sccs = tarjan_scc(&all_nodes, &adj);
    let cycles: Vec<_> = sccs.into_iter().filter(|scc| scc.len() > 1).collect();

    #[derive(Debug, Serialize)]
    struct CycleRow {
        cycle_id: usize,
        size: usize,
        files: String,
    }

    let rows: Vec<CycleRow> = cycles
        .iter()
        .enumerate()
        .map(|(i, scc)| CycleRow {
            cycle_id: i + 1,
            size: scc.len(),
            files: scc.join(", "),
        })
        .collect();

    if rows.is_empty() {
        return Ok("No circular dependencies found.\n".to_string());
    }

    let headers = &["cycle_id", "size", "files"];
    format_output(&rows, headers, format)
}

fn tarjan_scc(nodes: &[String], adj: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    struct TarjanState<'a> {
        adj: &'a HashMap<String, Vec<String>>,
        index_counter: usize,
        stack: Vec<String>,
        on_stack: HashMap<String, bool>,
        index: HashMap<String, usize>,
        lowlink: HashMap<String, usize>,
        result: Vec<Vec<String>>,
    }

    fn strongconnect(v: &str, state: &mut TarjanState) {
        let idx = state.index_counter;
        state.index.insert(v.to_string(), idx);
        state.lowlink.insert(v.to_string(), idx);
        state.index_counter += 1;
        state.stack.push(v.to_string());
        state.on_stack.insert(v.to_string(), true);

        if let Some(neighbors) = state.adj.get(v) {
            for w in neighbors {
                if !state.index.contains_key(w.as_str()) {
                    strongconnect(w, state);
                    let w_low = state.lowlink[w.as_str()];
                    let v_low = state.lowlink[v];
                    if w_low < v_low {
                        state.lowlink.insert(v.to_string(), w_low);
                    }
                } else if state.on_stack.get(w.as_str()).copied().unwrap_or(false) {
                    let w_idx = state.index[w.as_str()];
                    let v_low = state.lowlink[v];
                    if w_idx < v_low {
                        state.lowlink.insert(v.to_string(), w_idx);
                    }
                }
            }
        }

        if state.lowlink[v] == state.index[v] {
            let mut scc = Vec::new();
            loop {
                let w = state.stack.pop().unwrap();
                state.on_stack.insert(w.clone(), false);
                scc.push(w.clone());
                if w == v {
                    break;
                }
            }
            state.result.push(scc);
        }
    }

    let mut state = TarjanState {
        adj,
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashMap::new(),
        index: HashMap::new(),
        lowlink: HashMap::new(),
        result: Vec::new(),
    };

    for node in nodes {
        if !state.index.contains_key(node.as_str()) {
            strongconnect(node, &mut state);
        }
    }

    state.result
}

// --- Duplication ---

#[derive(Debug, Serialize)]
struct DuplicationRow {
    structural_hash: u64,
    symbol_kind: String,
    line_count: i64,
    cyclomatic: i64,
    cognitive: i64,
    group_size: i64,
    instances: String,
}

pub fn run_duplication(
    engine: &QueryEngine,
    file: Option<&str>,
    min_group: usize,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_complexity() {
        return Ok("No complexity data available. Duplication analysis requires an audit with complexity data.\n".to_string());
    }

    let file_filter = if let Some(f) = file {
        format!(
            " AND file_path LIKE '{}%'",
            f.replace('\'', "''")
        )
    } else {
        String::new()
    };

    let sql = format!(
        "SELECT structural_hash, symbol_kind, line_count, \
         cyclomatic_complexity, cognitive_complexity, \
         COUNT(*) AS group_size, \
         STRING_AGG(symbol_name || ' (' || file_path || ':' || CAST(start_line AS VARCHAR) || ')', ', ') AS instances \
         FROM complexity \
         WHERE structural_hash != 0{} \
         GROUP BY structural_hash, symbol_kind, line_count, cyclomatic_complexity, cognitive_complexity \
         HAVING COUNT(*) >= {} \
         ORDER BY group_size DESC, line_count DESC \
         LIMIT {}",
        file_filter, min_group, limit
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare duplication query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DuplicationRow {
                structural_hash: row.get(0)?,
                symbol_kind: row.get(1)?,
                line_count: row.get(2)?,
                cyclomatic: row.get(3)?,
                cognitive: row.get(4)?,
                group_size: row.get(5)?,
                instances: row.get(6)?,
            })
        })
        .context("failed to query duplication")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect duplication rows")?;

    if rows.is_empty() {
        return Ok("No duplicate function structures found.\n".to_string());
    }

    let headers = &[
        "structural_hash",
        "symbol_kind",
        "line_count",
        "cyclomatic",
        "cognitive",
        "group_size",
        "instances",
    ];
    format_output(&rows, headers, format)
}

// --- Combined Audit Overview (complexity + quality) ---

pub fn run_audit_overview(engine: &QueryEngine, format: &OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => run_audit_overview_json(engine),
        OutputFormat::Csv => run_audit_overview_csv(engine),
        OutputFormat::Table => run_audit_overview_table(engine),
    }
}

// -- Complexity overview structs --

#[derive(Debug, Serialize)]
struct ComplexityStats {
    total_symbols: i64,
    avg_cyclomatic: f64,
    max_cyclomatic: i64,
    avg_cognitive: f64,
    max_cognitive: i64,
    avg_line_count: f64,
    max_line_count: i64,
}

#[derive(Debug, Serialize)]
struct ComplexityBucket {
    range: String,
    count: i64,
}

#[derive(Debug, Serialize)]
struct TopComplexSymbol {
    symbol_name: String,
    file_path: String,
    lines: i64,
    cyclomatic: i64,
    cognitive: i64,
}

#[derive(Debug, Serialize)]
struct FileComplexity {
    file_path: String,
    symbol_count: i64,
    avg_cyclomatic: f64,
    max_cyclomatic: i64,
    total_cognitive: i64,
}

fn complexity_stats(engine: &QueryEngine) -> Result<Option<ComplexityStats>> {
    if !engine.has_complexity() {
        return Ok(None);
    }
    let mut stmt = engine.conn.prepare(
        "SELECT COUNT(*), \
         ROUND(AVG(cyclomatic_complexity), 1), MAX(cyclomatic_complexity), \
         ROUND(AVG(cognitive_complexity), 1), MAX(cognitive_complexity), \
         ROUND(AVG(line_count), 1), MAX(line_count) \
         FROM complexity",
    ).context("complexity stats query")?;
    let stats = stmt.query_row([], |row| {
        Ok(ComplexityStats {
            total_symbols: row.get(0)?,
            avg_cyclomatic: row.get(1)?,
            max_cyclomatic: row.get(2)?,
            avg_cognitive: row.get(3)?,
            max_cognitive: row.get(4)?,
            avg_line_count: row.get(5)?,
            max_line_count: row.get(6)?,
        })
    }).context("complexity stats")?;
    Ok(Some(stats))
}

fn complexity_buckets(engine: &QueryEngine) -> Result<Vec<ComplexityBucket>> {
    if !engine.has_complexity() {
        return Ok(Vec::new());
    }
    let mut stmt = engine.conn.prepare(
        "SELECT \
         CASE \
           WHEN cyclomatic_complexity BETWEEN 1 AND 5 THEN '1-5 (simple)' \
           WHEN cyclomatic_complexity BETWEEN 6 AND 10 THEN '6-10 (moderate)' \
           WHEN cyclomatic_complexity BETWEEN 11 AND 20 THEN '11-20 (complex)' \
           ELSE '21+ (very complex)' \
         END AS range, \
         COUNT(*) AS count \
         FROM complexity \
         GROUP BY range \
         ORDER BY MIN(cyclomatic_complexity)",
    ).context("distribution query")?;
    stmt.query_map([], |row| {
        Ok(ComplexityBucket {
            range: row.get(0)?,
            count: row.get(1)?,
        })
    }).context("distribution")?
    .collect::<Result<Vec<_>, _>>()
    .context("collect distribution")
}

fn top_complex_symbols(engine: &QueryEngine) -> Result<Vec<TopComplexSymbol>> {
    if !engine.has_complexity() {
        return Ok(Vec::new());
    }
    let mut stmt = engine.conn.prepare(
        "SELECT symbol_name, file_path, line_count, cyclomatic_complexity, cognitive_complexity \
         FROM complexity ORDER BY cyclomatic_complexity DESC LIMIT 10",
    ).context("top complex query")?;
    stmt.query_map([], |row| {
        Ok(TopComplexSymbol {
            symbol_name: row.get(0)?,
            file_path: row.get(1)?,
            lines: row.get(2)?,
            cyclomatic: row.get(3)?,
            cognitive: row.get(4)?,
        })
    }).context("top complex")?
    .collect::<Result<Vec<_>, _>>()
    .context("collect top complex")
}

fn file_complexity(engine: &QueryEngine) -> Result<Vec<FileComplexity>> {
    if !engine.has_complexity() {
        return Ok(Vec::new());
    }
    let mut stmt = engine.conn.prepare(
        "SELECT file_path, COUNT(*) AS symbol_count, \
         ROUND(AVG(cyclomatic_complexity), 1) AS avg_cyclomatic, \
         MAX(cyclomatic_complexity) AS max_cyclomatic, \
         SUM(cognitive_complexity) AS total_cognitive \
         FROM complexity GROUP BY file_path \
         ORDER BY avg_cyclomatic DESC LIMIT 10",
    ).context("file complexity query")?;
    stmt.query_map([], |row| {
        Ok(FileComplexity {
            file_path: row.get(0)?,
            symbol_count: row.get(1)?,
            avg_cyclomatic: row.get(2)?,
            max_cyclomatic: row.get(3)?,
            total_cognitive: row.get(4)?,
        })
    }).context("file complexity")?
    .collect::<Result<Vec<_>, _>>()
    .context("collect file complexity")
}

// -- Quality overview structs --

#[derive(Debug, Serialize)]
struct DeadCodeSummary {
    total_exported: i64,
    zero_internal_imports: i64,
    dead_code_pct: f64,
}

#[derive(Debug, Serialize)]
struct CouplingSummary {
    total_files_with_deps: i64,
    avg_fan_in: f64,
    max_fan_in: i64,
    avg_fan_out: f64,
    max_fan_out: i64,
    cycle_count: usize,
}

#[derive(Debug, Serialize)]
struct DuplicationSummary {
    duplicate_groups: i64,
    duplicated_functions: i64,
}

fn dead_code_summary(engine: &QueryEngine) -> Result<DeadCodeSummary> {
    if !engine.has_imports() {
        return Ok(DeadCodeSummary {
            total_exported: 0,
            zero_internal_imports: 0,
            dead_code_pct: 0.0,
        });
    }

    let sql = "SELECT \
        (SELECT COUNT(*) FROM symbols WHERE is_exported = true) AS total_exported, \
        (SELECT COUNT(*) FROM symbols s \
         LEFT JOIN (SELECT DISTINCT imported_name FROM imports WHERE is_external = false) i \
         ON i.imported_name = s.name \
         WHERE s.is_exported = true AND i.imported_name IS NULL) AS zero_imports";

    let mut stmt = engine.conn.prepare(sql).context("dead code summary query")?;
    let (total_exported, zero_imports): (i64, i64) = stmt
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("dead code summary")?;

    let pct = if total_exported > 0 {
        (zero_imports as f64 / total_exported as f64 * 100.0 * 10.0).round() / 10.0
    } else {
        0.0
    };

    Ok(DeadCodeSummary {
        total_exported,
        zero_internal_imports: zero_imports,
        dead_code_pct: pct,
    })
}

fn coupling_summary(engine: &QueryEngine) -> Result<CouplingSummary> {
    if !engine.has_imports() {
        return Ok(CouplingSummary {
            total_files_with_deps: 0,
            avg_fan_in: 0.0,
            max_fan_in: 0,
            avg_fan_out: 0.0,
            max_fan_out: 0,
            cycle_count: 0,
        });
    }

    let sql = "WITH fan_out AS ( \
             SELECT source_file AS file_path, COUNT(DISTINCT module_specifier) AS fan_out \
             FROM imports WHERE is_external = false GROUP BY source_file \
         ), \
         fan_in AS ( \
             SELECT module_specifier AS file_path, COUNT(DISTINCT source_file) AS fan_in \
             FROM imports WHERE is_external = false GROUP BY module_specifier \
         ), \
         combined AS ( \
             SELECT COALESCE(fo.file_path, fi.file_path) AS file_path, \
                    COALESCE(fi.fan_in, 0) AS fan_in, \
                    COALESCE(fo.fan_out, 0) AS fan_out \
             FROM fan_out fo FULL OUTER JOIN fan_in fi ON fo.file_path = fi.file_path \
         ) \
         SELECT COUNT(*), \
                ROUND(AVG(fan_in), 1), COALESCE(MAX(fan_in), 0), \
                ROUND(AVG(fan_out), 1), COALESCE(MAX(fan_out), 0) \
         FROM combined";

    let mut stmt = engine.conn.prepare(sql).context("coupling summary query")?;
    let (total, avg_fi, max_fi, avg_fo, max_fo): (i64, f64, i64, f64, i64) = stmt
        .query_row([], |row| {
            Ok((
                row.get(0)?,
                row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                row.get(2)?,
                row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                row.get(4)?,
            ))
        })
        .context("coupling summary")?;

    // Count cycles
    let cycle_count = count_cycles(engine)?;

    Ok(CouplingSummary {
        total_files_with_deps: total,
        avg_fan_in: avg_fi,
        max_fan_in: max_fi,
        avg_fan_out: avg_fo,
        max_fan_out: max_fo,
        cycle_count,
    })
}

fn count_cycles(engine: &QueryEngine) -> Result<usize> {
    if !engine.has_imports() {
        return Ok(0);
    }
    let cycle_sql = "SELECT DISTINCT source_file, module_specifier \
                     FROM imports WHERE is_external = false";
    let mut stmt = engine.conn.prepare(cycle_sql).context("cycle edge query")?;
    let edges = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .context("cycle edges")?
        .collect::<Result<Vec<_>, _>>()
        .context("collect cycle edges")?;

    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut all_nodes: Vec<String> = Vec::new();
    for (from, to) in &edges {
        adj.entry(from.clone()).or_default().push(to.clone());
        if !all_nodes.contains(from) {
            all_nodes.push(from.clone());
        }
        if !all_nodes.contains(to) {
            all_nodes.push(to.clone());
        }
    }

    let sccs = tarjan_scc(&all_nodes, &adj);
    Ok(sccs.iter().filter(|scc| scc.len() > 1).count())
}

fn duplication_summary(engine: &QueryEngine) -> Result<DuplicationSummary> {
    if !engine.has_complexity() {
        return Ok(DuplicationSummary {
            duplicate_groups: 0,
            duplicated_functions: 0,
        });
    }

    let sql = "WITH dups AS ( \
        SELECT structural_hash, COUNT(*) AS cnt \
        FROM complexity \
        WHERE structural_hash != 0 \
        GROUP BY structural_hash, symbol_kind, line_count, cyclomatic_complexity, cognitive_complexity \
        HAVING COUNT(*) >= 2 \
    ) \
    SELECT COUNT(*), COALESCE(SUM(cnt), 0) FROM dups";

    let mut stmt = engine.conn.prepare(sql).context("duplication summary query")?;
    let (groups, funcs): (i64, i64) = stmt
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("duplication summary")?;

    Ok(DuplicationSummary {
        duplicate_groups: groups,
        duplicated_functions: funcs,
    })
}

// -- Format-specific renderers --

fn run_audit_overview_json(engine: &QueryEngine) -> Result<String> {
    let mut combined = serde_json::json!({
        "complexity": {
            "stats": complexity_stats(engine)?,
            "distribution": complexity_buckets(engine)?,
            "top_complex_symbols": top_complex_symbols(engine)?,
            "file_complexity": file_complexity(engine)?,
        },
        "quality": {
            "dead_code": dead_code_summary(engine)?,
            "coupling": coupling_summary(engine)?,
            "duplication": duplication_summary(engine)?,
        },
    });

    if engine.has_security() {
        let sec = super::security::security_summary(engine)?;
        combined["security"] = serde_json::json!({
            "unsafe_calls": sec.unsafe_calls,
            "string_risks": sec.string_risks,
            "hardcoded_secrets": sec.hardcoded_secrets,
            "total": sec.total,
            "high_severity": sec.high_severity,
            "medium_severity": sec.medium_severity,
        });
    }

    Ok(serde_json::to_string_pretty(&combined)?)
}

fn run_audit_overview_csv(engine: &QueryEngine) -> Result<String> {
    let mut out = String::from("metric,value\n");

    // Complexity metrics
    if let Some(stats) = complexity_stats(engine)? {
        out.push_str(&format!("total_symbols,{}\n", stats.total_symbols));
        out.push_str(&format!("avg_cyclomatic,{}\n", stats.avg_cyclomatic));
        out.push_str(&format!("max_cyclomatic,{}\n", stats.max_cyclomatic));
        out.push_str(&format!("avg_cognitive,{}\n", stats.avg_cognitive));
        out.push_str(&format!("max_cognitive,{}\n", stats.max_cognitive));
        out.push_str(&format!("avg_line_count,{}\n", stats.avg_line_count));
        out.push_str(&format!("max_line_count,{}\n", stats.max_line_count));
    }

    // Quality metrics
    let dc = dead_code_summary(engine)?;
    out.push_str(&format!("total_exported,{}\n", dc.total_exported));
    out.push_str(&format!("dead_code_candidates,{}\n", dc.zero_internal_imports));
    out.push_str(&format!("dead_code_pct,{}\n", dc.dead_code_pct));

    let cp = coupling_summary(engine)?;
    out.push_str(&format!("files_with_deps,{}\n", cp.total_files_with_deps));
    out.push_str(&format!("avg_fan_in,{}\n", cp.avg_fan_in));
    out.push_str(&format!("max_fan_in,{}\n", cp.max_fan_in));
    out.push_str(&format!("avg_fan_out,{}\n", cp.avg_fan_out));
    out.push_str(&format!("max_fan_out,{}\n", cp.max_fan_out));
    out.push_str(&format!("dependency_cycles,{}\n", cp.cycle_count));

    let dp = duplication_summary(engine)?;
    out.push_str(&format!("duplicate_groups,{}\n", dp.duplicate_groups));
    out.push_str(&format!("duplicated_functions,{}\n", dp.duplicated_functions));

    if engine.has_security() {
        let sec = super::security::security_summary(engine)?;
        out.push_str(&format!("security_unsafe_calls,{}\n", sec.unsafe_calls));
        out.push_str(&format!("security_string_risks,{}\n", sec.string_risks));
        out.push_str(&format!("security_hardcoded_secrets,{}\n", sec.hardcoded_secrets));
        out.push_str(&format!("security_total,{}\n", sec.total));
        out.push_str(&format!("security_high_severity,{}\n", sec.high_severity));
        out.push_str(&format!("security_medium_severity,{}\n", sec.medium_severity));
    }

    Ok(out)
}

fn run_audit_overview_table(engine: &QueryEngine) -> Result<String> {
    let mut out = String::new();

    // --- Complexity sections ---
    if let Some(stats) = complexity_stats(engine)? {
        let summary_text = format!(
            "{} symbols analyzed\n\
             Cyclomatic:  avg {}, max {}\n\
             Cognitive:   avg {}, max {}\n\
             Line length: avg {}, max {}\n",
            stats.total_symbols,
            stats.avg_cyclomatic, stats.max_cyclomatic,
            stats.avg_cognitive, stats.max_cognitive,
            stats.avg_line_count, stats.max_line_count,
        );
        out.push_str(&format_section("Complexity Summary", &summary_text));
    }

    let buckets = complexity_buckets(engine)?;
    if !buckets.is_empty() {
        let mut dist_text = String::new();
        for bucket in &buckets {
            dist_text.push_str(&format!(
                "  {:<25} {:>6} symbols\n",
                bucket.range, bucket.count,
            ));
        }
        out.push_str(&format_section("Cyclomatic Distribution", &dist_text));
    }

    let top_syms = top_complex_symbols(engine)?;
    if !top_syms.is_empty() {
        let mut top_text = String::new();
        top_text.push_str(&format!(
            "  {:<30} {:<40} {:>5} {:>5} {:>5}\n",
            "SYMBOL", "FILE", "LINES", "CYC", "COG"
        ));
        top_text.push_str(&format!(
            "  {:<30} {:<40} {:>5} {:>5} {:>5}\n",
            "-".repeat(30), "-".repeat(40), "-----", "-----", "-----"
        ));
        for sym in &top_syms {
            top_text.push_str(&format!(
                "  {:<30} {:<40} {:>5} {:>5} {:>5}\n",
                sym.symbol_name, sym.file_path, sym.lines, sym.cyclomatic, sym.cognitive,
            ));
        }
        out.push_str(&format_section("Most Complex Symbols", &top_text));
    }

    let file_cplx = file_complexity(engine)?;
    if !file_cplx.is_empty() {
        let mut file_text = String::new();
        file_text.push_str(&format!(
            "  {:<45} {:>5} {:>8} {:>8} {:>8}\n",
            "FILE", "SYMS", "AVG_CYC", "MAX_CYC", "TOT_COG"
        ));
        file_text.push_str(&format!(
            "  {:<45} {:>5} {:>8} {:>8} {:>8}\n",
            "-".repeat(45), "-----", "--------", "--------", "--------"
        ));
        for fc in &file_cplx {
            file_text.push_str(&format!(
                "  {:<45} {:>5} {:>8.1} {:>8} {:>8}\n",
                fc.file_path, fc.symbol_count, fc.avg_cyclomatic, fc.max_cyclomatic, fc.total_cognitive,
            ));
        }
        out.push_str(&format_section("File Complexity (by avg cyclomatic)", &file_text));
    }

    // --- Quality sections ---
    let dc = dead_code_summary(engine)?;
    let dc_text = format!(
        "  Total exported symbols:       {}\n\
         \x20 With zero internal imports:    {}\n\
         \x20 Dead code candidate rate:      {:.1}%\n",
        dc.total_exported, dc.zero_internal_imports, dc.dead_code_pct
    );
    out.push_str(&format_section("Dead Code", &dc_text));

    let cp = coupling_summary(engine)?;
    let cp_text = format!(
        "  Files with internal deps:     {}\n\
         \x20 Fan-in:   avg {:.1}, max {}\n\
         \x20 Fan-out:  avg {:.1}, max {}\n\
         \x20 Dependency cycles:            {}\n",
        cp.total_files_with_deps,
        cp.avg_fan_in, cp.max_fan_in,
        cp.avg_fan_out, cp.max_fan_out,
        cp.cycle_count
    );
    out.push_str(&format_section("Coupling", &cp_text));

    let dp = duplication_summary(engine)?;
    let dp_text = format!(
        "  Duplicate groups:             {}\n\
         \x20 Total duplicated functions:    {}\n",
        dp.duplicate_groups, dp.duplicated_functions
    );
    out.push_str(&format_section("Duplication", &dp_text));

    if engine.has_security() {
        let sec = super::security::security_summary(engine)?;
        let sec_text = format!(
            "  Total issues:                 {}\n\
             \x20 Unsafe calls:                 {}\n\
             \x20 String risks:                 {}\n\
             \x20 Hardcoded secrets:             {}\n\
             \x20 High severity:                 {}\n\
             \x20 Medium severity:               {}\n",
            sec.total, sec.unsafe_calls, sec.string_risks,
            sec.hardcoded_secrets, sec.high_severity, sec.medium_severity
        );
        out.push_str(&format_section("Security", &sec_text));
    }

    Ok(out)
}
