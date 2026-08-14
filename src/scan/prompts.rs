//! Review prompts: the four built-ins, `--prompts` loading, and the
//! shared system prompt every review agent starts from.

use anyhow::{Context, Result};
use std::path::Path;

pub struct ReviewPrompt {
    pub name: String,
    pub body: String,
}

const BUILTINS: [(&str, &str); 4] = [
    (
        "architecture",
        include_str!("prompts/builtin/architecture.md"),
    ),
    ("bugs", include_str!("prompts/builtin/bugs.md")),
    (
        "maintainability",
        include_str!("prompts/builtin/maintainability.md"),
    ),
    ("security", include_str!("prompts/builtin/security.md")),
];

/// Built-ins by default; a `--prompts` file or directory replaces them.
pub fn load(custom: Option<&Path>) -> Result<Vec<ReviewPrompt>> {
    let Some(path) = custom else {
        return Ok(BUILTINS
            .iter()
            .map(|(n, b)| ReviewPrompt {
                name: n.to_string(),
                body: b.to_string(),
            })
            .collect());
    };
    let mut out = Vec::new();
    if path.is_file() {
        out.push(read_prompt(path)?);
    } else {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .with_context(|| format!("cannot read prompts dir {}", path.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        entries.sort();
        for p in entries {
            out.push(read_prompt(&p)?);
        }
    }
    anyhow::ensure!(
        !out.is_empty(),
        "no .md prompt files found in {}",
        path.display()
    );
    Ok(out)
}

fn read_prompt(path: &Path) -> Result<ReviewPrompt> {
    Ok(ReviewPrompt {
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        body: std::fs::read_to_string(path)
            .with_context(|| format!("cannot read prompt file {}", path.display()))?,
    })
}

pub fn init_prompts(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, body) in BUILTINS {
        let dest = dir.join(format!("{name}.md"));
        anyhow::ensure!(!dest.exists(), "{} already exists", dest.display());
        std::fs::write(&dest, body)?;
        println!("wrote {}", dest.display());
    }
    Ok(())
}

/// The worked example queries shown to the agent under QUERY HINTS, as
/// `(what it answers, SQL)`.
///
/// They live in a const, rather than inline in the prompt text, so
/// `hint_queries_all_run` can execute every one of them against a real
/// empty schema. A hint that names a dropped column teaches the agent a
/// query that always errors, and nothing else in the build would catch
/// it.
///
/// That empty-schema test proves a hint *parses*, not that it *returns* in
/// reasonable time. Every hint here was also run against a real 1093-file
/// store (30,717 import edges); all 15 came back in under 40ms.
///
/// Cost lesson, measured on that store — do not undo it. An all-pairs
/// `WITH RECURSIVE` over `imports` (anchor = the whole table) cost 34ms at
/// depth 2, 1.0s at depth 3, and did not finish in 150s at depth 4. Roughly
/// 30x per hop. A review only gets 10 minutes total, so one such query ends
/// the review with nothing reported. Short cycles are therefore expressed as
/// plain self-joins, and the only recursive hint is seeded from a single
/// literal file. Keep it that way.
const HINT_QUERIES: &[(&str, &str)] = &[
    // --- orient yourself first ---
    (
        "What values does a column actually hold (do this before filtering on one)",
        "SELECT kind, count(*) AS n FROM symbol GROUP BY 1 ORDER BY n DESC",
    ),
    (
        "Biggest files, as review candidates",
        "SELECT path, length(content) AS bytes FROM file ORDER BY bytes DESC LIMIT 20",
    ),
    (
        "Skip tests, barrels and generated code",
        "SELECT f.path FROM file f JOIN file_classification c ON c.path = f.path \
WHERE NOT c.is_test AND NOT c.is_generated AND NOT c.is_barrel",
    ),
    // --- one file / one symbol ---
    (
        "Declarations in a file (exclude parameter/variable noise)",
        "SELECT name, kind FROM symbol WHERE file_path = 'src/db/schema.rs' \
AND kind NOT IN ('parameter', 'variable')",
    ),
    (
        "Line span of a symbol, to feed read_source",
        "SELECT s.file_path, sp.start_line, sp.end_line FROM span sp \
JOIN symbol s ON s.id = sp.entity_id WHERE s.name = 'target_fn'",
    ),
    (
        "Longest functions and methods",
        "SELECT s.file_path, s.name, sp.end_line - sp.start_line AS lines FROM symbol s \
JOIN span sp ON sp.entity_id = s.id WHERE s.kind IN ('function', 'method') \
ORDER BY lines DESC LIMIT 20",
    ),
    // --- across files ---
    (
        "Every reference to a name, anywhere in the repo",
        "SELECT file_path, occurrence_kind, count(*) AS n FROM occurrence \
WHERE name = 'target_fn' GROUP BY 1, 2 ORDER BY n DESC",
    ),
    (
        "Callers of a function",
        "SELECT s.file_path, s.name FROM call_edge e JOIN symbol s ON s.id = e.caller_id \
JOIN symbol t ON t.id = e.callee_id WHERE t.name = 'target_fn'",
    ),
    (
        "What a function calls (the other direction)",
        "SELECT t.file_path, t.name FROM call_edge e JOIN symbol s ON s.id = e.caller_id \
JOIN symbol t ON t.id = e.callee_id WHERE s.name = 'target_fn'",
    ),
    (
        "Who imports a file",
        "SELECT importer_file_id FROM imports WHERE imported_id = 'src/auth/token.rs'",
    ),
    (
        "Hub files: imported by the most other files",
        "SELECT imported_id, count(*) AS importers FROM imports \
GROUP BY 1 ORDER BY importers DESC LIMIT 20",
    ),
    (
        "Exported declarations never referenced outside their own file",
        "SELECT s.file_path, s.name, s.kind FROM symbol s WHERE s.exported \
AND s.kind NOT IN ('parameter', 'variable') AND NOT EXISTS (SELECT 1 FROM occurrence o \
WHERE o.name = s.name AND o.file_path <> s.file_path) LIMIT 50",
    ),
    (
        "Inheritance pairs, resolved to real names",
        "SELECT c.file_path, c.name AS child, p.name AS parent FROM extends e \
JOIN symbol c ON c.id = e.child_id JOIN symbol p ON p.id = e.parent_id",
    ),
    // --- multi-hop. Read the cost warning above before using these. ---
    (
        "Two-file import cycle (A imports B, B imports A) — cheap, start here",
        "SELECT a.importer_file_id, a.imported_id FROM imports a \
JOIN imports b ON b.importer_file_id = a.imported_id AND b.imported_id = a.importer_file_id \
WHERE a.importer_file_id < a.imported_id LIMIT 20",
    ),
    (
        "Three-file import cycle — still a plain join, still cheap",
        "SELECT a.importer_file_id AS f1, b.importer_file_id AS f2, c.importer_file_id AS f3 \
FROM imports a JOIN imports b ON b.importer_file_id = a.imported_id \
JOIN imports c ON c.importer_file_id = b.imported_id AND c.imported_id = a.importer_file_id \
WHERE a.importer_file_id < b.importer_file_id AND a.importer_file_id < c.importer_file_id LIMIT 20",
    ),
    (
        "Everything ONE file depends on, transitively (seeded walk, cheap)",
        "WITH RECURSIVE dep(f, depth) AS (\
SELECT 'src/main.rs', 0 \
UNION ALL \
SELECT i.imported_id, d.depth + 1 FROM dep d \
JOIN imports i ON i.importer_file_id = d.f WHERE d.depth < 6) \
SELECT DISTINCT f FROM dep",
    ),
];

/// Shared system prompt: role, tools, the real DDL, example queries.
pub fn system_prompt() -> String {
    let ddl = crate::db::schema::create_statements().join(";\n");
    let hints: String = HINT_QUERIES
        .iter()
        .map(|(what, sql)| format!("- {what}: {sql}\n"))
        .collect();
    format!(
        "You are a code-review agent. The codebase you are reviewing has been \
parsed into a DuckDB database. You cannot access the filesystem, the network, \
or run code — the database is your only window into the codebase. It holds \
every parsed source file in full, in file.content, but only files in the \
languages this tool parses. Non-code files (.env, .json, .yaml, .toml, \
Dockerfile, lockfiles, CI config) have no row at all, so you cannot check \
them and must not claim anything about them. Confine every finding to what \
the database holds; if the database cannot show something, say nothing about it.

TOOLS
- query: run read-only SQL (SELECT/WITH) against the schema below. The result \
carries at most 200 rows, and its `total_rows` field counts the rows the query \
returned, not the size of the table — for a real total, run COUNT(*).
- read_source: fetch a file's source lines (path, start_line, end_line). \
Prefer this over selecting file.content directly — it returns only the lines \
you asked for. It returns a clear error for a path it does not have source \
text for, so you never need to guess about blank results.
- report_finding: record one finding (severity, file, line, message). Call it \
once per distinct issue, as you confirm each one. Findings are the ONLY \
output that counts; prose in your final answer is discarded.

SCHEMA
{ddl}

HOW THE TABLES CONNECT
Almost every join in this schema hangs off one of two keys. Learn these and you \
can reach anything without guessing.

1. file.path — a repository-relative path string. The same value appears as \
symbol.file_path, span.file_path, occurrence.file_path, scope.file_path, \
comment.file_path, call_site.file_path, call_edge.file_path, \
file_classification.path, and BOTH columns of imports.
2. symbol.id — an opaque id for one declaration. The same value appears as \
span.entity_id, call_edge.caller_id and callee_id, extends.child_id and \
parent_id, implements.impl_id and interface_id, parameter.function_id, \
returns_type.function_id, throws.function_id, field_type.symbol_id, \
comment.documents_id, occurrence.enclosing_symbol_id, and symbol_id in every \
<lang>_attrs table.

Never invent an id. Look the symbol up by name in symbol, take its id, join on that.

Nesting is self-referential: symbol.parent_id points at another symbol.id (a \
method's class), and scope.parent_id points at another scope.id. Both are NULL \
at the top level.

Three traps that produce confidently wrong answers:
- imports is FILE-level, not symbol-level. Despite the `_id` names, both \
columns are file paths. It can show that nothing imports a file. It can NEVER \
show that one specific symbol is unused — use occurrence for that.
- symbol holds far more than declarations. On a typical repo, `parameter` and \
`variable` rows outnumber every real declaration kind combined. Filter on kind \
or your counts will be meaningless.
- `function` and `method` are separate kinds. Filtering on one silently drops \
the other; use kind IN ('function', 'method').

WHICH TABLE ANSWERS WHICH QUESTION
- Where is this name used, anywhere? occurrence. One row per identifier \
mention, with occurrence_kind such as read, write, call, type_use, import_use. \
It includes local variables, so aggregate by file_path rather than reading raw rows.
- Who calls this function? call_edge, which is already resolved. call_site is \
the raw pre-resolution form; prefer call_edge.
- What does this file depend on, or who depends on it? imports.
- Where is this symbol's code? span, then hand the line range to read_source.
- Is this file even worth reviewing? file_classification (is_test, is_barrel, \
is_generated).
- What is this function's shape? parameter, returns_type, throws, and the \
per-language <lang>_attrs tables.

COST: MULTI-HOP QUERIES CAN HANG
The import graph is dense, so walking it from every file at once explodes. \
Measured on a 1093-file repository with 30,717 import edges, an all-pairs \
recursive walk over imports took 34ms at depth 2, 1 second at depth 3, and did \
not finish inside 150 seconds at depth 4. Roughly thirty times worse per extra \
hop. There is a hard 10-minute limit on the whole review, so one careless walk \
can end it with nothing reported.

Two safe shapes, both measured at 3ms on that same repository:
- Fixed short cycles: express a 2-file or 3-file cycle as a plain self-join of \
imports. No recursion needed. Most real cycles are this short, so start here.
- Seeded walk: start a WITH RECURSIVE from ONE named file and bound the depth. \
Cheap because it explores one neighbourhood, not the whole graph.

Only reach for an all-pairs recursive walk if both fail you, and then bound it \
at depth 2. Always carry a depth column and always compare it against a limit, \
or the walk will never terminate on a cyclic graph.

QUERY HINTS
- Paths are relative to the project root, exactly as stored in file.path. Read \
a real path out of the file table before you filter on one.
- Some tables are empty for some languages, because a fact is only there if \
that language's extractor produces it. COUNT(*) a table before you build a \
review around it, and drop the angle if it is empty rather than guessing.
- Before filtering on a text column, GROUP BY it once to see the values it \
really holds. Do not assume the vocabulary.
- Aggregate in SQL. Use count, GROUP BY, ORDER BY and LIMIT to rank candidates. \
Only 200 rows come back, so an unaggregated scan hides the rest without telling you.
- Never SELECT file.content in bulk; use read_source for code.
{hints}

METHOD
1. COUNT(*) the tables this review depends on. Empty table, dropped angle.
2. Query broad and ranked to get a shortlist, not a data dump.
3. Narrow to a few suspects, then read_source only those line ranges.
4. Confirm in the real source before reporting. A join result is a lead, not \
evidence.
5. Verify each line number against read_source output, then report_finding.
6. Stop once the review's focus areas are covered."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_load_by_default() {
        let ps = load(None).unwrap();
        let names: Vec<_> = ps.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["architecture", "bugs", "maintainability", "security"]
        );
    }

    #[test]
    fn custom_dir_replaces_builtins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my-check.md"), "look for X").unwrap();
        let ps = load(Some(dir.path())).unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "my-check");
    }

    #[test]
    fn single_file_prompt_works() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("solo.md");
        std::fs::write(&f, "check only this").unwrap();
        let ps = load(Some(&f)).unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "solo");
    }

    #[test]
    fn system_prompt_contains_schema() {
        let sp = system_prompt();
        assert!(sp.contains("CREATE TABLE file"));
        assert!(sp.contains("read_source"));
    }

    /// Every worked example we hand the agent must actually run. An empty
    /// database is enough: a wrong table or column name is a plan-time
    /// error, so it fails before any row is needed.
    #[test]
    fn hint_queries_all_run() {
        let store = crate::db::DbStore::open_in_memory().unwrap();
        for (what, sql) in HINT_QUERIES {
            let r = store.run_query(sql, Default::default());
            assert!(r.is_ok(), "hint '{what}' does not run: {:?}", r.err());
        }
    }

    /// A recursive hint must be seeded from a literal and bounded by a depth
    /// limit. An unseeded walk (anchor = the whole `imports` table) blew past
    /// 150s at depth 4 on a real store, which burns the agent's entire
    /// 10-minute budget for zero findings. See the const's doc comment.
    #[test]
    fn recursive_hints_are_seeded_and_bounded() {
        for (what, sql) in HINT_QUERIES {
            if !sql.to_uppercase().contains("RECURSIVE") {
                continue;
            }
            assert!(
                sql.contains("depth <"),
                "recursive hint '{what}' has no depth bound"
            );
            // The anchor is the text between the CTE's `AS (` and the first
            // `UNION`. Seeded means it selects a literal, not from `imports`.
            let anchor = sql
                .split_once(" AS (")
                .and_then(|(_, rest)| rest.split_once("UNION"))
                .map(|(a, _)| a.to_uppercase())
                .unwrap_or_default();
            assert!(
                !anchor.contains("FROM IMPORTS"),
                "recursive hint '{what}' walks the whole imports table; seed it \
from one file instead"
            );
        }
    }

    /// The hints reach the agent only through the prompt, so the const
    /// and the prompt text must not drift apart.
    #[test]
    fn hint_queries_appear_in_the_prompt() {
        let sp = system_prompt();
        for (what, sql) in HINT_QUERIES {
            assert!(sp.contains(sql), "hint '{what}' is missing from the prompt");
        }
    }

    #[test]
    fn init_prompts_writes_four_files() {
        let dir = tempfile::tempdir().unwrap();
        init_prompts(dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}
