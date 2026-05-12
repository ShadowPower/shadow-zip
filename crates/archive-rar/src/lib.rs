use std::path::{Path, PathBuf};

use shadow_zip_archive_core::{ArchiveBackend, OpenArchive, helper_plan};
use shadow_zip_domain::*;

pub struct RarBackend {
    helper_available: bool,
    helper_path: PathBuf,
}

impl RarBackend {
    pub fn new(helper_available: bool) -> Self {
        Self {
            helper_available,
            helper_path: PathBuf::from("unrar"),
        }
    }

    pub fn discover() -> Self {
        which::which("unrar")
            .or_else(|_| which::which("rar"))
            .map(|helper_path| Self {
                helper_available: true,
                helper_path,
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
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        if !self.helper_available {
            return Err(ArchiveError::new(
                ArchiveErrorKind::BackendUnavailable,
                "RAR support requires an UnRAR-compatible helper",
            )
            .with_backend(self.name()));
        }

        let display_name = source.display_name();
        Ok(Box::new(RarArchive {
            source,
            helper_path: self.helper_path.clone(),
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
    helper_path: PathBuf,
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
        let archive_path = path.to_string_lossy().into_owned();
        let args = ["v", "-c-", archive_path.as_str()];
        let plan = helper_plan(
            ExternalHelperKind::Unrar,
            &self.helper_path,
            args,
            ["v", "-c-", "<archive>"],
        );
        let _ = plan;
        Ok(parse_unrar_listing("", &self.info))
    }

    fn extract_all(
        &mut self,
        destination: &Path,
        _options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let mut plan = TaskPlan::new(
            TaskKind::Extract,
            format!("Extract RAR to {}", destination.display()),
        );
        plan.requires_external_helper = true;
        plan.execution =
            self::unrar_plan(&self.helper_path, &self.source, destination, &["x", "-y"]);
        Ok(plan)
    }

    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        _options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let mut plan = TaskPlan::new(
            TaskKind::Extract,
            format!("Extract RAR to {}", destination.display()),
        );
        plan.estimated_entries = Some(entries.len() as u64);
        plan.requires_external_helper = true;
        plan.execution =
            self::unrar_plan(&self.helper_path, &self.source, destination, &["x", "-y"]);
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

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        let mut plan = TaskPlan::new(TaskKind::Test, "Test RAR archive");
        plan.requires_external_helper = true;
        plan.execution = TaskExecutionPlan::ExternalHelper(ExternalHelperPlan {
            helper_kind: ExternalHelperKind::Unrar,
            executable: self.helper_path.clone(),
            args: self
                .source
                .path()
                .map(|p| vec!["t".into(), p.display().to_string()])
                .unwrap_or_else(|| vec!["t".into()]),
            working_dir: None,
            timeout_ms: 30 * 60 * 1000,
            output_limit_bytes: 4 * 1024 * 1024,
            redacted_args: vec!["t".into()],
        });
        Ok(plan)
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

fn unrar_plan(
    helper_path: &Path,
    source: &ArchiveSource,
    destination: &Path,
    args: &[&str],
) -> TaskExecutionPlan {
    TaskExecutionPlan::ExternalHelper(ExternalHelperPlan {
        helper_kind: ExternalHelperKind::Unrar,
        executable: helper_path.to_path_buf(),
        args: args
            .iter()
            .map(|arg| (*arg).to_string())
            .chain(source.path().map(|path| path.display().to_string()))
            .chain([destination.display().to_string()])
            .collect(),
        working_dir: None,
        timeout_ms: 60 * 60 * 1000,
        output_limit_bytes: 8 * 1024 * 1024,
        redacted_args: args
            .iter()
            .map(|arg| (*arg).to_string())
            .chain(["<destination>".into()])
            .collect(),
    })
}

fn rar_capabilities(helper_available: bool) -> ArchiveCapabilities {
    let helper_level = if helper_available {
        CapabilityLevel::External
    } else {
        CapabilityLevel::Unsupported
    };

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
