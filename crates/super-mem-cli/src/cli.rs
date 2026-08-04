use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    #[arg(long, default_value = "default")]
    pub namespace: String,
    /// Optional workspace identity.
    #[arg(long)]
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
    #[arg(long)]
    pub include_superseded: bool,
}

#[derive(Clone, Debug, Args)]
pub struct IdArgs {
    /// Memory UUID.
    pub memory_id: String,
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
}

#[derive(Clone, Debug, Args)]
pub struct McpArgs {
    /// Trusted root used to rediscover Git identity on every tool call.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Hard isolation namespace pinned for the lifetime of this server.
    #[arg(long, default_value = "default")]
    pub namespace: String,
    /// Optional hard workspace identity pinned for the lifetime of this server.
    #[arg(long)]
    pub workspace: Option<String>,
}
