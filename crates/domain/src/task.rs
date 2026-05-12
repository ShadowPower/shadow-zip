use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ArchiveFormat, CompressionMethod};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub id: Uuid,
    pub kind: TaskKind,
    pub title: String,
    pub estimated_bytes: Option<u64>,
    pub estimated_entries: Option<u64>,
    pub requires_password: bool,
    pub requires_external_helper: bool,
    pub warnings: Vec<TaskWarning>,
    pub execution: TaskExecutionPlan,
    pub recovery: TaskRecoveryPolicy,
}

impl TaskPlan {
    pub fn new(kind: TaskKind, title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            title: title.into(),
            estimated_bytes: None,
            estimated_entries: None,
            requires_password: false,
            requires_external_helper: false,
            warnings: Vec::new(),
            execution: TaskExecutionPlan::Noop,
            recovery: TaskRecoveryPolicy::RecordAndCleanup,
        }
    }

    pub fn estimated_entries(mut self, count: usize) -> Self {
        self.estimated_entries = Some(count as u64);
        self
    }

    pub fn warn(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.warnings.push(TaskWarning {
            code: code.into(),
            message: message.into(),
        });
        self
    }

    pub fn native(mut self, plan: NativePipelinePlan) -> Self {
        self.execution = TaskExecutionPlan::NativePipeline(plan);
        self
    }

    pub fn external(mut self, plan: ExternalHelperPlan) -> Self {
        self.requires_external_helper = true;
        self.execution = TaskExecutionPlan::ExternalHelper(plan);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskRecoveryPolicy {
    None,
    RecordAndCleanup,
    ResumeWhereSupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskExecutionPlan {
    Noop,
    NativePipeline(NativePipelinePlan),
    NativeHandler(NativeHandlerPlan),
    ExternalHelper(ExternalHelperPlan),
    Composite(Vec<TaskExecutionPlan>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeHandlerPlan {
    pub handler: String,
    pub payload: serde_json::Value,
    pub estimated_steps: u64,
    pub temp_policy: TempFilePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePipelinePlan {
    pub steps: Vec<PipelineStep>,
    pub cancellation_points: Vec<CancellationPoint>,
    pub temp_policy: TempFilePolicy,
}

impl NativePipelinePlan {
    pub fn new(steps: impl Into<Vec<PipelineStep>>) -> Self {
        Self {
            steps: steps.into(),
            cancellation_points: standard_cancellation_points(),
            temp_policy: TempFilePolicy::None,
        }
    }

    pub fn quick(steps: impl Into<Vec<PipelineStep>>) -> Self {
        Self {
            steps: steps.into(),
            cancellation_points: vec![
                CancellationPoint::BeforeOpen,
                CancellationPoint::BetweenEntries,
            ],
            temp_policy: TempFilePolicy::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PipelineStep {
    ProbeArchive,
    ReadCentralDirectory,
    ReadSevenZipHeader,
    StreamDecompress { codec: CompressionMethod },
    StreamTarEntries,
    ValidateEntryPath,
    CheckDestination,
    ResolveConflict,
    CreateDirectory,
    WriteFile,
    PreserveMetadata,
    DecodeImageMetadata,
    DecodeImageBitmap,
    ResizeImage,
    WriteArchiveHeader,
    WriteArchiveEntry,
    FinalizeArchive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancellationPoint {
    BeforeOpen,
    BetweenEntries,
    BeforeReadBuffer,
    AfterReadBuffer,
    BeforeWriteBuffer,
    AfterWriteBuffer,
    BeforeExternalProcess,
    AfterExternalProcess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TempFilePolicy {
    None,
    MetadataOnly,
    BoundedPreviewTemp,
    RequiredByExternalHelper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalHelperPlan {
    pub helper_kind: ExternalHelperKind,
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub timeout_ms: u64,
    pub output_limit_bytes: u64,
    pub redacted_args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalHelperKind {
    Unrar,
    SevenZip,
    Libarchive,
    PlatformOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    Open,
    Extract,
    Create,
    Test,
    BuildIndex,
    Preview,
    CacheCleanup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskProgress {
    pub stage: TaskStage,
    pub current_path: Option<String>,
    pub processed_bytes: u64,
    pub total_bytes: Option<u64>,
    pub processed_entries: u64,
    pub total_entries: Option<u64>,
    pub bytes_per_second: Option<f64>,
    pub eta: Option<Duration>,
    pub warnings: Vec<TaskWarning>,
    pub summary: Option<TaskSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskSummary {
    pub processed_entries: u64,
    pub skipped_entries: u64,
    pub blocked_entries: u64,
    pub failed_entries: u64,
    pub processed_bytes: u64,
    pub warnings: Vec<TaskWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStage {
    Queued,
    Preparing,
    Listing,
    Scanning,
    Reading,
    Writing,
    Decoding,
    Finalizing,
    Completed,
    Cancelling,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskPriority {
    UserBlocking,
    Normal,
    Background,
    Maintenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub formats: Vec<ArchiveFormat>,
    pub capabilities: crate::ArchiveCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub format: ArchiveFormat,
    pub confidence: ProbeConfidence,
    pub backend_name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProbeConfidence {
    Impossible,
    Extension,
    Signature,
    Strong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryStream {
    pub entry: crate::EntryId,
    pub access_cost: AccessCost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessCost {
    Random,
    SequentialFromStart,
    SolidBlockScan,
    ExternalHelper,
}

fn standard_cancellation_points() -> Vec<CancellationPoint> {
    vec![
        CancellationPoint::BeforeOpen,
        CancellationPoint::BetweenEntries,
        CancellationPoint::BeforeReadBuffer,
        CancellationPoint::AfterReadBuffer,
        CancellationPoint::BeforeWriteBuffer,
        CancellationPoint::AfterWriteBuffer,
    ]
}
