use crate::error::StaticFilesError;
use async_trait::async_trait;
#[cfg(feature = "s3")]
use s3::{AddressingStyle, Auth, Client, Credentials};
use std::fs;
use std::path::{Path, PathBuf};

/// A pluggable backend for storing and retrieving files by a relative path/key.
#[async_trait]
pub trait Storage: Send + Sync {
    /// Stores `content` at `path`.
    async fn save(&self, path: &str, content: Vec<u8>) -> Result<String, StaticFilesError>;
    /// Reads the full contents stored at `path`.
    async fn open(&self, path: &str) -> Result<Vec<u8>, StaticFilesError>;
    /// Returns whether something is stored at `path`.
    async fn exists(&self, path: &str) -> Result<bool, StaticFilesError>;
    /// Deletes whatever is stored at `path`, if anything.
    async fn delete(&self, path: &str) -> Result<(), StaticFilesError>;
    /// Returns the public URL for `path`.
    fn url(&self, path: &str) -> String;
}

/// A [`Storage`] backend rooted at a single local directory.
pub struct LocalDiskStorage {
    root: PathBuf,
    base_url: String,
}

/// A [`Storage`] backend backed by an S3-compatible object store.
#[cfg(feature = "s3")]
pub struct S3Storage {
    client: Client,
    bucket: String,
    base_url: String,
}

#[cfg(feature = "s3")]
impl S3Storage {
    /// Creates an S3-backed storage for `bucket`.
    pub fn new(
        bucket: &str,
        region: &str,
        endpoint: Option<&str>,
        access_key: &str,
        secret_key: &str,
        base_url: &str,
    ) -> Result<Self, StaticFilesError> {
        let endpoint = endpoint.unwrap_or("https://s3.amazonaws.com");
        let credentials = Credentials::new(access_key, secret_key)
            .map_err(|e| StaticFilesError::S3(e.to_string()))?;
        let client = Client::builder(endpoint)
            .map_err(|e| StaticFilesError::S3(e.to_string()))?
            .region(region)
            .auth(Auth::Static(credentials))
            .addressing_style(AddressingStyle::Path)
            .build()
            .map_err(|e| StaticFilesError::S3(e.to_string()))?;
        Ok(Self {
            client,
            bucket: bucket.to_string(),
            base_url: base_url.to_string(),
        })
    }

    /// Creates the configured bucket if it does not already exist.
    pub async fn create_bucket(&self) -> Result<(), StaticFilesError> {
        match self.client.buckets().create(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(e)
                if e.to_string().contains("BucketAlreadyOwnedByYou")
                    || e.to_string().contains("BucketAlreadyExists") =>
            {
                Ok(())
            }
            Err(e) => Err(StaticFilesError::S3(e.to_string())),
        }
    }
}

#[cfg(feature = "s3")]
#[async_trait]
impl Storage for S3Storage {
    async fn save(&self, path: &str, content: Vec<u8>) -> Result<String, StaticFilesError> {
        self.client
            .objects()
            .put(&self.bucket, path)
            .body_bytes(content)
            .send()
            .await
            .map_err(|e| StaticFilesError::S3(e.to_string()))?;
        Ok(path.to_string())
    }
    async fn open(&self, path: &str) -> Result<Vec<u8>, StaticFilesError> {
        self.client
            .objects()
            .get(&self.bucket, path)
            .send()
            .await
            .map_err(|e| StaticFilesError::S3(e.to_string()))?
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| StaticFilesError::S3(e.to_string()))
    }
    async fn exists(&self, path: &str) -> Result<bool, StaticFilesError> {
        match self.client.objects().head(&self.bucket, path).send().await {
            Ok(_) => Ok(true),
            Err(e) if e.to_string().contains("404") || e.to_string().contains("NotFound") => {
                Ok(false)
            }
            Err(e) => Err(StaticFilesError::S3(e.to_string())),
        }
    }
    async fn delete(&self, path: &str) -> Result<(), StaticFilesError> {
        self.client
            .objects()
            .delete(&self.bucket, path)
            .send()
            .await
            .map_err(|e| StaticFilesError::S3(e.to_string()))
            .map(|_| ())
    }
    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

impl LocalDiskStorage {
    /// Creates a local-disk backend.
    pub fn new(root: impl Into<PathBuf>, base_url: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            base_url: base_url.into(),
        }
    }

    fn resolve_path(&self, path: &str) -> Result<Option<PathBuf>, StaticFilesError> {
        if path.is_empty() {
            return Ok(None);
        }
        let path_obj = Path::new(path);
        let target = self.root.join(path_obj);
        if !target.exists() {
            return Ok(None);
        }
        let canonical_target = match fs::canonicalize(&target) {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        let canonical_dir = match fs::canonicalize(&self.root) {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        if canonical_target.starts_with(&canonical_dir) {
            Ok(Some(canonical_target))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl Storage for LocalDiskStorage {
    async fn save(&self, path: &str, content: Vec<u8>) -> Result<String, StaticFilesError> {
        let target = self.root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, content)?;
        Ok(path.to_string())
    }

    async fn open(&self, path: &str) -> Result<Vec<u8>, StaticFilesError> {
        let target = self.resolve_path(path)?.ok_or_else(|| {
            StaticFilesError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "file not found",
            ))
        })?;
        Ok(fs::read(target)?)
    }

    async fn exists(&self, path: &str) -> Result<bool, StaticFilesError> {
        Ok(self.resolve_path(path)?.is_some())
    }

    async fn delete(&self, path: &str) -> Result<(), StaticFilesError> {
        let target = self.root.join(path);
        match fs::remove_file(target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}
