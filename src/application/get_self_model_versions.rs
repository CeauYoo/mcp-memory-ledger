use serde::Serialize;

use crate::{
    domain::types::{MemoryScope, Namespace, Owner},
    error::AppError,
    ports::self_model_version_store::{
        MAX_SELF_MODEL_VERSION_LIMIT, ScopedSelfModelVersionRecord, SelfModelVersionQuery,
        SelfModelVersionStore,
    },
};

pub const DEFAULT_SELF_MODEL_VERSION_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetSelfModelVersionsInput {
    pub namespace: Namespace,
    pub allow_global_version_metadata: bool,
    pub limit: usize,
    pub before_version: Option<u64>,
}

impl GetSelfModelVersionsInput {
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.allow_global_version_metadata {
            return Err(AppError::InvalidParams(
                "get_self_model_versions requires allow_global_version_metadata: true; global version counters reveal activity outside the requested namespace and are experimental single-user metadata, not authorization".into(),
            ));
        }
        if !(1..=MAX_SELF_MODEL_VERSION_LIMIT).contains(&self.limit) {
            return Err(AppError::InvalidParams(format!(
                "get_self_model_versions limit must be between 1 and {MAX_SELF_MODEL_VERSION_LIMIT}"
            )));
        }
        if self
            .before_version
            .is_some_and(|version| version > i64::MAX as u64)
        {
            return Err(AppError::InvalidParams(
                "before_version exceeds the supported version range".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GetSelfModelVersionsResult {
    pub owner: Owner,
    pub namespace: String,
    pub current_version: u64,
    pub limit: usize,
    pub has_more: bool,
    pub next_before_version: Option<u64>,
    /// Explain why this read is unsuitable as a tenant-isolation boundary.
    pub global_metadata_notice: &'static str,
    pub records: Vec<ScopedSelfModelVersionRecord>,
}

pub async fn execute<D>(
    deps: &D,
    input: GetSelfModelVersionsInput,
) -> Result<GetSelfModelVersionsResult, AppError>
where
    D: SelfModelVersionStore + Sync,
{
    input.validate()?;
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    let owner = scope.owner().expect("namespace-derived scope has an owner");
    let page = deps
        .query_self_model_versions(SelfModelVersionQuery {
            scope,
            limit: input.limit,
            before_version: input.before_version,
        })
        .await?;
    let next_before_version = if page.has_more {
        page.records.last().map(|record| record.version)
    } else {
        None
    };
    Ok(GetSelfModelVersionsResult {
        owner,
        namespace: input.namespace.as_str().to_owned(),
        current_version: page.current_version,
        limit: input.limit,
        has_more: page.has_more,
        next_before_version,
        global_metadata_notice: "Global version counters may reveal activity in other namespaces. Experimental single-user metadata; namespace attribution is not authorization. Only verified same-scope written patches are included; inherited state is omitted; previous values require independently verified same-scope provenance.",
        records: page.records,
    })
}
