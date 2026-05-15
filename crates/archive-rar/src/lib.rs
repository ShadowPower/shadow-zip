use std::path::Path;

use shadow_zip_archive_core::{ArchiveBackend, OpenArchive};
use shadow_zip_domain::*;

pub struct RarBackend {
    helper_available: bool,
}

impl RarBackend {
    pub fn new(helper_available: bool) -> Self {
        Self { helper_available }
    }

    pub fn discover() -> Self {
        which::which("unrar")
            .or_else(|_| which::which("rar"))
            .map(|_| Self {
                helper_available: true,
            })
            .unwrap_or_default()
    }
}

impl Default for RarBackend {
    fn default() -> Self {
        Self::new(false)
    }
}

impl ArchiveBackend for RarBackend {
    fn name(&self) -> &'static str {
        "unrar"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let confidence = source
            .path()
            .and_then(|path| path.extension())
            .and_then(|ext| ext.to_str())
            .filter(|ext| ext.eq_ignore_ascii_case("rar"))
            .map(|_| ProbeConfidence::Extension)
            .unwrap_or(ProbeConfidence::Impossible);

        Ok(ProbeResult {
            format: ArchiveFormat::Rar,
            confidence,
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let display_name = source.display_name();
        Ok(Box::new(RarArchive {
            source,
            password: options.password,
            info: ArchiveInfo {
                format: ArchiveFormat::Rar,
                display_name,
                total_bytes: None,
                entry_count: None,
                codecs: vec!["RAR".into(), "RAR5".into()],
                filters: Vec::new(),
                is_solid: false,
                is_encrypted: false,
                has_header_encryption: false,
                is_multi_volume: false,
            },
        }))
    }

    fn create_plan(
        &self,
        _inputs: &[InputPath],
        _output: &Path,
        _options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "RAR creation is intentionally not built in because it requires RARLAB licensing",
        )
        .with_backend(self.name()))
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::Rar],
            capabilities: rar_capabilities(self.helper_available),
        }
    }
}

struct RarArchive {
    source: ArchiveSource,
    password: Option<String>,
    info: ArchiveInfo,
}

impl OpenArchive for RarArchive {
    fn info(&self) -> ArchiveInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        rar_capabilities(true)
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "RAR listing requires a local file",
            ));
        };
        let archive = rar_archive(path, self.password.as_ref()).open_for_listing();
        let mut archive = archive.map_err(map_unrar_crate_error)?;
        let mut listing = ArchiveListing::default();
        for (index, header) in (&mut archive).enumerate() {
            let header = header.map_err(map_unrar_crate_error)?;
            let name = header.filename.to_string_lossy().replace('\\', "/");
            listing.entries.push(ArchiveEntry {
                id: EntryId(index as u64),
                raw_path: name.clone(),
                normalized_path: name.clone(),
                display_path: name.clone(),
                kind: if header.is_directory() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: Some(header.unpacked_size),
                compressed_size: None,
                modified_at: None,
                method: Some(self.info.format.to_string()),
                encrypted: header.is_encrypted(),
                safety: classify_entry_path(&name),
            });
        }
        listing.is_complete = true;
        self.info.entry_count = Some(listing.entries.len() as u64);
        self.info.is_encrypted = listing.entries.iter().any(|entry| entry.encrypted);
        self.info.has_header_encryption = archive.has_encrypted_headers();
        self.info.is_solid = archive.is_solid();
        Ok(listing)
    }

    fn extract_all(
        &mut self,
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract_to(destination, None, options)?;
        let plan = TaskPlan::new(
            TaskKind::Extract,
            format!("Extract RAR to {}", destination.display()),
        );
        Ok(plan)
    }

    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract_to(destination, Some(entries), options)?;
        let mut plan = TaskPlan::new(
            TaskKind::Extract,
            format!("Extract RAR to {}", destination.display()),
        );
        plan.estimated_entries = Some(entries.len() as u64);
        Ok(plan)
    }

    fn open_entry_stream(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryStream, ArchiveError> {
        Ok(EntryStream {
            entry,
            access_cost: AccessCost::ExternalHelper,
        })
    }

    fn test(&mut self, options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        self.test_archive(options)?;
        Ok(TaskPlan::new(TaskKind::Test, "Test RAR archive"))
    }
}

impl RarArchive {
    fn extract_to(
        &self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<(), ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "RAR extraction requires a local file",
            ));
        };
        let password = options.password.as_ref().or(self.password.as_ref());
        let mut archive = rar_archive(path, password)
            .open_for_processing()
            .map_err(map_unrar_crate_error)?;
        let mut index = 0_u64;
        while let Some(header) = archive.read_header().map_err(map_unrar_crate_error)? {
            let current = EntryId(index);
            index += 1;
            archive = if selected.is_none_or(|ids| ids.contains(&current)) {
                if header.entry().is_file() {
                    header
                        .extract_with_base(destination)
                        .map_err(map_unrar_crate_error)?
                } else {
                    header.skip().map_err(map_unrar_crate_error)?
                }
            } else {
                header.skip().map_err(map_unrar_crate_error)?
            };
        }
        Ok(())
    }

    fn test_archive(&self, options: TestOptions) -> Result<(), ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "RAR testing requires a local file",
            ));
        };
        let password = options.password.as_ref().or(self.password.as_ref());
        let mut archive = rar_archive(path, password)
            .open_for_processing()
            .map_err(map_unrar_crate_error)?;
        while let Some(header) = archive.read_header().map_err(map_unrar_crate_error)? {
            archive = if header.entry().is_file() {
                header.test().map_err(map_unrar_crate_error)?
            } else {
                header.skip().map_err(map_unrar_crate_error)?
            };
        }
        Ok(())
    }
}

fn rar_archive<'a>(path: &'a Path, password: Option<&'a String>) -> unrar::Archive<'a> {
    match password {
        Some(password) => unrar::Archive::with_password(path, password),
        None => unrar::Archive::new(path),
    }
}

pub fn parse_unrar_listing(output: &str, info: &ArchiveInfo) -> ArchiveListing {
    let mut listing = ArchiveListing::default();
    for (index, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("UNRAR")
            || trimmed.starts_with("Archive:")
            || trimmed.starts_with("----")
        {
            continue;
        }
        let columns = trimmed.split_whitespace().collect::<Vec<_>>();
        let name = columns
            .last()
            .copied()
            .unwrap_or(trimmed)
            .replace('\\', "/");
        let size = columns.iter().find_map(|part| part.parse::<u64>().ok());
        let kind = if name.ends_with('/') {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        listing.entries.push(ArchiveEntry {
            id: EntryId(index as u64),
            raw_path: name.clone(),
            normalized_path: name.clone(),
            display_path: name.clone(),
            kind,
            size,
            compressed_size: None,
            modified_at: None,
            method: Some(info.format.to_string()),
            encrypted: trimmed.contains('*'),
            safety: classify_entry_path(&name),
        });
    }
    listing.is_complete = true;
    listing
}

pub fn map_unrar_error(stderr: &str) -> ArchiveError {
    let lower = stderr.to_ascii_lowercase();
    let kind = if lower.contains("password") || lower.contains("encrypted") {
        ArchiveErrorKind::InvalidPassword
    } else if lower.contains("checksum") || lower.contains("crc") || lower.contains("corrupt") {
        ArchiveErrorKind::CorruptArchive
    } else if lower.contains("cannot open") || lower.contains("not found") {
        ArchiveErrorKind::Io
    } else {
        ArchiveErrorKind::ExternalHelperFailed
    };
    ArchiveError::new(kind, "UnRAR helper failed")
        .with_backend("unrar")
        .with_technical_detail(RedactionPolicy::default().redact_text(stderr))
}

fn map_unrar_crate_error(error: unrar::error::UnrarError) -> ArchiveError {
    map_unrar_error(&error.to_string())
}

fn rar_capabilities(helper_available: bool) -> ArchiveCapabilities {
    let _ = helper_available;
    let helper_level = CapabilityLevel::Full;

    ArchiveCapabilities {
        list: helper_level,
        extract_all: helper_level,
        extract_selected: helper_level,
        create: CapabilityLevel::Unsupported,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Limited,
        password_read: helper_level,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: helper_level,
        multi_volume_read: helper_level,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Limited,
    }
}
