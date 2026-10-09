//! Reflection attribution is audit metadata, never permission to read or mutate.
//! Global identity and commitment effects are represented independently from their source.
use crate::{
    domain::types::{MemoryScope, Namespace, Owner},
    error::AppError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionScopeStatus {
    #[default]
    Unknown,
    Verified,
    LegacyUnambiguous,
}

impl ReflectionScopeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Verified => "verified",
            Self::LegacyUnambiguous => "legacy_unambiguous",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ReflectionScopeMetadata {
    pub status: ReflectionScopeStatus,
    pub origin_scopes: Vec<MemoryScope>,
    pub affected_scopes: Vec<MemoryScope>,
}

/// Unknown-owner legacy records must not acquire a known scope from namespace alone.
pub fn known_scope(owner: Owner, namespace: &Namespace) -> Option<MemoryScope> {
    let scope = MemoryScope::for_namespace(namespace.clone());
    (scope.owner() == Some(owner)).then_some(scope)
}

impl ReflectionScopeMetadata {
    /// Resolve only from sources loaded in the existing write transaction. Explicit
    /// attribution narrows source validation; it does not authorize any global update.
    pub fn resolve(
        explicit_origin: Option<MemoryScope>,
        target: Option<MemoryScope>,
        evidence: &[Option<MemoryScope>],
        changes_global_model: bool,
    ) -> Result<Self, AppError> {
        let mut sources = Vec::new();
        if let Some(scope) = &target {
            push_unique(&mut sources, scope.clone());
        }
        for scope in evidence.iter().flatten() {
            push_unique(&mut sources, scope.clone());
        }
        let unknown_source = evidence.iter().any(Option::is_none);
        let mut affected_scopes = target.clone().into_iter().collect::<Vec<_>>();
        if changes_global_model {
            push_unique(&mut affected_scopes, MemoryScope::self_());
        }
        let explicit = explicit_origin.is_some();
        if let Some(scope) = explicit_origin
            && (!scope.is_explicitly_scoped()
                || sources.is_empty()
                || unknown_source
                || sources.iter().any(|source| source != &scope))
        {
            return Err(AppError::InvalidParams(
                "reflection origin scope requires existing same-scope target or evidence sources"
                    .into(),
            ));
        }
        // Legacy targetless calls preserve their mutation behavior, but cannot gain
        // read visibility simply by requesting a global identity/commitment patch.
        let verified = !unknown_source && sources.len() == 1 && (target.is_some() || explicit);
        Ok(Self {
            status: if verified {
                ReflectionScopeStatus::Verified
            } else {
                ReflectionScopeStatus::Unknown
            },
            origin_scopes: if verified { sources } else { Vec::new() },
            affected_scopes,
        })
    }

    /// Full targetless payloads may be read only when every source and effect fits
    /// the requested scope. Affected scopes never form an alternate read route.
    pub fn permits_targetless_read(&self, requested: &MemoryScope) -> bool {
        self.status != ReflectionScopeStatus::Unknown
            && requested.is_explicitly_scoped()
            && self.origin_scopes.as_slice() == std::slice::from_ref(requested)
            && self.affected_scopes.iter().all(|scope| scope == requested)
    }
}

pub(crate) fn push_unique(scopes: &mut Vec<MemoryScope>, scope: MemoryScope) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_and_effect_are_independent_and_effect_never_grants_visibility() {
        let project = MemoryScope::for_namespace(Namespace::for_project("a"));
        let metadata = ReflectionScopeMetadata::resolve(
            Some(project.clone()),
            None,
            &[Some(project.clone())],
            true,
        )
        .unwrap();
        assert_eq!(metadata.origin_scopes, vec![project.clone()]);
        assert_eq!(metadata.affected_scopes, vec![MemoryScope::self_()]);
        assert!(!metadata.permits_targetless_read(&project));
        assert!(!metadata.permits_targetless_read(&MemoryScope::self_()));
    }
    #[test]
    fn no_source_or_unknown_owner_cannot_establish_explicit_origin() {
        let project = MemoryScope::for_namespace(Namespace::for_project("a"));
        assert!(ReflectionScopeMetadata::resolve(Some(project.clone()), None, &[], false).is_err());
        assert!(
            ReflectionScopeMetadata::resolve(Some(project.clone()), None, &[None], false).is_err()
        );
        assert_eq!(
            known_scope(Owner::Unknown, project.namespace().unwrap()),
            None
        );
    }
    #[test]
    fn mixed_legacy_sources_remain_unknown_without_changing_authority() {
        let metadata = ReflectionScopeMetadata::resolve(
            None,
            Some(MemoryScope::self_()),
            &[Some(MemoryScope::for_namespace(Namespace::world()))],
            true,
        )
        .unwrap();
        assert_eq!(metadata.status, ReflectionScopeStatus::Unknown);
        assert!(metadata.origin_scopes.is_empty());
        assert_eq!(metadata.affected_scopes, vec![MemoryScope::self_()]);
    }
}
