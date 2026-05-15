use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntryId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveSource {
    LocalPath(PathBuf),
}

impl ArchiveSource {
    pub fn display_name(&self) -> String {
        match self {
            Self::LocalPath(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .map_or_else(|| path.to_string_lossy().into_owned(), ToOwned::to_owned),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::LocalPath(path) => Some(path),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    Tar,
    TarGz,
    TarXz,
    TarZst,
    Rar,
    Unknown,
}

impl fmt::Display for ArchiveFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Zip => "ZIP",
            Self::SevenZip => "7z",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::TarXz => "tar.xz",
            Self::TarZst => "tar.zst",
            Self::Rar => "RAR",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveInfo {
    pub format: ArchiveFormat,
    pub display_name: String,
    pub total_bytes: Option<u64>,
    pub entry_count: Option<u64>,
    pub codecs: Vec<String>,
    pub filters: Vec<String>,
    pub is_solid: bool,
    pub is_encrypted: bool,
    pub has_header_encryption: bool,
    pub is_multi_volume: bool,
}

impl ArchiveInfo {
    pub fn unknown(display_name: impl Into<String>) -> Self {
        Self {
            format: ArchiveFormat::Unknown,
            display_name: display_name.into(),
            total_bytes: None,
            entry_count: None,
            codecs: Vec::new(),
            filters: Vec::new(),
            is_solid: false,
            is_encrypted: false,
            has_header_encryption: false,
            is_multi_volume: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityLevel {
    Full,
    High,
    Medium,
    Limited,
    External,
    Unsupported,
}

impl CapabilityLevel {
    pub fn usable(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveCapabilities {
    pub list: CapabilityLevel,
    pub extract_all: CapabilityLevel,
    pub extract_selected: CapabilityLevel,
    pub create: CapabilityLevel,
    pub update: CapabilityLevel,
    pub random_access: CapabilityLevel,
    pub password_read: CapabilityLevel,
    pub password_write: CapabilityLevel,
    pub header_encryption: CapabilityLevel,
    pub multi_volume_read: CapabilityLevel,
    pub multi_volume_write: CapabilityLevel,
    pub entry_stream_preview: CapabilityLevel,
}

impl ArchiveCapabilities {
    pub fn unsupported() -> Self {
        Self {
            list: CapabilityLevel::Unsupported,
            extract_all: CapabilityLevel::Unsupported,
            extract_selected: CapabilityLevel::Unsupported,
            create: CapabilityLevel::Unsupported,
            update: CapabilityLevel::Unsupported,
            random_access: CapabilityLevel::Unsupported,
            password_read: CapabilityLevel::Unsupported,
            password_write: CapabilityLevel::Unsupported,
            header_encryption: CapabilityLevel::Unsupported,
            multi_volume_read: CapabilityLevel::Unsupported,
            multi_volume_write: CapabilityLevel::Unsupported,
            entry_stream_preview: CapabilityLevel::Unsupported,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub id: EntryId,
    pub raw_path: String,
    pub normalized_path: String,
    pub display_path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub compressed_size: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
    pub method: Option<String>,
    pub encrypted: bool,
    pub safety: EntrySafety,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntrySafety {
    Safe,
    Blocked { reason: SafetyBlockReason },
    RequiresPolicy { reason: SafetyBlockReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyBlockReason {
    ParentTraversal,
    AbsolutePath,
    WindowsDrivePath,
    UncPath,
    DevicePath,
    PathTooLong,
    SymlinkEscapesDestination,
    UnsupportedLinkPolicy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveListing {
    pub entries: Vec<ArchiveEntry>,
    pub directories: BTreeMap<String, Vec<EntryId>>,
    pub is_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListingMode {
    Fast,
    Full,
    Incremental,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveSessionSnapshot {
    pub id: SessionId,
    pub source: ArchiveSource,
    pub info: ArchiveInfo,
    pub capabilities: ArchiveCapabilities,
    pub listing: ArchiveListing,
    pub selected_entries: BTreeSet<EntryId>,
    pub current_directory: String,
    pub filter: EntryFilter,
    pub sort: EntrySort,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryFilter {
    pub query: String,
    pub kinds: Vec<EntryKind>,
    pub only_encrypted: bool,
    pub only_unsafe: bool,
}

impl EntryFilter {
    pub fn matches(&self, entry: &ArchiveEntry) -> bool {
        let query_matches = self.query.is_empty()
            || entry
                .display_path
                .to_ascii_lowercase()
                .contains(&self.query.to_ascii_lowercase());
        let kind_matches = self.kinds.is_empty() || self.kinds.contains(&entry.kind);
        let encryption_matches = !self.only_encrypted || entry.encrypted;
        let safety_matches = !self.only_unsafe || !matches!(entry.safety, EntrySafety::Safe);

        query_matches && kind_matches && encryption_matches && safety_matches
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EntrySort {
    pub column: EntrySortColumn,
    pub direction: SortDirection,
}

impl Default for EntrySort {
    fn default() -> Self {
        Self {
            column: EntrySortColumn::Name,
            direction: SortDirection::Ascending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntrySortColumn {
    Name,
    Size,
    PackedSize,
    Type,
    Modified,
    Method,
    Encrypted,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirectoryTree {
    pub nodes: BTreeMap<String, DirectoryNode>,
}

impl DirectoryTree {
    pub fn from_listing(listing: &ArchiveListing) -> Self {
        let mut tree = Self::with_root(listing);
        for entry in &listing.entries {
            tree.insert_path(&entry.normalized_path);
        }
        tree
    }

    fn with_root(listing: &ArchiveListing) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "/".into(),
            DirectoryNode {
                path: "/".into(),
                name: "/".into(),
                entry_count: listing.entries.len() as u64,
                total_uncompressed_bytes: listing
                    .entries
                    .iter()
                    .filter_map(|entry| entry.size)
                    .sum(),
                children: Vec::new(),
            },
        );
        Self { nodes }
    }

    fn insert_path(&mut self, normalized_path: &str) {
        let mut current = String::new();
        for part in normalized_path.split('/').filter(|part| !part.is_empty()) {
            let parent = if current.is_empty() { "/" } else { &current }.to_string();
            current = if current.is_empty() {
                format!("/{part}")
            } else {
                format!("{current}/{part}")
            };
            self.nodes
                .entry(current.clone())
                .or_insert_with(|| DirectoryNode {
                    path: current.clone(),
                    name: part.to_string(),
                    entry_count: 0,
                    total_uncompressed_bytes: 0,
                    children: Vec::new(),
                });
            if let Some(parent_node) = self.nodes.get_mut(&parent)
                && !parent_node.children.contains(&current)
            {
                parent_node.children.push(current.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryNode {
    pub path: String,
    pub name: String,
    pub entry_count: u64,
    pub total_uncompressed_bytes: u64,
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualListWindow {
    pub first_index: usize,
    pub visible_count: usize,
    pub overscan: usize,
    pub row_height_px: f32,
}

impl VirtualListWindow {
    pub fn range(&self, total: usize) -> std::ops::Range<usize> {
        let start = self.first_index.saturating_sub(self.overscan);
        let end = (self.first_index + self.visible_count + self.overscan).min(total);
        start..end
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenOptions {
    pub password: Option<String>,
    pub prefer_cached_index: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamOptions {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputPath {
    pub path: PathBuf,
    pub archive_path: Option<String>,
}
