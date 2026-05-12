use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::*;
use shadow_zip_archive_7z::SevenZipBackend;
use shadow_zip_archive_core::{ArchiveBackend, ArchiveService, OpenArchive, PreflightService};
use shadow_zip_archive_libarchive::LibarchiveBackend;
use shadow_zip_archive_rar::RarBackend;
use shadow_zip_archive_tar::TarBackend;
use shadow_zip_archive_zip::ZipBackend;
use shadow_zip_cache::{CacheConfig, CacheService};
use shadow_zip_domain::*;
use shadow_zip_i18n::Locale;
use shadow_zip_platform::{NoopPlatformIntegration, PlatformConfig};
use shadow_zip_preview::{PreviewLimits, PreviewRequest, PreviewService};
use shadow_zip_task_engine::TaskEngine;
use shadow_zip_ui::{Workbench, WorkbenchActions};

fn main() {
    let bootstrap = Bootstrap::load();
    Application::new().run(move |cx| {
        let locale = bootstrap.locale;
        let controller = bootstrap.controller.clone();
        let bounds = Bounds::centered(None, size(px(1180.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(920.0), px(620.0))),
                ..Default::default()
            },
            |_window, cx| cx.new(|_cx| Workbench::with_actions(locale, controller)),
        )
        .expect("open main window");
        cx.activate(true);
    });
}

struct Bootstrap {
    locale: Locale,
    controller: Arc<AppController>,
}

impl Bootstrap {
    fn load() -> Self {
        let app_config = AppConfig::default();
        let platform_config = PlatformConfig::default();
        let locale = app_config
            .locale
            .as_deref()
            .or(platform_config.locale_override.as_deref())
            .map(Locale::from_system_tag)
            .unwrap_or(Locale::ZhCn);

        let controller = Arc::new(AppController::new(app_config, platform_config));
        Self { locale, controller }
    }
}

pub struct AppController {
    archive_service: ArchiveService,
    task_engine: TaskEngine,
    preview_service: PreviewService,
    preflight_service: PreflightService,
    cache_service: CacheService,
    _platform: NoopPlatformIntegration,
    sessions: parking_lot::Mutex<BTreeMap<SessionId, ArchiveSession>>,
    recent_files: parking_lot::Mutex<Vec<RecentFile>>,
    diagnostics: parking_lot::Mutex<Vec<DiagnosticEvent>>,
    config: AppConfig,
    _platform_config: PlatformConfig,
}

impl AppController {
    pub fn new(config: AppConfig, platform_config: PlatformConfig) -> Self {
        let backends: Vec<Box<dyn ArchiveBackend>> = vec![
            Box::new(ZipBackend),
            Box::new(SevenZipBackend),
            Box::new(TarBackend),
            Box::new(RarBackend::new(
                platform_config.external_helpers.unrar_path.is_some(),
            )),
            Box::new(LibarchiveBackend::new(
                platform_config.external_helpers.libarchive_path.is_some(),
            )),
        ];

        Self {
            archive_service: ArchiveService::new(backends),
            task_engine: TaskEngine::default(),
            preview_service: PreviewService::new(PreviewLimits {
                max_input_bytes: config.preview.max_input_bytes,
                max_output_pixels: config.preview.max_output_pixels,
                ..PreviewLimits::default()
            }),
            preflight_service: PreflightService::new(config.security.clone()),
            cache_service: CacheService::new(CacheConfig::with_root(default_cache_root())),
            _platform: NoopPlatformIntegration,
            sessions: parking_lot::Mutex::new(BTreeMap::new()),
            recent_files: parking_lot::Mutex::new(Vec::new()),
            diagnostics: parking_lot::Mutex::new(Vec::new()),
            config,
            _platform_config: platform_config,
        }
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
        let listing_mode = if matches!(
            info.format,
            ArchiveFormat::Tar
                | ArchiveFormat::TarGz
                | ArchiveFormat::TarXz
                | ArchiveFormat::TarZst
        ) {
            ListingMode::Incremental
        } else {
            ListingMode::Fast
        };
        let listing = open.listing(listing_mode)?;
        let id = SessionId::new();

        self.sessions.lock().insert(
            id,
            ArchiveSession {
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
        fs::write(path, text).map_err(|error| {
            ArchiveError::new(ArchiveErrorKind::Io, "Could not write configuration")
                .with_technical_detail(error.to_string())
        })
    }

    pub fn diagnostics(&self) -> Vec<DiagnosticEvent> {
        self.diagnostics.lock().clone()
    }

    pub fn recent_files(&self) -> Vec<RecentFile> {
        self.recent_files.lock().clone()
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

    fn _record_error(&self, area: impl Into<String>, error: &ArchiveError) {
        self.diagnostics.lock().push(DiagnosticEvent {
            timestamp_unix_ms: now_ms(),
            area: area.into(),
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            technical_detail: error.technical_detail.clone(),
        });
    }

    pub fn preflight_extract(
        &self,
        session_id: SessionId,
        entries: Option<&[EntryId]>,
        destination: PathBuf,
    ) -> Result<ExtractPreflight, ArchiveError> {
        let sessions = self.sessions.lock();
        let session = sessions.get(&session_id).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Archive session no longer exists",
            )
        })?;

        let listing = match entries {
            Some(ids) => ArchiveListing {
                entries: session
                    .listing
                    .entries
                    .iter()
                    .filter(|entry| ids.contains(&entry.id))
                    .cloned()
                    .collect(),
                directories: Default::default(),
                is_complete: session.listing.is_complete,
            },
            None => session.listing.clone(),
        };

        Ok(self.preflight_service.check_listing(&listing, destination))
    }

    pub fn extract(
        &self,
        session_id: SessionId,
        entries: Option<&[EntryId]>,
        destination: PathBuf,
        options: ExtractOptions,
    ) -> Result<uuid::Uuid, ArchiveError> {
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Archive session no longer exists",
            )
        })?;

        let plan = match entries {
            Some(entries) => session
                .open
                .extract_selected(entries, &destination, options)?,
            None => session.open.extract_all(&destination, options)?,
        };

        Ok(self.task_engine.enqueue(plan, TaskPriority::UserBlocking))
    }

    pub fn create_archive(
        &self,
        inputs: Vec<InputPath>,
        output: PathBuf,
        options: CreateOptions,
    ) -> Result<uuid::Uuid, ArchiveError> {
        let backend = self
            .archive_service
            .backends()
            .iter()
            .find(|backend| {
                backend
                    .backend_capabilities()
                    .formats
                    .iter()
                    .any(|format| *format == options.format)
            })
            .ok_or_else(|| {
                ArchiveError::new(
                    ArchiveErrorKind::UnsupportedFormat,
                    "No backend can create the requested archive format",
                )
            })?;
        let plan = backend.create_plan(&inputs, &output, options)?;
        Ok(self.task_engine.enqueue(plan, TaskPriority::UserBlocking))
    }

    pub fn test_archive(&self, session_id: SessionId) -> Result<uuid::Uuid, ArchiveError> {
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Archive session no longer exists",
            )
        })?;
        let plan = session.open.test(TestOptions {
            password: session.password_memory.clone(),
        })?;
        Ok(self.task_engine.enqueue(plan, TaskPriority::Normal))
    }

    pub fn cleanup_cache(&self) -> uuid::Uuid {
        self.task_engine
            .enqueue(self.cache_service.cleanup_plan(), TaskPriority::Maintenance)
    }

    pub fn remember_password_for_session(
        &self,
        session_id: SessionId,
        password: Option<String>,
    ) -> Result<(), ArchiveError> {
        if !self.config.remember_passwords_for_session {
            return Ok(());
        }
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Archive session no longer exists",
            )
        })?;
        session.password_memory = password;
        Ok(())
    }

    pub fn request_preview(
        &self,
        session_id: SessionId,
        entry_id: EntryId,
    ) -> Result<uuid::Uuid, ArchiveError> {
        let mut sessions = self.sessions.lock();
        let session = sessions.get_mut(&session_id).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Archive session no longer exists",
            )
        })?;
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
            request.entry_name = entry.display_path.clone();
            request.entry_size = entry.size;
        }
        let plan = self.preview_service.plan(&request, stream.access_cost);
        Ok(self.task_engine.enqueue(plan, TaskPriority::UserBlocking))
    }
}

impl WorkbenchActions for AppController {
    fn open_archive(&self, path: PathBuf) -> Result<ArchiveSessionSnapshot, ArchiveError> {
        AppController::open_archive(self, path)
    }

    fn extract_all(
        &self,
        session_id: SessionId,
        destination: PathBuf,
    ) -> Result<uuid::Uuid, ArchiveError> {
        self.extract(session_id, None, destination, ExtractOptions::default())
    }

    fn test_archive(&self, session_id: SessionId) -> Result<uuid::Uuid, ArchiveError> {
        AppController::test_archive(self, session_id)
    }

    fn request_preview(
        &self,
        session_id: SessionId,
        entry_id: EntryId,
    ) -> Result<uuid::Uuid, ArchiveError> {
        AppController::request_preview(self, session_id, entry_id)
    }

    fn recent_files(&self) -> Vec<RecentFile> {
        AppController::recent_files(self)
    }
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

pub struct ArchiveSession {
    pub id: SessionId,
    pub source: ArchiveSource,
    pub info: ArchiveInfo,
    pub capabilities: ArchiveCapabilities,
    pub listing: ArchiveListing,
    pub open: Box<dyn OpenArchive>,
    pub password_memory: Option<String>,
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
