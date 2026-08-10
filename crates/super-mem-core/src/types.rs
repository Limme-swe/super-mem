//! Public domain and request/response types.

use std::{collections::BTreeMap, fmt};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Copy,
            Debug,
            Eq,
            Hash,
            JsonSchema,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Creates a time-sortable `UUIDv7` identifier.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(EventId, "A stable event identifier.");
uuid_id!(MemoryId, "A stable logical memory identifier.");
uuid_id!(QueryId, "A stable recall query identifier.");
uuid_id!(LinkId, "A stable memory-link identifier.");

/// The desired `SQLite` commit durability.
#[derive(Clone, Copy, Debug, Default, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Durability {
    /// WAL plus `synchronous=NORMAL`; safe across process crashes, but the most
    /// recent commit can be lost after power loss.
    #[default]
    Balanced,
    /// WAL plus `synchronous=FULL`; every acknowledged commit is fsynced.
    Durable,
}

/// Engine configuration that does not affect the public database format.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineOptions {
    /// Commit durability.
    pub durability: Durability,
    /// Busy timeout used when another process owns `SQLite`'s writer lock.
    pub busy_timeout_ms: u64,
    /// Default maximum number of recalled memories.
    pub default_recall_limit: usize,
    /// Default context budget.
    pub default_token_budget: usize,
    /// Whether secret-shaped strings are redacted before persistence.
    pub redact_secrets: bool,
    /// Maximum accepted UTF-8 bytes for one text field.
    pub max_text_bytes: usize,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            durability: Durability::Balanced,
            busy_timeout_ms: 5_000,
            default_recall_limit: 12,
            default_token_budget: 1_500,
            redact_secrets: true,
            max_text_bytes: 1_048_576,
        }
    }
}

/// Repository state supplied by a harness.
#[derive(Clone, Debug, Default, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepositoryContext {
    /// Stable repository identity chosen by the adapter.
    pub repo_id: String,
    /// Optional normalized local root, used only as metadata.
    pub root: Option<String>,
    /// Git common directory, which differs from the worktree for linked
    /// worktrees and submodules.
    pub common_dir: Option<String>,
    /// Current branch name.
    pub branch: Option<String>,
    /// Current commit identifier.
    pub head_oid: Option<String>,
    /// Normalized remote URL or its caller-provided opaque identity.
    pub remote: Option<String>,
    /// Stable fingerprint of tracked and untracked worktree changes.
    pub dirty_hash: Option<String>,
}

/// Relationship between a memory's commit and the current commit.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "relation")]
pub enum GitRelation {
    /// Both identifiers resolve to the same commit.
    Same,
    /// The stored commit is an ancestor of the current commit.
    Ancestor {
        /// Commits by which current is ahead.
        behind: u32,
    },
    /// The current commit is an ancestor of the stored commit.
    Descendant {
        /// Commits by which the stored commit is ahead.
        ahead: u32,
    },
    /// Neither commit is an ancestor of the other.
    Diverged {
        /// Commits exclusive to the stored side.
        ahead: u32,
        /// Commits exclusive to the current side.
        behind: u32,
    },
    /// Git was unavailable or one of the commits could not be resolved.
    Unknown,
}

/// Hierarchical scope used to isolate memories.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    /// Isolation boundary. Different namespaces are never mixed.
    pub namespace: String,
    /// Optional workspace identity.
    pub workspace_id: Option<String>,
    /// Optional repository state.
    pub repository: Option<RepositoryContext>,
    /// Optional agent task/session identity.
    pub session_id: Option<String>,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            namespace: "default".to_owned(),
            workspace_id: None,
            repository: None,
            session_id: None,
        }
    }
}

impl Scope {
    /// Returns the legacy stable repository/branch or workspace digest.
    ///
    /// For repository scopes, callers must also compare `workspace_id`: it is
    /// an independent isolation boundary kept outside this digest so existing
    /// database and snapshot identities remain compatible.
    pub(crate) fn key(&self) -> String {
        // Session, paths, remotes, commit IDs, and dirty-worktree hashes are
        // provenance/freshness metadata rather than durable identity.
        let encoded = if let Some(repository) = &self.repository {
            serde_json::to_vec(&(
                &self.namespace,
                "repository",
                &repository.repo_id,
                &repository.branch,
            ))
        } else {
            serde_json::to_vec(&(&self.namespace, "workspace", &self.workspace_id))
        }
        .expect("Scope serialization is infallible");
        blake3::hash(&encoded).to_hex().to_string()
    }

    /// Returns the repository identifier, if present.
    pub fn repo_id(&self) -> Option<&str> {
        self.repository.as_ref().map(|repo| repo.repo_id.as_str())
    }

    /// Returns the branch, if present.
    pub fn branch(&self) -> Option<&str> {
        self.repository
            .as_ref()
            .and_then(|repo| repo.branch.as_deref())
    }
}

/// A typed durable memory.
#[derive(Clone, Copy, Debug, Eq, Hash, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// A potentially verifiable proposition.
    Fact,
    /// A user or team preference.
    Preference,
    /// A requirement or invariant.
    Constraint,
    /// A choice and its rationale.
    Decision,
    /// A reusable method.
    Procedure,
    /// A task or session summary.
    Episode,
    /// A successful, failed, or partial attempt.
    Outcome,
    /// Work that remains to be done.
    Task,
    /// A low-level observation promoted from the event stream.
    Observation,
}

impl MemoryKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Constraint => "constraint",
            Self::Decision => "decision",
            Self::Procedure => "procedure",
            Self::Episode => "episode",
            Self::Outcome => "outcome",
            Self::Task => "task",
            Self::Observation => "observation",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "fact" => Self::Fact,
            "preference" => Self::Preference,
            "constraint" => Self::Constraint,
            "decision" => Self::Decision,
            "procedure" => Self::Procedure,
            "episode" => Self::Episode,
            "outcome" => Self::Outcome,
            "task" => Self::Task,
            "observation" => Self::Observation,
            _ => return None,
        })
    }
}

/// Lifecycle state of a memory head.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryState {
    /// Current and usable.
    Active,
    /// Conflicting evidence exists.
    Contested,
    /// Replaced by a newer record.
    Superseded,
    /// Explicitly removed from normal retrieval.
    Retracted,
}

impl MemoryState {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "active" => Self::Active,
            "contested" => Self::Contested,
            "superseded" => Self::Superseded,
            "retracted" => Self::Retracted,
            _ => return None,
        })
    }
}

/// Provenance trust used as a ranking guard, never as proof of truth.
#[derive(Clone, Copy, Debug, Default, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Imported or otherwise unverified content.
    External,
    /// A model or agent proposed the content.
    #[default]
    Agent,
    /// A tool result directly supports the content.
    ToolVerified,
    /// A user explicitly authored or confirmed the content.
    UserConfirmed,
}

impl TrustLevel {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Agent => "agent",
            Self::ToolVerified => "tool_verified",
            Self::UserConfirmed => "user_confirmed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "external" => Self::External,
            "agent" => Self::Agent,
            "tool_verified" => Self::ToolVerified,
            "user_confirmed" => Self::UserConfirmed,
            _ => return None,
        })
    }

    pub(crate) const fn factor(self) -> f64 {
        match self {
            Self::External => 0.60,
            Self::Agent => 0.80,
            Self::ToolVerified => 0.95,
            Self::UserConfirmed => 1.00,
        }
    }
}

/// Kind of immutable source event.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// User or assistant conversational text.
    ConversationTurn,
    /// A tool invocation.
    ToolCall,
    /// A tool's result.
    ToolResult,
    /// A shell command and its outcome.
    CommandResult,
    /// A file was created, changed, or removed.
    FileChange,
    /// A test or build outcome.
    Verification,
    /// An explicitly authored memory.
    ExplicitMemory,
    /// A structured end-of-task checkpoint.
    Checkpoint,
    /// A human-authored note.
    ManualNote,
    /// A lifecycle action such as retraction.
    Lifecycle,
}

impl EventKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ConversationTurn => "conversation_turn",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::CommandResult => "command_result",
            Self::FileChange => "file_change",
            Self::Verification => "verification",
            Self::ExplicitMemory => "explicit_memory",
            Self::Checkpoint => "checkpoint",
            Self::ManualNote => "manual_note",
            Self::Lifecycle => "lifecycle",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "conversation_turn" => Self::ConversationTurn,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "command_result" => Self::CommandResult,
            "file_change" => Self::FileChange,
            "verification" => Self::Verification,
            "explicit_memory" => Self::ExplicitMemory,
            "checkpoint" => Self::Checkpoint,
            "manual_note" => Self::ManualNote,
            "lifecycle" => Self::Lifecycle,
            _ => return None,
        })
    }
}

/// An artifact referenced by a memory or current query.
#[derive(Clone, Debug, Default, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtifactRef {
    /// Stable repository identity.
    pub repo_id: String,
    /// Repository-relative path.
    pub path: String,
    /// Optional language-level symbol.
    pub symbol: Option<String>,
    /// Optional BLAKE3 or harness-supplied content fingerprint.
    pub content_hash: Option<String>,
    /// Optional Git object identifier.
    pub git_oid: Option<String>,
    /// Optional language label.
    pub language: Option<String>,
}

/// Named entity attached to a memory.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct EntityRef {
    /// Entity category, for example `crate`, `symbol`, or `person`.
    pub kind: String,
    /// Canonical normalized identity.
    pub canonical: String,
    /// Human-readable display value.
    pub display: String,
}

/// Evidence span in an immutable event.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    /// Source event.
    pub event_id: EventId,
    /// Optional UTF-8 byte start.
    pub span_start: Option<usize>,
    /// Optional exclusive UTF-8 byte end.
    pub span_end: Option<usize>,
    /// Relationship such as `supports` or `derived_from`.
    pub relation: String,
}

/// A directed link from the memory being written to another memory.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct LinkInput {
    /// Target memory.
    pub target: MemoryId,
    /// Typed relationship.
    pub relation: String,
    /// Integer weight in `[0, 1000]`.
    pub weight: u16,
}

/// Fully materialized immutable source event.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct Event {
    /// Database sequence.
    pub seq: i64,
    /// Stable event ID.
    pub event_id: EventId,
    /// Event category.
    pub kind: EventKind,
    /// Event scope.
    pub scope: Scope,
    /// Redacted source text.
    pub content: String,
    /// Structured redacted metadata.
    pub attributes: BTreeMap<String, Value>,
    /// Source trust.
    pub trust: TrustLevel,
    /// Time reported by the source.
    pub occurred_at: DateTime<Utc>,
    /// Time committed by Super-mem.
    pub ingested_at: DateTime<Utc>,
    /// Number of redacted secret-shaped values.
    pub redaction_count: usize,
}

/// Current view of a logical memory.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct Memory {
    /// Stable logical ID.
    pub memory_id: MemoryId,
    /// Current revision number.
    pub revision: u32,
    /// Memory kind.
    pub kind: MemoryKind,
    /// Lifecycle state.
    pub state: MemoryState,
    /// Hierarchical scope.
    pub scope: Scope,
    /// Optional stable semantic key used for revision upserts.
    pub canonical_key: Option<String>,
    /// Short label.
    pub title: String,
    /// Grounded memory content.
    pub body: String,
    /// Importance in `[0, 1]`.
    pub importance: f32,
    /// Confidence in `[0, 1]`.
    pub confidence: f32,
    /// Source trust.
    pub trust: TrustLevel,
    /// When the claim began to apply.
    pub valid_from: Option<DateTime<Utc>>,
    /// When the claim stopped applying.
    pub valid_until: Option<DateTime<Utc>>,
    /// Explicit expiration time.
    pub expires_at: Option<DateTime<Utc>>,
    /// First database record time.
    pub created_at: DateTime<Utc>,
    /// Current revision record time.
    pub updated_at: DateTime<Utc>,
    /// Structured extension data.
    pub attributes: BTreeMap<String, Value>,
    /// Search tags.
    pub tags: Vec<String>,
    /// Named entities.
    pub entities: Vec<EntityRef>,
    /// Referenced code artifacts.
    pub artifacts: Vec<ArtifactRef>,
    /// Grounding evidence.
    pub evidence: Vec<EvidenceRef>,
}

/// One immutable link as it existed on a specific source revision.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct MemoryLink {
    /// Stable link ID.
    pub link_id: LinkId,
    /// Source memory.
    pub source_memory_id: MemoryId,
    /// Source revision that recorded this link.
    pub source_revision: u32,
    /// Target memory.
    pub target_memory_id: MemoryId,
    /// Typed relationship.
    pub relation: String,
    /// Integer weight in `[0, 1000]`.
    pub weight: u16,
    /// Event that created this revision of the link.
    pub created_event_id: EventId,
    /// Commit time.
    pub created_at: DateTime<Utc>,
}

/// One immutable memory revision and the fidelity of its historical metadata.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct MemoryRevision {
    /// Materialized revision content and metadata.
    pub memory: Memory,
    /// Whether ranking, validity, trust, and lifecycle metadata were captured
    /// at write time. Older v1-v3 stores can recover revision text exactly but
    /// only know current-head metadata for pre-migration revisions.
    pub metadata_complete: bool,
}

/// Inspectable provenance ledger for one logical memory.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct MemoryHistory {
    /// Current materialized head.
    pub current: Memory,
    /// Every immutable revision, oldest first.
    pub revisions: Vec<MemoryRevision>,
    /// Immutable source and lifecycle events associated with the memory.
    pub events: Vec<Event>,
    /// Immutable links with this memory as source or target.
    pub links: Vec<MemoryLink>,
    /// Retrieval feedback in database order.
    pub feedback: Vec<MemoryFeedback>,
}

/// An event ingestion request.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
#[serde(default)]
pub struct ObserveRequest {
    /// Caller-selected event ID, or a generated `UUIDv7`.
    pub event_id: Option<EventId>,
    /// Caller idempotency key within the scope namespace.
    pub idempotency_key: Option<String>,
    /// Event category.
    pub kind: EventKind,
    /// Scope.
    pub scope: Scope,
    /// Source content.
    pub content: String,
    /// Structured attributes.
    pub attributes: BTreeMap<String, Value>,
    /// Provenance trust.
    pub trust: TrustLevel,
    /// Source time.
    pub occurred_at: Option<DateTime<Utc>>,
}

impl Default for ObserveRequest {
    fn default() -> Self {
        Self {
            event_id: None,
            idempotency_key: None,
            kind: EventKind::ManualNote,
            scope: Scope::default(),
            content: String::new(),
            attributes: BTreeMap::new(),
            trust: TrustLevel::Agent,
            occurred_at: None,
        }
    }
}

/// Receipt for an observed event.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ObserveReceipt {
    /// Event ID.
    pub event_id: EventId,
    /// Monotonic database sequence.
    pub database_seq: i64,
    /// Whether an existing idempotent result was returned.
    pub deduplicated: bool,
    /// Commit durability.
    pub durability: Durability,
    /// Redacted values.
    pub redaction_count: usize,
}

/// Request to create a memory or revise an existing memory.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
#[serde(default)]
pub struct RememberRequest {
    /// Existing logical ID to revise.
    pub memory_id: Option<MemoryId>,
    /// Caller idempotency key within the namespace.
    pub idempotency_key: Option<String>,
    /// Type of memory.
    pub kind: MemoryKind,
    /// Scope.
    pub scope: Scope,
    /// Optional stable key. An active memory with the same scope, kind, and key
    /// is revised instead of duplicated.
    pub canonical_key: Option<String>,
    /// Short label.
    pub title: String,
    /// Memory content.
    pub body: String,
    /// Importance in `[0, 1]`.
    pub importance: f32,
    /// Confidence in `[0, 1]`.
    pub confidence: f32,
    /// Provenance trust.
    pub trust: TrustLevel,
    /// Reality-validity start.
    pub valid_from: Option<DateTime<Utc>>,
    /// Reality-validity end.
    pub valid_until: Option<DateTime<Utc>>,
    /// Explicit expiration.
    pub expires_at: Option<DateTime<Utc>>,
    /// Structured extension data.
    pub attributes: BTreeMap<String, Value>,
    /// Search tags.
    pub tags: Vec<String>,
    /// Named entities.
    pub entities: Vec<EntityRef>,
    /// Referenced artifacts.
    pub artifacts: Vec<ArtifactRef>,
    /// Existing source evidence in addition to the write event itself.
    pub evidence: Vec<EvidenceRef>,
    /// Links to existing memories.
    pub links: Vec<LinkInput>,
}

impl Default for RememberRequest {
    fn default() -> Self {
        Self {
            memory_id: None,
            idempotency_key: None,
            kind: MemoryKind::Fact,
            scope: Scope::default(),
            canonical_key: None,
            title: String::new(),
            body: String::new(),
            importance: 0.5,
            confidence: 0.7,
            trust: TrustLevel::Agent,
            valid_from: None,
            valid_until: None,
            expires_at: None,
            attributes: BTreeMap::new(),
            tags: Vec::new(),
            entities: Vec::new(),
            artifacts: Vec::new(),
            evidence: Vec::new(),
            links: Vec::new(),
        }
    }
}

/// Receipt for a durable memory write.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct WriteReceipt {
    /// Event grounding the write.
    pub event_id: EventId,
    /// Created or revised memories.
    pub memory_ids: Vec<MemoryId>,
    /// Monotonic database sequence.
    pub database_seq: i64,
    /// FTS is transactionally current through this sequence.
    pub lexical_index_seq: i64,
    /// Whether an existing idempotent result was returned.
    pub deduplicated: bool,
    /// Commit durability.
    pub durability: Durability,
    /// Number of redacted values.
    pub redaction_count: usize,
}

/// Overall checkpoint outcome.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOutcome {
    /// Goal was achieved and verified.
    Success,
    /// Goal was not achieved.
    Failure,
    /// Some useful progress was made.
    Partial,
}

/// A decision captured at a checkpoint.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct CheckpointDecision {
    /// Decision summary.
    pub summary: String,
    /// Supporting rationale.
    pub rationale: Option<String>,
    /// Optional stable revision key.
    pub canonical_key: Option<String>,
}

/// One attempted approach and its result.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct CheckpointAttempt {
    /// Action attempted.
    pub action: String,
    /// Observed result.
    pub result: String,
    /// Whether this attempt succeeded.
    pub succeeded: bool,
    /// Optional normalized error or command fingerprint.
    pub fingerprint: Option<String>,
    /// Optional stable key used to revise a recurring automatic outcome.
    pub canonical_key: Option<String>,
    /// Why an automatically captured event was promoted into durable memory.
    pub promotion_reason: Option<String>,
}

/// Atomic task checkpoint request.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointRequest {
    /// Caller idempotency key.
    pub idempotency_key: Option<String>,
    /// Task scope.
    pub scope: Scope,
    /// Original goal.
    pub goal: String,
    /// Concise result summary.
    pub summary: String,
    /// Overall result.
    pub outcome: CheckpointOutcome,
    /// Commands, tests, or observations that verified the result.
    pub verification: Vec<String>,
    /// Decisions worth reusing.
    pub decisions: Vec<CheckpointDecision>,
    /// Successful and failed attempts.
    pub attempts: Vec<CheckpointAttempt>,
    /// Work intentionally left open.
    pub open_tasks: Vec<String>,
    /// Referenced code artifacts.
    pub artifacts: Vec<ArtifactRef>,
    /// Existing grounding events.
    pub evidence: Vec<EvidenceRef>,
    /// Source trust.
    pub trust: TrustLevel,
    /// Search tags.
    pub tags: Vec<String>,
}

impl Default for CheckpointRequest {
    fn default() -> Self {
        Self {
            idempotency_key: None,
            scope: Scope::default(),
            goal: String::new(),
            summary: String::new(),
            outcome: CheckpointOutcome::Partial,
            verification: Vec::new(),
            decisions: Vec::new(),
            attempts: Vec::new(),
            open_tasks: Vec::new(),
            artifacts: Vec::new(),
            evidence: Vec::new(),
            trust: TrustLevel::Agent,
            tags: Vec::new(),
        }
    }
}

/// Current artifact and error hints supplied during recall.
#[derive(Clone, Debug, Default, JsonSchema, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextHints {
    /// Known current artifact fingerprints.
    pub artifacts: Vec<ArtifactRef>,
    /// Current normalized error or command fingerprint.
    pub error_fingerprint: Option<String>,
    /// Explicit entity identities to boost.
    pub entities: Vec<String>,
    /// Optional caller-generated dense query vector. The core never invokes
    /// or downloads an embedding model.
    pub dense: Option<DenseQuery>,
}

/// A caller-generated vector for one immutable search profile.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct DenseQuery {
    /// Search profile whose model, preprocessing, and dimensions produced the vector.
    pub profile_id: String,
    /// Finite, non-zero floating-point vector.
    pub vector: Vec<f32>,
    /// Optional minimum cosine similarity for the dense candidate channel.
    pub min_similarity: Option<f32>,
}

/// Immutable registration for a background search encoder or expander.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct SearchProfileRegistration {
    /// Stable digest-derived identity for model, tokenizer, preprocessing, and metric.
    pub profile_id: String,
    /// Digest of the complete generator configuration.
    pub model_digest: String,
    /// Dense-vector dimensions, or `None` for document expansion only.
    pub dimensions: Option<usize>,
}

/// One registered background search profile.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct SearchProfile {
    /// Stable profile identity.
    pub profile_id: String,
    /// Digest of the complete generator configuration.
    pub model_digest: String,
    /// Dense-vector dimensions, if this profile supports dense retrieval.
    pub dimensions: Option<usize>,
    /// Core random-hyperplane signature algorithm version.
    pub signature_version: u32,
    /// Whether this profile may contribute retrieval candidates.
    pub active: bool,
    /// Registration time.
    pub created_at: DateTime<Utc>,
}

/// One current memory that still needs background search enrichment.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct PendingSearchDocument {
    /// Logical memory identity.
    pub memory_id: MemoryId,
    /// Current immutable revision.
    pub revision: u32,
    /// Canonical content hash used for compare-and-swap registration.
    pub content_hash: String,
    /// Current title.
    pub title: String,
    /// Current body.
    pub body: String,
    /// Current tags.
    pub tags: Vec<String>,
    /// Current named entities.
    pub entities: Vec<EntityRef>,
    /// Current code artifacts.
    pub artifacts: Vec<ArtifactRef>,
}

/// Derived search material for one current memory revision.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct SearchProjectionInput {
    /// Logical memory identity.
    pub memory_id: MemoryId,
    /// Revision that was encoded.
    pub revision: u32,
    /// Canonical content hash that was encoded.
    pub content_hash: String,
    /// Bounded likely queries, aliases, or semantic concepts generated off the write path.
    #[serde(default)]
    pub expansions: Vec<String>,
    /// Optional dense vector produced by the registered profile.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
}

/// Atomic batch registration of background search projections.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct RegisterSearchProjectionsRequest {
    /// Scope authorized to enrich these memories.
    pub scope: Scope,
    /// Immutable profile that generated every record.
    pub profile_id: String,
    /// Current-revision projections to register.
    pub projections: Vec<SearchProjectionInput>,
}

/// Result of registering a projection batch.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct SearchProjectionReceipt {
    /// Profile receiving the projections.
    pub profile_id: String,
    /// Projections inserted or replaced.
    pub registered: usize,
    /// Byte-identical projections that were already present.
    pub unchanged: usize,
    /// Latest canonical event sequence observed by the transaction.
    pub database_seq: i64,
}

/// Coverage for one search profile inside one authorized scope.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct SearchIndexStatus {
    /// Registered immutable profile.
    pub profile: SearchProfile,
    /// Non-retracted current memory heads in scope.
    pub eligible: u64,
    /// Current heads with a matching revision and content hash projection.
    pub indexed: u64,
    /// Current heads still awaiting registration.
    pub pending: u64,
    /// Stored projections whose source revision is no longer current.
    pub stale: u64,
}

/// Request for a compiled context pack.
#[derive(Clone, Debug, Default, JsonSchema, Serialize, Deserialize)]
#[serde(default)]
pub struct RecallRequest {
    /// Natural-language or code-oriented query.
    pub query: String,
    /// Current task scope.
    pub scope: Scope,
    /// Maximum final memories before token budgeting.
    pub limit: Option<usize>,
    /// Approximate token budget.
    pub token_budget: Option<usize>,
    /// Optional kind allow-list.
    pub kinds: Vec<MemoryKind>,
    /// Query historical validity at this time.
    pub as_of: Option<DateTime<Utc>>,
    /// Include artifact-stale memories.
    pub include_stale: bool,
    /// Include memories from a descendant or diverged Git history.
    pub include_divergent: bool,
    /// Include superseded memory heads.
    pub include_superseded: bool,
    /// Current repository and task hints.
    pub hints: ContextHints,
}

/// Scope and artifact applicability classification.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    /// Same repository state or a verified unchanged artifact.
    Exact,
    /// The memory comes from an ancestor or otherwise compatible repository state.
    Compatible,
    /// A referenced artifact has changed.
    Stale,
    /// The memory comes from a descendant or genuinely diverged history.
    Divergent,
    /// Neither side provides enough repository identity for versioned comparison.
    Unversioned,
    /// Different namespace or repository.
    Inapplicable,
}

impl Applicability {
    /// Stable lowercase wire and rendering label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Compatible => "compatible",
            Self::Stale => "stale",
            Self::Divergent => "divergent",
            Self::Unversioned => "unversioned",
            Self::Inapplicable => "inapplicable",
        }
    }

    /// Stable relevance multiplier used by deterministic ranking.
    pub const fn ranking_weight(self) -> f64 {
        match self {
            Self::Exact => 1.00,
            Self::Compatible => 0.90,
            Self::Unversioned => 0.75,
            Self::Divergent => 0.45,
            Self::Stale => 0.35,
            Self::Inapplicable => 0.0,
        }
    }
}

/// Retrieval signal emitted for explainability.
#[derive(Clone, Copy, Debug, Eq, Hash, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSignal {
    /// Literal phrase occurred.
    Exact,
    /// FTS lexical match.
    Lexical,
    /// All bounded lexical query terms matched.
    LexicalStrict,
    /// Deterministic code-identifier or coding-concept alias matched.
    CodeAlias,
    /// All bounded query terms matched deterministic code aliases.
    CodeAliasStrict,
    /// Background document expansion matched without query-time inference.
    SemanticExpansion,
    /// Caller-supplied dense vector matched a registered projection.
    DenseVector,
    /// Identifier, tag, or artifact token matched.
    Sparse,
    /// Entity identity matched.
    Entity,
    /// Candidate came from the recent-memory source.
    Recency,
    /// Current artifact hash verified the record.
    ArtifactVerified,
    /// Error fingerprint matched.
    ErrorFingerprint,
}

/// A ranked memory selected before context rendering.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct RecallHit {
    /// Materialized memory.
    pub memory: Memory,
    /// Deterministic fused relevance score.
    pub score: f64,
    /// Scope/artifact classification.
    pub applicability: Applicability,
    /// Candidate sources that contributed.
    pub signals: Vec<RetrievalSignal>,
    /// Stable concise explanations.
    pub reasons: Vec<String>,
}

/// One item in the final context pack.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ContextItem {
    /// Memory ID.
    pub memory_id: MemoryId,
    /// Memory revision.
    pub revision: u32,
    /// Title.
    pub title: String,
    /// Complete or safely truncated body.
    pub body: String,
    /// Deterministic score.
    pub score: f64,
    /// Applicability classification.
    pub applicability: Applicability,
    /// Explainable reason codes.
    pub reasons: Vec<String>,
    /// Approximate token contribution.
    pub estimated_tokens: usize,
    /// Source event IDs.
    pub citations: Vec<EventId>,
}

/// A semantically grouped part of a context pack.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ContextSection {
    /// Stable section name.
    pub name: String,
    /// Selected memories.
    pub items: Vec<ContextItem>,
}

/// Token-budgeted context returned to an agent.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ContextPack {
    /// Query correlation ID.
    pub query_id: QueryId,
    /// Latest committed database sequence observed by the query.
    pub database_seq: i64,
    /// Token budget requested.
    pub token_budget: usize,
    /// Conservative estimated tokens used.
    pub estimated_tokens: usize,
    /// Structured sections.
    pub sections: Vec<ContextSection>,
    /// Staleness, conflicts, or other caveats.
    pub warnings: Vec<String>,
    /// Ready-to-inject data-only text rendering.
    pub rendered: String,
    /// Full selected hits for callers that prefer structured ranking data.
    pub hits: Vec<RecallHit>,
}

/// Explicit retrieval feedback.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSignal {
    /// Used by the caller.
    Used,
    /// Improved the task.
    Helpful,
    /// Distracted or harmed the task.
    Harmful,
    /// Factually incorrect.
    Incorrect,
    /// No longer current.
    Outdated,
    /// Caller explicitly dismissed it.
    Dismissed,
}

impl FeedbackSignal {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Used => "used",
            Self::Helpful => "helpful",
            Self::Harmful => "harmful",
            Self::Incorrect => "incorrect",
            Self::Outdated => "outdated",
            Self::Dismissed => "dismissed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "used" => Self::Used,
            "helpful" => Self::Helpful,
            "harmful" => Self::Harmful,
            "incorrect" => Self::Incorrect,
            "outdated" => Self::Outdated,
            "dismissed" => Self::Dismissed,
            _ => return None,
        })
    }
}

/// One immutable retrieval-feedback record.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct MemoryFeedback {
    /// Monotonic feedback row ID.
    pub feedback_id: i64,
    /// Query that produced the memory, when supplied.
    pub query_id: Option<QueryId>,
    /// Memory that received the signal.
    pub memory_id: MemoryId,
    /// Recorded feedback signal.
    pub signal: FeedbackSignal,
    /// Optional redacted note.
    pub note: Option<String>,
    /// Commit time.
    pub created_at: DateTime<Utc>,
}

/// Feedback request; signals affect retrieval utility but never factual confidence.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct FeedbackRequest {
    /// Query that produced the memory, when known.
    pub query_id: Option<QueryId>,
    /// Memory receiving feedback.
    pub memory_id: MemoryId,
    /// Signal.
    pub signal: FeedbackSignal,
    /// Optional concise reason.
    pub note: Option<String>,
}

/// Request to remove a memory from ordinary retrieval.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct RetractRequest {
    /// Target memory.
    pub memory_id: MemoryId,
    /// Human-readable reason.
    pub reason: String,
    /// Optional idempotency key.
    pub idempotency_key: Option<String>,
}

/// Lightweight database health and size information.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct Status {
    /// Schema version.
    pub schema_version: u32,
    /// Latest event sequence.
    pub database_seq: i64,
    /// Total immutable events.
    pub events: u64,
    /// Active or contested memories.
    pub active_memories: u64,
    /// Superseded memories.
    pub superseded_memories: u64,
    /// Retracted memories.
    pub retracted_memories: u64,
    /// Current `SQLite` page storage in bytes, excluding WAL.
    pub database_bytes: u64,
    /// Configured durability.
    pub durability: Durability,
}

/// Result of restoring a full JSON Lines snapshot into an empty database.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ImportReceipt {
    /// Newly imported immutable events.
    pub events_imported: usize,
    /// Newly imported memory heads and their revision histories.
    pub memories_imported: usize,
    /// Newly imported directed links.
    pub links_imported: usize,
    /// Newly imported retrieval feedback records.
    pub feedback_imported: usize,
    /// Reserved for future merge modes; full-snapshot restore always reports zero.
    pub records_skipped: usize,
    /// Values redacted during import; full-snapshot restore rejects unsafe input and reports zero.
    pub redaction_count: usize,
    /// Latest database sequence after import.
    pub database_seq: i64,
}
