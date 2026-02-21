use anyhow::{Context, Result, bail};
use s3::creds::Credentials;
use s3::{Bucket, Region};

#[derive(Debug, Clone)]
pub struct S3Config {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket_name: String,
    pub endpoint: String,
    pub region: String,
}

impl S3Config {
    pub fn from_env() -> Result<Self> {
        let access_key_id = std::env::var("S3_ACCESS_KEY_ID")
            .context("S3_ACCESS_KEY_ID environment variable not set")?;
        let secret_access_key = std::env::var("S3_SECRET_ACCESS_KEY")
            .context("S3_SECRET_ACCESS_KEY environment variable not set")?;
        let bucket_name = std::env::var("S3_BUCKET_NAME")
            .context("S3_BUCKET_NAME environment variable not set")?;
        let endpoint =
            std::env::var("S3_ENDPOINT").context("S3_ENDPOINT environment variable not set")?;
        let region = std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        Ok(Self {
            access_key_id,
            secret_access_key,
            bucket_name,
            endpoint,
            region,
        })
    }
}

#[derive(Debug, Clone)]
pub struct S3File {
    pub key: String,
    pub size: u64,
}

pub struct S3Client {
    bucket: Box<Bucket>,
}

impl S3Client {
    pub fn new(config: &S3Config) -> Result<Self> {
        let credentials = Credentials::new(
            Some(&config.access_key_id),
            Some(&config.secret_access_key),
            None,
            None,
            None,
        )
        .context("failed to create S3 credentials")?;

        let region = Region::Custom {
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
        };

        let bucket = Bucket::new(&config.bucket_name, region, credentials)
            .context("failed to create S3 bucket")?
            .with_path_style();

        Ok(Self { bucket })
    }

    pub fn list_files(&self, prefix: &str, extensions: &[&str]) -> Result<Vec<S3File>> {
        let results = self
            .bucket
            .list(prefix.to_string(), None)
            .context("failed to list S3 objects")?;

        let mut files = Vec::new();
        for result in &results {
            for obj in &result.contents {
                let key = &obj.key;
                // Filter by extension if extensions list is non-empty
                if !extensions.is_empty() {
                    let matches = key
                        .rsplit('.')
                        .next()
                        .is_some_and(|ext| extensions.contains(&ext));
                    if !matches {
                        continue;
                    }
                }
                files.push(S3File {
                    key: key.clone(),
                    size: obj.size,
                });
            }
        }

        Ok(files)
    }

    pub fn get_file_string(&self, key: &str) -> Result<String> {
        let response = self
            .bucket
            .get_object(key)
            .with_context(|| format!("failed to get S3 object: {key}"))?;

        if response.status_code() != 200 {
            bail!("S3 GET {} returned status {}", key, response.status_code());
        }

        let bytes = response.to_vec();
        String::from_utf8(bytes).with_context(|| format!("S3 object {key} is not valid UTF-8"))
    }

    pub fn put_file(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        let response = self
            .bucket
            .put_object(key, bytes)
            .with_context(|| format!("failed to put S3 object: {key}"))?;

        if response.status_code() != 200 {
            bail!(
                "S3 PUT {} (content-type: {}) returned status {}",
                key,
                content_type,
                response.status_code()
            );
        }

        Ok(())
    }

    pub fn object_exists(&self, key: &str) -> Result<bool> {
        match self.bucket.head_object(key) {
            Ok((_, code)) => Ok(code == 200),
            Err(_) => Ok(false),
        }
    }
}
