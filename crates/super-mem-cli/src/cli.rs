use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

/// Local, evidence-first memory for coding agents.
#[derive(Clone, Debug, Parser)]
#[command(name = "supermem", version, about)]
pub struct Cli {
    /// `SQLite` database path. Defaults to the platform user data directory.
    #[arg(long, global = true, env = "SUPER_MEM_DB")]
    pub(crate) db: Option<PathBuf>,

    /// Emit machine-readable JSON for ordinary CLI commands.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum Command {
    /// Initialize the database and print its status.
    Init,
    /// Write or revise a durable typed memory.
    Remember(RememberArgs),
    /// Append a source event without promoting it to durable memory.
    Observe(ObserveArgs),
    /// Atomically record an outcome, decisions, attempts, and open work.
    Checkpoint(CheckpointArgs),
    /// Compile relevant memories into a bounded context pack.
    Recall(RecallArgs),
    /// Inspect one memory by ID.
    Inspect(IdArgs),
    /// Record retrieval-quality feedback.
    Feedback(FeedbackArgs),
    /// Retract one memory from ordinary retrieval.
    Retract(RetractArgs),
    /// Print database health and size information.
    Status(StatusArgs),
    /// Manage optional background expansion and dense-vector search indexes.
    Index(IndexArgs),
    /// Diagnose the database and repository integration.
    Doctor(DoctorArgs),
    /// Export the canonical JSONL representation.
    Export(ExportArgs),
    /// Import canonical JSONL from a file or stdin.
    Import(ImportArgs),
    /// Permanently delete the local database on supported platforms. Never exposed over MCP.
    Purge(PurgeArgs),
    /// Process one Codex or Claude Code hook payload from stdin.
    Hook(HookArgs),
    /// Serve four compact tools over MCP stdio with launch-pinned isolation.
    Mcp(McpArgs),
}

#[derive(Clone, Debug, Default, Args)]
pub struct ScopeArgs {
    /// Hard isolation namespace.
    #[arg(long, env = "SUPER_MEM_NAMESPACE", default_value = "default")]
    pub namespace: String,
    /// Optional workspace identity.
    #[arg(long, env = "SUPER_MEM_WORKSPACE")]
    pub workspace: Option<String>,
    /// Explicit repository identity; otherwise discovered from --cwd.
    #[arg(long)]
    pub repo_id: Option<String>,
    /// Optional harness session identity.
    #[arg(long)]
    pub session: Option<String>,
    /// Working directory used for repository discovery.
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Override the discovered branch.
    #[arg(long)]
    pub branch: Option<String>,
    /// Override the discovered Git commit.
    #[arg(long)]
    pub head: Option<String>,
    /// Override the discovered remote identity.
    #[arg(long)]
    pub remote: Option<String>,
    /// Calling harness, retained as provenance but not an isolation boundary.
    #[arg(long)]
    pub harness: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum MemoryKindArg {
    #[default]
    Fact,
    Preference,
    Constraint,
    Decision,
    Procedure,
    Episode,
    Outcome,
    Task,
    Observation,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum TrustArg {
    External,
    #[default]
    Agent,
    ToolVerified,
    UserConfirmed,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum OutcomeArg {
    Success,
    Failure,
    #[default]
    Partial,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum ObserveKindArg {
    UserPrompt,
    AssistantFinal,
    ToolCall,
    ToolResult,
    CommandResult,
    FileChange,
    Verification,
    CompactSummary,
    SessionStart,
    SessionEnd,
    ManualNote,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum FeedbackArg {
    Used,
    Helpful,
    Harmful,
    Incorrect,
    Outdated,
    Dismissed,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum RecallFormat {
    #[default]
    Context,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "lower")]
pub enum HarnessArg {
    Codex,
    Claude,
}

#[derive(Clone, Debug, Args)]
pub struct RememberArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    /// Memory body. Prefer --body-stdin for sensitive or long content.
    #[arg(
        long,
        required_unless_present = "body_stdin",
        conflicts_with = "body_stdin"
    )]
    pub body: Option<String>,
    /// Read the memory body from stdin so it is not exposed in process listings.
    #[arg(long, conflicts_with = "body")]
    pub body_stdin: bool,
    /// Short label; derived from the first body line when omitted.
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    pub kind: MemoryKindArg,
    /// Stable key used to revise an existing memory instead of duplicating it.
    #[arg(long)]
    pub canonical_key: Option<String>,
    /// Caller idempotency key.
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long, default_value_t = 0.5)]
    pub importance: f32,
    #[arg(long, default_value_t = 0.7)]
    pub confidence: f32,
    #[arg(long, value_enum, default_value_t)]
    pub trust: TrustArg,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Repository-relative file to fingerprint as applicability evidence. Repeatable.
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct ObserveArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long, value_enum)]
    pub kind: ObserveKindArg,
    /// Event content. Prefer --content-stdin for prompts, source, or secrets.
    #[arg(
        long,
        required_unless_present = "content_stdin",
        conflicts_with = "content_stdin"
    )]
    pub content: Option<String>,
    /// Read event content from stdin so it is not exposed in process listings.
    #[arg(long, conflicts_with = "content")]
    pub content_stdin: bool,
    /// Stable host event/message ID used for idempotent retries.
    #[arg(long)]
    pub event_id: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long, value_enum)]
    pub trust: Option<TrustArg>,
    /// Harness tool name associated with a tool, command, or file result.
    #[arg(long)]
    pub tool_name: Option<String>,
    /// Whether the harness reported the tool execution as successful.
    #[arg(long)]
    pub succeeded: Option<bool>,
    /// Mark this event as the result of a verification command.
    #[arg(long, action = ArgAction::Set, default_value_t = false)]
    pub verification: bool,
    /// Stable, non-sensitive fingerprint for a failed execution.
    #[arg(long)]
    pub error_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct CheckpointArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long, default_value = "coding task")]
    pub goal: String,
    /// Checkpoint summary. Prefer --summary-stdin for sensitive or long content.
    #[arg(
        long,
        required_unless_present = "summary_stdin",
        conflicts_with = "summary_stdin"
    )]
    pub summary: Option<String>,
    /// Read the checkpoint summary from stdin.
    #[arg(long, conflicts_with = "summary")]
    pub summary_stdin: bool,
    #[arg(long, value_enum, default_value_t)]
    pub outcome: OutcomeArg,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub verification: Vec<String>,
    #[arg(long)]
    pub open_task: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    #[arg(long, value_enum, default_value_t)]
    pub trust: TrustArg,
    /// Repository-relative file to fingerprint. Repeatable.
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    /// Do not automatically fingerprint changed Git files.
    #[arg(long)]
    pub no_auto_artifacts: bool,
}

#[derive(Clone, Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct RecallArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    /// Recall query. Prefer --query-stdin when it contains source or secrets.
    #[arg(
        long,
        required_unless_present = "query_stdin",
        conflicts_with = "query_stdin"
    )]
    pub query: Option<String>,
    /// Read the recall query from stdin so it is not exposed in process listings.
    #[arg(long, conflicts_with = "query")]
    pub query_stdin: bool,
    /// Observe this query as a user prompt before recall in the same process.
    #[arg(long)]
    pub observe_prompt: bool,
    /// Stable host message ID for idempotent --observe-prompt retries.
    #[arg(long, requires = "observe_prompt")]
    pub event_id: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, default_value_t = 1500)]
    pub token_budget: usize,
    #[arg(long, value_enum, default_value_t)]
    pub format: RecallFormat,
    #[arg(long)]
    pub include_stale: bool,
    /// Include memories from descendant or diverged Git history.
    #[arg(long)]
    pub include_divergent: bool,
    #[arg(long)]
    pub include_superseded: bool,
    /// Repository-relative file whose current fingerprint should guide recall. Repeatable.
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    /// Registered profile that produced --dense-vector-file.
    #[arg(long, requires = "dense_vector_file")]
    pub dense_profile: Option<String>,
    /// JSON array containing a caller-generated query vector.
    #[arg(long, requires = "dense_profile")]
    pub dense_vector_file: Option<PathBuf>,
    /// Optional minimum cosine similarity for dense candidates.
    #[arg(long, requires = "dense_profile")]
    pub dense_min_similarity: Option<f32>,
}

#[derive(Clone, Debug, Args)]
pub struct IdArgs {
    /// Memory UUID.
    pub memory_id: String,
    /// Include every revision, cited event, and historical link.
    #[arg(long)]
    pub history: bool,
}

#[derive(Clone, Debug, Args)]
pub struct FeedbackArgs {
    pub memory_id: String,
    #[arg(long, value_enum)]
    pub signal: FeedbackArg,
    #[arg(long)]
    pub query_id: Option<String>,
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct RetractArgs {
    pub memory_id: String,
    #[arg(long)]
    pub reason: String,
    #[arg(long)]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Default, Args)]
pub struct StatusArgs {
    /// Perform the health check without printing output.
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Clone, Debug, Args)]
pub struct IndexArgs {
    #[command(subcommand)]
    pub command: IndexCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub enum IndexCommand {
    /// Register one immutable generator/model profile.
    AddProfile(IndexAddProfileArgs),
    /// List registered profiles and whether they are active.
    ListProfiles,
    /// Allow a profile to contribute retrieval candidates.
    Activate(IndexProfileArgs),
    /// Keep a profile's data but exclude it from retrieval.
    Deactivate(IndexProfileArgs),
    /// Remove a profile and all of its rebuildable projections.
    RemoveProfile(IndexRemoveProfileArgs),
    /// List current memories missing a projection for a profile.
    Pending(IndexPendingArgs),
    /// Register a JSON or JSONL batch generated outside the write path.
    Register(IndexRegisterArgs),
    /// Show profile coverage in one exact namespace/workspace/repository scope.
    Status(IndexStatusArgs),
    /// Rebuild deterministic aliases and FTS from canonical rows.
    Rebuild,
}

#[derive(Clone, Debug, Args)]
pub struct IndexAddProfileArgs {
    /// Digest-derived profile identity.
    #[arg(long)]
    pub profile_id: String,
    /// Digest covering weights, tokenizer, preprocessing, dimensions, and metric.
    #[arg(long)]
    pub model_digest: String,
    /// Dense dimensions. Omit for document-expansion-only profiles.
    #[arg(long)]
    pub dimensions: Option<usize>,
}

#[derive(Clone, Debug, Args)]
pub struct IndexProfileArgs {
    #[arg(long)]
    pub profile_id: String,
}

#[derive(Clone, Debug, Args)]
pub struct IndexRemoveProfileArgs {
    #[arg(long)]
    pub profile_id: String,
    /// Confirm deletion of this profile's rebuildable projections.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Args)]
pub struct IndexPendingArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long)]
    pub profile_id: String,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
}

#[derive(Clone, Debug, Args)]
pub struct IndexRegisterArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long)]
    pub profile_id: String,
    /// JSON array or JSONL projection input. Omit or use `-` for stdin.
    pub input: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct IndexStatusArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long)]
    pub profile_id: String,
}

#[derive(Clone, Debug, Default, Args)]
pub struct DoctorArgs {
    /// Directory in which to test repository discovery.
    #[arg(long)]
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Args)]
pub struct ExportArgs {
    /// Output file. Omit to write JSONL to stdout.
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Args)]
pub struct ImportArgs {
    /// Input file. Omit or use `-` for stdin. Import currently buffers the snapshot.
    pub input: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct PurgeArgs {
    /// Confirm deletion of the database and sidecars. Stop every process using it first.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Args)]
pub struct HookArgs {
    #[arg(value_enum)]
    pub harness: HarnessArg,
    /// Hard isolation namespace. Must match the MCP server configuration.
    #[arg(long, env = "SUPER_MEM_NAMESPACE", default_value = "default")]
    pub namespace: String,
    /// Optional hard workspace identity. Must match the MCP server configuration.
    #[arg(long, env = "SUPER_MEM_WORKSPACE")]
    pub workspace: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct McpArgs {
    /// Trusted root used to rediscover Git identity on every tool call.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Hard isolation namespace pinned for the lifetime of this server.
    #[arg(long, env = "SUPER_MEM_NAMESPACE", default_value = "default")]
    pub namespace: String,
    /// Optional hard workspace identity pinned for the lifetime of this server.
    #[arg(long, env = "SUPER_MEM_WORKSPACE")]
    pub workspace: Option<String>,
}
