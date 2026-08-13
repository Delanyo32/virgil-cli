//! The three tools a review agent gets: `query`, `read_source`,
//! `report_finding`.
//!
//! Each agent holds its own `DbStore` clone, so the `Mutex` is there to
//! satisfy `ToolExecute`'s `Sync` bound, not to arbitrate contention.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cersei::prelude::*;
use duckdb::types::Value;
use serde::Deserialize;

use crate::db::DbStore;
use crate::scan::report::{Finding, Severity};

const MAX_ROWS: usize = 200;
const MAX_LINES: u32 = 400;

/// Read-only guard + run + serialize. The returned string goes straight
/// to the model, errors included — a failed query is information, not a
/// crash.
pub(crate) fn run_sql(store: &DbStore, sql: &str) -> String {
    let head = sql.trim_start().to_ascii_lowercase();
    // ponytail: prefix guard, not a SQL parser — the store is local and
    // the model is the only caller; this just stops accidental writes.
    if !(head.starts_with("select") || head.starts_with("with")) {
        return "ERROR: only SELECT/WITH queries are allowed".into();
    }
    // ponytail: truncation happens after DuckDB has materialised every
    // row, so an unbounded `SELECT *` on a huge repo costs the memory
    // before it costs the tokens. Push a `LIMIT` into the SQL if that
    // ever bites.
    match store.run_query(sql, BTreeMap::new()) {
        Ok(rows) => {
            let total = rows.rows.len();
            let shown: Vec<Vec<serde_json::Value>> = rows
                .rows
                .iter()
                .take(MAX_ROWS)
                .map(|r| r.iter().map(value_to_json).collect())
                .collect();
            serde_json::json!({
                "headers": rows.headers,
                "rows": shown,
                "total_rows": total,
                "truncated": total > MAX_ROWS,
            })
            .to_string()
        }
        Err(e) => format!("ERROR: {e}"),
    }
}

fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null => J::Null,
        Value::Boolean(b) => J::from(*b),
        Value::Text(s) | Value::Enum(s) => J::from(s.as_str()),
        Value::TinyInt(n) => J::from(*n),
        Value::SmallInt(n) => J::from(*n),
        Value::Int(n) => J::from(*n),
        Value::BigInt(n) => J::from(*n),
        Value::UTinyInt(n) => J::from(*n),
        Value::USmallInt(n) => J::from(*n),
        Value::UInt(n) => J::from(*n),
        Value::UBigInt(n) => J::from(*n),
        Value::Float(n) => J::from(*n),
        Value::Double(n) => J::from(*n),
        Value::List(items) | Value::Array(items) => {
            J::from(items.iter().map(value_to_json).collect::<Vec<_>>())
        }
        // ponytail: dates, blobs, structs render as their Debug form —
        // the model reads them, it never parses them back.
        other => J::from(format!("{other:?}")),
    }
}

/// Slice `[from, to]` (1-indexed, inclusive) out of the stored source
/// text for `path`. At most [`MAX_LINES`] lines per call.
pub(crate) fn read_lines(store: &DbStore, path: &str, from: u32, to: u32) -> Result<String> {
    let rows = store.run_query(
        "SELECT content FROM file WHERE path = $path",
        BTreeMap::from([("path".to_string(), Value::Text(path.to_string()))]),
    )?;
    let row = rows
        .rows
        .first()
        .with_context(|| format!("no such file in the code database: {path}"))?;
    let Some(Value::Text(content)) = row.first() else {
        anyhow::bail!("file.content is not text for {path}");
    };
    // An empty `content` means the file was unreadable when the project
    // was parsed. Say so, rather than handing back a blank slice.
    if content.is_empty() {
        anyhow::bail!(
            "no source text stored for {path} (file was empty or unreadable when parsed)"
        );
    }
    // Saturating throughout: the line numbers come from the model, and
    // `u32::MAX` must clamp, not panic.
    let from = from.max(1);
    let to = to.min(from.saturating_add(MAX_LINES - 1));
    let slice: Vec<&str> = content
        .lines()
        .skip(from as usize - 1)
        .take(to.saturating_add(1).saturating_sub(from) as usize)
        .collect();
    Ok(slice.join("\n"))
}

#[derive(Tool)]
#[tool(
    name = "query",
    description = "Run a read-only SQL query against the code database. \
        Tables and columns are listed in the system prompt.",
    permission = "read_only"
)]
pub struct QueryTool(pub Arc<Mutex<DbStore>>);

#[derive(Tool)]
#[tool(
    name = "read_source",
    description = "Read a line range of a file's source text, exactly as it \
        was parsed. Use it to confirm a suspicion before reporting it.",
    permission = "read_only"
)]
pub struct ReadSourceTool(pub Arc<Mutex<DbStore>>);

#[derive(Tool)]
#[tool(
    name = "report_finding",
    description = "Record one problem you are confident about. Call it once \
        per problem; call nothing if the code is clean.",
    permission = "none"
)]
pub struct ReportFindingTool(pub Arc<Mutex<Vec<Finding>>>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct QueryInput {
    /// A read-only SQL query against the code database.
    pub sql: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadSourceInput {
    /// File path exactly as stored in the `file` table.
    pub path: String,
    /// First line, 1-indexed.
    pub start_line: u32,
    /// Last line, inclusive. At most 400 lines per call.
    pub end_line: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReportFindingInput {
    /// "high" | "medium" | "low" | "info"
    pub severity: Severity,
    pub file: String,
    pub line: Option<u32>,
    /// One clear sentence: what is wrong and why it matters.
    pub message: String,
}

// The `#[derive(Tool)]` above generates `input_schema` from the `Input`
// type and an `execute` that rejects bad payloads before `run` is
// reached, so none of these hand-validate their input.
#[async_trait]
impl ToolExecute for QueryTool {
    type Input = QueryInput;
    async fn run(&self, input: QueryInput, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success(run_sql(&self.0.lock().unwrap(), &input.sql))
    }
}

#[async_trait]
impl ToolExecute for ReadSourceTool {
    type Input = ReadSourceInput;
    async fn run(&self, input: ReadSourceInput, _ctx: &ToolContext) -> ToolResult {
        match read_lines(
            &self.0.lock().unwrap(),
            &input.path,
            input.start_line,
            input.end_line,
        ) {
            Ok(text) => ToolResult::success(text),
            Err(e) => ToolResult::error(e.to_string()),
        }
    }
}

#[async_trait]
impl ToolExecute for ReportFindingTool {
    type Input = ReportFindingInput;
    async fn run(&self, input: ReportFindingInput, _ctx: &ToolContext) -> ToolResult {
        self.0.lock().unwrap().push(Finding {
            // The runner stamps the review name; the agent never sees it.
            review: String::new(),
            severity: input.severity,
            file: input.file,
            line: input.line,
            message: input.message,
        });
        ToolResult::success("recorded")
    }
}

pub fn make_tools(
    store: DbStore,
    findings: Arc<Mutex<Vec<Finding>>>,
) -> (QueryTool, ReadSourceTool, ReportFindingTool) {
    let store = Arc::new(Mutex::new(store));
    (
        QueryTool(store.clone()),
        ReadSourceTool(store),
        ReportFindingTool(findings),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_one_file() -> crate::db::DbStore {
        let store = crate::db::DbStore::open_in_memory().unwrap();
        store
            .run_script(
                "INSERT INTO file VALUES ('src/a.rs', 'rust', 'r', 'line one\nline two\nline three')",
                Default::default(),
            )
            .unwrap();
        store
    }

    #[test]
    fn query_rejects_writes() {
        let out = run_sql(&store_with_one_file(), "DELETE FROM file");
        assert!(out.starts_with("ERROR"), "writes must be rejected: {out}");
    }

    #[test]
    fn query_returns_json_rows() {
        let out = run_sql(
            &store_with_one_file(),
            "SELECT path, language, length(content) AS n FROM file",
        );
        // Numbers stay numbers, strings stay strings.
        assert_eq!(
            out,
            r#"{"headers":["path","language","n"],"rows":[["src/a.rs","rust",28]],"total_rows":1,"truncated":false}"#
        );
    }

    #[test]
    fn read_source_slices_lines() {
        let store = store_with_one_file();
        let out = read_lines(&store, "src/a.rs", 2, 3).unwrap();
        assert_eq!(out, "line two\nline three");
    }

    #[test]
    fn read_source_unknown_file_is_error() {
        let store = store_with_one_file();
        assert!(read_lines(&store, "nope.rs", 1, 5).is_err());
    }

    #[test]
    fn unreadable_file_is_an_error_not_blank_text() {
        let store = store_with_one_file();
        store
            .run_script(
                "INSERT INTO file VALUES ('src/b.rs', 'rust', 'r', '')",
                Default::default(),
            )
            .unwrap();
        let err = read_lines(&store, "src/b.rs", 1, 5)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no source text"), "got: {err}");
    }

    /// The model picks the line numbers, so extremes must clamp, not panic.
    #[test]
    fn absurd_line_numbers_clamp() {
        let store = store_with_one_file();
        assert_eq!(
            read_lines(&store, "src/a.rs", u32::MAX, u32::MAX).unwrap(),
            ""
        );
        assert_eq!(read_lines(&store, "src/a.rs", 0, 1).unwrap(), "line one");
    }

    /// The names the derive publishes are what the model calls.
    #[test]
    fn make_tools_publishes_the_three_names() {
        let (q, r, f) = make_tools(
            crate::db::DbStore::open_in_memory().unwrap(),
            Arc::new(Mutex::new(Vec::new())),
        );
        assert_eq!(
            (q.name(), r.name(), f.name()),
            ("query", "read_source", "report_finding")
        );
    }

    /// The model sends severity as a lowercase string; `Severity` must accept it.
    #[test]
    fn report_finding_input_parses_the_model_payload() {
        let input: ReportFindingInput =
            serde_json::from_str(r#"{"severity":"high","file":"a.rs","line":3,"message":"boom"}"#)
                .unwrap();
        assert_eq!(input.severity, Severity::High);
        assert_eq!(input.line, Some(3));
    }
}
