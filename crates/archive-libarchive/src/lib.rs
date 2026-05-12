use std::path::{Path, PathBuf};

use shadow_zip_archive_core::{ArchiveBackend, OpenArchive};
use shadow_zip_domain::*;

pub struct LibarchiveBackend {
    available: bool,
    executable: PathBuf,
}

impl LibarchiveBackend {
    pub fn new(available: bool) -> Self {
        Self {
            available,
            executable: PathBuf::from("bsdtar"),
        }
    }

    pub fn with_executable(executable: PathBuf) -> Self {
        Self {
            available: true,
            executable,
        }
    }
}

impl Default for LibarchiveBackend {
    fn default() -> Self {
        Self::new(false)
    }
}

impl ArchiveBackend for LibarchiveBackend {
    fn name(&self) -> &'static str {
        "libarchive-fallback"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let confidence = if self.available && source.path().is_some() {
            ProbeConfidence::Extension
        } else {
            ProbeConfidence::Impossible
        };
        Ok(ProbeResult {
            format: ArchiveFormat::Unknown,
            confidence,
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        if !self.available {
            return Err(ArchiveError::new(
                ArchiveErrorKind::BackendUnavailable,
                "libarchive fallback is not available",
            )
            .with_backend(self.name()));
        }

        let info = ArchiveInfo::unknown(source.display_name());
        Ok(Box::new(LibarchiveArchive {
            source,
            executable: self.executable.clone(),
            info,
        }))
    }

    fn create_plan(
        &self,
        _inputs: &[InputPath],
        output: &Path,
        _options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let mut plan = TaskPlan::new(
            TaskKind::Create,
            format!("Create with fallback {}", output.display()),
        );
        plan.requires_external_helper = true;
        plan.execution = libarchive_plan(&["create"], Some(output));
        Ok(plan)
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::Unknown],
            capabilities: fallback_capabilities(self.available),
        }
    }
}

struct LibarchiveArchive {
    source: ArchiveSource,
    executable: PathBuf,
    info: ArchiveInfo,
}

impl OpenArchive for LibarchiveArchive {
    fn info(&self) -> ArchiveInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        fallback_capabilities(true)
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        Ok(parse_bsdtar_listing("", &self.info))
    }

    fn extract_all(
        &mut self,
        destination: &Path,
        _options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let mut plan = TaskPlan::new(
            TaskKind::Extract,
            format!("Fallback extract to {}", destination.display()),
        );
        plan.requires_external_helper = true;
        plan.execution = self.libarchive_plan(&["-xf"], Some(destination));
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
            format!("Fallback extract to {}", destination.display()),
        );
        plan.estimated_entries = Some(entries.len() as u64);
        plan.requires_external_helper = true;
        plan.execution = self.libarchive_plan(&["-xf"], Some(destination));
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
        let mut plan = TaskPlan::new(TaskKind::Test, "Fallback archive test");
        plan.requires_external_helper = true;
        plan.execution = self.libarchive_plan(&["-tf"], None);
        Ok(plan)
    }
}

impl LibarchiveArchive {
    fn libarchive_plan(&self, args: &[&str], destination: Option<&Path>) -> TaskExecutionPlan {
        let source = self.source.path().map(|path| path.display().to_string());
        TaskExecutionPlan::ExternalHelper(ExternalHelperPlan {
            helper_kind: ExternalHelperKind::Libarchive,
            executable: self.executable.clone(),
            args: args
                .iter()
                .map(|arg| (*arg).to_string())
                .chain(source)
                .chain(destination.map(|path| path.display().to_string()))
                .collect(),
            working_dir: destination.map(Path::to_path_buf),
            timeout_ms: 60 * 60 * 1000,
            output_limit_bytes: 8 * 1024 * 1024,
            redacted_args: args
                .iter()
                .map(|arg| (*arg).to_string())
                .chain(["<archive>".to_string()])
                .chain(destination.map(|_| "<destination>".to_string()))
                .collect(),
        })
    }
}

fn libarchive_plan(args: &[&str], path: Option<&Path>) -> TaskExecutionPlan {
    TaskExecutionPlan::ExternalHelper(ExternalHelperPlan {
        helper_kind: ExternalHelperKind::Libarchive,
        executable: Path::new("bsdtar").to_path_buf(),
        args: args
            .iter()
            .map(|arg| (*arg).to_string())
            .chain(path.map(|path| path.display().to_string()))
            .collect(),
        working_dir: None,
        timeout_ms: 60 * 60 * 1000,
        output_limit_bytes: 8 * 1024 * 1024,
        redacted_args: args
            .iter()
            .map(|arg| (*arg).to_string())
            .chain(path.map(|_| "<path>".to_string()))
            .collect(),
    })
}

pub fn parse_bsdtar_listing(output: &str, info: &ArchiveInfo) -> ArchiveListing {
    let mut listing = ArchiveListing::default();
    for (index, line) in output.lines().enumerate() {
        let path = line.trim().replace('\\', "/");
        if path.is_empty() {
            continue;
        }
        listing.entries.push(ArchiveEntry {
            id: EntryId(index as u64),
            raw_path: path.clone(),
            normalized_path: path.clone(),
            display_path: path.clone(),
            kind: if path.ends_with('/') {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: None,
            compressed_size: None,
            modified_at: None,
            method: Some(info.format.to_string()),
            encrypted: false,
            safety: classify_entry_path(&path),
        });
    }
    listing.is_complete = true;
    listing
}

pub fn map_libarchive_error(stderr: &str) -> ArchiveError {
    let lower = stderr.to_ascii_lowercase();
    let kind = if lower.contains("unsupported") {
        ArchiveErrorKind::UnsupportedFormat
    } else if lower.contains("password") {
        ArchiveErrorKind::InvalidPassword
    } else if lower.contains("truncated") || lower.contains("corrupt") {
        ArchiveErrorKind::CorruptArchive
    } else {
        ArchiveErrorKind::ExternalHelperFailed
    };
    ArchiveError::new(kind, "libarchive fallback failed")
        .with_backend("libarchive")
        .with_technical_detail(RedactionPolicy::default().redact_text(stderr))
}

fn fallback_capabilities(available: bool) -> ArchiveCapabilities {
    let level = if available {
        CapabilityLevel::External
    } else {
        CapabilityLevel::Unsupported
    };

    ArchiveCapabilities {
        list: level,
        extract_all: level,
        extract_selected: level,
        create: CapabilityLevel::Limited,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Limited,
        password_read: CapabilityLevel::Limited,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Limited,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Limited,
    }
}
