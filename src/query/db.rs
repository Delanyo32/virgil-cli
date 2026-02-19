use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use duckdb::Connection;

pub struct QueryEngine {
    pub conn: Connection,
    data_dir: PathBuf,
}

impl std::fmt::Debug for QueryEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryEngine")
            .field("data_dir", &self.data_dir)
            .finish()
    }
}

impl QueryEngine {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let files_path = data_dir.join("files.parquet");
        let symbols_path = data_dir.join("symbols.parquet");

        if !files_path.exists() {
            bail!(
                "files.parquet not found in {}",
                data_dir.display()
            );
        }
        if !symbols_path.exists() {
            bail!(
                "symbols.parquet not found in {}",
                data_dir.display()
            );
        }

        let conn = Connection::open_in_memory()
            .context("failed to open DuckDB in-memory connection")?;

        // Register parquet files as named views so queries can use
        // `FROM files` and `FROM symbols` instead of read_parquet().
        conn.execute(
            &format!(
                "CREATE VIEW files AS SELECT * FROM read_parquet('{}')",
                files_path.to_string_lossy().replace('\'', "''")
            ),
            [],
        )
        .context("failed to create files view")?;

        conn.execute(
            &format!(
                "CREATE VIEW symbols AS SELECT * FROM read_parquet('{}')",
                symbols_path.to_string_lossy().replace('\'', "''")
            ),
            [],
        )
        .context("failed to create symbols view")?;

        // Conditionally register imports view (backward compatible)
        let imports_path = data_dir.join("imports.parquet");
        if imports_path.exists() {
            let imports_escaped = imports_path.to_string_lossy().replace('\'', "''");

            // Check if the parquet file has the is_external column
            let has_is_external = {
                let sql = format!(
                    "SELECT COUNT(*) FROM parquet_schema('{}') WHERE name = 'is_external'",
                    imports_escaped
                );
                let count: i64 = conn
                    .query_row(&sql, [], |row| row.get(0))
                    .unwrap_or(0);
                count > 0
            };

            let view_sql = if has_is_external {
                format!(
                    "CREATE VIEW imports AS SELECT * FROM read_parquet('{}')",
                    imports_escaped
                )
            } else {
                // Synthesize is_external for old parquet files
                format!(
                    "CREATE VIEW imports AS SELECT *, \
                     (module_specifier NOT LIKE '.%' AND module_specifier NOT LIKE '#%') AS is_external \
                     FROM read_parquet('{}')",
                    imports_escaped
                )
            };

            conn.execute(&view_sql, [])
                .context("failed to create imports view")?;
        }

        Ok(Self {
            conn,
            data_dir: data_dir.to_path_buf(),
        })
    }

    pub fn has_imports(&self) -> bool {
        self.data_dir.join("imports.parquet").exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_fails_without_parquet_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = QueryEngine::new(dir.path()).unwrap_err();
        assert!(err.to_string().contains("files.parquet not found"));
    }
}
