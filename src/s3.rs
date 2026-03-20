use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSetBuilder};

use crate::language::Language;

/// Parsed S3 URI: `s3://bucket/prefix`.
#[derive(Debug, Clone)]
pub struct S3Location {
    pub bucket: String,
    pub prefix: String,
}

impl S3Location {
    /// Parse an `s3://bucket/prefix` URI.
    pub fn parse(uri: &str) -> Result<Self> {
        let stripped = uri
            .strip_prefix("s3://")
            .ok_or_else(|| anyhow::anyhow!("S3 URI must start with s3:// — got: {uri}"))?;

        if stripped.is_empty() {
            bail!("S3 URI must include a bucket name: s3://bucket[/prefix]");
        }

        let (bucket, prefix) = match stripped.find('/') {
            Some(idx) => {
                let b = &stripped[..idx];
                let p = stripped[idx + 1..].trim_end_matches('/');
                (b.to_string(), p.to_string())
            }
            None => (stripped.to_string(), String::new()),
        };

        if bucket.is_empty() {
            bail!("S3 URI must include a bucket name");
        }

        Ok(S3Location { bucket, prefix })
    }
}

/// Build an S3 client, supporting custom endpoints (R2, MinIO, etc.).
///
/// Checks these env vars:
/// - `S3_ENDPOINT` or `AWS_ENDPOINT_URL` — custom endpoint URL
/// - `S3_ACCESS_KEY_ID` or `AWS_ACCESS_KEY_ID` — access key
/// - `S3_SECRET_ACCESS_KEY` or `AWS_SECRET_ACCESS_KEY` — secret key
/// - `AWS_REGION` — region (defaults to "auto" for R2 compatibility)
async fn build_client() -> aws_sdk_s3::Client {
    let endpoint = std::env::var("S3_ENDPOINT")
        .or_else(|_| std::env::var("AWS_ENDPOINT_URL"))
        .ok();

    let access_key = std::env::var("S3_ACCESS_KEY_ID")
        .or_else(|_| std::env::var("AWS_ACCESS_KEY_ID"))
        .ok();

    let secret_key = std::env::var("S3_SECRET_ACCESS_KEY")
        .or_else(|_| std::env::var("AWS_SECRET_ACCESS_KEY"))
        .ok();

    let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "auto".to_string());

    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region));

    if let (Some(ak), Some(sk)) = (access_key, secret_key) {
        config_loader = config_loader.credentials_provider(
            aws_sdk_s3::config::Credentials::new(ak, sk, None, None, "env"),
        );
    }

    let sdk_config = config_loader.load().await;

    let mut s3_config = aws_sdk_s3::config::Builder::from(&sdk_config);

    if let Some(ep) = endpoint {
        s3_config = s3_config
            .endpoint_url(&ep)
            .force_path_style(true);
    }

    aws_sdk_s3::Client::from_conf(s3_config.build())
}

/// List object keys under `location` that match the given language extensions.
/// Applies exclude patterns via globset. Paginates through all results.
pub fn list_objects(
    location: &S3Location,
    languages: &[Language],
    exclude_patterns: &[String],
) -> Result<Vec<String>> {
    block_on(async {
        let client = build_client().await;

        // Collect all valid extensions
        let extensions: Vec<&str> = languages
            .iter()
            .flat_map(|l| l.all_extensions().iter().copied())
            .collect();

        // Build exclude globset
        let exclude_set = if !exclude_patterns.is_empty() {
            let mut builder = GlobSetBuilder::new();
            for pattern in exclude_patterns {
                builder.add(
                    Glob::new(pattern)
                        .with_context(|| format!("invalid exclude glob: {pattern}"))?,
                );
            }
            Some(builder.build().context("failed to build exclude globset")?)
        } else {
            None
        };

        let prefix = if location.prefix.is_empty() {
            None
        } else {
            // Ensure prefix ends with / for directory listing
            let p = if location.prefix.ends_with('/') {
                location.prefix.clone()
            } else {
                format!("{}/", location.prefix)
            };
            Some(p)
        };

        let mut keys = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut req = client
                .list_objects_v2()
                .bucket(&location.bucket)
                .max_keys(1000);

            if let Some(ref p) = prefix {
                req = req.prefix(p);
            }
            if let Some(ref token) = continuation_token {
                req = req.continuation_token(token);
            }

            let resp = req
                .send()
                .await
                .with_context(|| format!("failed to list objects in s3://{}", location.bucket))?;

            for obj in resp.contents() {
                let key: &str = obj.key().unwrap_or_default();
                if key.is_empty() {
                    continue;
                }

                // Get relative path (strip prefix)
                let relative: &str = match &prefix {
                    Some(p) => key.strip_prefix(p.as_str()).unwrap_or(key),
                    None => key,
                };

                // Skip directories (keys ending with /)
                if relative.is_empty() || relative.ends_with('/') {
                    continue;
                }

                // Check extension
                let has_valid_ext = relative
                    .rsplit('.')
                    .next()
                    .is_some_and(|ext| extensions.contains(&ext));

                if !has_valid_ext {
                    continue;
                }

                // Check exclude patterns
                if let Some(ref excludes) = exclude_set
                    && excludes.is_match(relative) {
                        continue;
                    }

                keys.push(relative.to_string());
            }

            if resp.is_truncated() == Some(true) {
                continuation_token = resp.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(keys)
    })
}

/// Download objects concurrently, returning file contents and sizes.
/// Skips non-UTF-8 files with a warning.
#[allow(clippy::type_complexity)]
pub fn download_objects(
    location: &S3Location,
    keys: &[String],
    max_file_size: Option<u64>,
) -> Result<(HashMap<String, Arc<str>>, HashMap<String, u64>)> {
    block_on(async {
        let client = build_client().await;

        let prefix = if location.prefix.is_empty() {
            String::new()
        } else if location.prefix.ends_with('/') {
            location.prefix.clone()
        } else {
            format!("{}/", location.prefix)
        };

        // Download concurrently with bounded parallelism
        let semaphore = Arc::new(tokio::sync::Semaphore::new(64));
        let client = Arc::new(client);

        let mut handles = Vec::with_capacity(keys.len());

        for key in keys {
            let sem = semaphore.clone();
            let client = client.clone();
            let bucket = location.bucket.clone();
            let full_key = format!("{}{}", prefix, key);
            let relative = key.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                let resp = client
                    .get_object()
                    .bucket(&bucket)
                    .key(&full_key)
                    .send()
                    .await;

                match resp {
                    Ok(output) => {
                        let size = output.content_length().unwrap_or(0) as u64;

                        if let Some(max_size) = max_file_size
                            && size > max_size {
                                return None;
                            }

                        match output.body.collect().await {
                            Ok(bytes) => {
                                let bytes = bytes.into_bytes();
                                match std::str::from_utf8(&bytes) {
                                    Ok(s) => Some((relative, Arc::from(s), size)),
                                    Err(_) => {
                                        eprintln!("Warning: skipping non-UTF-8 file: {full_key}");
                                        None
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Warning: failed to read {full_key}: {e}");
                                None
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: failed to download {full_key}: {e}");
                        None
                    }
                }
            }));
        }

        let mut files = HashMap::with_capacity(keys.len());
        let mut sizes = HashMap::with_capacity(keys.len());

        for handle in handles {
            if let Ok(Some((rel, content, size))) = handle.await {
                sizes.insert(rel.clone(), size);
                files.insert(rel, content);
            }
        }

        Ok((files, sizes))
    })
}

/// Run an async block on a single-threaded tokio runtime.
fn block_on<F: std::future::Future<Output = T>, T>(f: F) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime")
        .block_on(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_uri() {
        let loc = S3Location::parse("s3://my-bucket/path/to/repo").unwrap();
        assert_eq!(loc.bucket, "my-bucket");
        assert_eq!(loc.prefix, "path/to/repo");
    }

    #[test]
    fn parse_uri_no_prefix() {
        let loc = S3Location::parse("s3://my-bucket").unwrap();
        assert_eq!(loc.bucket, "my-bucket");
        assert_eq!(loc.prefix, "");
    }

    #[test]
    fn parse_uri_trailing_slash() {
        let loc = S3Location::parse("s3://my-bucket/prefix/").unwrap();
        assert_eq!(loc.bucket, "my-bucket");
        assert_eq!(loc.prefix, "prefix");
    }

    #[test]
    fn parse_invalid_scheme() {
        assert!(S3Location::parse("http://bucket/key").is_err());
    }

    #[test]
    fn parse_empty_after_scheme() {
        assert!(S3Location::parse("s3://").is_err());
    }
}
