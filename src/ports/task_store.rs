use crate::{domain::task::*, error::AppError};
use async_trait::async_trait;
#[async_trait]
pub trait TaskStore {
    async fn create_task(
        &self,
        r: CreateTaskRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError>;
    async fn revise_task(
        &self,
        r: ReviseTaskRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError>;
    async fn get_task(&self, r: GetTaskRequest) -> Result<Option<TaskRevision>, AppError>;
    async fn list_tasks(&self, r: ListTasksRequest) -> Result<Vec<TaskRevision>, AppError>;
    async fn record_task_checkpoint(
        &self,
        r: RecordTaskCheckpointRequest,
    ) -> Result<TaskWriteResult<TaskCheckpoint>, AppError>;
    /// One SQLite read snapshot for the task head, requirements and latest checkpoint.
    async fn get_task_snapshot(
        &self,
        namespace: &str,
        task_id: &str,
    ) -> Result<Option<TaskSnapshot>, AppError>;
}
