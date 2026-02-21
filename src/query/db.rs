use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use duckdb::Connection;

use crate::s3::S3Config;

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
            bail!("files.parquet not found in {}", data_dir.display());
        }
        if !symbols_path.exists() {
            bail!("symbols.parquet not found in {}", data_dir.display());
        }

        let conn =
            Connection::open_in_memory().context("failed to open DuckDB in-memory connection")?;

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
                let count: i64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap_or(0);
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

        // Conditionally register comments view (backward compatible)
        let comments_path = data_dir.join("comments.parquet");
        if comments_path.exists() {
            conn.execute(
                &format!(
                    "CREATE VIEW comments AS SELECT * FROM read_parquet('{}')",
                    comments_path.to_string_lossy().replace('\'', "''")
                ),
                [],
            )
            .context("failed to create comments view")?;
        }

        // Conditionally register errors view (backward compatible)
        let errors_path = data_dir.join("errors.parquet");
        if errors_path.exists() {
            conn.execute(
                &format!(
                    "CREATE VIEW errors AS SELECT * FROM read_parquet('{}')",
                    errors_path.to_string_lossy().replace('\'', "''")
                ),
                [],
            )
            .context("failed to create errors view")?;
        }

        Ok(Self {
            conn,
            data_dir: data_dir.to_path_buf(),
        })
    }

    pub fn new_s3(s3_config: &S3Config, data_prefix: &str) -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("failed to open DuckDB in-memory connection")?;

        // Install and load httpfs for S3 access
        conn.execute("INSTALL httpfs", [])
            .context("failed to install httpfs")?;
        conn.execute("LOAD httpfs", [])
            .context("failed to load httpfs")?;

        // Create S3 secret with credentials
        // DuckDB httpfs prepends the scheme, so strip it from the endpoint
        let endpoint = s3_config
            .endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let secret_sql = format!(
            "CREATE SECRET s3_secret (
                TYPE S3,
                KEY_ID '{}',
                SECRET '{}',
                ENDPOINT '{}',
                REGION '{}',
                URL_STYLE 'path'
            )",
            s3_config.access_key_id.replace('\'', "''"),
            s3_config.secret_access_key.replace('\'', "''"),
            endpoint.replace('\'', "''"),
            s3_config.region.replace('\'', "''"),
        );
        conn.execute(&secret_sql, [])
            .context("failed to create S3 secret")?;

        let prefix = data_prefix.trim_end_matches('/');
        let bucket = &s3_config.bucket_name;

        // Required views: files and symbols
        let files_url = format!("s3://{bucket}/{prefix}/files.parquet");
        conn.execute(
            &format!("CREATE VIEW files AS SELECT * FROM read_parquet('{files_url}')"),
            [],
        )
        .context("failed to create files view from S3")?;

        let symbols_url = format!("s3://{bucket}/{prefix}/symbols.parquet");
        conn.execute(
            &format!("CREATE VIEW symbols AS SELECT * FROM read_parquet('{symbols_url}')"),
            [],
        )
        .context("failed to create symbols view from S3")?;

        // Optional views — swallow errors if parquet doesn't exist
        let imports_url = format!("s3://{bucket}/{prefix}/imports.parquet");
        let _ = conn.execute(
            &format!("CREATE VIEW imports AS SELECT * FROM read_parquet('{imports_url}')"),
            [],
        );

        let comments_url = format!("s3://{bucket}/{prefix}/comments.parquet");
        let _ = conn.execute(
            &format!("CREATE VIEW comments AS SELECT * FROM read_parquet('{comments_url}')"),
            [],
        );

        let errors_url = format!("s3://{bucket}/{prefix}/errors.parquet");
        let _ = conn.execute(
            &format!("CREATE VIEW errors AS SELECT * FROM read_parquet('{errors_url}')"),
            [],
        );

        Ok(Self {
            conn,
            data_dir: PathBuf::from(data_prefix),
        })
    }

    pub fn has_imports(&self) -> bool {
        self.has_view("imports")
    }

    pub fn has_comments(&self) -> bool {
        self.has_view("comments")
    }

    pub fn has_errors(&self) -> bool {
        self.has_view("errors")
    }

    fn has_view(&self, view_name: &str) -> bool {
        let sql = format!(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = '{view_name}'"
        );
        self.conn
            .query_row(&sql, [], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
            > 0
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
