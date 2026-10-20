//! Authoritative, typed knowledge supplied to selection. Procedure text is inert
//! knowledge, not an instruction to execute or an authorization to act.
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    application::{
        recall_memory_enhanced::{content_digest, record_reference},
        search_memory::SearchMemoryRecord,
    },
    domain::experience::{ExperienceCandidate, PersistedEpisode},
    error::AppError,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "source_type", content = "record", rename_all = "snake_case")]
pub enum SelectionSource {
    Memory(SearchMemoryRecord),
    Episode(PersistedEpisode),
    Experience(ExperienceCandidate),
}

impl SelectionSource {
    pub fn reference(&self) -> String {
        match self {
            Self::Memory(record) => record_reference(record).to_owned(),
            Self::Episode(record) => format!("experience_episode:{}", record.episode_id),
            Self::Experience(record) => format!("experience_candidate:{}", record.candidate_id),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Memory(SearchMemoryRecord::Claim { .. }) => "claim",
            Self::Memory(SearchMemoryRecord::Event { .. }) => "event",
            Self::Memory(SearchMemoryRecord::Episode { .. }) => "episode",
            Self::Memory(SearchMemoryRecord::Reflection { .. }) => "reflection",
            Self::Episode(_) => "persisted_episode",
            Self::Experience(_) => "experience_candidate",
        }
    }

    pub fn version(&self) -> Option<i64> {
        match self {
            Self::Experience(record) => Some(record.version),
            _ => None,
        }
    }

    pub fn content_digest(&self) -> Result<String, AppError> {
        // Keep historical memory identities byte-for-byte compatible with the
        // existing enhanced-recall cache and applicability declarations.
        if let Self::Memory(record) = self {
            return content_digest(record);
        }
        let mut digest = Sha256::new();
        digest.update(b"selection_source:v1\0");
        digest.update(
            serde_json::to_vec(self).map_err(|error| AppError::Message(error.to_string()))?,
        );
        Ok(format!("{:x}", digest.finalize()))
    }

    pub fn embedding_text(&self) -> String {
        match self {
            Self::Memory(SearchMemoryRecord::Claim {
                subject,
                predicate,
                object,
                ..
            }) => format!("{subject}\n{predicate}\n{object}"),
            Self::Memory(SearchMemoryRecord::Event { summary, .. }) => summary.clone(),
            Self::Memory(_) => String::new(),
            Self::Episode(record) => {
                let content = &record.content;
                std::iter::once(content.title.as_str())
                    .chain(std::iter::once(content.objective.as_str()))
                    .chain(content.actions.iter().map(String::as_str))
                    .chain(content.observations.iter().map(String::as_str))
                    .chain(std::iter::once(content.outcome.as_str()))
                    .chain(std::iter::once(content.lesson.as_str()))
                    .chain(content.limitations.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Self::Experience(record) => record.content.search_text(),
        }
    }
}
