use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::{ComplexitySortField, OutputFormat};
use crate::query::db::QueryEngine;
use crate::query::format::{format_output, format_section};

#[derive(Debug, Serialize)]
struct ComplexityRow {
    file_path: String,
    symbol_name: String,
    symbol_kind: String,
    start_line: u32,
    end_line: u32,
    line_count: u32,
    cyclomatic: u32,
    cognitive: u32,
}

pub fn run_complexity(
    engine: &QueryEngine,
    file: Option<&str>,
    kind: Option<&str>,
    sort: &ComplexitySortField,
    limit: usize,
    threshold: Option<u32>,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_complexity() {
        return Ok("No complexity data available. Re-create the audit to generate complexity metrics.\n".to_string());
    }

    let mut conditions = Vec::new();
    let mut sql = String::from(
        "SELECT file_path, symbol_name, symbol_kind, start_line, end_line, \
         line_count, \
         cyclomatic_complexity, cognitive_complexity \
         FROM complexity",
    );

    if let Some(f) = file {
        conditions.push(format!("file_path LIKE '{}%'", f.replace('\'', "''")));
    }
    if let Some(k) = kind {
        conditions.push(format!("symbol_kind = '{}'", k.replace('\'', "''")));
    }
    if let Some(t) = threshold {
        conditions.push(format!("cyclomatic_complexity >= {}", t));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    let order = match sort {
        ComplexitySortField::Cyclomatic => "cyclomatic_complexity DESC",
        ComplexitySortField::Cognitive => "cognitive_complexity DESC",
        ComplexitySortField::Name => "symbol_name ASC",
        ComplexitySortField::File => "file_path ASC, start_line ASC",
        ComplexitySortField::Lines => "line_count DESC",
    };
    sql.push_str(&format!(" ORDER BY {} LIMIT {}", order, limit));

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare complexity query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ComplexityRow {
                file_path: row.get(0)?,
                symbol_name: row.get(1)?,
                symbol_kind: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                line_count: row.get(5)?,
                cyclomatic: row.get(6)?,
                cognitive: row.get(7)?,
            })
        })
        .context("failed to query complexity")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect complexity rows")?;

    let headers = &[
        "file_path",
        "symbol_name",
        "symbol_kind",
        "start_line",
        "end_line",
        "line_count",
        "cyclomatic",
        "cognitive",
    ];

    format_output(&rows, headers, format)
}

#[derive(Debug, Serialize)]
struct OverviewStats {
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

pub fn run_complexity_overview(engine: &QueryEngine, format: &OutputFormat) -> Result<String> {
    if !engine.has_complexity() {
        return Ok("No complexity data available. Re-create the audit to generate complexity metrics.\n".to_string());
    }

    // Summary stats
    let mut stmt = engine.conn.prepare(
        "SELECT COUNT(*), \
         ROUND(AVG(cyclomatic_complexity), 1), MAX(cyclomatic_complexity), \
         ROUND(AVG(cognitive_complexity), 1), MAX(cognitive_complexity), \
         ROUND(AVG(line_count), 1), MAX(line_count) \
         FROM complexity",
    ).context("failed to prepare overview stats")?;
    let stats = stmt.query_row([], |row| {
        Ok(OverviewStats {
            total_symbols: row.get(0)?,
            avg_cyclomatic: row.get(1)?,
            max_cyclomatic: row.get(2)?,
            avg_cognitive: row.get(3)?,
            max_cognitive: row.get(4)?,
            avg_line_count: row.get(5)?,
            max_line_count: row.get(6)?,
        })
    }).context("failed to query overview stats")?;

    // Distribution buckets
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
    ).context("failed to prepare distribution query")?;
    let buckets = stmt.query_map([], |row| {
        Ok(ComplexityBucket {
            range: row.get(0)?,
            count: row.get(1)?,
        })
    }).context("failed to query distribution")?
    .collect::<Result<Vec<_>, _>>()
    .context("failed to collect distribution")?;

    // Top 10 most complex symbols
    let mut stmt = engine.conn.prepare(
        "SELECT symbol_name, file_path, line_count, cyclomatic_complexity, cognitive_complexity \
         FROM complexity \
         ORDER BY cyclomatic_complexity DESC \
         LIMIT 10",
    ).context("failed to prepare top complex query")?;
    let top_symbols = stmt.query_map([], |row| {
        Ok(TopComplexSymbol {
            symbol_name: row.get(0)?,
            file_path: row.get(1)?,
            lines: row.get(2)?,
            cyclomatic: row.get(3)?,
            cognitive: row.get(4)?,
        })
    }).context("failed to query top complex")?
    .collect::<Result<Vec<_>, _>>()
    .context("failed to collect top complex")?;

    // Per-file aggregation (top 10 files by avg complexity)
    let mut stmt = engine.conn.prepare(
        "SELECT file_path, COUNT(*) AS symbol_count, \
         ROUND(AVG(cyclomatic_complexity), 1) AS avg_cyclomatic, \
         MAX(cyclomatic_complexity) AS max_cyclomatic, \
         SUM(cognitive_complexity) AS total_cognitive \
         FROM complexity \
         GROUP BY file_path \
         ORDER BY avg_cyclomatic DESC \
         LIMIT 10",
    ).context("failed to prepare file complexity query")?;
    let file_complexity = stmt.query_map([], |row| {
        Ok(FileComplexity {
            file_path: row.get(0)?,
            symbol_count: row.get(1)?,
            avg_cyclomatic: row.get(2)?,
            max_cyclomatic: row.get(3)?,
            total_cognitive: row.get(4)?,
        })
    }).context("failed to query file complexity")?
    .collect::<Result<Vec<_>, _>>()
    .context("failed to collect file complexity")?;

    match format {
        OutputFormat::Json => {
            let combined = serde_json::json!({
                "stats": stats,
                "distribution": buckets,
                "top_complex_symbols": top_symbols,
                "file_complexity": file_complexity,
            });
            Ok(serde_json::to_string_pretty(&combined)?)
        }
        OutputFormat::Csv => {
            // Flat symbol-level CSV
            let mut stmt = engine.conn.prepare(
                "SELECT file_path, symbol_name, symbol_kind, line_count, \
                 cyclomatic_complexity, cognitive_complexity \
                 FROM complexity ORDER BY cyclomatic_complexity DESC",
            ).context("csv query")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            }).context("csv rows")?
            .collect::<Result<Vec<_>, _>>()
            .context("collect csv")?;

            let mut out = String::from("file_path,symbol_name,symbol_kind,lines,cyclomatic,cognitive\n");
            for (fp, sn, sk, lc, cy, co) in rows {
                out.push_str(&format!("{},{},{},{},{},{}\n", fp, sn, sk, lc, cy, co));
            }
            Ok(out)
        }
        OutputFormat::Table => {
            let mut out = String::new();

            // Section 1: Summary
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

            // Section 2: Distribution
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

            // Section 3: Top Complex Symbols
            if !top_symbols.is_empty() {
                let mut top_text = String::new();
                top_text.push_str(&format!(
                    "  {:<30} {:<40} {:>5} {:>5} {:>5}\n",
                    "SYMBOL", "FILE", "LINES", "CYC", "COG"
                ));
                top_text.push_str(&format!(
                    "  {:<30} {:<40} {:>5} {:>5} {:>5}\n",
                    "-".repeat(30), "-".repeat(40), "-----", "-----", "-----"
                ));
                for sym in &top_symbols {
                    top_text.push_str(&format!(
                        "  {:<30} {:<40} {:>5} {:>5} {:>5}\n",
                        sym.symbol_name, sym.file_path, sym.lines, sym.cyclomatic, sym.cognitive,
                    ));
                }
                out.push_str(&format_section("Most Complex Symbols", &top_text));
            }

            // Section 4: File Complexity
            if !file_complexity.is_empty() {
                let mut file_text = String::new();
                file_text.push_str(&format!(
                    "  {:<45} {:>5} {:>8} {:>8} {:>8}\n",
                    "FILE", "SYMS", "AVG_CYC", "MAX_CYC", "TOT_COG"
                ));
                file_text.push_str(&format!(
                    "  {:<45} {:>5} {:>8} {:>8} {:>8}\n",
                    "-".repeat(45), "-----", "--------", "--------", "--------"
                ));
                for fc in &file_complexity {
                    file_text.push_str(&format!(
                        "  {:<45} {:>5} {:>8.1} {:>8} {:>8}\n",
                        fc.file_path, fc.symbol_count, fc.avg_cyclomatic, fc.max_cyclomatic, fc.total_cognitive,
                    ));
                }
                out.push_str(&format_section("File Complexity (by avg cyclomatic)", &file_text));
            }

            Ok(out)
        }
    }
}
