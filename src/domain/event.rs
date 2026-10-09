use crate::domain::{
    DomainError,
    feedback::FeedbackMetadata,
    types::{EventKind, Namespace, Owner},
};

pub const MAX_EVIDENCE_MANIFEST_ITEMS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventReference(String);

impl EventReference {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || value.chars().any(char::is_whitespace) {
            return Err(DomainError::InvalidEventReference);
        }

        let event_id = value.strip_prefix("event:").unwrap_or(&value);
        if event_id.is_empty() || event_id.starts_with("event:") {
            return Err(DomainError::InvalidEventReference);
        }

        Ok(Self(event_id.to_string()))
    }

    pub(crate) fn from_event_id(event_id: impl Into<String>) -> Self {
        Self(event_id.into())
    }

    pub fn event_id(&self) -> &str {
        &self.0
    }

    pub fn canonical(&self) -> String {
        format!("event:{}", self.event_id())
    }
}

impl serde::Serialize for EventReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> serde::Deserialize<'de> for EventReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::parse(value).map_err(|_| serde::de::Error::custom("invalid event reference"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    owner: Owner,
    namespace: Namespace,
    kind: EventKind,
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    feedback: Option<FeedbackMetadata>,
}

impl Event {
    pub fn new(owner: Owner, kind: EventKind, summary: impl Into<String>) -> Self {
        Self {
            owner,
            namespace: Namespace::for_owner(owner),
            kind,
            summary: summary.into(),
            feedback: None,
        }
    }

    pub fn new_with_namespace(
        owner: Owner,
        namespace: Namespace,
        kind: EventKind,
        summary: impl Into<String>,
    ) -> Result<Self, DomainError> {
        if !namespace.matches_owner(owner) {
            return Err(DomainError::NamespaceOwnerMismatch);
        }

        Ok(Self {
            owner,
            namespace,
            kind,
            summary: summary.into(),
            feedback: None,
        })
    }

    pub fn with_feedback(mut self, feedback: FeedbackMetadata) -> Result<Self, DomainError> {
        feedback.validate()?;
        self.feedback = Some(feedback);
        Ok(self)
    }

    pub fn feedback(&self) -> Option<&FeedbackMetadata> {
        self.feedback.as_ref()
    }

    pub fn owner(&self) -> Owner {
        self.owner
    }

    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}
