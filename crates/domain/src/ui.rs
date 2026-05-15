use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    AppConfig, ArchiveEntry, ArchiveInfo, ArchiveSessionSnapshot, ConflictResolutionBatch,
    CreateArchiveDraft, DirectoryTree, EntryFilter, EntryId, EntrySort, ErrorPresentation,
    ExtractOptions, ExtractPreflight, OverwritePolicy, PasswordRequest, RecentFile, SessionId,
    TaskProgress, TaskWarning, VirtualListWindow,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MenuCommand {
    FileOpen,
    FileNewArchive,
    FileClose,
    FileExit,
    EditSelectAll,
    EditCopyPath,
    EditDelete,
    FindSearch,
    OptionsSettings,
    ViewPreview,
    ViewProperties,
    ViewCodePage,
    HelpAbout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ToolbarCommand {
    Open,
    Extract,
    NewArchive,
    Add,
    Delete,
    Test,
    View,
    CodePage,
    Settings,
    HelperDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntryCommand {
    Open,
    Preview,
    Extract,
    ExtractTo,
    CopyPath,
    Properties,
    TestSelected,
    Delete,
    Reveal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchState {
    pub locale: Option<String>,
    pub session: Option<ArchiveSessionSnapshot>,
    pub recent_files: Vec<RecentFile>,
    pub menu: MenuBarState,
    pub ribbon: RibbonState,
    pub tree: ArchiveTreeState,
    pub list: FileListState,
    pub preview: SidebarPreviewState,
    pub status: StatusBarState,
    pub overlays: Vec<OverlayState>,
    pub shortcuts: Vec<KeyboardShortcut>,
    pub drag_drop: DragDropState,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            locale: None,
            session: None,
            recent_files: Vec::new(),
            menu: MenuBarState::default(),
            ribbon: RibbonState::default(),
            tree: ArchiveTreeState::default(),
            list: FileListState::default(),
            preview: SidebarPreviewState::default(),
            status: StatusBarState::default(),
            overlays: Vec::new(),
            shortcuts: KeyboardShortcut::defaults(),
            drag_drop: DragDropState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuBarState {
    pub menus: Vec<MenuDefinition>,
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self {
            menus: vec![
                MenuDefinition::new(
                    "menu.file",
                    vec![
                        MenuCommand::FileOpen,
                        MenuCommand::FileNewArchive,
                        MenuCommand::FileClose,
                        MenuCommand::FileExit,
                    ],
                ),
                MenuDefinition::new(
                    "menu.edit",
                    vec![
                        MenuCommand::EditSelectAll,
                        MenuCommand::EditCopyPath,
                        MenuCommand::EditDelete,
                    ],
                ),
                MenuDefinition::new("menu.find", vec![MenuCommand::FindSearch]),
                MenuDefinition::new("menu.options", vec![MenuCommand::OptionsSettings]),
                MenuDefinition::new(
                    "menu.view",
                    vec![
                        MenuCommand::ViewPreview,
                        MenuCommand::ViewProperties,
                        MenuCommand::ViewCodePage,
                    ],
                ),
                MenuDefinition::new("menu.help", vec![MenuCommand::HelpAbout]),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuDefinition {
    pub label_key: String,
    pub commands: Vec<MenuCommand>,
}

impl MenuDefinition {
    pub fn new(label_key: impl Into<String>, commands: Vec<MenuCommand>) -> Self {
        Self {
            label_key: label_key.into(),
            commands,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RibbonState {
    pub commands: Vec<RibbonCommandState>,
    pub search_query: String,
}

impl Default for RibbonState {
    fn default() -> Self {
        Self {
            commands: [
                ToolbarCommand::Open,
                ToolbarCommand::Extract,
                ToolbarCommand::NewArchive,
                ToolbarCommand::Add,
                ToolbarCommand::Delete,
                ToolbarCommand::Test,
                ToolbarCommand::View,
                ToolbarCommand::CodePage,
                ToolbarCommand::Settings,
            ]
            .into_iter()
            .map(RibbonCommandState::enabled)
            .collect(),
            search_query: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RibbonCommandState {
    pub command: ToolbarCommand,
    pub enabled: bool,
    pub checked: bool,
}

impl RibbonCommandState {
    pub fn enabled(command: ToolbarCommand) -> Self {
        Self {
            command,
            enabled: true,
            checked: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveTreeState {
    pub root_label: String,
    pub selected_path: String,
    pub expanded_paths: BTreeSet<String>,
    pub nodes: DirectoryTree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileListState {
    pub current_directory: String,
    pub filter: EntryFilter,
    pub sort: EntrySort,
    pub selected_entries: BTreeSet<EntryId>,
    pub focused_entry: Option<EntryId>,
    pub virtual_window: VirtualListWindow,
    pub include_parent_row: bool,
    pub context_menu: Option<EntryContextMenuState>,
}

impl Default for FileListState {
    fn default() -> Self {
        Self {
            current_directory: "/".into(),
            filter: EntryFilter::default(),
            sort: EntrySort::default(),
            selected_entries: BTreeSet::new(),
            focused_entry: None,
            virtual_window: VirtualListWindow {
                first_index: 0,
                visible_count: 80,
                overscan: 20,
                row_height_px: 28.0,
            },
            include_parent_row: true,
            context_menu: None,
        }
    }
}

impl FileListState {
    pub fn select_one(&mut self, entry: EntryId) {
        self.focused_entry = Some(entry);
        self.selected_entries.clear();
        self.selected_entries.insert(entry);
    }

    pub fn selected_count(&self) -> u64 {
        self.selected_entries.len() as u64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryContextMenuState {
    pub entry_ids: Vec<EntryId>,
    pub commands: Vec<EntryCommand>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SidebarPreviewState {
    pub selected_entry: Option<EntryId>,
    pub mode: SidebarPreviewMode,
    pub title: String,
    pub detail: String,
    pub image: Option<ImagePreviewState>,
    pub pending_task: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarPreviewMode {
    #[default]
    Empty,
    Image,
    Text,
    Metadata,
    Unsupported,
    Loading,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePreviewState {
    pub cache_key: Option<String>,
    pub dimensions: PixelSizeModel,
    pub fit: PreviewFitMode,
    pub zoom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewFitMode {
    FitPanel,
    ActualSize,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PixelSizeModel {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusBarState {
    pub total_files: u64,
    pub total_folders: u64,
    pub selected_entries: u64,
    pub compressed_bytes: Option<u64>,
    pub uncompressed_bytes: Option<u64>,
    pub active_task: Option<TaskProgress>,
    pub warnings: Vec<TaskWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OverlayState {
    CreateArchive(CreateArchiveDraft),
    Extract(ExtractDialogState),
    Conflict(ConflictResolutionBatch),
    Password(PasswordRequest),
    Settings(AppConfig),
    Error(ErrorPresentation),
    Properties(PropertiesPanelState),
    HelperDiagnostics(Vec<HelperDiagnosticModel>),
}

impl OverlayState {
    pub fn create_default(output: PathBuf) -> Self {
        OverlayState::CreateArchive(CreateArchiveDraft::default_for(
            crate::ArchiveFormat::Zip,
            Vec::new(),
            output,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractDialogState {
    pub session_id: SessionId,
    pub destination: PathBuf,
    pub scope: ExtractScope,
    pub options: ExtractOptions,
    pub preflight: Option<ExtractPreflight>,
    pub open_after_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtractScope {
    All,
    Selected(Vec<EntryId>),
    CurrentDirectory(String),
}

impl ExtractDialogState {
    pub fn all(session_id: SessionId, destination: PathBuf) -> Self {
        Self {
            session_id,
            destination,
            scope: ExtractScope::All,
            options: ExtractOptions {
                overwrite_policy: OverwritePolicy::AskBatch,
                ..ExtractOptions::default()
            },
            preflight: None,
            open_after_complete: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertiesPanelState {
    pub archive: Option<ArchiveInfo>,
    pub entry: Option<ArchiveEntry>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperDiagnosticModel {
    pub name: String,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub available: bool,
    pub supported_formats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardShortcut {
    pub command: MenuCommand,
    pub accelerator: String,
}

impl KeyboardShortcut {
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::new(MenuCommand::FileOpen, "Ctrl+O"),
            Self::new(MenuCommand::FileNewArchive, "Ctrl+N"),
            Self::new(MenuCommand::EditSelectAll, "Ctrl+A"),
            Self::new(MenuCommand::FindSearch, "Ctrl+F"),
            Self::new(MenuCommand::ViewPreview, "Space"),
            Self::new(MenuCommand::EditDelete, "Delete"),
        ]
    }

    fn new(command: MenuCommand, accelerator: impl Into<String>) -> Self {
        Self {
            command,
            accelerator: accelerator.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DragDropState {
    pub hover: bool,
    pub pending_paths: Vec<PathBuf>,
    pub intent: DragDropIntent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DragDropIntent {
    #[default]
    None,
    OpenArchive,
    CreateArchive,
    ExtractOut,
}
