#![allow(clippy::result_large_err)]

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use fs_err as fs;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shadow_zip_archive_7z::SevenZipBackend;
use shadow_zip_archive_core::{ArchiveBackend, ArchiveService, OpenArchive, PreflightService};
use shadow_zip_archive_libarchive::LibarchiveBackend;
use shadow_zip_archive_rar::RarBackend;
use shadow_zip_archive_tar::{TarBackend, create_tar_archive};
use shadow_zip_archive_zip::{ZipBackend, create_zip_archive};
use shadow_zip_cache::{CacheConfig, CacheService, CacheSummary};
use shadow_zip_domain::*;
use shadow_zip_platform::{
    HelperDiagnostic, HelperDiscovery, NoopPlatformIntegration, PlatformConfig,
};
use shadow_zip_preview::{
    PixelSize, PreviewLimits, PreviewMode, PreviewRequest, PreviewResult, PreviewService,
};
use shadow_zip_task_engine::{ProgressSink, TaskEngine};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct InspectRequest {
    pub archive: PathBuf,
    pub open_options: OpenOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    pub info: ArchiveInfo,
    pub capabilities: ArchiveCapabilities,
}

#[derive(Debug, Clone)]
pub struct ListRequest {
    pub archive: PathBuf,
    pub open_options: OpenOptions,
    pub filter: EntryFilter,
    pub sort: EntrySort,
    pub listing_mode: ListingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResult {
    pub info: ArchiveInfo,
    pub capabilities: ArchiveCapabilities,
    pub listing: ArchiveListing,
    pub visible_entries: Vec<ArchiveEntry>,
}

#[derive(Debug, Clone)]
pub struct TreeRequest {
    pub archive: PathBuf,
    pub open_options: OpenOptions,
    pub filter: EntryFilter,
    pub sort: EntrySort,
    pub listing_mode: ListingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeResult {
    pub info: ArchiveInfo,
    pub tree: DirectoryTree,
    pub listing: ArchiveListing,
}

#[derive(Debug, Clone, Default)]
pub enum EntrySelection {
    #[default]
    All,
    Ids(Vec<EntryId>),
    Paths {
        paths: Vec<String>,
        all_matches: bool,
    },
    Globs {
        include: Vec<String>,
        exclude: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ExtractRequest {
    pub archive: PathBuf,
    pub destination: PathBuf,
    pub selection: EntrySelection,
    pub open_options: OpenOptions,
    pub extract_options: ExtractOptions,
    pub require_preflight_clear: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractResult {
    pub task_id: Uuid,
    pub preflight: ExtractPreflight,
    pub summary: TaskSummary,
    pub warnings: Vec<TaskWarning>,
}

#[derive(Debug, Clone)]
pub struct CreateRequest {
    pub inputs: Vec<InputPath>,
    pub output: PathBuf,
    pub options: CreateOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResult {
    pub task_id: Uuid,
    pub summary: TaskSummary,
    pub warnings: Vec<TaskWarning>,
}

#[derive(Debug, Clone)]
pub struct TestArchiveRequest {
    pub archive: PathBuf,
    pub open_options: OpenOptions,
    pub options: TestOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestArchiveResult {
    pub task_id: Uuid,
    pub summary: TaskSummary,
    pub warnings: Vec<TaskWarning>,
}

#[derive(Debug, Clone)]
pub struct PreviewEntryRequest {
    pub archive: PathBuf,
    pub entry: EntrySelection,
    pub open_options: OpenOptions,
    pub mode: PreviewMode,
    pub target_size: PixelSize,
}

#[derive(Debug, Clone)]
pub struct PreviewEntryResult {
    pub result: PreviewResult,
    pub access_cost: AccessCost,
    pub warnings: Vec<TaskWarning>,
}

#[derive(Debug, Clone)]
pub struct CatEntryRequest {
    pub archive: PathBuf,
    pub entry: EntrySelection,
    pub open_options: OpenOptions,
    pub stream_options: StreamOptions,
}

#[derive(Debug, Clone)]
pub struct CatEntryResult {
    pub entry: ArchiveEntry,
    pub bytes: Vec<u8>,
    pub access_cost: AccessCost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPathResult {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValueResult {
    pub key: Option<String>,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheCleanupResult {
    pub task_id: Uuid,
    pub before: CacheSummary,
    pub after: CacheSummary,
}

#[derive(Debug, Clone)]
pub struct DiagnoseRequest {
    pub archive: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendDiagnostic {
    pub backend_name: String,
    pub probe: Option<ProbeResult>,
    pub error: Option<ArchiveError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendInfoResult {
    pub backend_name: String,
    pub formats: Vec<ArchiveFormat>,
    pub capabilities: ArchiveCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnoseResult {
    pub archive: PathBuf,
    pub backends: Vec<BackendDiagnostic>,
    pub helpers: Vec<HelperDiagnostic>,
}

pub trait ArchiveUseCases {
    fn inspect(&self, request: InspectRequest) -> Result<InspectResult, ArchiveError>;
    fn list(&self, request: ListRequest) -> Result<ListResult, ArchiveError>;
    fn tree(&self, request: TreeRequest) -> Result<TreeResult, ArchiveError>;
    fn preflight_extract(&self, request: &ExtractRequest)
    -> Result<ExtractPreflight, ArchiveError>;
    fn extract(
        &self,
        request: ExtractRequest,
        progress: Option<&dyn ProgressSink>,
    ) -> Result<ExtractResult, ArchiveError>;
    fn create(
        &self,
        request: CreateRequest,
        progress: Option<&dyn ProgressSink>,
    ) -> Result<CreateResult, ArchiveError>;
    fn test_archive(
        &self,
        request: TestArchiveRequest,
        progress: Option<&dyn ProgressSink>,
    ) -> Result<TestArchiveResult, ArchiveError>;
    fn cat(&self, request: CatEntryRequest) -> Result<CatEntryResult, ArchiveError>;
    fn preview(&self, request: PreviewEntryRequest) -> Result<PreviewEntryResult, ArchiveError>;
    fn diagnose(&self, request: DiagnoseRequest) -> DiagnoseResult;
    fn backends(&self) -> Vec<BackendInfoResult>;
    fn helpers(&self) -> Vec<HelperDiagnostic>;
    fn cache_summary(&self) -> CacheSummary;
    fn cleanup_cache(&self) -> Result<CacheCleanupResult, ArchiveError>;
    fn recent_files(&self) -> Vec<RecentFile>;
    fn clear_recent_files(&self);
}

pub struct AppCore {
    archive_service: ArchiveService,
    task_engine: TaskEngine,
    preview_service: PreviewService,
    preflight_service: PreflightService,
    cache_service: Mutex<CacheService>,
    _platform: NoopPlatformIntegration,
    sessions: Mutex<BTreeMap<SessionId, CoreArchiveSession>>,
    recent_files: Mutex<Vec<RecentFile>>,
    diagnostics: Mutex<Vec<DiagnosticEvent>>,
    config: AppConfig,
    platform_config: PlatformConfig,
}

impl AppCore {
    pub fn new(config: AppConfig, platform_config: PlatformConfig) -> Self {
        let backends = build_backends(&platform_config);
        Self {
            archive_service: ArchiveService::new(backends),
            task_engine: TaskEngine::default(),
            preview_service: PreviewService::new(PreviewLimits {
                max_input_bytes: config.preview.max_input_bytes,
                max_output_pixels: config.preview.max_output_pixels,
                ..PreviewLimits::default()
            }),
            preflight_service: PreflightService::new(config.security.clone()),
            cache_service: Mutex::new(CacheService::new(CacheConfig::with_root(
                default_cache_root(),
            ))),
            _platform: NoopPlatformIntegration,
            sessions: Mutex::new(BTreeMap::new()),
            recent_files: Mutex::new(Vec::new()),
            diagnostics: Mutex::new(Vec::new()),
            config,
            platform_config,
        }
    }

    pub fn default_shared() -> Arc<Self> {
        Arc::new(Self::new(AppConfig::default(), PlatformConfig::default()))
    }

    pub fn load_config(path: PathBuf) -> AppConfig {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_config(&self, path: PathBuf) -> Result<(), ArchiveError> {
        let text = serde_json::to_string_pretty(&self.config).map_err(|error| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Could not serialize configuration",
            )
            .with_technical_detail(error.to_string())
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        fs::write(path, text).map_err(io_error)
    }

    pub fn open_archive(&self, path: PathBuf) -> Result<ArchiveSessionSnapshot, ArchiveError> {
        let source = ArchiveSource::LocalPath(path);
        let mut open = self.archive_service.open_best(
            source.clone(),
            OpenOptions {
                prefer_cached_index: true,
                ..OpenOptions::default()
            },
        )?;
        let info = open.info();
        let capabilities = open.capabilities();
        let listing_mode = listing_mode_for(info.format);
        let listing = open.listing(listing_mode)?;
        let id = SessionId::new();

        self.sessions.lock().insert(
            id,
            CoreArchiveSession {
                id,
                source: source.clone(),
                info: info.clone(),
                capabilities: capabilities.clone(),
                listing: listing.clone(),
                open,
                password_memory: None,
            },
        );
        self.record_recent_file(&source, &info);

        Ok(ArchiveSessionSnapshot {
            id,
            source,
            info,
            capabilities,
            listing,
            selected_entries: Default::default(),
            current_directory: "/".into(),
            filter: EntryFilter::default(),
            sort: EntrySort::default(),
        })
    }

    pub fn extract_session(
        &self,
        session_id: SessionId,
        entries: Option<&[EntryId]>,
        destination: PathBuf,
        options: ExtractOptions,
    ) -> Result<Uuid, ArchiveError> {
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(missing_session)?;
        let plan = match entries {
            Some(entries) => session
                .open
                .extract_selected(entries, &destination, options)?,
            None => session.open.extract_all(&destination, options)?,
        };
        let id = self.task_engine.enqueue(plan, TaskPriority::UserBlocking);
        Ok(id)
    }

    pub fn test_session(&self, session_id: SessionId) -> Result<Uuid, ArchiveError> {
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(missing_session)?;
        let plan = session.open.test(TestOptions {
            password: session.password_memory.clone(),
        })?;
        Ok(self.task_engine.enqueue(plan, TaskPriority::Normal))
    }

    pub fn request_preview_session(
        &self,
        session_id: SessionId,
        entry_id: EntryId,
    ) -> Result<Uuid, ArchiveError> {
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(missing_session)?;
        let stream = session
            .open
            .open_entry_stream(entry_id, StreamOptions::default())?;
        let entry = session
            .listing
            .entries
            .iter()
            .find(|entry| entry.id == entry_id);
        let mut request = PreviewRequest::metadata(session_id, entry_id);
        if let Some(entry) = entry {
            request.entry_name.clone_from(&entry.display_path);
            request.entry_size = entry.size;
        }
        let plan = self.preview_service.plan(&request, stream.access_cost);
        Ok(self.task_engine.enqueue(plan, TaskPriority::UserBlocking))
    }

    pub fn diagnostics(&self) -> Vec<DiagnosticEvent> {
        self.diagnostics.lock().clone()
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn open_for_path(
        &self,
        archive: PathBuf,
        options: OpenOptions,
    ) -> Result<(ArchiveSource, Box<dyn OpenArchive>), ArchiveError> {
        let source = ArchiveSource::LocalPath(archive);
        let open = self.archive_service.open_best(source.clone(), options)?;
        Ok((source, open))
    }

    fn list_internal(&self, request: ListRequest) -> Result<ListResult, ArchiveError> {
        let (source, mut open) = self.open_for_path(request.archive, request.open_options)?;
        let info = open.info();
        let capabilities = open.capabilities();
        let listing = open.listing(request.listing_mode)?;
        let mut visible_entries = listing
            .entries
            .iter()
            .filter(|entry| request.filter.matches(entry))
            .cloned()
            .collect::<Vec<_>>();
        sort_entries(&mut visible_entries, request.sort);
        self.record_recent_file(&source, &info);
        Ok(ListResult {
            info,
            capabilities,
            listing,
            visible_entries,
        })
    }

    fn record_recent_file(&self, source: &ArchiveSource, info: &ArchiveInfo) {
        if !self.config.recent_files.enabled {
            return;
        }
        let mut recent = self.recent_files.lock();
        recent.retain(|item| item.source != *source);
        recent.insert(
            0,
            RecentFile {
                source: source.clone(),
                display_name: info.display_name.clone(),
                last_opened_unix_ms: now_ms(),
                format: info.format,
            },
        );
        recent.truncate(self.config.recent_files.max_items);
    }
}

impl ArchiveUseCases for AppCore {
    fn inspect(&self, request: InspectRequest) -> Result<InspectResult, ArchiveError> {
        let (source, mut open) = self.open_for_path(request.archive, request.open_options)?;
        let mut info = open.info();
        if info.entry_count.is_none() {
            let listing = open.listing(listing_mode_for(info.format))?;
            info.entry_count = Some(listing.entries.len() as u64);
        }
        let capabilities = open.capabilities();
        self.record_recent_file(&source, &info);
        Ok(InspectResult { info, capabilities })
    }

    fn list(&self, request: ListRequest) -> Result<ListResult, ArchiveError> {
        self.list_internal(request)
    }

    fn tree(&self, request: TreeRequest) -> Result<TreeResult, ArchiveError> {
        let result = self.list_internal(ListRequest {
            archive: request.archive,
            open_options: request.open_options,
            filter: request.filter,
            sort: request.sort,
            listing_mode: request.listing_mode,
        })?;
        Ok(TreeResult {
            info: result.info,
            tree: DirectoryTree::from_listing(&result.listing),
            listing: result.listing,
        })
    }

    fn preflight_extract(
        &self,
        request: &ExtractRequest,
    ) -> Result<ExtractPreflight, ArchiveError> {
        let list = self.list_internal(ListRequest {
            archive: request.archive.clone(),
            open_options: request.open_options.clone(),
            filter: EntryFilter::default(),
            sort: EntrySort::default(),
            listing_mode: ListingMode::Full,
        })?;
        let entries = resolve_selection(&list.listing, &request.selection)?;
        let listing = selected_listing(&list.listing, &entries);
        let preflight = self
            .preflight_service
            .check_listing(&listing, request.destination.clone());
        if let Some(warning) = preflight
            .warnings
            .iter()
            .find(|warning| warning.code == "destination-invalid")
        {
            return Err(ArchiveError::new(
                ArchiveErrorKind::PermissionDenied,
                warning.message.clone(),
            ));
        }
        Ok(preflight)
    }

    fn extract(
        &self,
        request: ExtractRequest,
        _progress: Option<&dyn ProgressSink>,
    ) -> Result<ExtractResult, ArchiveError> {
        let preflight = self.preflight_extract(&request)?;
        if request.require_preflight_clear {
            if !preflight.blocked_entries.is_empty() {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::PathTraversalBlocked,
                    "Extraction was blocked by safety preflight",
                ));
            }
            if !preflight.conflicts.is_empty()
                && matches!(
                    request.extract_options.overwrite_policy,
                    OverwritePolicy::AskBatch
                )
            {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Internal,
                    "Extraction requires an explicit conflict policy",
                ));
            }
        }

        let (source, mut open) = self.open_for_path(request.archive, request.open_options)?;
        let info = open.info();
        let listing = open.listing(ListingMode::Full)?;
        let entries = resolve_selection(&listing, &request.selection)?;
        let plan = if matches!(request.selection, EntrySelection::All) {
            open.extract_all(&request.destination, request.extract_options)?
        } else {
            open.extract_selected(&entries, &request.destination, request.extract_options)?
        };
        let warnings = plan.warnings.clone();
        let task_id = self.task_engine.enqueue(plan, TaskPriority::UserBlocking);
        self.record_recent_file(&source, &info);
        Ok(ExtractResult {
            task_id,
            preflight,
            summary: TaskSummary {
                processed_entries: entries.len() as u64,
                processed_bytes: 0,
                ..TaskSummary::default()
            },
            warnings,
        })
    }

    fn create(
        &self,
        request: CreateRequest,
        _progress: Option<&dyn ProgressSink>,
    ) -> Result<CreateResult, ArchiveError> {
        if request.inputs.is_empty() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Create archive requires at least one input",
            ));
        }
        if request
            .options
            .password
            .as_deref()
            .is_some_and(str::is_empty)
        {
            return Err(ArchiveError::new(
                ArchiveErrorKind::PasswordRequired,
                "Encryption is enabled but password is empty",
            ));
        }
        if request.volume_size_too_small() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Volume size must be at least 1 MiB",
            ));
        }

        match request.options.format {
            ArchiveFormat::Zip => {
                create_zip_archive(
                    &request.inputs,
                    request.output.clone(),
                    request.options.clone(),
                )?;
            }
            ArchiveFormat::Tar
            | ArchiveFormat::TarGz
            | ArchiveFormat::TarXz
            | ArchiveFormat::TarZst => {
                create_tar_archive(&request.inputs, &request.output, request.options.clone())?;
            }
            ArchiveFormat::Rar => {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::UnsupportedFormat,
                    "RAR creation is intentionally not built in because it requires RARLAB licensing",
                ));
            }
            ArchiveFormat::SevenZip | ArchiveFormat::Unknown => {
                let backend = self
                    .archive_service
                    .backends()
                    .iter()
                    .find(|backend| {
                        backend
                            .backend_capabilities()
                            .formats
                            .contains(&request.options.format)
                    })
                    .ok_or_else(|| {
                        ArchiveError::new(
                            ArchiveErrorKind::UnsupportedFormat,
                            "No backend can create the requested archive format",
                        )
                    })?;
                let plan = backend.create_plan(
                    &request.inputs,
                    &request.output,
                    request.options.clone(),
                )?;
                let warnings = plan.warnings.clone();
                let task_id = self.task_engine.enqueue(plan, TaskPriority::UserBlocking);
                return Ok(CreateResult {
                    task_id,
                    summary: TaskSummary {
                        processed_entries: request.inputs.len() as u64,
                        ..TaskSummary::default()
                    },
                    warnings,
                });
            }
        }

        Ok(CreateResult {
            task_id: Uuid::new_v4(),
            summary: TaskSummary {
                processed_entries: request.inputs.len() as u64,
                ..TaskSummary::default()
            },
            warnings: Vec::new(),
        })
    }

    fn test_archive(
        &self,
        request: TestArchiveRequest,
        _progress: Option<&dyn ProgressSink>,
    ) -> Result<TestArchiveResult, ArchiveError> {
        let (source, mut open) = self.open_for_path(request.archive, request.open_options)?;
        let info = open.info();
        let listing = open.listing(listing_mode_for(info.format))?;
        let plan = open.test(request.options)?;
        let warnings = plan.warnings.clone();
        let task_id = self.task_engine.enqueue(plan, TaskPriority::Normal);
        self.record_recent_file(&source, &info);
        Ok(TestArchiveResult {
            task_id,
            summary: TaskSummary {
                processed_entries: listing.entries.len() as u64,
                processed_bytes: listing.entries.iter().filter_map(|entry| entry.size).sum(),
                ..TaskSummary::default()
            },
            warnings,
        })
    }

    fn cat(&self, request: CatEntryRequest) -> Result<CatEntryResult, ArchiveError> {
        let (source, mut open) = self.open_for_path(request.archive, request.open_options)?;
        let info = open.info();
        let listing = open.listing(ListingMode::Full)?;
        let ids = resolve_selection(&listing, &request.entry)?;
        if ids.len() != 1 {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Exactly one archive entry must be selected",
            ));
        }
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.id == ids[0])
            .cloned()
            .ok_or_else(|| ArchiveError::new(ArchiveErrorKind::Internal, "Entry id not found"))?;
        if entry.kind != EntryKind::File {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Only file entries can be written to stdout",
            )
            .with_entry_path(entry.display_path));
        }
        if !matches!(entry.safety, EntrySafety::Safe) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::PathTraversalBlocked,
                "Archive entry path was blocked by the safety policy",
            )
            .with_entry_path(entry.display_path));
        }
        let mut reader = open.open_entry_reader(ids[0], request.stream_options)?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let read = reader.source.read_chunk(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        self.record_recent_file(&source, &info);
        Ok(CatEntryResult {
            entry,
            bytes,
            access_cost: reader.access_cost,
        })
    }

    fn preview(&self, request: PreviewEntryRequest) -> Result<PreviewEntryResult, ArchiveError> {
        let (source, mut open) = self.open_for_path(request.archive, request.open_options)?;
        let info = open.info();
        let listing = open.listing(ListingMode::Full)?;
        let ids = resolve_selection(&listing, &request.entry)?;
        if ids.len() != 1 {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Exactly one archive entry must be selected for preview",
            ));
        }
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.id == ids[0])
            .cloned()
            .ok_or_else(|| ArchiveError::new(ArchiveErrorKind::Internal, "Entry id not found"))?;
        let mut reader = open.open_entry_reader(ids[0], StreamOptions::default())?;
        let mut preview_request = PreviewRequest::metadata(SessionId::new(), ids[0]);
        preview_request.entry_name = entry.display_path.clone();
        preview_request.entry_size = entry.size;
        preview_request.mode = request.mode;
        preview_request.target_size = request.target_size;
        let mut warnings = Vec::new();
        if matches!(
            reader.access_cost,
            AccessCost::SequentialFromStart
                | AccessCost::SolidBlockScan
                | AccessCost::ExternalHelper
        ) {
            warnings.push(TaskWarning {
                code: "preview-access-cost".into(),
                message: "This archive may need non-random reads before preview data is available"
                    .into(),
            });
        }
        let result = self.preview_service.process(
            &preview_request,
            reader.access_cost,
            reader.source.as_mut(),
        )?;
        self.record_recent_file(&source, &info);
        Ok(PreviewEntryResult {
            result,
            access_cost: reader.access_cost,
            warnings,
        })
    }

    fn diagnose(&self, request: DiagnoseRequest) -> DiagnoseResult {
        let source = ArchiveSource::LocalPath(request.archive.clone());
        let backends = self
            .archive_service
            .backends()
            .iter()
            .map(|backend| {
                let probe = backend.probe(&source);
                match probe {
                    Ok(probe) => BackendDiagnostic {
                        backend_name: backend.name().to_string(),
                        probe: Some(probe),
                        error: None,
                    },
                    Err(error) => BackendDiagnostic {
                        backend_name: backend.name().to_string(),
                        probe: None,
                        error: Some(error),
                    },
                }
            })
            .collect();
        DiagnoseResult {
            archive: request.archive,
            backends,
            helpers: self.helpers(),
        }
    }

    fn backends(&self) -> Vec<BackendInfoResult> {
        self.archive_service
            .backends()
            .iter()
            .map(|backend| {
                let capabilities = backend.backend_capabilities();
                BackendInfoResult {
                    backend_name: backend.name().to_string(),
                    formats: capabilities.formats,
                    capabilities: capabilities.capabilities,
                }
            })
            .collect()
    }

    fn helpers(&self) -> Vec<HelperDiagnostic> {
        let discovery = HelperDiscovery::new(self.platform_config.external_helpers.clone());
        vec![discovery.unrar(), discovery.libarchive()]
    }

    fn cache_summary(&self) -> CacheSummary {
        self.cache_service.lock().cache_summary()
    }

    fn cleanup_cache(&self) -> Result<CacheCleanupResult, ArchiveError> {
        let mut cache = self.cache_service.lock();
        let before = cache.cache_summary();
        let plan = cache.cleanup_plan();
        let task_id = self.task_engine.enqueue(plan, TaskPriority::Maintenance);
        cache.clear_all()?;
        let after = cache.cache_summary();
        Ok(CacheCleanupResult {
            task_id,
            before,
            after,
        })
    }

    fn recent_files(&self) -> Vec<RecentFile> {
        self.recent_files.lock().clone()
    }

    fn clear_recent_files(&self) {
        self.recent_files.lock().clear();
    }
}

impl CreateRequest {
    fn volume_size_too_small(&self) -> bool {
        self.options
            .volume_size
            .is_some_and(|size| size < 1024 * 1024)
    }
}

pub struct CoreArchiveSession {
    pub id: SessionId,
    pub source: ArchiveSource,
    pub info: ArchiveInfo,
    pub capabilities: ArchiveCapabilities,
    pub listing: ArchiveListing,
    pub open: Box<dyn OpenArchive>,
    pub password_memory: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiagnosticEvent {
    pub timestamp_unix_ms: i64,
    pub area: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub technical_detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warn,
    Error,
}

pub fn build_backends(platform_config: &PlatformConfig) -> Vec<Box<dyn ArchiveBackend>> {
    vec![
        Box::new(ZipBackend),
        Box::new(SevenZipBackend),
        Box::new(TarBackend),
        Box::new(RarBackend::new(true)),
        Box::new(LibarchiveBackend::new(
            platform_config.external_helpers.libarchive_path.is_some(),
        )),
    ]
}

pub fn listing_mode_for(format: ArchiveFormat) -> ListingMode {
    if matches!(
        format,
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarXz | ArchiveFormat::TarZst
    ) {
        ListingMode::Incremental
    } else {
        ListingMode::Fast
    }
}

pub fn resolve_selection(
    listing: &ArchiveListing,
    selection: &EntrySelection,
) -> Result<Vec<EntryId>, ArchiveError> {
    match selection {
        EntrySelection::All => Ok(listing.entries.iter().map(|entry| entry.id).collect()),
        EntrySelection::Ids(ids) => Ok(ids.clone()),
        EntrySelection::Paths { paths, all_matches } => {
            let mut ids = Vec::new();
            for path in paths {
                let matches = listing
                    .entries
                    .iter()
                    .filter(|entry| entry.display_path == *path || entry.normalized_path == *path)
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>();
                if matches.is_empty() {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::Internal,
                        format!("No archive entry matches '{path}'"),
                    ));
                }
                if matches.len() > 1 && !all_matches {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::Internal,
                        format!("Archive entry path '{path}' is ambiguous; use entry id"),
                    ));
                }
                ids.extend(matches);
            }
            ids.sort();
            ids.dedup();
            Ok(ids)
        }
        EntrySelection::Globs { include, exclude } => {
            let ids = listing
                .entries
                .iter()
                .filter(|entry| {
                    let included = include.is_empty()
                        || include
                            .iter()
                            .any(|pattern| wildcard_match(pattern, &entry.normalized_path));
                    let excluded = exclude
                        .iter()
                        .any(|pattern| wildcard_match(pattern, &entry.normalized_path));
                    included && !excluded
                })
                .map(|entry| entry.id)
                .collect::<Vec<_>>();
            Ok(ids)
        }
    }
}

fn selected_listing(listing: &ArchiveListing, entries: &[EntryId]) -> ArchiveListing {
    ArchiveListing {
        entries: listing
            .entries
            .iter()
            .filter(|entry| entries.contains(&entry.id))
            .cloned()
            .collect(),
        directories: Default::default(),
        is_complete: listing.is_complete,
    }
}

fn sort_entries(entries: &mut [ArchiveEntry], sort: EntrySort) {
    entries.sort_by(|left, right| {
        let ordering = match sort.column {
            EntrySortColumn::Name => left.display_path.cmp(&right.display_path),
            EntrySortColumn::Size => left.size.cmp(&right.size),
            EntrySortColumn::PackedSize => left.compressed_size.cmp(&right.compressed_size),
            EntrySortColumn::Type => format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)),
            EntrySortColumn::Modified => left.modified_at.cmp(&right.modified_at),
            EntrySortColumn::Method => left.method.cmp(&right.method),
            EntrySortColumn::Encrypted => left.encrypted.cmp(&right.encrypted),
            EntrySortColumn::Path => left.normalized_path.cmp(&right.normalized_path),
        };
        if matches!(sort.direction, SortDirection::Descending) {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');
    for (index, part) in parts.iter().filter(|part| !part.is_empty()).enumerate() {
        if index == 0 && !starts_with_wildcard {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
            continue;
        }
        let Some(found) = rest.find(part) else {
            return false;
        };
        rest = &rest[found + part.len()..];
    }
    ends_with_wildcard || rest.is_empty()
}

fn build_archive_error(kind: ArchiveErrorKind, message: &str) -> ArchiveError {
    ArchiveError::new(kind, message)
}

fn missing_session() -> ArchiveError {
    build_archive_error(
        ArchiveErrorKind::Internal,
        "Archive session no longer exists",
    )
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_technical_detail(error.to_string())
}

fn default_cache_root() -> PathBuf {
    std::env::temp_dir().join("shadow-zip")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_core_constructs_with_default_config() {
        let _core = AppCore::new(AppConfig::default(), PlatformConfig::default());
    }

    #[test]
    fn wildcard_matching_supports_simple_patterns() {
        assert!(wildcard_match("docs/*.txt", "docs/readme.txt"));
        assert!(wildcard_match("*.txt", "docs/readme.txt"));
        assert!(!wildcard_match("images/*.png", "docs/readme.txt"));
    }
}
