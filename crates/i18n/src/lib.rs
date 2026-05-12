use std::{borrow::Cow, collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Locale {
    EnUs,
    ZhCn,
}

impl Locale {
    pub fn from_system_tag(tag: &str) -> Self {
        let normalized = tag.replace('_', "-").to_ascii_lowercase();
        if normalized.starts_with("zh") {
            Self::ZhCn
        } else {
            Self::EnUs
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::EnUs
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessageKey {
    AppName,
    MenuFile,
    MenuEdit,
    MenuFind,
    MenuOptions,
    MenuView,
    MenuHelp,
    ToolbarOpen,
    ToolbarExtract,
    ToolbarCompress,
    ToolbarNew,
    ToolbarAdd,
    ToolbarDelete,
    ToolbarTest,
    ToolbarView,
    ToolbarCodePage,
    ToolbarDiagnostics,
    ToolbarSearch,
    ToolbarSettings,
    SidebarRoot,
    FileListName,
    FileListSize,
    FileListPackedSize,
    FileListType,
    FileListModified,
    FileListMethod,
    FileListEncrypted,
    PreviewEmptyTitle,
    PreviewEmptyBody,
    PreviewImageMetadata,
    PreviewLoading,
    PreviewText,
    PreviewMetadata,
    PreviewUnsupported,
    TaskCenterTitle,
    StatusReady,
    StatusSequentialArchive,
    StatusSolidArchive,
    StatusFiles,
    StatusFolders,
    StatusSelected,
    ErrorUnsupportedFormat,
    ErrorUnsupportedCodec,
    ErrorUnsupportedFilter,
    ErrorPasswordRequired,
    ErrorInvalidPassword,
    ErrorCorruptArchive,
    ErrorInsufficientDiskSpace,
    ErrorPermissionDenied,
    ErrorPathTooLong,
    ErrorPathTraversalBlocked,
    ErrorSymlinkPolicyBlocked,
    ErrorBackendUnavailable,
    ErrorExternalHelperFailed,
    ErrorCancelled,
    ErrorIo,
    ErrorInternal,
    PanelConflicts,
    PanelPassword,
    PanelErrorDetails,
    SettingLanguage,
    SettingDefaultFormat,
    SettingPreviewInputLimit,
    SettingThumbnailCache,
    SettingAdvancedCodecs,
    SettingSessionPasswords,
    FieldFormat,
    FieldMethod,
    FieldLevel,
    FieldSolid,
    FieldEncryption,
    FieldStatus,
    FieldAction,
    FieldNone,
    FieldFolder,
    FieldVolume,
    FieldDestination,
    FieldScope,
    FieldOverwrite,
    FieldWarnings,
    FieldConflictingFiles,
    FieldDefaultPolicy,
    FieldPassword,
    FieldRememberSession,
    FieldRetry,
    FieldDetails,
    FieldProperties,
    FieldArchive,
    FieldEntry,
    FieldDiagnostics,
    FieldAvailable,
    FieldMissing,
    FieldHelperDiagnostics,
}

#[derive(Debug, Clone, Default)]
pub struct TranslationArgs {
    values: BTreeMap<Cow<'static, str>, String>,
}

impl TranslationArgs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<Cow<'static, str>>, value: impl ToString) -> Self {
        self.values.insert(key.into(), value.to_string());
        self
    }
}

pub struct Translator {
    locale: Locale,
    fallback: Locale,
}

impl Translator {
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            fallback: Locale::EnUs,
        }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
    }

    pub fn text(&self, key: MessageKey) -> Cow<'static, str> {
        lookup(self.locale, key)
            .or_else(|| lookup(self.fallback, key))
            .unwrap_or(Cow::Borrowed("Missing translation"))
    }

    pub fn format(&self, key: MessageKey, args: &TranslationArgs) -> String {
        let mut text = self.text(key).into_owned();
        for (key, value) in &args.values {
            text = text.replace(&format!("{{{key}}}"), value);
        }
        text
    }
}

fn lookup(locale: Locale, key: MessageKey) -> Option<Cow<'static, str>> {
    let text = match locale {
        Locale::EnUs => en_us(key),
        Locale::ZhCn => zh_cn(key),
    };
    text.map(Cow::Borrowed)
}

fn en_us(key: MessageKey) -> Option<&'static str> {
    Some(match key {
        MessageKey::AppName => "Shadow Zip",
        MessageKey::MenuFile => "File",
        MessageKey::MenuEdit => "Edit",
        MessageKey::MenuFind => "Find",
        MessageKey::MenuOptions => "Options",
        MessageKey::MenuView => "View",
        MessageKey::MenuHelp => "Help",
        MessageKey::ToolbarOpen => "Open",
        MessageKey::ToolbarExtract => "Extract",
        MessageKey::ToolbarCompress => "Compress",
        MessageKey::ToolbarNew => "New",
        MessageKey::ToolbarAdd => "Add",
        MessageKey::ToolbarDelete => "Delete",
        MessageKey::ToolbarTest => "Test",
        MessageKey::ToolbarView => "View",
        MessageKey::ToolbarCodePage => "Code page",
        MessageKey::ToolbarDiagnostics => "Diagnostics",
        MessageKey::ToolbarSearch => "Search",
        MessageKey::ToolbarSettings => "Settings",
        MessageKey::SidebarRoot => "Archive",
        MessageKey::FileListName => "Name",
        MessageKey::FileListSize => "Size",
        MessageKey::FileListPackedSize => "Packed",
        MessageKey::FileListType => "Type",
        MessageKey::FileListModified => "Modified",
        MessageKey::FileListMethod => "Method",
        MessageKey::FileListEncrypted => "Encrypted",
        MessageKey::PreviewEmptyTitle => "No preview",
        MessageKey::PreviewEmptyBody => "Select an entry to inspect metadata and preview content.",
        MessageKey::PreviewImageMetadata => "Image metadata",
        MessageKey::PreviewLoading => "Loading preview",
        MessageKey::PreviewText => "Text preview",
        MessageKey::PreviewMetadata => "Metadata",
        MessageKey::PreviewUnsupported => "Preview is not available for this entry.",
        MessageKey::TaskCenterTitle => "Tasks",
        MessageKey::StatusReady => "Ready",
        MessageKey::StatusSequentialArchive => {
            "Sequential archive: browsing and preview may require scanning from the beginning."
        }
        MessageKey::StatusSolidArchive => {
            "Solid archive: single-file preview and extraction may be slower."
        }
        MessageKey::StatusFiles => "Files",
        MessageKey::StatusFolders => "Folders",
        MessageKey::StatusSelected => "Selected",
        MessageKey::ErrorUnsupportedFormat => "This archive format is not supported.",
        MessageKey::ErrorUnsupportedCodec => "This archive uses an unsupported codec.",
        MessageKey::ErrorUnsupportedFilter => "This archive uses an unsupported filter.",
        MessageKey::ErrorPasswordRequired => "A password is required.",
        MessageKey::ErrorInvalidPassword => "The password is incorrect.",
        MessageKey::ErrorCorruptArchive => "The archive appears to be damaged.",
        MessageKey::ErrorInsufficientDiskSpace => "There is not enough free disk space.",
        MessageKey::ErrorPermissionDenied => "Permission was denied.",
        MessageKey::ErrorPathTooLong => "A path is too long for the configured policy.",
        MessageKey::ErrorPathTraversalBlocked => "A dangerous path was blocked.",
        MessageKey::ErrorSymlinkPolicyBlocked => "A symbolic link was blocked by policy.",
        MessageKey::ErrorBackendUnavailable => "The required archive backend is not available.",
        MessageKey::ErrorExternalHelperFailed => "An external helper failed.",
        MessageKey::ErrorCancelled => "The operation was cancelled.",
        MessageKey::ErrorIo => "A file system operation failed.",
        MessageKey::ErrorInternal => "An internal error occurred.",
        MessageKey::PanelConflicts => "Conflicts",
        MessageKey::PanelPassword => "Archive password",
        MessageKey::PanelErrorDetails => "Error details",
        MessageKey::SettingLanguage => "Language",
        MessageKey::SettingDefaultFormat => "Default format",
        MessageKey::SettingPreviewInputLimit => "Preview input limit",
        MessageKey::SettingThumbnailCache => "Thumbnail cache",
        MessageKey::SettingAdvancedCodecs => "Advanced codecs",
        MessageKey::SettingSessionPasswords => "Session passwords",
        MessageKey::FieldFormat => "Format",
        MessageKey::FieldMethod => "Method",
        MessageKey::FieldLevel => "Level",
        MessageKey::FieldSolid => "Solid",
        MessageKey::FieldEncryption => "Encryption",
        MessageKey::FieldStatus => "Status",
        MessageKey::FieldAction => "Action",
        MessageKey::FieldNone => "None",
        MessageKey::FieldFolder => "Folder",
        MessageKey::FieldVolume => "Volume",
        MessageKey::FieldDestination => "Destination",
        MessageKey::FieldScope => "Scope",
        MessageKey::FieldOverwrite => "Overwrite",
        MessageKey::FieldWarnings => "Warnings",
        MessageKey::FieldConflictingFiles => "Conflicting files",
        MessageKey::FieldDefaultPolicy => "Default policy",
        MessageKey::FieldPassword => "Password",
        MessageKey::FieldRememberSession => "Remember for session",
        MessageKey::FieldRetry => "Retry",
        MessageKey::FieldDetails => "Details",
        MessageKey::FieldProperties => "Properties",
        MessageKey::FieldArchive => "Archive",
        MessageKey::FieldEntry => "Entry",
        MessageKey::FieldDiagnostics => "Diagnostics",
        MessageKey::FieldAvailable => "Available",
        MessageKey::FieldMissing => "Missing",
        MessageKey::FieldHelperDiagnostics => "Helper diagnostics",
    })
}

fn zh_cn(key: MessageKey) -> Option<&'static str> {
    Some(match key {
        MessageKey::AppName => "Shadow Zip",
        MessageKey::MenuFile => "文件(F)",
        MessageKey::MenuEdit => "编辑(E)",
        MessageKey::MenuFind => "查找(I)",
        MessageKey::MenuOptions => "选项(O)",
        MessageKey::MenuView => "视图(V)",
        MessageKey::MenuHelp => "帮助(H)",
        MessageKey::ToolbarOpen => "打开",
        MessageKey::ToolbarExtract => "解压",
        MessageKey::ToolbarCompress => "压缩",
        MessageKey::ToolbarNew => "新建",
        MessageKey::ToolbarAdd => "添加",
        MessageKey::ToolbarDelete => "删除",
        MessageKey::ToolbarTest => "测试",
        MessageKey::ToolbarView => "查看",
        MessageKey::ToolbarCodePage => "代码页",
        MessageKey::ToolbarDiagnostics => "诊断",
        MessageKey::ToolbarSearch => "搜索",
        MessageKey::ToolbarSettings => "设置",
        MessageKey::SidebarRoot => "归档",
        MessageKey::FileListName => "名称",
        MessageKey::FileListSize => "大小",
        MessageKey::FileListPackedSize => "压缩后",
        MessageKey::FileListType => "类型",
        MessageKey::FileListModified => "修改时间",
        MessageKey::FileListMethod => "方法",
        MessageKey::FileListEncrypted => "加密",
        MessageKey::PreviewEmptyTitle => "无预览",
        MessageKey::PreviewEmptyBody => "选择条目后可查看元数据和内容预览。",
        MessageKey::PreviewImageMetadata => "图片元数据",
        MessageKey::PreviewLoading => "正在加载预览",
        MessageKey::PreviewText => "文本预览",
        MessageKey::PreviewMetadata => "元数据",
        MessageKey::PreviewUnsupported => "该条目暂不支持预览。",
        MessageKey::TaskCenterTitle => "任务",
        MessageKey::StatusReady => "就绪",
        MessageKey::StatusSequentialArchive => "顺序归档：浏览和预览可能需要从开头扫描。",
        MessageKey::StatusSolidArchive => "Solid 归档：单文件预览和解压可能较慢。",
        MessageKey::StatusFiles => "文件",
        MessageKey::StatusFolders => "文件夹",
        MessageKey::StatusSelected => "已选",
        MessageKey::ErrorUnsupportedFormat => "不支持此归档格式。",
        MessageKey::ErrorUnsupportedCodec => "此归档使用了不支持的压缩算法。",
        MessageKey::ErrorUnsupportedFilter => "此归档使用了不支持的过滤器。",
        MessageKey::ErrorPasswordRequired => "需要输入密码。",
        MessageKey::ErrorInvalidPassword => "密码不正确。",
        MessageKey::ErrorCorruptArchive => "归档可能已损坏。",
        MessageKey::ErrorInsufficientDiskSpace => "磁盘可用空间不足。",
        MessageKey::ErrorPermissionDenied => "没有足够权限。",
        MessageKey::ErrorPathTooLong => "路径超过当前策略允许的长度。",
        MessageKey::ErrorPathTraversalBlocked => "已阻止危险路径。",
        MessageKey::ErrorSymlinkPolicyBlocked => "已按策略阻止符号链接。",
        MessageKey::ErrorBackendUnavailable => "所需归档后端不可用。",
        MessageKey::ErrorExternalHelperFailed => "外部辅助程序执行失败。",
        MessageKey::ErrorCancelled => "操作已取消。",
        MessageKey::ErrorIo => "文件系统操作失败。",
        MessageKey::ErrorInternal => "发生内部错误。",
        MessageKey::PanelConflicts => "冲突",
        MessageKey::PanelPassword => "归档密码",
        MessageKey::PanelErrorDetails => "错误详情",
        MessageKey::SettingLanguage => "语言",
        MessageKey::SettingDefaultFormat => "默认格式",
        MessageKey::SettingPreviewInputLimit => "预览输入上限",
        MessageKey::SettingThumbnailCache => "缩略图缓存",
        MessageKey::SettingAdvancedCodecs => "高级压缩算法",
        MessageKey::SettingSessionPasswords => "会话密码记忆",
        MessageKey::FieldFormat => "格式",
        MessageKey::FieldMethod => "方法",
        MessageKey::FieldLevel => "等级",
        MessageKey::FieldSolid => "Solid",
        MessageKey::FieldEncryption => "加密",
        MessageKey::FieldStatus => "状态",
        MessageKey::FieldAction => "操作",
        MessageKey::FieldNone => "无",
        MessageKey::FieldFolder => "文件夹",
        MessageKey::FieldVolume => "分卷",
        MessageKey::FieldDestination => "目标位置",
        MessageKey::FieldScope => "范围",
        MessageKey::FieldOverwrite => "覆盖策略",
        MessageKey::FieldWarnings => "警告",
        MessageKey::FieldConflictingFiles => "冲突文件",
        MessageKey::FieldDefaultPolicy => "默认策略",
        MessageKey::FieldPassword => "密码",
        MessageKey::FieldRememberSession => "会话记忆",
        MessageKey::FieldRetry => "重试次数",
        MessageKey::FieldDetails => "详情",
        MessageKey::FieldProperties => "属性",
        MessageKey::FieldArchive => "归档",
        MessageKey::FieldEntry => "条目",
        MessageKey::FieldDiagnostics => "诊断",
        MessageKey::FieldAvailable => "可用",
        MessageKey::FieldMissing => "缺失",
        MessageKey::FieldHelperDiagnostics => "辅助程序诊断",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_english_and_chinese() {
        assert_eq!(
            Translator::new(Locale::EnUs).text(MessageKey::ToolbarOpen),
            "Open"
        );
        assert_eq!(
            Translator::new(Locale::ZhCn).text(MessageKey::ToolbarOpen),
            "打开"
        );
    }

    #[test]
    fn detects_chinese_system_tags() {
        assert_eq!(Locale::from_system_tag("zh-Hans-CN"), Locale::ZhCn);
        assert_eq!(Locale::from_system_tag("en-US"), Locale::EnUs);
    }
}
