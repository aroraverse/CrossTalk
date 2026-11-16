//! The local request/reply protocol.
//!
//! Requests and responses are newline-delimited JSON objects. Every request
//! may carry an `id`; the daemon echoes it back on the matching response so a
//! client can correlate replies on a shared connection.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Operations understood by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Liveness and identity of the running daemon.
    Status,
    /// Register (or re-register) a local agent session.
    Register,
    /// Update mutable fields of a registered session.
    Update,
    /// List all registered sessions.
    ListSessions,
    /// Look up a single session.
    GetSession,
    /// Remove a registered session.
    RemoveSession,
    /// Send a message from one session to another.
    SendMessage,
    /// Fetch a session's inbox.
    Inbox,
    /// Count a session's unread messages.
    UnreadCount,
    /// Mark messages as read.
    MarkRead,
    /// Ask a human a question.
    AskUser,
    /// List questions asked by agents.
    ListQuestions,
    /// Answer a question.
    AnswerQuestion,
    /// Ask the daemon to shut down.
    Shutdown,
    /// Diagnostic information about the daemon and its state.
    Diagnostics,
    /// Machine identity and peer connection status.
    NetInfo,
    /// Create a team administered by this machine.
    TeamCreate,
    /// Issue an invitation for a team.
    TeamInvite,
    /// Join a team with an invitation token.
    TeamJoin,
    /// List the teams this machine belongs to.
    TeamList,
    /// List the roster of a team.
    TeamMembers,
    /// Leave a team.
    TeamLeave,
    /// Revoke a member's membership (admin only).
    TeamRevoke,
    /// List known peer machines.
    PeerList,
    /// Connect to a peer address explicitly.
    NetConnect,
    /// Local and remote sessions in one directory.
    Directory,
    /// Recent outbound messages and their delivery state.
    Outbox,
    /// Search stored messages by a body substring (M4).
    SearchMessages,
    /// The full conversation thread between two sessions (M4).
    Conversation,
    /// Conversation partners of a session with unread counts (M4).
    Conversations,
    /// Record a file a session is editing, for overlap warnings (M4).
    RecordFileActivity,
    /// Files currently edited by more than one session (M4).
    FileOverlaps,
    /// A single aggregated snapshot for the dashboard (M4).
    Overview,
    /// Report or set the anchor (always-on mailbox/history) role (M4).
    AnchorStatus,
    /// Enable or disable the anchor role on this machine (M4).
    SetAnchor,
    /// Resolve a loose recipient selector to exactly one session.
    ResolveRecipient,
    /// Wait (bounded) for the reply to a message or question.
    WaitReply,
    /// Claim unread messages for hand-over to an agent: returns them once and
    /// marks them read, so a hook or runner never processes one twice.
    ClaimMessages,
    /// Look at a single message and its delivery state.
    GetMessage,
    /// A message from the human operator to a local session.
    SendAsHuman,
    /// Every conversation (pair of participants) with last message and unread count.
    AllConversations,
    /// Inspect which agents are installed and configured.
    SetupStatus,
    /// Configure one agent (MCP server and hooks), with a backup first.
    SetupApply,
}

/// A single request frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Optional correlation id, echoed back on the response.
    #[serde(default)]
    pub id: Option<String>,
    /// The operation to perform.
    pub method: Method,
    /// Operation parameters. Shape depends on `method`.
    #[serde(default)]
    pub params: Value,
}

/// Error detail attached to a failed response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    /// Stable machine readable code.
    pub code: String,
    /// Human readable explanation.
    pub message: String,
}

/// A single response frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Correlation id copied from the request.
    #[serde(default)]
    pub id: Option<String>,
    /// `true` when the operation succeeded.
    pub ok: bool,
    /// Payload of a successful operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Detail of a failed operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

impl Response {
    /// Build a successful response.
    pub fn ok(id: Option<String>, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Build a failed response.
    pub fn fail(id: Option<String>, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ErrorInfo {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/// Parameters for [`Method::Register`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegisterParams {
    /// Stable session key. Generated by the daemon when omitted.
    #[serde(default)]
    pub session_key: Option<String>,
    /// Human readable session name.
    pub name: String,
    /// Agent type, e.g. `claude` or `codex`. Purely informational for M0.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Provider specific handle used for push delivery, e.g. a Claude session id.
    #[serde(default)]
    pub agent_ref: Option<String>,
    /// Working directory of the agent.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Repository the agent is working in.
    #[serde(default)]
    pub repo: Option<String>,
    /// Branch the agent is working on.
    #[serde(default)]
    pub branch: Option<String>,
    /// Initial status.
    #[serde(default)]
    pub status: Option<String>,
    /// Short activity description.
    #[serde(default)]
    pub activity: Option<String>,
}

/// Parameters for [`Method::Update`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateParams {
    /// Session to update.
    pub session_key: String,
    /// New status.
    #[serde(default)]
    pub status: Option<String>,
    /// New activity description.
    #[serde(default)]
    pub activity: Option<String>,
    /// New provider handle for push delivery.
    #[serde(default)]
    pub agent_ref: Option<String>,
}

/// Parameters for [`Method::SendMessage`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendMessageParams {
    /// Sending session key.
    pub sender_key: String,
    /// Receiving session key.
    pub recipient_key: String,
    /// Message body.
    pub body: String,
    /// Id of the message (or question) this one replies to.
    #[serde(default)]
    pub reply_to: Option<String>,
}

/// Parameters for [`Method::Inbox`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InboxParams {
    /// Session whose inbox is read.
    pub session_key: String,
    /// Maximum number of messages to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only return unread messages.
    #[serde(default)]
    pub unread_only: Option<bool>,
}

/// Parameters for [`Method::MarkRead`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarkReadParams {
    /// Session whose messages are marked.
    pub session_key: String,
    /// Specific message ids. When omitted every message is marked.
    #[serde(default)]
    pub message_ids: Option<Vec<String>>,
}

/// Parameters for [`Method::AskUser`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskUserParams {
    /// Session asking the question.
    pub session_key: String,
    /// The question text.
    pub question: String,
}

/// Parameters for [`Method::ListQuestions`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListQuestionsParams {
    /// Only list questions with this status (`pending` or `answered`).
    #[serde(default)]
    pub status: Option<String>,
}

/// Parameters for [`Method::AnswerQuestion`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnswerQuestionParams {
    /// Id of the question being answered.
    pub question_id: String,
    /// The human's answer.
    pub answer: String,
}

/// Parameters for [`Method::GetSession`] and [`Method::RemoveSession`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionKeyParams {
    /// The session key.
    pub session_key: String,
}

/// Parameters for [`Method::TeamCreate`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamCreateParams {
    /// Human readable team name.
    pub name: String,
}

/// Parameters for [`Method::TeamInvite`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamInviteParams {
    /// Team to invite to.
    pub team_id: String,
    /// Invitation lifetime in seconds. Defaults to one hour.
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

/// Parameters for [`Method::TeamJoin`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamJoinParams {
    /// Invitation token produced by `team-invite`.
    pub token: String,
}

/// Parameters for [`Method::TeamMembers`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamMembersParams {
    /// Team whose roster is requested.
    pub team_id: String,
}

/// Parameters for [`Method::TeamLeave`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamLeaveParams {
    /// Team to leave.
    pub team_id: String,
}

/// Parameters for [`Method::NetConnect`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetConnectParams {
    /// JSON encoded iroh `EndpointAddr`, as printed by `machine-id --json`.
    pub addr: String,
}

/// Parameters for [`Method::TeamRevoke`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamRevokeParams {
    /// Team the member is revoked from.
    pub team_id: String,
    /// Peer id to revoke.
    pub peer_id: String,
}

/// Parameters for [`Method::SearchMessages`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchMessagesParams {
    /// Case-insensitive substring to look for in message bodies.
    pub query: String,
    /// Restrict to messages this session sent or received.
    #[serde(default)]
    pub session_key: Option<String>,
    /// Maximum number of matches to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Restrict to messages written by "human" or "agent" senders.
    #[serde(default)]
    pub sender: Option<String>,
    /// Only messages at or after this unix time.
    #[serde(default)]
    pub since: Option<i64>,
    /// Return neighbouring messages as context and the richer view.
    #[serde(default)]
    pub context: Option<bool>,
}

/// Parameters for [`Method::Conversation`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationParams {
    /// One side of the conversation.
    pub session_key: String,
    /// The other session in the thread.
    pub with: String,
    /// Maximum number of messages.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters for [`Method::Conversations`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationsParams {
    /// Session whose conversation partners are listed.
    pub session_key: String,
}

/// Parameters for [`Method::RecordFileActivity`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecordFileActivityParams {
    /// Session editing the file.
    pub session_key: String,
    /// Repository the file belongs to, when known.
    #[serde(default)]
    pub repo: Option<String>,
    /// File path (relative to the repository when possible).
    pub path: String,
    /// `read`, `edit` or `write`. Defaults to `edit`.
    #[serde(default)]
    pub action: Option<String>,
}

/// Parameters for [`Method::FileOverlaps`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileOverlapsParams {
    /// Only consider activity within this many seconds. Defaults to 900.
    #[serde(default)]
    pub window_seconds: Option<i64>,
}

/// Parameters for [`Method::SetAnchor`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetAnchorParams {
    /// Whether this machine should act as the team anchor.
    pub enabled: bool,
}

/// Parameters for [Method::ResolveRecipient].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolveRecipientParams {
    /// Session asking (its project scopes the search).
    #[serde(default)]
    pub session_key: Option<String>,
    /// Exact session key, provider (codex, claude) or session name.
    pub selector: String,
}

/// Parameters for [Method::WaitReply].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WaitReplyParams {
    /// Session that sent the original message and is waiting.
    pub session_key: String,
    /// Id of the message (or question) a reply is awaited for.
    pub message_id: String,
    /// Only accept a reply from this session.
    #[serde(default)]
    pub from: Option<String>,
    /// Maximum time to wait, in seconds. Clamped to 1..=900.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Mark the reply as read when returned. Defaults to true.
    #[serde(default)]
    pub mark_read: Option<bool>,
}

/// Parameters for [Method::ClaimMessages].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimMessagesParams {
    /// Session whose unread messages are claimed.
    pub session_key: String,
    /// Maximum number of messages to claim. Defaults to 5.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters for [Method::GetMessage].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetMessageParams {
    /// The message id.
    pub message_id: String,
}
/// Parameters for [`Method::SendAsHuman`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendAsHumanParams {
    /// Local session to message.
    pub recipient_key: String,
    /// Message body.
    pub body: String,
    /// Message or question this answers.
    #[serde(default)]
    pub reply_to: Option<String>,
}

/// Parameters for [`Method::SetupApply`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetupApplyParams {
    /// `claude` or `codex`.
    pub agent: String,
}
