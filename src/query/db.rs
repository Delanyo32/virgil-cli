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

        Ok(Self {
            conn,
            data_dir: data_dir.to_path_buf(),
        })
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
