use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use shadow_zip_domain::*;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct QueuedTask {
    pub plan: TaskPlan,
    pub priority: TaskPriority,
    pub enqueued_at: Instant,
}

#[derive(Default)]
pub struct TaskEngine {
    queue: Mutex<VecDeque<QueuedTask>>,
    states: Mutex<BTreeMap<Uuid, TaskState>>,
    cancellations: Mutex<BTreeMap<Uuid, CancellationToken>>,
    progress_aggregator: ProgressAggregator,
    recovery_records: Mutex<BTreeMap<Uuid, TaskRecoveryRecord>>,
}

impl TaskEngine {
    pub fn enqueue(&self, plan: TaskPlan, priority: TaskPriority) -> Uuid {
        let id = plan.id;
        let queued = QueuedTask {
            plan,
            priority,
            enqueued_at: Instant::now(),
        };

        let mut queue = self.queue.lock();
        let insert_at = queue
            .iter()
            .position(|existing| priority < existing.priority)
            .unwrap_or(queue.len());
        queue.insert(insert_at, queued);
        self.states.lock().insert(
            id,
            TaskState {
                id,
                title: queue[insert_at].plan.title.clone(),
                kind: queue[insert_at].plan.kind,
                priority,
                lifecycle: TaskLifecycle::Queued,
                progress: None,
                error: None,
            },
        );
        self.cancellations
            .lock()
            .insert(id, CancellationToken::new());
        id
    }

    pub fn next(&self) -> Option<QueuedTask> {
        self.queue.lock().pop_front()
    }

    pub fn snapshot(&self) -> Vec<QueuedTask> {
        self.queue.lock().iter().cloned().collect()
    }

    pub fn task_states(&self) -> Vec<TaskState> {
        self.states.lock().values().cloned().collect()
    }

    pub fn update_progress(&self, id: Uuid, progress: TaskProgress) {
        if let Some(state) = self.states.lock().get_mut(&id) {
            state.lifecycle = match progress.stage {
                TaskStage::Completed => TaskLifecycle::Completed,
                TaskStage::Cancelling => TaskLifecycle::Cancelling,
                TaskStage::Cancelled => TaskLifecycle::Cancelled,
                TaskStage::Failed => TaskLifecycle::Failed,
                _ => TaskLifecycle::Running,
            };
            state.progress = Some(progress);
        }
    }

    pub fn fail(&self, id: Uuid, error: ArchiveError) {
        if let Some(state) = self.states.lock().get_mut(&id) {
            state.lifecycle = TaskLifecycle::Failed;
            state.error = Some(error);
        }
    }

    pub fn cancel(&self, id: Uuid) {
        if let Some(token) = self.cancellations.lock().get(&id) {
            token.cancel();
        }
        if let Some(state) = self.states.lock().get_mut(&id) {
            state.lifecycle = TaskLifecycle::Cancelling;
        }
    }

    pub fn cancellation_token(&self, id: Uuid) -> Option<CancellationToken> {
        self.cancellations.lock().get(&id).cloned()
    }

    pub fn run_next(&self, executor: &dyn TaskExecutor) -> Option<TaskResult> {
        let queued = self.next()?;
        let id = queued.plan.id;
        let token = self
            .cancellation_token(id)
            .unwrap_or_else(CancellationToken::new);

        if let Some(state) = self.states.lock().get_mut(&id) {
            state.lifecycle = TaskLifecycle::Running;
        }

        let sink = AggregatingProgressSink {
            task_id: id,
            engine: self,
        };

        let result = executor.execute(&queued.plan, token, &sink);
        match &result {
            TaskResult::Completed => self.update_progress(id, completed_progress()),
            TaskResult::Cancelled => self.update_progress(id, cancelled_progress()),
            TaskResult::Failed(error) => self.fail(id, error.clone()),
        }
        Some(result)
    }

    pub fn spawn_worker(
        self: Arc<Self>,
        executor: Arc<dyn TaskExecutor + Send + Sync>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            while let Some(queued) = self.next() {
                let id = queued.plan.id;
                self.recovery_records
                    .lock()
                    .insert(id, TaskRecoveryRecord::from_plan(&queued.plan));
                if let Some(state) = self.states.lock().get_mut(&id) {
                    state.lifecycle = TaskLifecycle::Running;
                }
                let token = self
                    .cancellation_token(id)
                    .unwrap_or_else(CancellationToken::new);
                let sink = AggregatingProgressSink {
                    task_id: id,
                    engine: &self,
                };
                let result = executor.execute(&queued.plan, token, &sink);
                match &result {
                    TaskResult::Completed => {
                        self.recovery_records.lock().remove(&id);
                        self.update_progress(id, completed_progress());
                    }
                    TaskResult::Cancelled => self.update_progress(id, cancelled_progress()),
                    TaskResult::Failed(error) => self.fail(id, error.clone()),
                }
            }
        })
    }

    pub fn retry(&self, id: Uuid) -> Option<Uuid> {
        let record = self.recovery_records.lock().get(&id)?.clone();
        Some(self.enqueue(record.plan, record.priority))
    }

    pub fn recovery_records(&self) -> Vec<TaskRecoveryRecord> {
        self.recovery_records.lock().values().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct TaskState {
    pub id: Uuid,
    pub title: String,
    pub kind: TaskKind,
    pub priority: TaskPriority,
    pub lifecycle: TaskLifecycle,
    pub progress: Option<TaskProgress>,
    pub error: Option<ArchiveError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskLifecycle {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TaskRecoveryRecord {
    pub id: Uuid,
    pub plan: TaskPlan,
    pub priority: TaskPriority,
    pub created_at: Instant,
    pub cleanup_paths: Vec<std::path::PathBuf>,
}

impl TaskRecoveryRecord {
    fn from_plan(plan: &TaskPlan) -> Self {
        Self {
            id: plan.id,
            plan: plan.clone(),
            priority: TaskPriority::Normal,
            created_at: Instant::now(),
            cleanup_paths: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> Result<(), ArchiveError> {
        if self.is_cancelled() {
            Err(ArchiveError::new(
                ArchiveErrorKind::Cancelled,
                "Task was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

pub trait TaskExecutor {
    fn execute(
        &self,
        plan: &TaskPlan,
        cancellation: CancellationToken,
        progress: &dyn ProgressSink,
    ) -> TaskResult;
}

pub type NativeTaskHandler = Arc<
    dyn Fn(&NativeHandlerPlan, CancellationToken, &dyn ProgressSink) -> TaskResult + Send + Sync,
>;

#[derive(Default, Clone)]
pub struct TaskRuntime {
    handlers: Arc<Mutex<BTreeMap<String, NativeTaskHandler>>>,
    recovery_store: Option<PathBuf>,
}

impl TaskRuntime {
    pub fn register(&self, name: impl Into<String>, handler: NativeTaskHandler) {
        self.handlers.lock().insert(name.into(), handler);
    }

    pub fn with_recovery_store(mut self, path: PathBuf) -> Self {
        self.recovery_store = Some(path);
        self
    }

    fn execute_native_handler(
        &self,
        plan: &NativeHandlerPlan,
        cancellation: CancellationToken,
        progress: &dyn ProgressSink,
    ) -> TaskResult {
        self.handlers
            .lock()
            .get(&plan.handler)
            .cloned()
            .map(|handler| handler(plan, cancellation, progress))
            .unwrap_or_else(|| {
                TaskResult::Failed(ArchiveError::new(
                    ArchiveErrorKind::Internal,
                    format!("No native task handler registered for {}", plan.handler),
                ))
            })
    }
}

pub struct RuntimeExecutor {
    runtime: TaskRuntime,
}

impl RuntimeExecutor {
    pub fn new(runtime: TaskRuntime) -> Self {
        Self { runtime }
    }
}

impl TaskExecutor for RuntimeExecutor {
    fn execute(
        &self,
        plan: &TaskPlan,
        cancellation: CancellationToken,
        progress: &dyn ProgressSink,
    ) -> TaskResult {
        match &plan.execution {
            TaskExecutionPlan::NativeHandler(handler) => {
                self.runtime
                    .execute_native_handler(handler, cancellation, progress)
            }
            _ => PlanInterpreter.execute(plan, cancellation, progress),
        }
    }
}

pub trait ProgressSink {
    fn emit(&self, progress: TaskProgress);
}

pub enum TaskResult {
    Completed,
    Cancelled,
    Failed(ArchiveError),
}

struct AggregatingProgressSink<'a> {
    task_id: Uuid,
    engine: &'a TaskEngine,
}

impl ProgressSink for AggregatingProgressSink<'_> {
    fn emit(&self, progress: TaskProgress) {
        if self.engine.progress_aggregator.should_publish(self.task_id) {
            self.engine.update_progress(self.task_id, progress);
        }
    }
}

#[derive(Default)]
struct ProgressAggregator {
    last_publish: Mutex<BTreeMap<Uuid, Instant>>,
}

impl ProgressAggregator {
    fn should_publish(&self, id: Uuid) -> bool {
        let mut last_publish = self.last_publish.lock();
        let now = Instant::now();
        let should_publish = last_publish
            .get(&id)
            .map(|last| now.duration_since(*last) >= Duration::from_millis(33))
            .unwrap_or(true);
        if should_publish {
            last_publish.insert(id, now);
        }
        should_publish
    }
}

fn completed_progress() -> TaskProgress {
    TaskProgress {
        stage: TaskStage::Completed,
        current_path: None,
        processed_bytes: 0,
        total_bytes: None,
        processed_entries: 0,
        total_entries: None,
        bytes_per_second: None,
        eta: None,
        warnings: Vec::new(),
        summary: Some(TaskSummary::default()),
    }
}

fn cancelled_progress() -> TaskProgress {
    TaskProgress {
        stage: TaskStage::Cancelled,
        ..completed_progress()
    }
}

pub struct PlanInterpreter;

impl TaskExecutor for PlanInterpreter {
    fn execute(
        &self,
        plan: &TaskPlan,
        cancellation: CancellationToken,
        progress: &dyn ProgressSink,
    ) -> TaskResult {
        let result = match &plan.execution {
            TaskExecutionPlan::Noop => Ok(()),
            TaskExecutionPlan::NativePipeline(pipeline) => {
                run_native_pipeline(pipeline, &cancellation, progress)
            }
            TaskExecutionPlan::NativeHandler(handler) => {
                simulate_native_handler(handler, &cancellation, progress)
            }
            TaskExecutionPlan::ExternalHelper(helper) => {
                run_external_helper(helper, &cancellation, progress)
            }
            TaskExecutionPlan::Composite(plans) => {
                for child in plans {
                    if let Err(error) = cancellation.check() {
                        return if error.kind == ArchiveErrorKind::Cancelled {
                            TaskResult::Cancelled
                        } else {
                            TaskResult::Failed(error)
                        };
                    }
                    let child_plan = TaskPlan {
                        execution: child.clone(),
                        ..plan.clone()
                    };
                    match self.execute(&child_plan, cancellation.clone(), progress) {
                        TaskResult::Completed => {}
                        TaskResult::Cancelled => return TaskResult::Cancelled,
                        TaskResult::Failed(error) => return TaskResult::Failed(error),
                    }
                }
                Ok(())
            }
        };

        match result {
            Ok(()) => TaskResult::Completed,
            Err(error) if error.kind == ArchiveErrorKind::Cancelled => TaskResult::Cancelled,
            Err(error) => TaskResult::Failed(error),
        }
    }
}

fn simulate_native_handler(
    handler: &NativeHandlerPlan,
    cancellation: &CancellationToken,
    progress: &dyn ProgressSink,
) -> Result<(), ArchiveError> {
    for index in 0..handler.estimated_steps.max(1) {
        cancellation.check()?;
        progress.emit(TaskProgress {
            stage: TaskStage::Writing,
            current_path: Some(handler.handler.clone()),
            processed_bytes: index,
            total_bytes: Some(handler.estimated_steps),
            processed_entries: index,
            total_entries: Some(handler.estimated_steps),
            bytes_per_second: None,
            eta: None,
            warnings: Vec::new(),
            summary: None,
        });
    }
    Ok(())
}

fn run_native_pipeline(
    pipeline: &NativePipelinePlan,
    cancellation: &CancellationToken,
    progress: &dyn ProgressSink,
) -> Result<(), ArchiveError> {
    for (index, step) in pipeline.steps.iter().enumerate() {
        cancellation.check()?;
        progress.emit(TaskProgress {
            stage: stage_for_step(step),
            current_path: Some(format!("{step:?}")),
            processed_bytes: index as u64,
            total_bytes: Some(pipeline.steps.len() as u64),
            processed_entries: index as u64,
            total_entries: Some(pipeline.steps.len() as u64),
            bytes_per_second: None,
            eta: None,
            warnings: Vec::new(),
            summary: None,
        });
    }
    Ok(())
}

fn run_external_helper(
    helper: &ExternalHelperPlan,
    cancellation: &CancellationToken,
    progress: &dyn ProgressSink,
) -> Result<(), ArchiveError> {
    cancellation.check()?;
    progress.emit(TaskProgress {
        stage: TaskStage::Preparing,
        current_path: Some(format!(
            "{} {:?}",
            helper.executable.display(),
            helper.redacted_args
        )),
        processed_bytes: 0,
        total_bytes: None,
        processed_entries: 0,
        total_entries: None,
        bytes_per_second: None,
        eta: None,
        warnings: Vec::new(),
        summary: None,
    });
    cancellation.check()
}

fn stage_for_step(step: &PipelineStep) -> TaskStage {
    match step {
        PipelineStep::ProbeArchive
        | PipelineStep::ReadCentralDirectory
        | PipelineStep::ReadSevenZipHeader => TaskStage::Listing,
        PipelineStep::StreamDecompress { .. } | PipelineStep::StreamTarEntries => {
            TaskStage::Reading
        }
        PipelineStep::ValidateEntryPath
        | PipelineStep::CheckDestination
        | PipelineStep::ResolveConflict => TaskStage::Preparing,
        PipelineStep::CreateDirectory
        | PipelineStep::WriteFile
        | PipelineStep::WriteArchiveEntry => TaskStage::Writing,
        PipelineStep::DecodeImageMetadata
        | PipelineStep::DecodeImageBitmap
        | PipelineStep::ResizeImage => TaskStage::Decoding,
        PipelineStep::WriteArchiveHeader
        | PipelineStep::PreserveMetadata
        | PipelineStep::FinalizeArchive => TaskStage::Finalizing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_blocking_tasks_run_before_background_tasks() {
        let engine = TaskEngine::default();
        let background = TaskPlan::new(TaskKind::CacheCleanup, "background");
        let user = TaskPlan::new(TaskKind::Extract, "extract");

        engine.enqueue(background, TaskPriority::Background);
        engine.enqueue(user, TaskPriority::UserBlocking);

        assert_eq!(engine.next().unwrap().priority, TaskPriority::UserBlocking);
    }

    #[test]
    fn cancellation_token_reports_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.check().is_err());
    }
}
