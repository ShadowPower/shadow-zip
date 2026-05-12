use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct ArchiveError {
    pub kind: ArchiveErrorKind,
    pub message: String,
    pub technical_detail: Option<String>,
    pub backend: Option<String>,
    pub archive_path: Option<PathBuf>,
    pub entry_path: Option<String>,
    pub suggestion: Option<String>,
    pub causes: Vec<ArchiveError>,
}

impl ArchiveError {
    pub fn new(kind: ArchiveErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            technical_detail: None,
            backend: None,
            archive_path: None,
            entry_path: None,
            suggestion: None,
            causes: Vec::new(),
        }
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    pub fn with_entry_path(mut self, entry_path: impl Into<String>) -> Self {
        self.entry_path = Some(entry_path.into());
        self
    }

    pub fn with_technical_detail(mut self, detail: impl Into<String>) -> Self {
        self.technical_detail = Some(detail.into());
        self
    }

    pub fn with_causes(mut self, causes: Vec<ArchiveError>) -> Self {
        self.causes = causes;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveErrorKind {
    UnsupportedFormat,
    UnsupportedCodec,
    UnsupportedFilter,
    PasswordRequired,
    InvalidPassword,
    CorruptArchive,
    InsufficientDiskSpace,
    PermissionDenied,
    PathTooLong,
    PathTraversalBlocked,
    SymlinkPolicyBlocked,
    BackendUnavailable,
    ExternalHelperFailed,
    Cancelled,
    Io,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPresentation {
    pub title: String,
    pub message: String,
    pub technical_detail: Option<String>,
    pub suggested_action: Option<String>,
    pub can_retry_with_password: bool,
    pub can_try_fallback: bool,
}

impl From<ArchiveError> for ErrorPresentation {
    fn from(error: ArchiveError) -> Self {
        Self {
            title: format!("{:?}", error.kind),
            message: error.message,
            technical_detail: error.technical_detail,
            suggested_action: error.suggestion,
            can_retry_with_password: matches!(
                error.kind,
                ArchiveErrorKind::PasswordRequired | ArchiveErrorKind::InvalidPassword
            ),
            can_try_fallback: matches!(
                error.kind,
                ArchiveErrorKind::UnsupportedCodec
                    | ArchiveErrorKind::UnsupportedFilter
                    | ArchiveErrorKind::BackendUnavailable
            ),
        }
    }
}
