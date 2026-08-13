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

/// Shared system prompt: role, tools, the real DDL, example queries.
pub fn system_prompt() -> String {
    let ddl = crate::db::schema::create_statements().join(";\n");
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

QUERY HINTS
- Paths are relative to the project root, exactly as stored in file.path. Read \
a real path out of the file table before you filter on one.
- Some tables are empty for some languages, because a fact is only there if \
that language's extractor produces it. COUNT(*) a table before you build a \
review around it.
- Files and sizes: SELECT path, length(content) AS bytes FROM file ORDER BY bytes DESC LIMIT 20
- Symbols in a file: SELECT name, kind FROM symbol WHERE file_path = 'db/schema.rs'
- Callers of a function: SELECT s.file_path, s.name FROM call_edge e \
JOIN symbol s ON s.id = e.caller_id JOIN symbol t ON t.id = e.callee_id \
WHERE t.name = 'target_fn'
- imports holds file-to-file edges: importer_file_id and imported_id are both \
file paths from file.path, despite the `_id` names.
- Who imports a file: SELECT importer_file_id FROM imports WHERE imported_id LIKE '%auth%'
- Line span of a symbol: SELECT sp.start_line, sp.end_line FROM span sp \
JOIN symbol s ON s.id = sp.entity_id WHERE s.name = 'target_fn'
- Skip tests and generated code: SELECT f.path FROM file f \
JOIN file_classification c ON c.path = f.path \
WHERE NOT c.is_test AND NOT c.is_generated
- Never SELECT file.content in bulk; use read_source for code.

METHOD
Start broad (queries), narrow to suspects, confirm by reading source, then \
report. Verify line numbers against read_source output before reporting. \
When you have covered the review's focus areas, stop."
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

    #[test]
    fn init_prompts_writes_four_files() {
        let dir = tempfile::tempdir().unwrap();
        init_prompts(dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}
