use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{EntryId, SafetyBlockReason, TaskWarning};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractOptions {
    pub password: Option<String>,
    pub overwrite_policy: OverwritePolicy,
    pub symlink_policy: SymlinkPolicy,
    pub preserve_permissions: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            password: None,
            overwrite_policy: OverwritePolicy::AskBatch,
            symlink_policy: SymlinkPolicy::Conservative,
            preserve_permissions: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverwritePolicy {
    AskBatch,
    Overwrite,
    Skip,
    Rename,
    KeepNewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymlinkPolicy {
    Conservative,
    PreserveLinks,
    FollowWithinDestination,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestOptions {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractPreflight {
    pub destination: PathBuf,
    pub total_entries: u64,
    pub estimated_bytes: Option<u64>,
    pub conflicts: Vec<PathConflict>,
    pub blocked_entries: Vec<BlockedEntry>,
    pub warnings: Vec<TaskWarning>,
}

impl ExtractPreflight {
    pub fn is_clear(&self) -> bool {
        self.conflicts.is_empty() && self.blocked_entries.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConflict {
    pub entry: EntryId,
    pub entry_path: String,
    pub target_path: PathBuf,
    pub source_size: Option<u64>,
    pub target_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedEntry {
    pub entry: EntryId,
    pub entry_path: String,
    pub reason: SafetyBlockReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionBatch {
    pub conflicts: Vec<PathConflict>,
    pub selected_index: usize,
    pub default_policy: OverwritePolicy,
    pub apply_to_remaining: bool,
}

impl ConflictResolutionBatch {
    pub fn resolve_all(&self) -> Vec<ConflictDecision> {
        self.conflicts
            .iter()
            .map(|conflict| ConflictDecision {
                entry: conflict.entry,
                policy: self.default_policy,
                target_path: conflict.target_path.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictDecision {
    pub entry: EntryId,
    pub policy: OverwritePolicy,
    pub target_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordRequest {
    pub archive_name: String,
    pub scope: PasswordScope,
    pub allow_session_memory: bool,
    pub retry_count: u32,
    pub header_encrypted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasswordScope {
    Archive,
    Entry(EntryId),
    CreateArchive,
}
