use gpui::*;
use shadow_zip_domain::*;
use shadow_zip_i18n::{Locale, MessageKey, Translator};

const BLUE: u32 = 0x087dd6;
const BLUE_DARK: u32 = 0x006fbd;
const BORDER: u32 = 0xd8dee8;
const GRID: u32 = 0xe8edf3;
const TEXT: u32 = 0x111827;
const MUTED: u32 = 0x6b7280;
const SURFACE: u32 = 0xffffff;
const TREE_BG: u32 = 0xf4f6f8;
const SELECTED: u32 = 0xd7d7d7;

const MAX_RENDERED_TREE_ROWS: usize = 500;

pub struct Workbench {
    translator: Translator,
    state: WorkbenchState,
    settings: AppConfig,
}

impl Workbench {
    pub fn new(locale: Locale) -> Self {
        Self {
            translator: Translator::new(locale),
            state: WorkbenchState::default(),
            settings: AppConfig::default(),
        }
    }

    pub fn set_state(&mut self, state: WorkbenchState) {
        if let Some(tag) = state.locale.as_deref() {
            self.translator.set_locale(Locale::from_system_tag(tag));
        }
        self.state = state;
    }

    pub fn set_session(&mut self, session: ArchiveSessionSnapshot) {
        self.state.tree = ArchiveTreeState {
            root_label: session.info.display_name.clone(),
            selected_path: session.current_directory.clone(),
            expanded_paths: ["/".to_string()].into_iter().collect(),
            nodes: DirectoryTree::from_listing(&session.listing),
        };
        self.state.status = status_from_listing(&session.listing);
        self.state.session = Some(session);
    }

    pub fn show_create_archive(&mut self, draft: Option<CreateArchiveDraft>) {
        let draft = draft.unwrap_or_else(|| {
            CreateArchiveDraft::default_for(ArchiveFormat::Zip, Vec::new(), "archive.zip".into())
        });
        self.state.overlays.push(OverlayState::CreateArchive(draft));
    }

    pub fn show_settings(&mut self) {
        self.state
            .overlays
            .push(OverlayState::Settings(self.settings.clone()));
    }

    pub fn show_conflicts(&mut self, batch: ConflictResolutionBatch) {
        self.state.overlays.push(OverlayState::Conflict(batch));
    }

    pub fn show_password_prompt(&mut self, request: PasswordRequest) {
        self.state.overlays.push(OverlayState::Password(request));
    }

    pub fn show_error(&mut self, error: ArchiveError) {
        self.state.overlays.push(OverlayState::Error(error.into()));
    }

    pub fn close_overlay(&mut self) {
        self.state.overlays.pop();
    }

    pub fn select_entry(&mut self, entry: EntryId) {
        self.state.list.focused_entry = Some(entry);
        self.state.list.selected_entries.clear();
        self.state.list.selected_entries.insert(entry);
        self.state.preview.selected_entry = Some(entry);
        self.state.preview.mode = SidebarPreviewMode::Loading;
    }

    fn t(&self, key: MessageKey) -> String {
        self.translator.text(key).into_owned()
    }
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(SURFACE))
            .text_color(rgb(TEXT))
            .child(self.menu_bar())
            .child(self.ribbon())
            .child(if self.state.session.is_some() {
                self.archive_workspace().into_any_element()
            } else {
                self.start_screen().into_any_element()
            })
            .child(self.status_bar())
            .child(self.overlay_layer())
    }
}

impl Workbench {
    fn menu_bar(&self) -> impl IntoElement {
        let labels = [
            MessageKey::MenuFile,
            MessageKey::MenuEdit,
            MessageKey::MenuFind,
            MessageKey::MenuOptions,
            MessageKey::MenuView,
            MessageKey::MenuHelp,
        ];
        row()
            .h_6()
            .px_2()
            .gap_4()
            .border_b_1()
            .border_color(rgb(0xcfd8e3))
            .bg(rgb(0xf7f9fc))
            .text_size(px(13.0))
            .children(labels.into_iter().map(|key| div().child(self.t(key))))
    }

    fn ribbon(&self) -> impl IntoElement {
        div()
            .h(px(100.0))
            .px_5()
            .flex()
            .items_center()
            .gap_5()
            .bg(rgb(BLUE))
            .children(
                self.state
                    .ribbon
                    .commands
                    .iter()
                    .map(|command| self.ribbon_button(command)),
            )
            .child(div().flex_1())
            .child(search_box(self.t(MessageKey::ToolbarSearch)))
            .child(ribbon_button_visual(
                "SET",
                self.t(MessageKey::ToolbarSettings),
                true,
            ))
    }

    fn ribbon_button(&self, command: &RibbonCommandState) -> impl IntoElement {
        let (icon, label) = match command.command {
            ToolbarCommand::Open => ("OPEN", self.t(MessageKey::ToolbarOpen)),
            ToolbarCommand::Extract => ("EXT", self.t(MessageKey::ToolbarExtract)),
            ToolbarCommand::NewArchive => ("NEW", self.t(MessageKey::ToolbarNew)),
            ToolbarCommand::Add => ("ADD", self.t(MessageKey::ToolbarAdd)),
            ToolbarCommand::Delete => ("DEL", self.t(MessageKey::ToolbarDelete)),
            ToolbarCommand::Test => ("TEST", self.t(MessageKey::ToolbarTest)),
            ToolbarCommand::View => ("VIEW", self.t(MessageKey::ToolbarView)),
            ToolbarCommand::CodePage => ("CP", self.t(MessageKey::ToolbarCodePage)),
            ToolbarCommand::Settings => ("SET", self.t(MessageKey::ToolbarSettings)),
            ToolbarCommand::HelperDiagnostics => ("?", self.t(MessageKey::ToolbarDiagnostics)),
        };
        ribbon_button_visual(icon, label, command.enabled)
    }

    fn start_screen(&self) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0xf7fbff))
            .child(
                div()
                    .w(px(680.0))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_size(px(26.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t(MessageKey::AppName)),
                    )
                    .child(
                        row()
                            .gap_4()
                            .child(start_action("OPEN", self.t(MessageKey::ToolbarOpen)))
                            .child(start_action("NEW", self.t(MessageKey::ToolbarNew))),
                    )
                    .child(recent_list(&self.state.recent_files)),
            )
    }

    fn archive_workspace(&self) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .overflow_hidden()
            .child(self.left_sidebar())
            .child(splitter())
            .child(self.file_list())
    }

    fn left_sidebar(&self) -> impl IntoElement {
        div()
            .w(px(248.0))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(TREE_BG))
            .child(self.archive_tree())
            .child(splitter_horizontal())
            .child(self.sidebar_preview())
    }

    fn archive_tree(&self) -> impl IntoElement {
        let root = self
            .state
            .session
            .as_ref()
            .map(|session| session.info.display_name.clone())
            .unwrap_or_else(|| self.t(MessageKey::SidebarRoot));
        let nodes = self
            .state
            .session
            .as_ref()
            .map(|session| DirectoryTree::from_listing(&session.listing))
            .unwrap_or_default();

        div()
            .flex_1()
            .id("archive-tree")
            .overflow_y_scroll()
            .text_size(px(13.0))
            .child(tree_row("[A]", root, true, 0))
            .children(
                nodes
                    .nodes
                    .values()
                    .filter(|node| node.path != "/")
                    .take(MAX_RENDERED_TREE_ROWS)
                    .map(|node| {
                        tree_row(
                            "[D]",
                            node.name.clone(),
                            node.path == self.state.tree.selected_path,
                            node.path.matches('/').count(),
                        )
                    }),
            )
    }

    fn sidebar_preview(&self) -> impl IntoElement {
        let title = match self.state.preview.mode {
            SidebarPreviewMode::Image => self.t(MessageKey::PreviewImageMetadata),
            SidebarPreviewMode::Loading => self.t(MessageKey::PreviewLoading),
            SidebarPreviewMode::Text => self.t(MessageKey::PreviewText),
            SidebarPreviewMode::Metadata => self.t(MessageKey::PreviewMetadata),
            SidebarPreviewMode::Unsupported => self.t(MessageKey::PreviewUnsupported),
            SidebarPreviewMode::Empty => self.t(MessageKey::PreviewEmptyTitle),
        };
        div()
            .h(px(210.0))
            .p_2()
            .bg(rgb(0xfafafa))
            .child(label(title).font_weight(FontWeight::SEMIBOLD))
            .child(
                div()
                    .mt_2()
                    .h(px(160.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .text_color(rgb(MUTED))
                    .child(preview_body(
                        &self.state.preview,
                        self.t(MessageKey::PreviewEmptyBody),
                    )),
            )
    }

    fn file_list(&self) -> impl IntoElement {
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(SURFACE))
            .child(self.path_bar())
            .child(self.file_header())
            .child(self.file_rows())
    }

    fn path_bar(&self) -> impl IntoElement {
        let path = self
            .state
            .session
            .as_ref()
            .map(|session| session.info.display_name.clone())
            .unwrap_or_default();
        row()
            .h_7()
            .px_2()
            .border_b_1()
            .border_color(rgb(GRID))
            .text_size(px(13.0))
            .child("[A] ")
            .child(path)
    }

    fn file_header(&self) -> impl IntoElement {
        row()
            .h_7()
            .border_b_1()
            .border_color(rgb(GRID))
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .children([
                column(self.t(MessageKey::FileListName), 4),
                column(self.t(MessageKey::FileListPackedSize), 2),
                column(self.t(MessageKey::FileListSize), 2),
                column(self.t(MessageKey::FileListType), 2),
                column(self.t(MessageKey::FileListModified), 2),
                column(self.t(MessageKey::FileListMethod), 1),
                column(self.t(MessageKey::FileListEncrypted), 1),
            ])
    }

    fn file_rows(&self) -> impl IntoElement {
        let entries = self.visible_entries();
        let range = self.state.list.virtual_window.range(entries.len());
        div()
            .flex_1()
            .id("file-rows")
            .overflow_y_scroll()
            .child(parent_row(
                self.state.list.current_directory != "/",
                self.t(MessageKey::FieldFolder),
            ))
            .children(entries[range].iter().cloned().map(file_row))
    }

    fn visible_entries(&self) -> Vec<ArchiveEntry> {
        let mut entries = self
            .state
            .session
            .as_ref()
            .map(|session| session.listing.entries.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| self.state.list.filter.matches(entry))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            match (
                a.kind == EntryKind::Directory,
                b.kind == EntryKind::Directory,
            ) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a
                    .display_path
                    .to_ascii_lowercase()
                    .cmp(&b.display_path.to_ascii_lowercase()),
            }
        });
        entries
    }

    fn status_bar(&self) -> impl IntoElement {
        let status = &self.state.status;
        row()
            .h_8()
            .px_3()
            .gap_4()
            .border_t_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0xf8fafc))
            .text_size(px(12.0))
            .text_color(rgb(0x374151))
            .child("[A]")
            .child(format!(
                "{}: {}",
                self.t(MessageKey::StatusFiles),
                status.total_files
            ))
            .child(format!(
                "{}: {}",
                self.t(MessageKey::StatusFolders),
                status.total_folders
            ))
            .child(format!(
                "{}: {}",
                self.t(MessageKey::StatusSelected),
                status.selected_entries
            ))
            .child(div().flex_1())
            .child(format_size(status.compressed_bytes))
    }

    fn overlay_layer(&self) -> impl IntoElement {
        let Some(overlay) = self.state.overlays.last() else {
            return div().into_any_element();
        };
        match overlay {
            OverlayState::CreateArchive(draft) => {
                create_archive_panel(self, draft).into_any_element()
            }
            OverlayState::Extract(dialog) => extract_panel(self, dialog).into_any_element(),
            OverlayState::Conflict(batch) => conflict_panel(self, batch).into_any_element(),
            OverlayState::Password(request) => password_panel(self, request).into_any_element(),
            OverlayState::Settings(config) => settings_panel(self, config).into_any_element(),
            OverlayState::Error(error) => error_panel(self, error).into_any_element(),
            OverlayState::Properties(properties) => {
                properties_panel(self, properties).into_any_element()
            }
            OverlayState::HelperDiagnostics(items) => helper_panel(self, items).into_any_element(),
        }
    }
}

fn ribbon_button_visual(icon: &'static str, label_text: String, enabled: bool) -> impl IntoElement {
    div()
        .w(px(82.0))
        .h(px(82.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .text_color(rgb(if enabled { 0xffffff } else { 0xaad6f5 }))
        .child(div().text_size(px(34.0)).child(icon))
        .child(div().text_size(px(13.0)).child(label_text))
}

fn start_action(icon: &'static str, text: String) -> impl IntoElement {
    div()
        .w(px(240.0))
        .h(px(120.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .border_1()
        .border_color(rgb(0xb7d7f2))
        .bg(rgb(0xffffff))
        .child(
            div()
                .text_size(px(34.0))
                .text_color(rgb(BLUE_DARK))
                .child(icon),
        )
        .child(div().text_size(px(16.0)).child(text))
}

fn recent_list(recent: &[RecentFile]) -> impl IntoElement {
    div()
        .mt_3()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(SURFACE))
        .children(recent.iter().take(12).map(|item| {
            row()
                .h_8()
                .px_2()
                .border_b_1()
                .border_color(rgb(GRID))
                .child("[A] ")
                .child(item.display_name.clone())
                .child(div().flex_1())
                .child(format!("{}", item.format))
        }))
}

fn tree_row(icon: &'static str, text: String, selected: bool, depth: usize) -> impl IntoElement {
    row()
        .h_6()
        .pl(px((depth * 14 + 4) as f32))
        .bg(rgb(if selected { SELECTED } else { TREE_BG }))
        .child(icon)
        .child(" ")
        .child(text)
}

fn parent_row(show: bool, folder_label: String) -> impl IntoElement {
    if !show {
        return div().into_any_element();
    }
    row()
        .h_7()
        .border_b_1()
        .border_color(rgb(GRID))
        .text_size(px(13.0))
        .children([
            column("[D] ..".to_string(), 4),
            column("-".to_string(), 2),
            column("-".to_string(), 2),
            column(folder_label, 2),
            column("-".to_string(), 2),
            column("-".to_string(), 1),
            column("-".to_string(), 1),
        ])
        .into_any_element()
}

fn file_row(entry: ArchiveEntry) -> impl IntoElement {
    let icon = match entry.kind {
        EntryKind::Directory => "[D] ",
        EntryKind::Symlink => "[L] ",
        EntryKind::File => "[F] ",
        EntryKind::Other => "[O] ",
    };
    row()
        .h_7()
        .border_b_1()
        .border_color(rgb(GRID))
        .text_size(px(13.0))
        .children([
            column(format!("{icon}{}", entry.display_path), 4),
            column(format_size(entry.compressed_size), 2),
            column(format_size(entry.size), 2),
            column(format!("{:?}", entry.kind), 2),
            column(
                entry
                    .modified_at
                    .map(|time| time.to_rfc3339())
                    .unwrap_or_else(|| "-".into()),
                2,
            ),
            column(entry.method.unwrap_or_else(|| "-".into()), 1),
            column(if entry.encrypted { "yes" } else { "-" }.to_string(), 1),
        ])
}

fn preview_body(preview: &SidebarPreviewState, fallback: String) -> impl IntoElement {
    match preview.mode {
        SidebarPreviewMode::Image => div().child(preview.title.clone()),
        SidebarPreviewMode::Loading => div().child("Loading preview..."),
        SidebarPreviewMode::Unsupported => div().child(preview.detail.clone()),
        SidebarPreviewMode::Text | SidebarPreviewMode::Metadata => {
            div().child(preview.detail.clone())
        }
        SidebarPreviewMode::Empty => div().child(fallback),
    }
}

fn create_archive_panel(workbench: &Workbench, draft: &CreateArchiveDraft) -> impl IntoElement {
    info_panel(
        workbench.t(MessageKey::ToolbarNew),
        vec![
            (
                workbench.t(MessageKey::FieldFormat),
                draft.format.to_string(),
            ),
            (
                workbench.t(MessageKey::FieldMethod),
                draft.compression_method.to_string(),
            ),
            (
                workbench.t(MessageKey::FieldLevel),
                draft.compression_level.to_string(),
            ),
            (workbench.t(MessageKey::FieldSolid), draft.solid.to_string()),
            (
                workbench.t(MessageKey::FieldEncryption),
                draft
                    .encryption
                    .algorithm
                    .clone()
                    .unwrap_or_else(|| workbench.t(MessageKey::FieldNone)),
            ),
            (
                workbench.t(MessageKey::FieldVolume),
                draft
                    .volume_size
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
        ],
    )
}

fn extract_panel(workbench: &Workbench, dialog: &ExtractDialogState) -> impl IntoElement {
    info_panel(
        workbench.t(MessageKey::ToolbarExtract),
        vec![
            (
                workbench.t(MessageKey::FieldDestination),
                dialog.destination.display().to_string(),
            ),
            (
                workbench.t(MessageKey::FieldScope),
                format!("{:?}", dialog.scope),
            ),
            (
                workbench.t(MessageKey::FieldOverwrite),
                format!("{:?}", dialog.options.overwrite_policy),
            ),
            (
                workbench.t(MessageKey::FieldWarnings),
                dialog
                    .preflight
                    .as_ref()
                    .map(|p| p.warnings.len().to_string())
                    .unwrap_or_else(|| "0".into()),
            ),
        ],
    )
}

fn conflict_panel(workbench: &Workbench, batch: &ConflictResolutionBatch) -> impl IntoElement {
    info_panel(
        workbench.t(MessageKey::PanelConflicts),
        vec![
            (
                workbench.t(MessageKey::FieldConflictingFiles),
                batch.conflicts.len().to_string(),
            ),
            (
                workbench.t(MessageKey::FieldDefaultPolicy),
                format!("{:?}", batch.default_policy),
            ),
        ],
    )
}

fn password_panel(workbench: &Workbench, request: &PasswordRequest) -> impl IntoElement {
    info_panel(
        request.archive_name.clone(),
        vec![
            (workbench.t(MessageKey::FieldPassword), "********".into()),
            (
                workbench.t(MessageKey::FieldRememberSession),
                request.allow_session_memory.to_string(),
            ),
            (
                workbench.t(MessageKey::FieldRetry),
                request.retry_count.to_string(),
            ),
        ],
    )
}

fn settings_panel(workbench: &Workbench, config: &AppConfig) -> impl IntoElement {
    info_panel(
        workbench.t(MessageKey::ToolbarSettings),
        vec![
            (
                workbench.t(MessageKey::SettingLanguage),
                workbench.translator.locale().tag().into(),
            ),
            (
                workbench.t(MessageKey::SettingDefaultFormat),
                config.default_create_format.to_string(),
            ),
            (
                workbench.t(MessageKey::SettingPreviewInputLimit),
                format_size(Some(config.preview.max_input_bytes)),
            ),
            (
                workbench.t(MessageKey::SettingThumbnailCache),
                format_size(Some(config.preview.thumbnail_cache_bytes)),
            ),
            (
                workbench.t(MessageKey::SettingAdvancedCodecs),
                config.show_advanced_codecs.to_string(),
            ),
            (
                workbench.t(MessageKey::SettingSessionPasswords),
                config.remember_passwords_for_session.to_string(),
            ),
        ],
    )
}

fn error_panel(workbench: &Workbench, error: &ErrorPresentation) -> impl IntoElement {
    info_panel(
        error.title.clone(),
        vec![
            (workbench.t(MessageKey::FieldStatus), error.message.clone()),
            (
                workbench.t(MessageKey::FieldAction),
                error.suggested_action.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                workbench.t(MessageKey::FieldDetails),
                error.technical_detail.clone().unwrap_or_else(|| "-".into()),
            ),
        ],
    )
}

fn properties_panel(workbench: &Workbench, properties: &PropertiesPanelState) -> impl IntoElement {
    info_panel(
        workbench.t(MessageKey::FieldProperties),
        vec![
            (
                workbench.t(MessageKey::FieldArchive),
                properties
                    .archive
                    .as_ref()
                    .map(|info| info.display_name.clone())
                    .unwrap_or_else(|| "-".into()),
            ),
            (
                workbench.t(MessageKey::FieldEntry),
                properties
                    .entry
                    .as_ref()
                    .map(|entry| entry.display_path.clone())
                    .unwrap_or_else(|| "-".into()),
            ),
            (
                workbench.t(MessageKey::FieldDiagnostics),
                properties.diagnostics.len().to_string(),
            ),
        ],
    )
}

fn helper_panel(workbench: &Workbench, helpers: &[HelperDiagnosticModel]) -> impl IntoElement {
    floating_panel()
        .child(panel_title(workbench.t(MessageKey::FieldHelperDiagnostics)))
        .children(helpers.iter().map(|helper| {
            setting_row(
                helper.name.clone(),
                helper.version.clone().unwrap_or_else(|| {
                    if helper.available {
                        workbench.t(MessageKey::FieldAvailable)
                    } else {
                        workbench.t(MessageKey::FieldMissing)
                    }
                }),
            )
        }))
}

fn info_panel(title: impl IntoElement, rows: Vec<(String, String)>) -> impl IntoElement {
    floating_panel().child(panel_title(title)).children(
        rows.into_iter()
            .map(|(label, value)| setting_row(label, value)),
    )
}

fn status_from_listing(listing: &ArchiveListing) -> StatusBarState {
    StatusBarState {
        total_files: listing
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .count() as u64,
        total_folders: listing
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Directory)
            .count() as u64,
        selected_entries: 0,
        compressed_bytes: Some(
            listing
                .entries
                .iter()
                .filter_map(|entry| entry.compressed_size)
                .sum(),
        ),
        uncompressed_bytes: Some(listing.entries.iter().filter_map(|entry| entry.size).sum()),
        active_task: None,
        warnings: Vec::new(),
    }
}

fn row() -> Div {
    div().flex().items_center()
}

fn splitter() -> impl IntoElement {
    div().w(px(4.0)).h_full().bg(rgb(0xd9d9d9))
}

fn splitter_horizontal() -> impl IntoElement {
    div().h(px(4.0)).w_full().bg(rgb(0xd9d9d9))
}

fn floating_panel() -> Div {
    div()
        .absolute()
        .right_4()
        .top_16()
        .w(px(420.0))
        .p_4()
        .border_1()
        .border_color(rgb(0xcbd5e1))
        .bg(rgb(SURFACE))
        .shadow_lg()
}

fn search_box(label: String) -> impl IntoElement {
    row()
        .h_8()
        .w(px(180.0))
        .px_3()
        .border_1()
        .border_color(rgb(0x77bce8))
        .bg(rgb(0xffffff))
        .text_size(px(13.0))
        .text_color(rgb(MUTED))
        .child(label)
}

fn column(content: impl IntoElement, grow: u32) -> impl IntoElement {
    div()
        .flex_grow()
        .flex_basis(px((grow * 72) as f32))
        .px_2()
        .truncate()
        .child(content)
}

fn label(content: impl IntoElement) -> Div {
    div().text_size(px(13.0)).child(content)
}

fn panel_title(title: impl IntoElement) -> impl IntoElement {
    label(title)
        .mb_3()
        .text_size(px(15.0))
        .font_weight(FontWeight::SEMIBOLD)
}

fn setting_row(label_text: impl IntoElement, value: impl IntoElement) -> impl IntoElement {
    row()
        .h_8()
        .justify_between()
        .border_b_1()
        .border_color(rgb(0xf1f5f9))
        .text_size(px(13.0))
        .child(div().text_color(rgb(0x64748b)).child(label_text))
        .child(div().text_color(rgb(0x0f172a)).child(value))
}

fn format_size(size: Option<u64>) -> String {
    let Some(size) = size else {
        return "-".into();
    };
    match size {
        n if n >= 1024 * 1024 * 1024 => format!("{:.1} GB", n as f64 / 1024.0 / 1024.0 / 1024.0),
        n if n >= 1024 * 1024 => format!("{:.1} MB", n as f64 / 1024.0 / 1024.0),
        n if n >= 1024 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}
