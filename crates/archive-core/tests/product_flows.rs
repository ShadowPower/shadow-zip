use std::io::Cursor;

use shadow_zip_archive_core::{PreflightService, SafeWriter, StreamLimits};
use shadow_zip_domain::*;

fn entry(id: u64, path: &str, size: u64, packed: u64) -> ArchiveEntry {
    ArchiveEntry {
        id: EntryId(id),
        raw_path: path.into(),
        normalized_path: path.into(),
        display_path: path.into(),
        kind: EntryKind::File,
        size: Some(size),
        compressed_size: Some(packed),
        modified_at: None,
        method: Some("deflate".into()),
        encrypted: false,
        safety: classify_entry_path(path),
    }
}

#[test]
fn preflight_reports_conflicts_and_security_findings() {
    let dir = tempfile::tempdir().unwrap();
    fs_err::write(dir.path().join("exists.txt"), b"old").unwrap();
    let listing = ArchiveListing {
        entries: vec![
            entry(1, "exists.txt", 10, 5),
            entry(2, "../escape.txt", 1, 1),
        ],
        directories: Default::default(),
        is_complete: true,
    };

    let preflight = PreflightService::new(SecurityPolicy::default())
        .check_listing(&listing, dir.path().to_path_buf());

    assert_eq!(preflight.conflicts.len(), 1);
    assert_eq!(preflight.blocked_entries.len(), 1);
}

#[test]
fn safe_writer_supports_overwrite_skip_and_rename_policies() {
    let dir = tempfile::tempdir().unwrap();
    fs_err::write(dir.path().join("file.txt"), b"old").unwrap();

    let writer = SafeWriter::new(dir.path().to_path_buf(), StreamLimits::default())
        .with_overwrite_policy(OverwritePolicy::Rename);
    writer
        .write_stream("file.txt", &mut Cursor::new(b"new"), |_| Ok(()))
        .unwrap();

    assert!(dir.path().join("file (1).txt").exists());
}
