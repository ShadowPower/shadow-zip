use std::{path::PathBuf, sync::Arc};

use gpui::*;
use shadow_zip_app_core::{AppCore, ArchiveUseCases};
use shadow_zip_domain::*;
use shadow_zip_i18n::Locale;
use shadow_zip_platform::PlatformConfig;
use shadow_zip_ui::{Workbench, WorkbenchActions};

fn main() {
    let bootstrap = Bootstrap::load();
    Application::new().run(move |cx| {
        let locale = bootstrap.locale;
        let actions = bootstrap.actions.clone();
        let bounds = Bounds::centered(None, size(px(1180.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(920.0), px(620.0))),
                ..Default::default()
            },
            |_window, cx| cx.new(|_cx| Workbench::with_actions(locale, actions)),
        )
        .expect("open main window");
        cx.activate(true);
    });
}

struct Bootstrap {
    locale: Locale,
    actions: Arc<GuiActions>,
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
        let core = Arc::new(AppCore::new(app_config, platform_config));
        Self {
            locale,
            actions: Arc::new(GuiActions { core }),
        }
    }
}

struct GuiActions {
    core: Arc<AppCore>,
}

impl WorkbenchActions for GuiActions {
    fn open_archive(&self, path: PathBuf) -> Result<ArchiveSessionSnapshot, ArchiveError> {
        self.core.open_archive(path)
    }

    fn extract_all(
        &self,
        session_id: SessionId,
        destination: PathBuf,
    ) -> Result<uuid::Uuid, ArchiveError> {
        self.core
            .extract_session(session_id, None, destination, ExtractOptions::default())
    }

    fn test_archive(&self, session_id: SessionId) -> Result<uuid::Uuid, ArchiveError> {
        self.core.test_session(session_id)
    }

    fn request_preview(
        &self,
        session_id: SessionId,
        entry_id: EntryId,
    ) -> Result<uuid::Uuid, ArchiveError> {
        self.core.request_preview_session(session_id, entry_id)
    }

    fn recent_files(&self) -> Vec<RecentFile> {
        self.core.recent_files()
    }
}
