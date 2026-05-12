use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use shadow_zip_domain::{ArchiveError, ArchiveErrorKind, ExternalHelperKind, ExternalHelperPlan};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub enable_file_associations: bool,
    pub enable_notifications: bool,
    pub enable_shell_context_menu: bool,
    pub locale_override: Option<String>,
    pub external_helpers: ExternalHelperConfig,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            enable_file_associations: false,
            enable_notifications: false,
            enable_shell_context_menu: false,
            locale_override: None,
            external_helpers: ExternalHelperConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalHelperConfig {
    pub unrar_path: Option<PathBuf>,
    pub libarchive_path: Option<PathBuf>,
    pub timeout_seconds: u64,
    pub output_limit_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub file_association: CapabilityState,
    pub shell_context_menu: CapabilityState,
    pub notifications: CapabilityState,
    pub reveal_in_file_manager: CapabilityState,
    pub drag_out: CapabilityState,
    pub auto_update: CapabilityState,
    pub rar_create: LicensedCapability,
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        Self {
            file_association: CapabilityState::Planned,
            shell_context_menu: CapabilityState::Planned,
            notifications: CapabilityState::Available,
            reveal_in_file_manager: CapabilityState::Available,
            drag_out: CapabilityState::Planned,
            auto_update: CapabilityState::Planned,
            rar_create: LicensedCapability::Unavailable {
                reason: "RAR creation requires a commercial RARLAB license or external tool".into(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapabilityState {
    Available,
    DisabledByConfig,
    Planned,
    UnsupportedOnPlatform,
    MissingHelper(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LicensedCapability {
    Available {
        provider: String,
        license_summary: String,
    },
    ExternalOnly {
        executable: PathBuf,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAssociation {
    pub extension: String,
    pub mime_type: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperDiagnostic {
    pub kind: ExternalHelperKind,
    pub configured_path: Option<PathBuf>,
    pub resolved_path: Option<PathBuf>,
    pub version: Option<String>,
    pub available: bool,
    pub supported_formats: Vec<String>,
}

pub struct HelperDiscovery {
    config: ExternalHelperConfig,
}

impl HelperDiscovery {
    pub fn new(config: ExternalHelperConfig) -> Self {
        Self { config }
    }

    pub fn unrar(&self) -> HelperDiagnostic {
        self.discover(
            ExternalHelperKind::Unrar,
            self.config.unrar_path.clone(),
            &["unrar", "rar"],
            &["-v"],
            vec!["rar".into(), "rar5".into()],
        )
    }

    pub fn libarchive(&self) -> HelperDiagnostic {
        self.discover(
            ExternalHelperKind::Libarchive,
            self.config.libarchive_path.clone(),
            &["bsdtar", "tar"],
            &["--version"],
            vec![
                "zip".into(),
                "7z".into(),
                "tar".into(),
                "cpio".into(),
                "iso".into(),
            ],
        )
    }

    fn discover(
        &self,
        kind: ExternalHelperKind,
        configured_path: Option<PathBuf>,
        names: &[&str],
        version_args: &[&str],
        supported_formats: Vec<String>,
    ) -> HelperDiagnostic {
        let resolved_path = configured_path
            .clone()
            .filter(|path| path.exists())
            .or_else(|| names.iter().find_map(|name| which::which(name).ok()));
        let version = resolved_path
            .as_ref()
            .and_then(|path| Command::new(path).args(version_args).output().ok())
            .map(|output| {
                let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if text.is_empty() {
                    text = String::from_utf8_lossy(&output.stderr).trim().to_string();
                }
                text.lines().next().unwrap_or_default().to_string()
            });
        HelperDiagnostic {
            kind,
            configured_path,
            resolved_path,
            available: version.is_some(),
            version,
            supported_formats,
        }
    }
}

pub fn default_file_associations() -> Vec<FileAssociation> {
    vec![
        assoc("zip", "application/zip", "ZIP archive"),
        assoc("7z", "application/x-7z-compressed", "7z archive"),
        assoc("tar", "application/x-tar", "tar archive"),
        assoc("tgz", "application/gzip", "tar.gz archive"),
        assoc("txz", "application/x-xz", "tar.xz archive"),
        assoc("tzst", "application/zstd", "tar.zst archive"),
        assoc("rar", "application/vnd.rar", "RAR archive"),
    ]
}

fn assoc(extension: &str, mime_type: &str, description: &str) -> FileAssociation {
    FileAssociation {
        extension: extension.into(),
        mime_type: mime_type.into(),
        description: description.into(),
    }
}

pub trait PlatformIntegration {
    fn capabilities(&self) -> PlatformCapabilities;
    fn install_file_associations(&self, associations: &[FileAssociation]);
    fn uninstall_file_associations(&self, associations: &[FileAssociation]);
    fn reveal_in_file_manager(&self, path: PathBuf);
    fn notify_task_finished(&self, title: &str, body: &str);
}

pub struct NoopPlatformIntegration;

impl PlatformIntegration for NoopPlatformIntegration {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::default()
    }

    fn install_file_associations(&self, _associations: &[FileAssociation]) {}
    fn uninstall_file_associations(&self, _associations: &[FileAssociation]) {}
    fn reveal_in_file_manager(&self, _path: PathBuf) {}
    fn notify_task_finished(&self, _title: &str, _body: &str) {}
}

pub struct DesktopPlatformIntegration {
    config: PlatformConfig,
}

impl DesktopPlatformIntegration {
    pub fn new(config: PlatformConfig) -> Self {
        Self { config }
    }
}

impl PlatformIntegration for DesktopPlatformIntegration {
    fn capabilities(&self) -> PlatformCapabilities {
        let mut capabilities = PlatformCapabilities::default();
        if !self.config.enable_file_associations {
            capabilities.file_association = CapabilityState::DisabledByConfig;
        }
        if !self.config.enable_shell_context_menu {
            capabilities.shell_context_menu = CapabilityState::DisabledByConfig;
        }
        if !self.config.enable_notifications {
            capabilities.notifications = CapabilityState::DisabledByConfig;
        }
        capabilities
    }

    fn install_file_associations(&self, associations: &[FileAssociation]) {
        platform_install_file_associations(associations);
    }

    fn uninstall_file_associations(&self, associations: &[FileAssociation]) {
        platform_uninstall_file_associations(associations);
    }

    fn reveal_in_file_manager(&self, path: PathBuf) {
        let _ = opener::reveal(path);
    }

    fn notify_task_finished(&self, title: &str, body: &str) {
        if self.config.enable_notifications {
            let _ = notify_rust::Notification::new()
                .summary(title)
                .body(body)
                .show();
        }
    }
}

pub struct HelperRunner;

impl HelperRunner {
    pub fn run(
        plan: &ExternalHelperPlan,
        cancel: impl Fn() -> bool,
    ) -> Result<HelperOutput, ArchiveError> {
        let mut child = Command::new(&plan.executable)
            .args(&plan.args)
            .current_dir(
                plan.working_dir
                    .as_ref()
                    .unwrap_or(&std::env::current_dir().unwrap_or_default()),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                ArchiveError::new(
                    ArchiveErrorKind::ExternalHelperFailed,
                    "Could not start external helper",
                )
                .with_technical_detail(format!("{}: {error}", plan.executable.display()))
            })?;

        let started = Instant::now();
        loop {
            if cancel() {
                let _ = child.kill();
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Cancelled,
                    "External helper was cancelled",
                ));
            }
            if started.elapsed() > Duration::from_millis(plan.timeout_ms) {
                let _ = child.kill();
                return Err(ArchiveError::new(
                    ArchiveErrorKind::ExternalHelperFailed,
                    "External helper timed out",
                ));
            }
            if let Some(status) = child.try_wait().map_err(helper_wait_error)? {
                let output = child.wait_with_output().map_err(helper_wait_error)?;
                let stdout = limited_output(output.stdout, plan.output_limit_bytes);
                let stderr = limited_output(output.stderr, plan.output_limit_bytes);
                if status.success() {
                    return Ok(HelperOutput { stdout, stderr });
                }
                return Err(ArchiveError::new(
                    ArchiveErrorKind::ExternalHelperFailed,
                    "External helper returned a non-zero status",
                )
                .with_technical_detail(format!(
                    "args={:?}; stderr={}",
                    plan.redacted_args,
                    String::from_utf8_lossy(&stderr)
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

pub struct HelperOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn helper_wait_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(
        ArchiveErrorKind::ExternalHelperFailed,
        "External helper failed",
    )
    .with_technical_detail(error.to_string())
}

fn limited_output(mut output: Vec<u8>, limit: u64) -> Vec<u8> {
    output.truncate(limit as usize);
    output
}

#[cfg(target_os = "windows")]
fn platform_install_file_associations(_associations: &[FileAssociation]) {
    // Installer writes HKCU/HKLM ProgID and context menu keys from this model.
}

#[cfg(target_os = "windows")]
fn platform_uninstall_file_associations(_associations: &[FileAssociation]) {}

#[cfg(target_os = "macos")]
fn platform_install_file_associations(_associations: &[FileAssociation]) {
    // macOS document types are emitted into the bundle Info.plist by packaging.
}

#[cfg(target_os = "macos")]
fn platform_uninstall_file_associations(_associations: &[FileAssociation]) {}

#[cfg(target_os = "linux")]
fn platform_install_file_associations(_associations: &[FileAssociation]) {
    // Packaging generates .desktop and MIME XML from FileAssociation records.
}

#[cfg(target_os = "linux")]
fn platform_uninstall_file_associations(_associations: &[FileAssociation]) {}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_install_file_associations(_associations: &[FileAssociation]) {}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_uninstall_file_associations(_associations: &[FileAssociation]) {}
