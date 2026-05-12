use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    ArchiveError, ArchiveErrorKind, ArchiveFormat, InputPath, SecurityPolicy, SymlinkPolicy,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub schema_version: u32,
    pub locale: Option<String>,
    pub default_extract_dir: Option<PathBuf>,
    pub default_create_format: ArchiveFormat,
    pub default_compression_level: u8,
    pub show_advanced_codecs: bool,
    pub remember_passwords_for_session: bool,
    pub preview: PreviewConfig,
    pub cache: CachePolicy,
    pub logging: LoggingConfig,
    pub recent_files: RecentFilesConfig,
    pub creation_profiles: Vec<CreateProfile>,
    pub security: SecurityPolicy,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            locale: None,
            default_extract_dir: None,
            default_create_format: ArchiveFormat::Zip,
            default_compression_level: 6,
            show_advanced_codecs: false,
            remember_passwords_for_session: true,
            preview: PreviewConfig::default(),
            cache: CachePolicy::default(),
            logging: LoggingConfig::default(),
            recent_files: RecentFilesConfig::default(),
            creation_profiles: CreateProfile::defaults(),
            security: SecurityPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewConfig {
    pub max_input_bytes: u64,
    pub max_output_pixels: u64,
    pub thumbnail_cache_bytes: u64,
    pub enable_neighbor_prefetch: bool,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_output_pixels: 32_000_000,
            thumbnail_cache_bytes: 128 * 1024 * 1024,
            enable_neighbor_prefetch: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePolicy {
    pub index_cache_bytes: u64,
    pub temp_cache_bytes: u64,
    pub cleanup_on_exit: bool,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            index_cache_bytes: 512 * 1024 * 1024,
            temp_cache_bytes: 512 * 1024 * 1024,
            cleanup_on_exit: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub include_technical_details: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            include_technical_details: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentFilesConfig {
    pub enabled: bool,
    pub max_items: usize,
}

impl Default for RecentFilesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentFile {
    pub source: crate::ArchiveSource,
    pub display_name: String,
    pub last_opened_unix_ms: i64,
    pub format: ArchiveFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateArchiveDraft {
    pub inputs: Vec<InputPath>,
    pub output_path: PathBuf,
    pub format: ArchiveFormat,
    pub compression_method: CompressionMethod,
    pub compression_level: u8,
    pub solid: bool,
    pub encryption: EncryptionOptions,
    pub volume_size: Option<u64>,
    pub path_mode: PathStorageMode,
    pub symlink_policy: SymlinkPolicy,
    pub after_complete: AfterCompleteAction,
}

impl CreateArchiveDraft {
    pub fn into_options(&self) -> CreateOptions {
        CreateOptions {
            format: self.format,
            compression_method: Some(self.compression_method.to_string()),
            compression_level: Some(self.compression_level),
            solid: self.solid,
            encrypt_file_names: self.encryption.encrypt_file_names,
            password: self.encryption.password.clone(),
            volume_size: self.volume_size,
            symlink_policy: self.symlink_policy,
        }
    }

    pub fn validate(&self) -> Result<(), ArchiveError> {
        if self.inputs.is_empty() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Create archive requires at least one input",
            ));
        }
        if self.output_path.as_os_str().is_empty() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Create archive requires an output path",
            ));
        }
        if self
            .encryption
            .password
            .as_deref()
            .is_some_and(str::is_empty)
        {
            return Err(ArchiveError::new(
                ArchiveErrorKind::PasswordRequired,
                "Encryption is enabled but password is empty",
            ));
        }
        if self.volume_size.is_some_and(|size| size < 1024 * 1024) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Volume size must be at least 1 MiB",
            ));
        }
        Ok(())
    }

    pub fn default_for(
        format: ArchiveFormat,
        inputs: Vec<InputPath>,
        output_path: PathBuf,
    ) -> Self {
        let profile = CreateProfile::defaults()
            .into_iter()
            .find(|profile| profile.format == format)
            .unwrap_or_else(|| {
                CreateProfile::new(
                    "ZIP Deflate",
                    ArchiveFormat::Zip,
                    CompressionMethod::Deflate,
                    6,
                )
            });
        Self {
            inputs,
            output_path,
            format: profile.format,
            compression_method: profile.method,
            compression_level: profile.level,
            solid: profile.solid,
            encryption: EncryptionOptions {
                password: None,
                encrypt_file_names: profile.encrypt_file_names,
                algorithm: None,
            },
            volume_size: None,
            path_mode: PathStorageMode::Relative,
            symlink_policy: SymlinkPolicy::Conservative,
            after_complete: AfterCompleteAction::RevealOutput,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOptions {
    pub format: ArchiveFormat,
    pub compression_method: Option<String>,
    pub compression_level: Option<u8>,
    pub solid: bool,
    pub encrypt_file_names: bool,
    pub password: Option<String>,
    pub volume_size: Option<u64>,
    pub symlink_policy: SymlinkPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionMethod {
    Store,
    Deflate,
    Lzma2,
    Zstandard,
    Lz4,
    Brotli,
    Xz,
    Gzip,
}

impl fmt::Display for CompressionMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Store => "store",
            Self::Deflate => "deflate",
            Self::Lzma2 => "lzma2",
            Self::Zstandard => "zstd",
            Self::Lz4 => "lz4",
            Self::Brotli => "brotli",
            Self::Xz => "xz",
            Self::Gzip => "gzip",
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncryptionOptions {
    pub password: Option<String>,
    pub encrypt_file_names: bool,
    pub algorithm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathStorageMode {
    Relative,
    PreserveRootFolder,
    Flatten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterCompleteAction {
    Nothing,
    RevealOutput,
    CloseDialog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProfile {
    pub name: String,
    pub format: ArchiveFormat,
    pub method: CompressionMethod,
    pub level: u8,
    pub solid: bool,
    pub encrypt_file_names: bool,
    pub compatibility_note: Option<String>,
}

impl CreateProfile {
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::new(
                "ZIP Deflate",
                ArchiveFormat::Zip,
                CompressionMethod::Deflate,
                6,
            ),
            Self {
                name: "7z LZMA2".into(),
                format: ArchiveFormat::SevenZip,
                method: CompressionMethod::Lzma2,
                level: 7,
                solid: true,
                encrypt_file_names: true,
                compatibility_note: Some("Best compression, slower random preview".into()),
            },
            Self {
                name: "tar.zst".into(),
                format: ArchiveFormat::TarZst,
                method: CompressionMethod::Zstandard,
                level: 3,
                solid: false,
                encrypt_file_names: false,
                compatibility_note: Some("Streaming Unix-friendly archive".into()),
            },
        ]
    }

    fn new(name: &str, format: ArchiveFormat, method: CompressionMethod, level: u8) -> Self {
        Self {
            name: name.into(),
            format,
            method,
            level,
            solid: false,
            encrypt_file_names: false,
            compatibility_note: None,
        }
    }
}
