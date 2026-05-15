use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

use chrono::Utc;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shadow_zip_domain::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub root_dir: PathBuf,
    pub thumbnail_capacity_bytes: u64,
    pub temp_capacity_bytes: u64,
    pub index_capacity_bytes: u64,
}

impl CacheConfig {
    pub fn with_root(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            thumbnail_capacity_bytes: 128 * 1024 * 1024,
            temp_capacity_bytes: 512 * 1024 * 1024,
            index_capacity_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveFingerprint {
    pub canonical_path: Option<PathBuf>,
    pub size: Option<u64>,
    pub modified_unix_ms: Option<i64>,
    pub content_hash: Option<String>,
}

impl ArchiveFingerprint {
    pub fn from_source(
        source: &ArchiveSource,
        include_content_hash: bool,
    ) -> Result<Self, ArchiveError> {
        let Some(path) = source.path() else {
            return Ok(Self {
                canonical_path: None,
                size: None,
                modified_unix_ms: None,
                content_hash: None,
            });
        };
        Self::from_path(path, include_content_hash)
    }

    pub fn from_path(path: &Path, include_content_hash: bool) -> Result<Self, ArchiveError> {
        let metadata = fs::metadata(path).map_err(cache_io_error)?;
        let modified_unix_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        Ok(Self {
            canonical_path: fs::canonicalize(path).ok(),
            size: Some(metadata.len()),
            modified_unix_ms,
            content_hash: include_content_hash
                .then(|| hash_prefix(path))
                .transpose()?,
        })
    }

    pub fn stable_key(&self) -> String {
        let path = self
            .canonical_path
            .as_ref()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| "memory".into());
        format!(
            "{}|{}|{}|{}",
            path,
            self.size.unwrap_or_default(),
            self.modified_unix_ms.unwrap_or_default(),
            self.content_hash.as_deref().unwrap_or("")
        )
    }

    pub fn matches(&self, other: &Self) -> bool {
        self.stable_key() == other.stable_key()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailCacheKey {
    pub archive: ArchiveFingerprint,
    pub entry_path: String,
    pub entry_size: Option<u64>,
    pub entry_modified_unix_ms: Option<i64>,
    pub target_width: u32,
    pub target_height: u32,
    pub decoder_version: String,
    pub orientation: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexCacheRecord {
    pub schema_version: u32,
    pub archive: ArchiveFingerprint,
    pub info: ArchiveInfo,
    pub listing: ArchiveListing,
    pub created_unix_ms: i64,
}

pub struct CacheService {
    config: CacheConfig,
    index_records: BTreeMap<String, IndexCacheRecord>,
    thumbnails: BTreeMap<String, CacheEntry>,
    temp_files: BTreeMap<PathBuf, CacheEntry>,
}

impl CacheService {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            index_records: BTreeMap::new(),
            thumbnails: BTreeMap::new(),
            temp_files: BTreeMap::new(),
        }
    }

    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    pub fn cleanup_plan(&self) -> TaskPlan {
        let mut plan = TaskPlan::new(TaskKind::CacheCleanup, "Clean archive cache");
        plan.execution = TaskExecutionPlan::NativePipeline(NativePipelinePlan {
            steps: vec![PipelineStep::FinalizeArchive],
            cancellation_points: vec![CancellationPoint::BetweenEntries],
            temp_policy: TempFilePolicy::None,
        });
        plan
    }

    pub fn get_index(&self, key: &str) -> Option<&IndexCacheRecord> {
        self.index_records.get(key)
    }

    pub fn get_valid_index(&self, fingerprint: &ArchiveFingerprint) -> Option<&IndexCacheRecord> {
        self.index_records
            .get(&fingerprint.stable_key())
            .filter(|record| {
                record.schema_version == CACHE_SCHEMA_VERSION && record.archive.matches(fingerprint)
            })
    }

    pub fn put_index(&mut self, record: IndexCacheRecord) {
        self.index_records
            .insert(record.archive.stable_key(), record);
        self.evict_indexes();
    }

    pub fn load_index_store(&mut self) -> Result<(), ArchiveError> {
        let path = self.config.root_dir.join("index-cache.json");
        if !path.exists() {
            return Ok(());
        }
        let store: CacheStore = serde_json::from_str(
            &fs::read_to_string(path).map_err(cache_io_error)?,
        )
        .or_else(|_| {
            fs::read_to_string(self.config.root_dir.join("index-cache.json"))
                .map_err(cache_io_error)
                .and_then(|text| {
                    serde_json::from_str::<BTreeMap<String, IndexCacheRecord>>(&text)
                        .map_err(cache_json_error)
                })
                .map(|index_records| CacheStore {
                    schema_version: CACHE_SCHEMA_VERSION,
                    index_records,
                    thumbnails: BTreeMap::new(),
                    temp_files: BTreeMap::new(),
                })
        })?;
        if store.schema_version != CACHE_SCHEMA_VERSION {
            self.clear_all()?;
            return Ok(());
        }
        self.index_records = store.index_records;
        self.thumbnails = store.thumbnails;
        self.temp_files = store.temp_files;
        Ok(())
    }

    pub fn save_index_store(&self) -> Result<(), ArchiveError> {
        fs::create_dir_all(&self.config.root_dir).map_err(cache_io_error)?;
        let text = serde_json::to_string_pretty(&CacheStore {
            schema_version: CACHE_SCHEMA_VERSION,
            index_records: self.index_records.clone(),
            thumbnails: self.thumbnails.clone(),
            temp_files: self.temp_files.clone(),
        })
        .map_err(cache_json_error)?;
        fs::write(self.config.root_dir.join("index-cache.json"), text).map_err(cache_io_error)
    }

    pub fn put_thumbnail_bytes(
        &mut self,
        key: String,
        bytes: &[u8],
        now_unix_ms: i64,
    ) -> Result<PathBuf, ArchiveError> {
        let dir = self.config.root_dir.join("thumbnails");
        fs::create_dir_all(&dir).map_err(cache_io_error)?;
        let path = dir.join(format!("{}.bin", stable_file_key(&key)));
        fs::write(&path, bytes).map_err(cache_io_error)?;
        self.put_thumbnail(key, path.clone(), bytes.len() as u64, now_unix_ms);
        Ok(path)
    }

    pub fn put_thumbnail(&mut self, key: String, path: PathBuf, size_bytes: u64, now_unix_ms: i64) {
        self.thumbnails.insert(
            key,
            CacheEntry {
                path,
                size_bytes,
                last_access_unix_ms: now_unix_ms,
                kind: CacheEntryKind::Thumbnail,
            },
        );
        self.evict_thumbnails();
    }

    pub fn register_temp_file(&mut self, path: PathBuf, size_bytes: u64, now_unix_ms: i64) {
        self.temp_files.insert(
            path.clone(),
            CacheEntry {
                path,
                size_bytes,
                last_access_unix_ms: now_unix_ms,
                kind: CacheEntryKind::TempFile,
            },
        );
        self.evict_temp_files();
    }

    pub fn cache_summary(&self) -> CacheSummary {
        CacheSummary {
            index_records: self.index_records.len(),
            index_bytes: serialized_len(&self.index_records),
            thumbnail_count: self.thumbnails.len(),
            thumbnail_bytes: self.thumbnails.values().map(|entry| entry.size_bytes).sum(),
            temp_count: self.temp_files.len(),
            temp_bytes: self.temp_files.values().map(|entry| entry.size_bytes).sum(),
        }
    }

    pub fn clear_all(&mut self) -> Result<(), ArchiveError> {
        self.index_records.clear();
        self.thumbnails.clear();
        self.temp_files.clear();
        if self.config.root_dir.exists() {
            fs::remove_dir_all(&self.config.root_dir).map_err(cache_io_error)?;
        }
        Ok(())
    }

    pub fn cleanup_stale_temp_files(&mut self) {
        self.temp_files.retain(|path, _| path.exists());
    }

    fn evict_thumbnails(&mut self) {
        evict_lru(&mut self.thumbnails, self.config.thumbnail_capacity_bytes);
    }

    fn evict_temp_files(&mut self) {
        evict_lru_by_path(&mut self.temp_files, self.config.temp_capacity_bytes);
    }

    fn evict_indexes(&mut self) {
        while serialized_len(&self.index_records) > self.config.index_capacity_bytes {
            let Some(key) = self
                .index_records
                .iter()
                .min_by_key(|(_, record)| record.created_unix_ms)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.index_records.remove(&key);
        }
    }
}

pub const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheStore {
    schema_version: u32,
    index_records: BTreeMap<String, IndexCacheRecord>,
    thumbnails: BTreeMap<String, CacheEntry>,
    temp_files: BTreeMap<PathBuf, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub last_access_unix_ms: i64,
    pub kind: CacheEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheEntryKind {
    Thumbnail,
    TempFile,
    Index,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSummary {
    pub index_records: usize,
    pub index_bytes: u64,
    pub thumbnail_count: usize,
    pub thumbnail_bytes: u64,
    pub temp_count: usize,
    pub temp_bytes: u64,
}

fn evict_lru(entries: &mut BTreeMap<String, CacheEntry>, capacity: u64) {
    while entries.values().map(|entry| entry.size_bytes).sum::<u64>() > capacity {
        let Some(key) = entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_unix_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        entries.remove(&key);
    }
}

fn evict_lru_by_path(entries: &mut BTreeMap<PathBuf, CacheEntry>, capacity: u64) {
    while entries.values().map(|entry| entry.size_bytes).sum::<u64>() > capacity {
        let Some(key) = entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_unix_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        entries.remove(&key);
    }
}

fn cache_io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "Cache I/O operation failed")
        .with_technical_detail(error.to_string())
}

fn cache_json_error(error: serde_json::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Internal, "Cache serialization failed")
        .with_technical_detail(error.to_string())
}

pub fn now_unix_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn hash_prefix(path: &Path) -> Result<String, ArchiveError> {
    let mut file = fs::File::open(path).map_err(cache_io_error)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 128 * 1024];
    let mut remaining = 8 * 1024 * 1024_u64;
    while remaining > 0 {
        let chunk_len = buf.len().min(remaining as usize);
        let read = file.read(&mut buf[..chunk_len]).map_err(cache_io_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        remaining -= read as u64;
    }
    Ok(hex::encode(hasher.finalize()))
}

fn stable_file_key(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

fn serialized_len<T: Serialize>(value: &T) -> u64 {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0)
}
