use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    ArchiveError, ArchiveErrorKind, ArchiveListing, EntryId, EntrySafety, SafetyBlockReason,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub max_entries: u64,
    pub max_total_uncompressed_bytes: u64,
    pub max_single_entry_bytes: u64,
    pub max_compression_ratio: f64,
    pub max_directory_depth: usize,
    pub max_path_bytes: usize,
    pub block_recursive_archives: bool,
    pub image: ImageSecurityPolicy,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            max_entries: 1_000_000,
            max_total_uncompressed_bytes: 512 * 1024 * 1024 * 1024,
            max_single_entry_bytes: 64 * 1024 * 1024 * 1024,
            max_compression_ratio: 10_000.0,
            max_directory_depth: 64,
            max_path_bytes: 4096,
            block_recursive_archives: false,
            image: ImageSecurityPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSecurityPolicy {
    pub max_pixels: u64,
    pub max_frames: u32,
    pub max_decoded_bytes: u64,
}

impl Default for ImageSecurityPolicy {
    fn default() -> Self {
        Self {
            max_pixels: 64_000_000,
            max_frames: 256,
            max_decoded_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionPolicy {
    pub replacement: String,
    pub sensitive_keys: Vec<String>,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            replacement: "<redacted>".into(),
            sensitive_keys: vec![
                "password".into(),
                "passphrase".into(),
                "token".into(),
                "secret".into(),
                "-p".into(),
                "--password".into(),
            ],
        }
    }
}

impl RedactionPolicy {
    pub fn redact_text(&self, text: &str) -> String {
        let mut output = text.to_string();
        for key in &self.sensitive_keys {
            output = output
                .split_whitespace()
                .map(|part| {
                    let lower = part.to_ascii_lowercase();
                    if lower.starts_with(&format!("{key}="))
                        || lower.starts_with(&format!("{key}:"))
                    {
                        let separator = if part.contains('=') { '=' } else { ':' };
                        format!(
                            "{}{}{}",
                            part.split(separator).next().unwrap_or(key),
                            separator,
                            self.replacement
                        )
                    } else {
                        part.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
        output
    }

    pub fn redact_args(&self, args: &[String]) -> Vec<String> {
        let mut redacted = Vec::with_capacity(args.len());
        let mut redact_next = false;
        for arg in args {
            let lower = arg.to_ascii_lowercase();
            if redact_next {
                redacted.push(self.replacement.clone());
                redact_next = false;
                continue;
            }
            if self
                .sensitive_keys
                .iter()
                .any(|key| lower == *key || lower.starts_with(&format!("{key}=")))
            {
                if arg.contains('=') {
                    redacted.push(format!(
                        "{}={}",
                        arg.split('=').next().unwrap_or_default(),
                        self.replacement
                    ));
                } else {
                    redacted.push(arg.clone());
                    redact_next = true;
                }
            } else {
                redacted.push(arg.clone());
            }
        }
        redacted
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionGuardReport {
    pub preflight: crate::ExtractPreflight,
    pub blocked_by_security: bool,
    pub requires_conflict_resolution: bool,
    pub estimated_write_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub severity: SecuritySeverity,
    pub entry: Option<EntryId>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecuritySeverity {
    Info,
    Warning,
    Block,
}

pub fn classify_entry_path(raw_path: &str) -> EntrySafety {
    if raw_path.starts_with(r"\\?\") || raw_path.starts_with(r"\\.\") {
        return blocked(SafetyBlockReason::DevicePath);
    }
    let normalized = raw_path.replace('\\', "/");
    if normalized.starts_with("//") {
        return blocked(SafetyBlockReason::UncPath);
    }
    if normalized.starts_with('/') {
        return blocked(SafetyBlockReason::AbsolutePath);
    }
    if normalized.len() > 240 {
        return EntrySafety::RequiresPolicy {
            reason: SafetyBlockReason::PathTooLong,
        };
    }
    if is_windows_drive_path(&normalized) {
        return blocked(SafetyBlockReason::WindowsDrivePath);
    }
    if Path::new(&normalized)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return blocked(SafetyBlockReason::ParentTraversal);
    }
    EntrySafety::Safe
}

pub fn safe_join(destination: &Path, raw_entry_path: &str) -> Result<PathBuf, ArchiveError> {
    match classify_entry_path(raw_entry_path) {
        EntrySafety::Safe => Ok(destination.join(raw_entry_path.replace('\\', "/"))),
        EntrySafety::Blocked { reason } | EntrySafety::RequiresPolicy { reason } => {
            Err(ArchiveError::new(
                ArchiveErrorKind::PathTraversalBlocked,
                "Archive entry path was blocked by the extraction safety policy",
            )
            .with_entry_path(raw_entry_path)
            .with_technical_detail(format!("{reason:?}")))
        }
    }
}

pub fn scan_listing_security(
    listing: &ArchiveListing,
    policy: &SecurityPolicy,
) -> Vec<SecurityFinding> {
    let mut findings = Vec::new();
    push_archive_level_findings(listing, policy, &mut findings);
    for entry in &listing.entries {
        if entry.raw_path.len() > policy.max_path_bytes {
            findings.push(block_entry(
                entry.id,
                "path-too-long",
                "Entry path is longer than the configured limit",
            ));
        }
        if entry.normalized_path.split('/').count() > policy.max_directory_depth {
            findings.push(block_entry(
                entry.id,
                "directory-too-deep",
                "Entry is nested too deeply",
            ));
        }
        if entry
            .size
            .is_some_and(|size| size > policy.max_single_entry_bytes)
        {
            findings.push(block_entry(
                entry.id,
                "entry-too-large",
                "Entry exceeds the configured size limit",
            ));
        }
        if suspicious_ratio(
            entry.size,
            entry.compressed_size,
            policy.max_compression_ratio,
        ) {
            findings.push(block_entry(
                entry.id,
                "suspicious-compression-ratio",
                "Entry has an abnormal compression ratio",
            ));
        }
        if policy.block_recursive_archives && looks_like_archive(&entry.normalized_path) {
            findings.push(warn_entry(
                entry.id,
                "nested-archive",
                "Entry appears to be another archive",
            ));
        }
    }
    findings
}

fn push_archive_level_findings(
    listing: &ArchiveListing,
    policy: &SecurityPolicy,
    findings: &mut Vec<SecurityFinding>,
) {
    if listing.entries.len() as u64 > policy.max_entries {
        findings.push(block_archive(
            "too-many-entries",
            "Archive contains too many entries",
        ));
    }
    let total = listing
        .entries
        .iter()
        .filter_map(|entry| entry.size)
        .sum::<u64>();
    if total > policy.max_total_uncompressed_bytes {
        findings.push(block_archive(
            "too-large-uncompressed",
            "Archive expands beyond the configured size limit",
        ));
    }
}

fn blocked(reason: SafetyBlockReason) -> EntrySafety {
    EntrySafety::Blocked { reason }
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn suspicious_ratio(size: Option<u64>, packed: Option<u64>, max_ratio: f64) -> bool {
    matches!((size, packed), (Some(size), Some(packed)) if packed > 0 && size as f64 / packed as f64 > max_ratio)
}

fn looks_like_archive(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".zip", ".7z", ".rar", ".tar", ".tar.gz", ".tar.xz", ".tar.zst",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}

fn block_archive(code: &str, message: &str) -> SecurityFinding {
    SecurityFinding {
        severity: SecuritySeverity::Block,
        entry: None,
        code: code.into(),
        message: message.into(),
    }
}

fn block_entry(entry: EntryId, code: &str, message: &str) -> SecurityFinding {
    SecurityFinding {
        severity: SecuritySeverity::Block,
        entry: Some(entry),
        code: code.into(),
        message: message.into(),
    }
}

fn warn_entry(entry: EntryId, code: &str, message: &str) -> SecurityFinding {
    SecurityFinding {
        severity: SecuritySeverity::Warning,
        entry: Some(entry),
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArchiveEntry, ArchiveListing, EntryKind};

    #[test]
    fn blocks_path_traversal() {
        assert!(matches!(
            classify_entry_path("../evil.txt"),
            EntrySafety::Blocked {
                reason: SafetyBlockReason::ParentTraversal
            }
        ));
    }

    #[test]
    fn blocks_windows_device_and_drive_relative_paths() {
        assert!(matches!(
            classify_entry_path(r"\\?\C:\evil.txt"),
            EntrySafety::Blocked {
                reason: SafetyBlockReason::DevicePath
            }
        ));
        assert!(matches!(
            classify_entry_path("C:evil.txt"),
            EntrySafety::Blocked {
                reason: SafetyBlockReason::WindowsDrivePath
            }
        ));
    }

    #[test]
    fn safe_join_stays_under_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = safe_join(dir.path(), "nested/file.txt").unwrap();
        assert!(path.starts_with(dir.path()));
    }

    #[test]
    fn detects_suspicious_compression_ratio() {
        let listing = ArchiveListing {
            entries: vec![ArchiveEntry {
                id: EntryId(1),
                raw_path: "huge.bin".into(),
                normalized_path: "huge.bin".into(),
                display_path: "huge.bin".into(),
                kind: EntryKind::File,
                size: Some(10_000_000),
                compressed_size: Some(1),
                modified_at: None,
                method: None,
                encrypted: false,
                safety: EntrySafety::Safe,
            }],
            directories: Default::default(),
            is_complete: true,
        };
        let findings = scan_listing_security(&listing, &SecurityPolicy::default());
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "suspicious-compression-ratio")
        );
    }
}
