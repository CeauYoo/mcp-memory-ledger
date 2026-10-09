use crate::{
    domain::{feedback_candidate::FeedbackCandidate, types::Namespace},
    error::AppError,
};
use async_trait::async_trait;

#[async_trait]
pub trait FeedbackCandidateStore {
    async fn get_feedback_candidate(
        &self,
        namespace: &Namespace,
        candidate_id: &str,
    ) -> Result<Option<FeedbackCandidate>, AppError>;
}
