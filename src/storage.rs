use anyhow::Result;
use opendal::{EntryMode, Operator};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use tokio::fs;

/// Storage provider types
#[derive(Debug, Clone)]
pub enum StorageProvider {
    /// Alibaba Cloud Object Storage Service
    Oss,
    /// Amazon Simple Storage Service
    S3,
    /// Local filesystem (for testing)
    Fs,
}

impl FromStr for StorageProvider {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "oss" => Ok(Self::Oss),
            "s3" => Ok(Self::S3),
            "fs" => Ok(Self::Fs),
            _ => Err(anyhow::anyhow!("Unsupported storage provider: {}", s)),
        }
    }
}

/// Unified storage configuration for different providers
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Storage provider type
    pub provider: StorageProvider,
    /// Storage bucket/container name
    pub bucket: String,
    /// Access key ID (for cloud providers)
    pub access_key_id: Option<String>,
    /// Access key secret (for cloud providers)
    pub access_key_secret: Option<String>,
    /// Endpoint URL (for custom endpoints like MinIO)
    pub endpoint: Option<String>,
    /// Region (for OSS/S3)
    pub region: Option<String>,
    /// Root path for filesystem provider
    pub root_path: Option<String>,
}

impl StorageConfig {
    /// Create OSS configuration
    pub fn oss(
        bucket: String,
        access_key_id: String,
        access_key_secret: String,
        region: Option<String>,
    ) -> Self {
        Self {
            provider: StorageProvider::Oss,
            bucket,
            access_key_id: Some(access_key_id),
            access_key_secret: Some(access_key_secret),
            endpoint: None,
            region,
            root_path: None,
        }
    }

    /// Create S3 configuration
    pub fn s3(
        bucket: String,
        access_key_id: String,
        secret_access_key: String,
        region: Option<String>,
    ) -> Self {
        Self {
            provider: StorageProvider::S3,
            bucket,
            access_key_id: Some(access_key_id),
            access_key_secret: Some(secret_access_key),
            endpoint: None,
            region,
            root_path: None,
        }
    }

    /// Create filesystem configuration for testing
    pub fn fs(root_path: String) -> Self {
        Self {
            provider: StorageProvider::Fs,
            bucket: "local".to_string(),
            access_key_id: None,
            access_key_secret: None,
            endpoint: None,
            region: None,
            root_path: Some(root_path),
        }
    }
}

/// Unified storage client using OpenDAL
pub struct StorageClient {
    operator: Operator,
    #[allow(dead_code)]
    provider: StorageProvider,
}

impl StorageClient {
    /// Create a new storage client with unified configuration
    pub async fn new(config: StorageConfig) -> Result<Self> {
        let operator = Self::build_operator(&config)?;
        Ok(Self {
            operator,
            provider: config.provider,
        })
    }

    /// Build OpenDAL operator based on storage configuration
    fn build_operator(config: &StorageConfig) -> Result<Operator> {
        match &config.provider {
            StorageProvider::Oss => {
                let mut builder = opendal::services::Oss::default().bucket(&config.bucket);

                if let Some(access_key_id) = &config.access_key_id {
                    builder = builder.access_key_id(access_key_id);
                }
                if let Some(access_key_secret) = &config.access_key_secret {
                    builder = builder.access_key_secret(access_key_secret);
                }
                if let Some(endpoint) = &config.endpoint {
                    builder = builder.endpoint(endpoint);
                }

                Ok(Operator::new(builder)?.finish())
            }
            StorageProvider::S3 => {
                let mut builder = opendal::services::S3::default().bucket(&config.bucket);

                if let Some(access_key_id) = &config.access_key_id {
                    builder = builder.access_key_id(access_key_id);
                }
                if let Some(secret_access_key) = &config.access_key_secret {
                    builder = builder.secret_access_key(secret_access_key);
                }
                if let Some(region) = &config.region {
                    builder = builder.region(region);
                }
                if let Some(endpoint) = &config.endpoint {
                    builder = builder.endpoint(endpoint);
                }

                Ok(Operator::new(builder)?.finish())
            }
            StorageProvider::Fs => {
                let root = config.root_path.as_deref().unwrap_or("./");
                let builder = opendal::services::Fs::default().root(root);
                Ok(Operator::new(builder)?.finish())
            }
        }
    }

    /// List directory contents (equivalent to hdfs dfs -ls)
    pub async fn list_directory(&self, path: &str, long: bool, recursive: bool) -> Result<()> {
        if recursive {
            self.list_recursive(path, long).await
        } else {
            self.list_single_level(path, long).await
        }
    }

    /// Download files from remote to local (equivalent to hdfs dfs -get)
    pub async fn download_files(&self, remote_path: &str, local_path: &str) -> Result<()> {
        fs::create_dir_all(local_path).await?;
        self.download_recursive(remote_path, local_path).await
    }

    /// Show disk usage statistics (equivalent to hdfs dfs -du)
    pub async fn disk_usage(&self, path: &str, summary: bool) -> Result<()> {
        if summary {
            let (total_size, total_files) = self.calculate_total_usage(path).await?;
            println!("{} {}", format_size(total_size), path);
            println!("Total files: {}", total_files);
        } else {
            self.show_detailed_usage(path).await?;
        }
        Ok(())
    }

    /// List directory contents recursively
    async fn list_recursive(&self, path: &str, long: bool) -> Result<()> {
        let entries = self.operator.list(path).await?;

        for entry in entries {
            let entry_path = entry.path();
            let meta = entry.metadata();
            let is_dir = meta.mode().is_dir();

            self.print_entry(&entry, long);

            if is_dir {
                Box::pin(self.list_recursive(entry_path, long)).await?;
            }
        }
        Ok(())
    }

    /// List only immediate children (non-recursive)
    async fn list_single_level(&self, path: &str, long: bool) -> Result<()> {
        let entries = self.operator.list(path).await?;

        for entry in entries {
            self.print_entry(&entry, long);
        }
        Ok(())
    }

    /// Print entry information based on format requirements
    fn print_entry(&self, entry: &opendal::Entry, long: bool) {
        if long {
            let file_info = FileInfo::from_entry(entry);
            println!("{}", file_info);
        } else {
            println!("{}", entry.path());
        }
    }

    /// Download files recursively
    async fn download_recursive(&self, remote_path: &str, local_path: &str) -> Result<()> {
        let entries = self.operator.list(remote_path).await?;

        for entry in entries {
            let meta = entry.metadata();
            let remote_file_path = entry.path();
            let relative_path = remote_file_path
                .strip_prefix(remote_path)
                .unwrap_or(remote_file_path);
            let local_file_path = Path::new(local_path).join(relative_path);

            if meta.mode() == EntryMode::DIR {
                fs::create_dir_all(&local_file_path).await?;
                Box::pin(self.download_recursive(remote_file_path, local_path)).await?;
            } else {
                if let Some(parent) = local_file_path.parent() {
                    fs::create_dir_all(parent).await?;
                }

                let data = self.operator.read(remote_file_path).await?;
                fs::write(&local_file_path, data.to_vec()).await?;
                println!(
                    "Downloaded: {} → {}",
                    remote_file_path,
                    local_file_path.display()
                );
            }
        }
        Ok(())
    }

    /// Calculate total disk usage recursively
    async fn calculate_total_usage(&self, path: &str) -> Result<(u64, usize)> {
        let mut total_size = 0;
        let mut file_count = 0;

        let entries = self.operator.list(path).await?;

        for entry in entries {
            let meta = entry.metadata();

            if meta.mode() == EntryMode::DIR {
                let (dir_size, dir_files) =
                    Box::pin(self.calculate_total_usage(entry.path())).await?;
                total_size += dir_size;
                file_count += dir_files;
            } else {
                total_size += meta.content_length();
                file_count += 1;
            }
        }

        Ok((total_size, file_count))
    }

    /// Show detailed disk usage for each item
    async fn show_detailed_usage(&self, path: &str) -> Result<()> {
        let entries = self.operator.list(path).await?;

        for entry in entries {
            let meta = entry.metadata();
            let entry_path = entry.path();

            if meta.mode() == EntryMode::DIR {
                let (dir_size, _) = Box::pin(self.calculate_total_usage(entry_path)).await?;
                println!("{} {}", format_size(dir_size), entry_path);
            } else {
                println!("{} {}", format_size(meta.content_length()), entry_path);
            }
        }
        Ok(())
    }

    /// Get the storage provider type
    #[allow(dead_code)]
    pub fn provider(&self) -> &StorageProvider {
        &self.provider
    }
}

/// File information for display
struct FileInfo {
    path: String,
    size: u64,
    modified: Option<String>,
    is_dir: bool,
}

impl FileInfo {
    /// Create FileInfo from OpenDAL Entry
    fn from_entry(entry: &opendal::Entry) -> Self {
        let meta = entry.metadata();
        Self {
            path: entry.path().to_string(),
            size: meta.content_length(),
            modified: meta.last_modified().map(|t| t.to_rfc3339()),
            is_dir: meta.mode().is_dir(),
        }
    }
}

impl fmt::Display for FileInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file_type = if self.is_dir { "DIR" } else { "FILE" };
        let size_str = if self.is_dir {
            "-".to_string()
        } else {
            format_size(self.size)
        };
        let modified = self.modified.as_deref().unwrap_or("Unknown");

        write!(
            f,
            "{:<6} {:>10} {} {}",
            file_type, size_str, modified, self.path
        )
    }
}

/// Format file size in human-readable format
fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    const THRESHOLD: u64 = 1024;

    if size < THRESHOLD {
        return format!("{}B", size);
    }

    let mut size_f = size as f64;
    let mut unit_index = 0;

    while size_f >= THRESHOLD as f64 && unit_index < UNITS.len() - 1 {
        size_f /= THRESHOLD as f64;
        unit_index += 1;
    }

    format!("{:.1}{}", size_f, UNITS[unit_index])
}
