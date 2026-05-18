use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use chrono::Utc;
use fs_err as fs;
use path_clean::PathClean;
use shadow_zip_domain::*;
use walkdir::WalkDir;

pub trait ArchiveBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError>;
    fn open(
        &self,
        source: ArchiveSource,
        options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError>;
    fn create_plan(
        &self,
        inputs: &[InputPath],
        output: &Path,
        options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError>;
    fn backend_capabilities(&self) -> BackendCapabilities;
}

pub trait OpenArchive: Send {
    fn info(&self) -> ArchiveInfo;
    fn capabilities(&self) -> ArchiveCapabilities;
    fn listing(&mut self, mode: ListingMode) -> Result<ArchiveListing, ArchiveError>;
    fn extract_all(
        &mut self,
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError>;
    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError>;
    fn open_entry_stream(
        &mut self,
        entry: EntryId,
        options: StreamOptions,
    ) -> Result<EntryStream, ArchiveError>;
    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let stream = self.open_entry_stream(entry, options)?;
        Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "This backend does not expose entry bytes",
        )
        .with_technical_detail(format!("access_cost={:?}", stream.access_cost)))
    }
    fn test(&mut self, options: TestOptions) -> Result<TaskPlan, ArchiveError>;
}

pub struct ArchiveService {
    backends: Vec<Box<dyn ArchiveBackend>>,
    sessions: BTreeMap<SessionId, ArchiveSession>,
    recent_files: Vec<RecentFile>,
}

impl ArchiveService {
    pub fn new(backends: Vec<Box<dyn ArchiveBackend>>) -> Self {
        Self {
            backends,
            sessions: BTreeMap::new(),
            recent_files: Vec::new(),
        }
    }

    pub fn backends(&self) -> &[Box<dyn ArchiveBackend>] {
        &self.backends
    }

    pub fn open_best(
        &self,
        source: ArchiveSource,
        options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let mut probes = self
            .backends
            .iter()
            .filter_map(|backend| backend.probe(&source).ok().map(|probe| (backend, probe)))
            .filter(|(_, probe)| probe.confidence > ProbeConfidence::Impossible)
            .collect::<Vec<_>>();

        probes.sort_by_key(|(_, probe)| probe.confidence);

        let mut failures = Vec::new();
        for (backend, _) in probes.into_iter().rev() {
            match backend.open(source.clone(), options.clone()) {
                Ok(open) => return Ok(open),
                Err(error) => failures.push(error),
            }
        }

        Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "No archive backend could open this source",
        )
        .with_causes(failures))
    }

    pub fn open_session(
        &mut self,
        source: ArchiveSource,
        options: OpenOptions,
        recent_config: &RecentFilesConfig,
    ) -> Result<SessionId, ArchiveError> {
        let mut archive = self.open_best(source.clone(), options.clone())?;
        let info = archive.info();
        let capabilities = archive.capabilities();
        let listing = archive.listing(ListingMode::Full)?;
        let id = SessionId::new();
        self.sessions.insert(
            id,
            ArchiveSession {
                id,
                source: source.clone(),
                info: info.clone(),
                capabilities,
                listing,
                selected_entries: BTreeSet::new(),
                current_directory: "/".into(),
                filter: EntryFilter::default(),
                sort: EntrySort::default(),
                password_memory: options.password,
            },
        );
        self.remember_recent_file(source, info.format, recent_config);
        Ok(id)
    }

    pub fn snapshot(&self, id: SessionId) -> Option<ArchiveSessionSnapshot> {
        self.sessions.get(&id).map(ArchiveSession::snapshot)
    }

    pub fn close_session(&mut self, id: SessionId) {
        if let Some(mut session) = self.sessions.remove(&id) {
            session.password_memory.take();
        }
    }

    pub fn recent_files(&self) -> &[RecentFile] {
        &self.recent_files
    }

    pub fn load_recent_files(&mut self, path: &Path) -> Result<(), ArchiveError> {
        if path.exists() {
            self.recent_files = serde_json::from_str(&fs::read_to_string(path).map_err(io_error)?)
                .map_err(|error| {
                    ArchiveError::new(ArchiveErrorKind::Internal, "Could not parse recent files")
                        .with_technical_detail(error.to_string())
                })?;
        }
        Ok(())
    }

    pub fn save_recent_files(&self, path: &Path) -> Result<(), ArchiveError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        fs::write(
            path,
            serde_json::to_string_pretty(&self.recent_files).unwrap_or_default(),
        )
        .map_err(io_error)
    }

    fn remember_recent_file(
        &mut self,
        source: ArchiveSource,
        format: ArchiveFormat,
        config: &RecentFilesConfig,
    ) {
        if !config.enabled {
            return;
        }
        self.recent_files.retain(|recent| recent.source != source);
        self.recent_files.insert(
            0,
            RecentFile {
                display_name: source.display_name(),
                source,
                last_opened_unix_ms: Utc::now().timestamp_millis(),
                format,
            },
        );
        self.recent_files.truncate(config.max_items);
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveSession {
    pub id: SessionId,
    pub source: ArchiveSource,
    pub info: ArchiveInfo,
    pub capabilities: ArchiveCapabilities,
    pub listing: ArchiveListing,
    pub selected_entries: BTreeSet<EntryId>,
    pub current_directory: String,
    pub filter: EntryFilter,
    pub sort: EntrySort,
    pub password_memory: Option<String>,
}

impl ArchiveSession {
    pub fn snapshot(&self) -> ArchiveSessionSnapshot {
        ArchiveSessionSnapshot {
            id: self.id,
            source: self.source.clone(),
            info: self.info.clone(),
            capabilities: self.capabilities.clone(),
            listing: self.listing.clone(),
            selected_entries: self.selected_entries.clone(),
            current_directory: self.current_directory.clone(),
            filter: self.filter.clone(),
            sort: self.sort,
        }
    }
}

pub fn extension_confidence(source: &ArchiveSource, extensions: &[&str]) -> ProbeConfidence {
    source
        .path()
        .and_then(|path| path.extension())
        .and_then(|ext| ext.to_str())
        .filter(|ext| {
            extensions
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
        .map(|_| ProbeConfidence::Extension)
        .unwrap_or(ProbeConfidence::Impossible)
}

pub fn create_pipeline() -> NativePipelinePlan {
    NativePipelinePlan::new(vec![
        PipelineStep::CheckDestination,
        PipelineStep::WriteArchiveHeader,
        PipelineStep::ValidateEntryPath,
        PipelineStep::WriteArchiveEntry,
        PipelineStep::FinalizeArchive,
    ])
}

pub fn random_access_extract_pipeline(open_step: PipelineStep) -> NativePipelinePlan {
    let mut steps = vec![open_step];
    steps.extend(extract_write_steps());
    NativePipelinePlan::new(steps)
}

pub fn sequential_extract_pipeline(mut prefix: Vec<PipelineStep>) -> NativePipelinePlan {
    prefix.extend(extract_write_steps());
    NativePipelinePlan::new(prefix)
}

pub fn quick_test_pipeline(steps: Vec<PipelineStep>) -> NativePipelinePlan {
    NativePipelinePlan::quick(steps)
}

pub fn helper_plan(
    helper_kind: ExternalHelperKind,
    executable: impl Into<std::path::PathBuf>,
    args: impl IntoIterator<Item = impl Into<String>>,
    redacted_args: impl IntoIterator<Item = impl Into<String>>,
) -> ExternalHelperPlan {
    ExternalHelperPlan {
        helper_kind,
        executable: executable.into(),
        args: args.into_iter().map(Into::into).collect(),
        working_dir: None,
        timeout_ms: 60 * 60 * 1000,
        output_limit_bytes: 8 * 1024 * 1024,
        redacted_args: redacted_args.into_iter().map(Into::into).collect(),
    }
}

fn extract_write_steps() -> Vec<PipelineStep> {
    vec![
        PipelineStep::ValidateEntryPath,
        PipelineStep::CheckDestination,
        PipelineStep::ResolveConflict,
        PipelineStep::CreateDirectory,
        PipelineStep::WriteFile,
        PipelineStep::PreserveMetadata,
    ]
}

pub trait ByteSource {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
}

pub trait ByteSink {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn finish(&mut self) -> io::Result<()>;
}

impl<T: Read> ByteSource for T {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.read(buffer)
    }
}

impl<T: Write> ByteSink for T {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(bytes)
    }

    fn finish(&mut self) -> io::Result<()> {
        self.flush()
    }
}

#[derive(Debug, Clone)]
pub struct StreamLimits {
    pub buffer_size: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            buffer_size: 128 * 1024,
            max_entry_bytes: 64 * 1024 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024 * 1024,
        }
    }
}

pub struct StreamPump {
    limits: StreamLimits,
}

impl StreamPump {
    pub fn new(limits: StreamLimits) -> Self {
        Self { limits }
    }

    pub fn copy(
        &self,
        source: &mut dyn ByteSource,
        sink: &mut dyn ByteSink,
        mut on_chunk: impl FnMut(u64) -> Result<(), ArchiveError>,
    ) -> Result<u64, ArchiveError> {
        let mut buffer = vec![0; self.limits.buffer_size];
        let mut total = 0;
        loop {
            let read = source.read_chunk(&mut buffer).map_err(io_error)?;
            if read == 0 {
                sink.finish().map_err(io_error)?;
                return Ok(total);
            }
            total += read as u64;
            if total > self.limits.max_entry_bytes || total > self.limits.max_total_bytes {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Internal,
                    "Stream exceeded configured size limit",
                ));
            }
            sink.write_chunk(&buffer[..read]).map_err(io_error)?;
            on_chunk(read as u64)?;
        }
    }
}

pub struct SafeWriter {
    destination_root: PathBuf,
    limits: StreamLimits,
    overwrite_policy: OverwritePolicy,
}

impl SafeWriter {
    pub fn new(destination_root: PathBuf, limits: StreamLimits) -> Self {
        Self {
            destination_root,
            limits,
            overwrite_policy: OverwritePolicy::AskBatch,
        }
    }

    pub fn with_overwrite_policy(mut self, overwrite_policy: OverwritePolicy) -> Self {
        self.overwrite_policy = overwrite_policy;
        self
    }

    pub fn target_path(&self, entry_path: &str) -> Result<PathBuf, ArchiveError> {
        let target = safe_join(&self.destination_root, entry_path)?.clean();
        let root = self.destination_root.clean();
        if !target.starts_with(&root) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::PathTraversalBlocked,
                "Resolved extraction path escaped the destination directory",
            )
            .with_entry_path(entry_path));
        }
        Ok(target)
    }

    pub fn write_stream(
        &self,
        entry_path: &str,
        source: &mut dyn ByteSource,
        on_chunk: impl FnMut(u64) -> Result<(), ArchiveError>,
    ) -> Result<u64, ArchiveError> {
        let target = self.target_path(entry_path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let target = self.resolve_conflict(target)?;
        let mut sink = FileSink::create(target)?;
        StreamPump::new(self.limits.clone()).copy(source, &mut sink, on_chunk)
    }

    pub fn create_dir(&self, entry_path: &str) -> Result<PathBuf, ArchiveError> {
        let target = self.target_path(entry_path)?;
        fs::create_dir_all(&target).map_err(io_error)?;
        Ok(target)
    }

    fn resolve_conflict(&self, target: PathBuf) -> Result<PathBuf, ArchiveError> {
        if !target.exists() {
            return Ok(target);
        }
        match self.overwrite_policy {
            OverwritePolicy::Overwrite => Ok(target),
            OverwritePolicy::Skip => Err(ArchiveError::new(
                ArchiveErrorKind::Cancelled,
                "Entry skipped because target already exists",
            )),
            OverwritePolicy::Rename => Ok(next_available_path(target)),
            OverwritePolicy::KeepNewer => Ok(target),
            OverwritePolicy::AskBatch => Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Extraction requires conflict resolution before writing",
            )),
        }
    }
}

pub struct FileSink {
    file: fs::File,
}

impl FileSink {
    pub fn create(path: PathBuf) -> Result<Self, ArchiveError> {
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(io_error)?;
        Ok(Self { file })
    }
}

impl ByteSink for FileSink {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }

    fn finish(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

pub struct PreflightService {
    policy: SecurityPolicy,
}

impl PreflightService {
    pub fn new(policy: SecurityPolicy) -> Self {
        Self { policy }
    }

    pub fn check_listing(
        &self,
        listing: &ArchiveListing,
        destination: PathBuf,
    ) -> ExtractPreflight {
        let destination_status = validate_destination(&destination);
        let estimated_bytes = Some(listing.entries.iter().filter_map(|entry| entry.size).sum());
        let disk_warning = estimated_bytes
            .and_then(|bytes| check_available_space(&destination, bytes).err())
            .map(|error| TaskWarning {
                code: "insufficient-disk-space".into(),
                message: error.message,
            });
        let blocked_entries = listing
            .entries
            .iter()
            .filter_map(|entry| match classify_entry_path(&entry.raw_path) {
                EntrySafety::Safe => None,
                EntrySafety::Blocked { reason } | EntrySafety::RequiresPolicy { reason } => {
                    Some(BlockedEntry {
                        entry: entry.id,
                        entry_path: entry.raw_path.clone(),
                        reason,
                    })
                }
            })
            .collect();

        let security_warnings = scan_listing_security(listing, &self.policy)
            .into_iter()
            .map(|finding| TaskWarning {
                code: finding.code,
                message: finding.message,
            })
            .chain(destination_status.err().map(|error| TaskWarning {
                code: "destination-invalid".into(),
                message: error.message,
            }))
            .chain(disk_warning)
            .collect();

        let conflicts = detect_conflicts(listing, &destination);

        ExtractPreflight {
            destination,
            total_entries: listing.entries.len() as u64,
            estimated_bytes,
            conflicts,
            blocked_entries,
            warnings: security_warnings,
        }
    }

    pub fn guard_listing(
        &self,
        listing: &ArchiveListing,
        destination: PathBuf,
    ) -> ExtractionGuardReport {
        let preflight = self.check_listing(listing, destination);
        ExtractionGuardReport {
            blocked_by_security: !preflight.blocked_entries.is_empty()
                || preflight.warnings.iter().any(|warning| {
                    warning.code.contains("too-") || warning.code.contains("blocked")
                }),
            requires_conflict_resolution: !preflight.conflicts.is_empty(),
            estimated_write_bytes: preflight.estimated_bytes,
            preflight,
        }
    }
}

pub struct ExtractionGuard {
    preflight: PreflightService,
    limits: StreamLimits,
}

impl ExtractionGuard {
    pub fn new(policy: SecurityPolicy, limits: StreamLimits) -> Self {
        Self {
            preflight: PreflightService::new(policy),
            limits,
        }
    }

    pub fn preflight(
        &self,
        listing: &ArchiveListing,
        destination: PathBuf,
    ) -> ExtractionGuardReport {
        self.preflight.guard_listing(listing, destination)
    }

    pub fn writer(&self, destination: PathBuf, options: &ExtractOptions) -> SafeWriter {
        SafeWriter::new(destination, self.limits.clone())
            .with_overwrite_policy(options.overwrite_policy)
    }

    pub fn ensure_can_extract(
        &self,
        listing: &ArchiveListing,
        destination: PathBuf,
    ) -> Result<ExtractionGuardReport, ArchiveError> {
        let report = self.preflight(listing, destination);
        if report.blocked_by_security {
            return Err(ArchiveError::new(
                ArchiveErrorKind::PathTraversalBlocked,
                "Extraction was blocked by safety preflight",
            ));
        }
        Ok(report)
    }
}

fn validate_destination(destination: &Path) -> Result<(), ArchiveError> {
    if !destination.exists() {
        fs::create_dir_all(destination).map_err(io_error)?;
    }
    if !destination.is_dir() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::PermissionDenied,
            "Extraction destination is not a directory",
        ));
    }
    let probe = destination.join(".shadow-zip-write-test");
    fs::write(&probe, b"test").map_err(|error| {
        ArchiveError::new(
            ArchiveErrorKind::PermissionDenied,
            "Extraction destination is not writable",
        )
        .with_technical_detail(error.to_string())
    })?;
    let _ = fs::remove_file(probe);
    Ok(())
}

fn check_available_space(destination: &Path, required_bytes: u64) -> Result<(), ArchiveError> {
    let available = fs2::available_space(destination).map_err(io_error)?;
    if available < required_bytes {
        return Err(ArchiveError::new(
            ArchiveErrorKind::InsufficientDiskSpace,
            format!(
                "Destination has {available} bytes available but {required_bytes} bytes are required"
            ),
        ));
    }
    Ok(())
}

pub struct ArchiveExecutor {
    limits: StreamLimits,
}

impl ArchiveExecutor {
    pub fn new(limits: StreamLimits) -> Self {
        Self { limits }
    }

    pub fn extract_streams(
        &self,
        entries: impl IntoIterator<Item = ExtractStreamItem>,
        destination: PathBuf,
        options: ExtractOptions,
        mut on_progress: impl FnMut(&ExtractStreamItem, u64) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let writer = SafeWriter::new(destination, self.limits.clone())
            .with_overwrite_policy(options.overwrite_policy);
        for item in entries {
            let ExtractStreamItem {
                entry,
                path,
                kind,
                mut source,
            } = item;
            match kind {
                EntryKind::Directory => {
                    writer.create_dir(&path)?;
                }
                EntryKind::File | EntryKind::Other => {
                    let progress_item = ExtractStreamItem {
                        entry,
                        path: path.clone(),
                        kind,
                        source: Box::new(io::empty()),
                    };
                    writer.write_stream(&path, source.as_mut(), |bytes| {
                        on_progress(&progress_item, bytes)
                    })?;
                }
                EntryKind::Symlink => {
                    if matches!(options.symlink_policy, SymlinkPolicy::Conservative) {
                        return Err(ArchiveError::new(
                            ArchiveErrorKind::SymlinkPolicyBlocked,
                            "Symlink extraction is blocked by policy",
                        )
                        .with_entry_path(path));
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct ExtractStreamItem {
    pub entry: EntryId,
    pub path: String,
    pub kind: EntryKind,
    pub source: Box<dyn ByteSource>,
}

pub struct EntryReader {
    pub entry: EntryId,
    pub access_cost: AccessCost,
    pub source: Box<dyn ByteSource>,
    pub size: Option<u64>,
}

pub struct InputScanner;

impl InputScanner {
    pub fn scan(inputs: &[InputPath]) -> Result<Vec<ScannedInput>, ArchiveError> {
        let mut scanned = Vec::new();
        for input in inputs {
            if input.path.is_dir() {
                for entry in WalkDir::new(&input.path).follow_links(false) {
                    let entry = entry.map_err(|error| {
                        ArchiveError::new(ArchiveErrorKind::Io, "Could not scan input directory")
                            .with_technical_detail(error.to_string())
                    })?;
                    let path = entry.path().to_path_buf();
                    let archive_path = input
                        .archive_path
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| input.path.file_name().unwrap_or_default().into())
                        .join(path.strip_prefix(&input.path).unwrap_or(&path));
                    scanned.push(ScannedInput {
                        source_path: path,
                        archive_path: archive_path.to_string_lossy().replace('\\', "/"),
                        is_dir: entry.file_type().is_dir(),
                        size: entry.metadata().ok().map(|metadata| metadata.len()),
                    });
                }
            } else {
                scanned.push(ScannedInput {
                    source_path: input.path.clone(),
                    archive_path: input.archive_path.clone().unwrap_or_else(|| {
                        input
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    }),
                    is_dir: false,
                    size: fs::metadata(&input.path)
                        .ok()
                        .map(|metadata| metadata.len()),
                });
            }
        }
        Ok(scanned)
    }
}

#[derive(Debug, Clone)]
pub struct ScannedInput {
    pub source_path: PathBuf,
    pub archive_path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

fn detect_conflicts(listing: &ArchiveListing, destination: &Path) -> Vec<PathConflict> {
    listing
        .entries
        .iter()
        .filter_map(|entry| {
            let Ok(target_path) = safe_join(destination, &entry.raw_path) else {
                return None;
            };
            if !target_path.exists() {
                return None;
            }
            let target_size = fs::metadata(&target_path)
                .ok()
                .map(|metadata| metadata.len());
            Some(PathConflict {
                entry: entry.id,
                entry_path: entry.raw_path.clone(),
                target_path,
                source_size: entry.size,
                target_size,
            })
        })
        .collect()
}

fn next_available_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file");
    let extension = path.extension().and_then(|ext| ext.to_str());
    for index in 1..10_000 {
        let name = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

fn io_error(error: io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_technical_detail(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use shadow_zip_domain::{EntrySafety, SafetyBlockReason};

    use super::*;

    #[test]
    fn stream_pump_copies_with_bounded_buffer() {
        let mut source = Cursor::new(vec![1_u8; 1024]);
        let mut sink = Vec::new();
        let copied = StreamPump::new(StreamLimits {
            buffer_size: 128,
            max_entry_bytes: 2048,
            max_total_bytes: 2048,
        })
        .copy(&mut source, &mut sink, |_| Ok(()))
        .unwrap();

        assert_eq!(copied, 1024);
        assert_eq!(sink.len(), 1024);
    }

    #[test]
    fn stream_pump_enforces_entry_limit() {
        let mut source = Cursor::new(vec![1_u8; 1024]);
        let mut sink = Vec::new();
        let error = StreamPump::new(StreamLimits {
            buffer_size: 128,
            max_entry_bytes: 512,
            max_total_bytes: 512,
        })
        .copy(&mut source, &mut sink, |_| Ok(()))
        .unwrap_err();

        assert_eq!(error.kind, ArchiveErrorKind::Internal);
    }

    #[rstest]
    #[case("../x", SafetyBlockReason::ParentTraversal)]
    #[case("/x", SafetyBlockReason::AbsolutePath)]
    #[case("C:/x", SafetyBlockReason::WindowsDrivePath)]
    fn preflight_blocks_dangerous_paths(#[case] path: &str, #[case] reason: SafetyBlockReason) {
        let listing = ArchiveListing {
            entries: vec![ArchiveEntry {
                id: EntryId(0),
                raw_path: path.into(),
                normalized_path: path.into(),
                display_path: path.into(),
                kind: EntryKind::File,
                size: Some(1),
                compressed_size: Some(1),
                modified_at: None,
                method: None,
                encrypted: false,
                safety: EntrySafety::Safe,
            }],
            directories: Default::default(),
            is_complete: true,
        };

        let preflight = PreflightService::new(SecurityPolicy::default())
            .check_listing(&listing, tempfile::tempdir().unwrap().path().to_path_buf());

        assert_eq!(preflight.blocked_entries[0].reason, reason);
    }

    #[test]
    fn safe_writer_renames_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "old").unwrap();
        let writer = SafeWriter::new(dir.path().to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(OverwritePolicy::Rename);
        let mut source = Cursor::new("new");

        writer
            .write_stream("a.txt", &mut source, |_| Ok(()))
            .unwrap();

        assert!(dir.path().join("a (1).txt").exists());
    }
}
