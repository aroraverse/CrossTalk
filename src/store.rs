//! SQLite persistence. This module is the *only* place that touches the
//! database; the daemon owns a single connection and serialises access to it.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::protocol::{
    AnswerQuestionParams, AskUserParams, InboxParams, MarkReadParams, RegisterParams,
    SendMessageParams, UpdateParams,
};

/// Maximum accepted message body size (64 KiB).
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// A registered agent session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// Stable session key used for routing.
    pub session_key: String,
    /// Human readable name, unique among live sessions.
    pub name: String,
    /// Agent type, e.g. `claude`.
    pub agent_type: Option<String>,
    /// Provider specific handle used for push delivery.
    pub agent_ref: Option<String>,
    /// Working directory of the agent.
    pub working_dir: Option<String>,
    /// Repository the agent works in.
    pub repo: Option<String>,
    /// Branch the agent works on.
    pub branch: Option<String>,
    /// Current status.
    pub status: String,
    /// Short activity description.
    pub activity: String,
    /// Creation time (unix seconds).
    pub created_at: i64,
    /// Last time the session was seen (unix seconds).
    pub last_seen: i64,
}

/// A message between two sessions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    /// Unique message id.
    pub id: String,
    /// Sending session key.
    pub sender_key: String,
    /// Receiving session key.
    pub recipient_key: String,
    /// Verbatim message body. Never interpreted, never executed.
    pub body: String,
    /// Creation time (unix seconds).
    pub created_at: i64,
    /// `unread` or `read`.
    pub state: String,
    /// `1` once the message has been handed to a push delivery target.
    pub delivered: bool,
    /// Id of the message (or question) this one answers, when it is a reply.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Conversation thread: the id of the first message of the exchange.
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// Columns every message query selects, in the order [`row_to_message`] reads.
const MESSAGE_COLUMNS: &str =
    "id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id";

/// Sender key reserved for the human operator. No agent or peer may use it.
pub const HUMAN_SENDER: &str = "human";

/// A question an agent asked a human.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Question {
    /// Unique question id.
    pub id: String,
    /// Session that asked.
    pub session_key: String,
    /// Question text.
    pub question: String,
    /// `pending` or `answered`.
    pub status: String,
    /// The human's answer, when present.
    pub answer: Option<String>,
    /// Creation time (unix seconds).
    pub created_at: i64,
    /// Answer time (unix seconds).
    pub answered_at: Option<i64>,
}

/// A team this machine belongs to.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Team {
    /// Stable team identifier.
    pub team_id: String,
    /// Display name.
    pub name: String,
    /// Peer id of the admin that signs membership records.
    pub admin_peer: String,
    /// Creation time (unix seconds).
    pub created_at: i64,
}

/// A stored membership record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredMember {
    /// Team the member belongs to.
    pub team_id: String,
    /// Peer id of the member machine.
    pub peer_id: String,
    /// Display name of the member machine.
    pub member_name: String,
    /// The signed record as JSON.
    pub record_json: String,
}

/// A stored revocation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredRevocation {
    /// Team the revocation applies to.
    pub team_id: String,
    /// Revoked peer id.
    pub peer_id: String,
    /// The signed record as JSON.
    pub record_json: String,
    /// When it was stored locally.
    pub revoked_at: i64,
}

/// A known peer machine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PeerRecord {
    /// Peer id.
    pub peer_id: String,
    /// Optional label.
    pub label: Option<String>,
    /// Last known dialable address (JSON encoded iroh `EndpointAddr`).
    pub addr_json: Option<String>,
    /// `connected`, `idle` or `unknown`.
    pub status: String,
    /// Last time the peer was seen.
    pub last_seen: Option<i64>,
}

/// A session hosted by another machine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoteSession {
    /// Session key.
    pub session_key: String,
    /// Machine hosting the session.
    pub peer_id: String,
    /// Session name.
    pub name: String,
    /// Agent type.
    pub agent_type: Option<String>,
    /// Current status.
    pub status: String,
    /// Short activity description.
    pub activity: String,
    /// Last seen time reported by the owner.
    pub last_seen: i64,
    /// Repository the session works in, when the owner reported it.
    #[serde(default)]
    pub repo: Option<String>,
}

/// A file an agent session touched in a repository, used to warn about two
/// agents editing overlapping files at the same time (M4).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileActivity {
    /// Session that touched the file.
    pub session_key: String,
    /// Repository the file belongs to (best effort, from the hook).
    pub repo: Option<String>,
    /// Path of the file, relative to the repository when known.
    pub path: String,
    /// `read`, `edit` or `write`.
    pub action: String,
    /// When the file was last touched (unix seconds).
    pub updated_at: i64,
}

/// Two sessions found to be editing the same file in the same repository (M4).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileOverlap {
    /// Repository the overlap was detected in (empty when unknown).
    pub repo: String,
    /// The shared file path.
    pub path: String,
    /// Session keys editing that file, most recent first.
    pub sessions: Vec<String>,
    /// Most recent touch across those sessions (unix seconds).
    pub last_activity: i64,
}

/// An outgoing message waiting for or awaiting acknowledgement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutboundMessage {
    /// Local message id, reused on every retry.
    pub id: String,
    /// Sending session.
    pub sender_key: String,
    /// Receiving session.
    pub recipient_key: String,
    /// Body.
    pub body: String,
    /// Creation time.
    pub created_at: i64,
    /// Destination machine.
    pub peer_id: Option<String>,
    /// Message being replied to, if any.
    pub reply_to: Option<String>,
    /// Conversation thread id.
    pub thread_id: Option<String>,
}

/// The SQLite store. Not thread safe on its own; the daemon wraps it in a mutex.
pub struct Store {
    conn: Connection,
    /// Bumped whenever a message is stored; lets waiters sleep until
    /// something arrives instead of polling the database.
    events: tokio::sync::watch::Sender<u64>,
}

impl Store {
    /// Subscribe to message arrivals. Subscribe *before* checking for a
    /// message so a message that lands in between is never missed.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.events.subscribe()
    }

    fn bump(&self) {
        self.events.send_modify(|epoch| *epoch += 1);
    }

    /// Open (or create) the database at `path` and apply the schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Open an in-memory database. Useful for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                session_key TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                agent_type  TEXT,
                agent_ref   TEXT,
                working_dir TEXT,
                repo        TEXT,
                branch      TEXT,
                status      TEXT NOT NULL DEFAULT 'idle',
                activity    TEXT NOT NULL DEFAULT '',
                created_at  INTEGER NOT NULL,
                last_seen   INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id            TEXT PRIMARY KEY,
                sender_key    TEXT NOT NULL,
                recipient_key TEXT NOT NULL,
                body          TEXT NOT NULL,
                created_at    INTEGER NOT NULL,
                state         TEXT NOT NULL DEFAULT 'unread',
                delivered     INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_messages_recipient
                ON messages (recipient_key, state, created_at);

            CREATE TABLE IF NOT EXISTS questions (
                id         TEXT PRIMARY KEY,
                session_key TEXT NOT NULL,
                question   TEXT NOT NULL,
                status     TEXT NOT NULL DEFAULT 'pending',
                answer     TEXT,
                created_at INTEGER NOT NULL,
                answered_at INTEGER
            );
            "#,
        )?;

        // ---- Milestone 1: teams, peers, remote sessions, remote messages ----

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS teams (
                team_id    TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                admin_peer TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS team_members (
                team_id     TEXT NOT NULL,
                peer_id     TEXT NOT NULL,
                member_name TEXT NOT NULL,
                record_json TEXT NOT NULL,
                added_at    INTEGER NOT NULL,
                PRIMARY KEY (team_id, peer_id)
            );

            CREATE TABLE IF NOT EXISTS used_invites (
                nonce   TEXT PRIMARY KEY,
                team_id TEXT NOT NULL,
                used_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS revocations (
                team_id     TEXT NOT NULL,
                peer_id     TEXT NOT NULL,
                record_json TEXT NOT NULL,
                revoked_at  INTEGER NOT NULL,
                PRIMARY KEY (team_id, peer_id)
            );

            CREATE TABLE IF NOT EXISTS remote_sessions (
                session_key TEXT PRIMARY KEY,
                peer_id     TEXT NOT NULL,
                name        TEXT NOT NULL,
                agent_type  TEXT,
                status      TEXT NOT NULL DEFAULT 'idle',
                activity    TEXT NOT NULL DEFAULT '',
                last_seen   INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS peers (
                peer_id   TEXT PRIMARY KEY,
                label     TEXT,
                addr_json TEXT,
                status    TEXT NOT NULL DEFAULT 'unknown',
                last_seen INTEGER
            );

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- M4: files each local session is editing, for overlap warnings.
            CREATE TABLE IF NOT EXISTS file_activity (
                session_key TEXT NOT NULL,
                repo        TEXT,
                path        TEXT NOT NULL,
                action      TEXT NOT NULL DEFAULT 'edit',
                updated_at  INTEGER NOT NULL,
                PRIMARY KEY (session_key, path)
            );

            CREATE INDEX IF NOT EXISTS idx_file_activity_path
                ON file_activity (repo, path, updated_at);
            "#,
        )?;

        // Columns added to the M0 messages table. Existing databases are
        // migrated in place; new databases get them here.
        add_column_if_missing(&conn, "messages", "origin", "TEXT NOT NULL DEFAULT 'local'")?;
        add_column_if_missing(&conn, "messages", "peer_id", "TEXT")?;
        add_column_if_missing(&conn, "messages", "reply_to", "TEXT")?;
        add_column_if_missing(&conn, "messages", "thread_id", "TEXT")?;
        add_column_if_missing(&conn, "remote_sessions", "repo", "TEXT")?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_reply ON messages (reply_to, recipient_key);",
        )?;
        add_column_if_missing(&conn, "messages", "remote_id", "TEXT")?;
        add_column_if_missing(
            &conn,
            "messages",
            "delivery_state",
            "TEXT NOT NULL DEFAULT 'local'",
        )?;
        conn.execute_batch(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_remote_dedupe
                ON messages (peer_id, remote_id) WHERE remote_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_messages_outbound
                ON messages (delivery_state, peer_id);
            "#,
        )?;

        let (events, _) = tokio::sync::watch::channel(0u64);
        Ok(Store { conn, events })
    }

    /// Register or refresh a session.
    ///
    /// When a session key is supplied the existing row is updated in place,
    /// which keeps the identity stable across agent restarts. Session names
    /// are made unique by appending a numeric suffix.
    pub fn register(&mut self, params: RegisterParams) -> Result<Session> {
        let name = params.name.trim();
        if name.is_empty() {
            return Err(Error::InvalidRequest(
                "session name must not be empty".into(),
            ));
        }

        let session_key = match params.session_key.as_deref().map(str::trim) {
            Some(key) if !key.is_empty() => key.to_string(),
            _ => new_id("sess"),
        };
        if session_key.eq_ignore_ascii_case(HUMAN_SENDER) {
            return Err(Error::InvalidRequest(format!(
                "`{HUMAN_SENDER}` is reserved for the human operator and cannot be a session key"
            )));
        }

        let now = now_seconds();
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT created_at FROM sessions WHERE session_key = ?1",
                params![session_key],
                |row| row.get(0),
            )
            .optional()?;

        let created_at = existing.unwrap_or(now);
        let unique_name = self.unique_name(name, &session_key)?;

        self.conn.execute(
            r#"
            INSERT INTO sessions (
                session_key, name, agent_type, agent_ref, working_dir, repo, branch,
                status, activity, created_at, last_seen
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(session_key) DO UPDATE SET
                name        = excluded.name,
                agent_type  = COALESCE(excluded.agent_type, sessions.agent_type),
                agent_ref   = COALESCE(excluded.agent_ref, sessions.agent_ref),
                working_dir = COALESCE(excluded.working_dir, sessions.working_dir),
                repo        = COALESCE(excluded.repo, sessions.repo),
                branch      = COALESCE(excluded.branch, sessions.branch),
                status      = excluded.status,
                activity    = excluded.activity,
                last_seen   = excluded.last_seen
            "#,
            params![
                session_key,
                unique_name,
                params.agent_type,
                params.agent_ref,
                params.working_dir,
                params.repo,
                params.branch,
                params.status.unwrap_or_else(|| "idle".to_string()),
                params.activity.unwrap_or_default(),
                created_at,
                now,
            ],
        )?;

        self.get_session(&session_key)?
            .ok_or_else(|| Error::NotFound(format!("session {session_key}")))
    }

    /// Make sure no two sessions share a name.
    fn unique_name(&self, name: &str, session_key: &str) -> Result<String> {
        let taken: Vec<String> = self
            .conn
            .prepare("SELECT session_key FROM sessions WHERE name = ?1")
            .and_then(|mut stmt| {
                stmt.query_map(params![name], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .unwrap_or_default();

        // The name stays as it is when no *other* session holds it.
        if taken.iter().all(|key| key == session_key) {
            return Ok(name.to_string());
        }
        let mut suffix = 2;
        loop {
            let candidate = format!("{name}-{suffix}");
            let conflict: Option<String> = self
                .conn
                .query_row(
                    "SELECT session_key FROM sessions WHERE name = ?1",
                    params![candidate],
                    |row| row.get(0),
                )
                .optional()?;
            if conflict.as_deref() != Some(session_key) {
                return Ok(candidate);
            }
            suffix += 1;
        }
    }

    /// Fetch a single session.
    pub fn get_session(&self, session_key: &str) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_key, name, agent_type, agent_ref, working_dir, repo, branch,
                        status, activity, created_at, last_seen
                 FROM sessions WHERE session_key = ?1",
                params![session_key],
                row_to_session,
            )
            .optional()?)
    }

    /// List every registered session, newest first.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_key, name, agent_type, agent_ref, working_dir, repo, branch,
                    status, activity, created_at, last_seen
             FROM sessions ORDER BY created_at DESC, name ASC",
        )?;
        let sessions = stmt
            .query_map([], row_to_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    /// Update the mutable fields of a session.
    pub fn update(&mut self, params: UpdateParams) -> Result<Session> {
        let key = params.session_key.trim();
        if key.is_empty() {
            return Err(Error::InvalidRequest(
                "session_key must not be empty".into(),
            ));
        }
        if self.get_session(key)?.is_none() {
            return Err(Error::NotFound(format!("session {key}")));
        }
        // Every update, even an empty one, is a heartbeat.
        self.touch(key)?;

        if let Some(status) = params.status {
            self.conn.execute(
                "UPDATE sessions SET status = ?2, last_seen = ?3 WHERE session_key = ?1",
                params![key, status, now_seconds()],
            )?;
        }
        if let Some(activity) = params.activity {
            self.conn.execute(
                "UPDATE sessions SET activity = ?2, last_seen = ?3 WHERE session_key = ?1",
                params![key, activity, now_seconds()],
            )?;
        }
        if let Some(agent_ref) = params.agent_ref {
            self.conn.execute(
                "UPDATE sessions SET agent_ref = ?2, last_seen = ?3 WHERE session_key = ?1",
                params![key, agent_ref, now_seconds()],
            )?;
        }

        self.get_session(key)?
            .ok_or_else(|| Error::NotFound(format!("session {key}")))
    }

    /// Touch the `last_seen` timestamp of a session.
    pub fn touch(&mut self, session_key: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_seen = ?2 WHERE session_key = ?1",
            params![session_key, now_seconds()],
        )?;
        Ok(())
    }

    /// Remove a session. Messages addressed to it are kept for history.
    pub fn remove_session(&mut self, session_key: &str) -> Result<bool> {
        let affected = self.conn.execute(
            "DELETE FROM sessions WHERE session_key = ?1",
            params![session_key],
        )?;
        Ok(affected > 0)
    }

    /// Store a message after validating both endpoints.
    ///
    /// The body is stored verbatim. It is never parsed, never executed and
    /// never treated as an instruction by the daemon.
    pub fn send_message(&mut self, params: SendMessageParams) -> Result<Message> {
        let sender = params.sender_key.trim();
        let recipient = params.recipient_key.trim();
        if sender.is_empty() || recipient.is_empty() {
            return Err(Error::InvalidRequest(
                "sender_key and recipient_key must not be empty".into(),
            ));
        }
        if sender == recipient {
            return Err(Error::InvalidRequest(
                "a session cannot send messages to itself".into(),
            ));
        }
        if params.body.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "message body must not be empty".into(),
            ));
        }
        if params.body.len() > MAX_BODY_BYTES {
            return Err(Error::InvalidRequest(format!(
                "message body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        if self.get_session(sender)?.is_none() {
            return Err(Error::NotFound(format!("sender session {sender}")));
        }
        if self.get_session(recipient)?.is_none() {
            return Err(Error::NotFound(format!("recipient session {recipient}")));
        }

        let (reply_to, thread_id) = self.thread_for(params.reply_to.as_deref())?;
        let id = new_id("msg");
        let message = Message {
            thread_id: Some(thread_id.unwrap_or_else(|| id.clone())),
            reply_to,
            id,
            sender_key: sender.to_string(),
            recipient_key: recipient.to_string(),
            body: params.body,
            created_at: now_seconds(),
            state: "unread".to_string(),
            delivered: false,
        };

        self.conn.execute(
            "INSERT INTO messages
                 (id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                message.id,
                message.sender_key,
                message.recipient_key,
                message.body,
                message.created_at,
                message.state,
                i64::from(message.delivered),
                message.reply_to,
                message.thread_id,
            ],
        )?;

        self.bump();
        Ok(message)
    }

    /// Validate a `reply_to` reference and find the thread it belongs to.
    ///
    /// The reference must be a known message or question id, so a typo cannot
    /// silently produce a reply nobody will ever match.
    fn thread_for(&self, reply_to: Option<&str>) -> Result<(Option<String>, Option<String>)> {
        let Some(reference) = reply_to.map(str::trim).filter(|r| !r.is_empty()) else {
            return Ok((None, None));
        };
        let parent: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT thread_id FROM messages WHERE id = ?1",
                params![reference],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(thread) = parent {
            return Ok((
                Some(reference.to_string()),
                Some(thread.unwrap_or_else(|| reference.to_string())),
            ));
        }
        let is_question: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM questions WHERE id = ?1",
                params![reference],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if is_question {
            return Ok((Some(reference.to_string()), Some(reference.to_string())));
        }
        Err(Error::InvalidRequest(format!(
            "reply_to `{reference}` is not a known message or question id"
        )))
    }

    /// The oldest message addressed to `session_key` that replies to
    /// `reply_to`, optionally only from `from`. Unrelated messages never match.
    pub fn find_reply(
        &self,
        session_key: &str,
        reply_to: &str,
        from: Option<&str>,
    ) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages
                     WHERE recipient_key = ?1 AND reply_to = ?2
                       AND (?3 IS NULL OR sender_key = ?3)
                     ORDER BY created_at ASC, id ASC LIMIT 1"
                ),
                params![session_key, reply_to, from],
                row_to_message,
            )
            .optional()?)
    }

    /// Fetch one message by id.
    pub fn get_message(&self, id: &str) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
                params![id],
                row_to_message,
            )
            .optional()?)
    }

    /// Fetch a session's inbox, oldest message first.
    pub fn inbox(&mut self, params: InboxParams) -> Result<Vec<Message>> {
        if self.get_session(&params.session_key)?.is_none() {
            return Err(Error::NotFound(format!("session {}", params.session_key)));
        }
        let limit = params.limit.unwrap_or(50).clamp(1, 500) as i64;
        let unread_only = params.unread_only.unwrap_or(false);

        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id
             FROM messages
             WHERE recipient_key = ?1 AND (?2 = 0 OR state = 'unread')
             ORDER BY created_at ASC, id ASC
             LIMIT ?3",
        )?;
        let messages = stmt
            .query_map(
                params![params.session_key, unread_only, limit],
                row_to_message,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// Count unread messages of a session.
    pub fn unread_count(&mut self, session_key: &str) -> Result<usize> {
        if self.get_session(session_key)?.is_none() {
            return Err(Error::NotFound(format!("session {session_key}")));
        }
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE recipient_key = ?1 AND state = 'unread'",
            params![session_key],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Mark messages as read. Without ids every message of the session is marked.
    pub fn mark_read(&mut self, params: MarkReadParams) -> Result<usize> {
        if self.get_session(&params.session_key)?.is_none() {
            return Err(Error::NotFound(format!("session {}", params.session_key)));
        }
        match params.message_ids {
            Some(ids) if !ids.is_empty() => {
                let mut updated = 0usize;
                for id in ids {
                    updated += self.conn.execute(
                        "UPDATE messages SET state = 'read'
                         WHERE id = ?1 AND recipient_key = ?2 AND state = 'unread'",
                        params![id, params.session_key],
                    )?;
                }
                Ok(updated)
            }
            _ => {
                let updated = self.conn.execute(
                    "UPDATE messages SET state = 'read'
                     WHERE recipient_key = ?1 AND state = 'unread'",
                    params![params.session_key],
                )?;
                Ok(updated)
            }
        }
    }

    /// Messages that still need to be handed to a push delivery target.
    pub fn undelivered_messages(&self) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id
             FROM messages WHERE delivered = 0 ORDER BY created_at ASC",
        )?;
        let messages = stmt
            .query_map([], row_to_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// Flag a message as delivered to its push target.
    pub fn mark_delivered(&mut self, message_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET delivered = 1 WHERE id = ?1",
            params![message_id],
        )?;
        Ok(())
    }

    /// Record a question for a human.
    pub fn ask_user(&mut self, params: AskUserParams) -> Result<Question> {
        if self.get_session(&params.session_key)?.is_none() {
            return Err(Error::NotFound(format!("session {}", params.session_key)));
        }
        if params.question.trim().is_empty() {
            return Err(Error::InvalidRequest("question must not be empty".into()));
        }
        let question = Question {
            id: new_id("q"),
            session_key: params.session_key,
            question: params.question,
            status: "pending".to_string(),
            answer: None,
            created_at: now_seconds(),
            answered_at: None,
        };
        self.conn.execute(
            "INSERT INTO questions (id, session_key, question, status, answer, created_at, answered_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL)",
            params![
                question.id,
                question.session_key,
                question.question,
                question.status,
                question.created_at
            ],
        )?;
        Ok(question)
    }

    /// List questions, optionally filtered by status.
    pub fn list_questions(&self, status: Option<&str>) -> Result<Vec<Question>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_key, question, status, answer, created_at, answered_at
             FROM questions
             WHERE (?1 IS NULL OR status = ?1)
             ORDER BY created_at DESC",
        )?;
        let questions = stmt
            .query_map(params![status], row_to_question)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(questions)
    }

    /// Answer a question and deliver the answer to the asking session's inbox.
    pub fn answer_question(&mut self, params: AnswerQuestionParams) -> Result<Question> {
        let question = self
            .conn
            .query_row(
                "SELECT id, session_key, question, status, answer, created_at, answered_at
                 FROM questions WHERE id = ?1",
                params![params.question_id],
                row_to_question,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("question {}", params.question_id)))?;

        if question.status == "answered" {
            return Err(Error::InvalidRequest(format!(
                "question {} is already answered",
                question.id
            )));
        }

        let now = now_seconds();
        self.conn.execute(
            "UPDATE questions SET status = 'answered', answer = ?2, answered_at = ?3 WHERE id = ?1",
            params![question.id, params.answer, now],
        )?;

        // The answer reaches the agent as an ordinary inbox message.
        // The answer is correlated to the question, so the asking agent can
        // wait for exactly this reply.
        let message = Message {
            id: new_id("msg"),
            sender_key: HUMAN_SENDER.to_string(),
            recipient_key: question.session_key.clone(),
            body: params.answer.clone(),
            created_at: now,
            state: "unread".to_string(),
            delivered: false,
            reply_to: Some(question.id.clone()),
            thread_id: Some(question.id.clone()),
        };
        self.conn.execute(
            "INSERT INTO messages
                 (id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                message.id,
                message.sender_key,
                message.recipient_key,
                message.body,
                message.created_at,
                message.state,
                i64::from(message.delivered),
                message.reply_to,
                message.thread_id,
            ],
        )?;

        self.bump();
        Ok(Question {
            status: "answered".to_string(),
            answer: Some(params.answer),
            answered_at: Some(now),
            ..question
        })
    }

    /// Count of stored sessions and messages, used by diagnostics.
    pub fn counts(&self) -> Result<(usize, usize)> {
        let sessions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        let messages: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
        Ok((sessions as usize, messages as usize))
    }

    // -----------------------------------------------------------------------
    // Milestone 1: teams, peers, remote sessions, remote messages
    // -----------------------------------------------------------------------

    /// Create (or refresh) a team this machine belongs to.
    pub fn upsert_team(&mut self, team: &Team) -> Result<()> {
        self.conn.execute(
            "INSERT INTO teams (team_id, name, admin_peer, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(team_id) DO UPDATE SET name = excluded.name, admin_peer = excluded.admin_peer",
            params![team.team_id, team.name, team.admin_peer, team.created_at],
        )?;
        Ok(())
    }

    /// Fetch one team.
    pub fn get_team(&self, team_id: &str) -> Result<Option<Team>> {
        Ok(self
            .conn
            .query_row(
                "SELECT team_id, name, admin_peer, created_at FROM teams WHERE team_id = ?1",
                params![team_id],
                |row| {
                    Ok(Team {
                        team_id: row.get(0)?,
                        name: row.get(1)?,
                        admin_peer: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// List every team this machine belongs to.
    pub fn list_teams(&self) -> Result<Vec<Team>> {
        let mut stmt = self.conn.prepare(
            "SELECT team_id, name, admin_peer, created_at FROM teams ORDER BY created_at",
        )?;
        let teams = stmt
            .query_map([], |row| {
                Ok(Team {
                    team_id: row.get(0)?,
                    name: row.get(1)?,
                    admin_peer: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(teams)
    }

    /// Remove a team and its membership, used by `team-leave`.
    pub fn delete_team(&mut self, team_id: &str) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM teams WHERE team_id = ?1", params![team_id])?;
        self.conn.execute(
            "DELETE FROM team_members WHERE team_id = ?1",
            params![team_id],
        )?;
        Ok(removed > 0)
    }

    /// Store a verified membership record.
    pub fn put_member(&mut self, record_json: &str, member: &StoredMember) -> Result<()> {
        self.conn.execute(
            "INSERT INTO team_members (team_id, peer_id, member_name, record_json, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(team_id, peer_id) DO UPDATE SET
                 member_name = excluded.member_name,
                 record_json = excluded.record_json",
            params![
                member.team_id,
                member.peer_id,
                member.member_name,
                record_json,
                now_seconds()
            ],
        )?;
        Ok(())
    }

    /// List the roster of a team.
    pub fn list_members(&self, team_id: &str) -> Result<Vec<StoredMember>> {
        let mut stmt = self.conn.prepare(
            "SELECT team_id, peer_id, member_name, record_json
             FROM team_members WHERE team_id = ?1 ORDER BY member_name",
        )?;
        let members = stmt
            .query_map(params![team_id], |row| {
                Ok(StoredMember {
                    team_id: row.get(0)?,
                    peer_id: row.get(1)?,
                    member_name: row.get(2)?,
                    record_json: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(members)
    }

    /// Every membership record this machine holds, across all teams.
    pub fn all_members(&self) -> Result<Vec<StoredMember>> {
        let mut stmt = self.conn.prepare(
            "SELECT team_id, peer_id, member_name, record_json FROM team_members ORDER BY team_id",
        )?;
        let members = stmt
            .query_map([], |row| {
                Ok(StoredMember {
                    team_id: row.get(0)?,
                    peer_id: row.get(1)?,
                    member_name: row.get(2)?,
                    record_json: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(members)
    }

    /// Remove one member from a team.
    pub fn delete_member(&mut self, team_id: &str, peer_id: &str) -> Result<bool> {
        let removed = self.conn.execute(
            "DELETE FROM team_members WHERE team_id = ?1 AND peer_id = ?2",
            params![team_id, peer_id],
        )?;
        Ok(removed > 0)
    }

    /// Persist a verified revocation and drop the member from the roster.
    ///
    /// Storing and removing the member happen together so a revoked peer is
    /// never advertised as a member once the revocation is known.
    pub fn put_revocation(&mut self, record: &StoredRevocation) -> Result<()> {
        let transaction = self.conn.transaction()?;
        transaction.execute(
            "INSERT INTO revocations (team_id, peer_id, record_json, revoked_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(team_id, peer_id) DO UPDATE SET
                 record_json = excluded.record_json,
                 revoked_at = excluded.revoked_at",
            params![
                record.team_id,
                record.peer_id,
                record.record_json,
                record.revoked_at
            ],
        )?;
        transaction.execute(
            "DELETE FROM team_members WHERE team_id = ?1 AND peer_id = ?2",
            params![record.team_id, record.peer_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// True when `peer_id` has been revoked from `team_id`.
    pub fn is_revoked(&self, team_id: &str, peer_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM revocations WHERE team_id = ?1 AND peer_id = ?2",
            params![team_id, peer_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// True when any local team has revoked `peer_id`.
    pub fn is_revoked_anywhere(&self, peer_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM revocations WHERE peer_id = ?1",
            params![peer_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Revocations known for one team.
    pub fn list_revocations(&self, team_id: &str) -> Result<Vec<StoredRevocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT team_id, peer_id, record_json, revoked_at
             FROM revocations WHERE team_id = ?1 ORDER BY peer_id",
        )?;
        let records = stmt
            .query_map(params![team_id], row_to_revocation)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// Every revocation this machine knows.
    pub fn all_revocations(&self) -> Result<Vec<StoredRevocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT team_id, peer_id, record_json, revoked_at FROM revocations ORDER BY team_id",
        )?;
        let records = stmt
            .query_map([], row_to_revocation)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// Consume an invitation nonce. Returns `false` when it was already used.
    pub fn consume_invite(&mut self, nonce: &str, team_id: &str) -> Result<bool> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO used_invites (nonce, team_id, used_at) VALUES (?1, ?2, ?3)",
            params![nonce, team_id, now_seconds()],
        )?;
        Ok(inserted > 0)
    }

    /// True when an invitation nonce has already been consumed.
    pub fn invite_used(&self, nonce: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM used_invites WHERE nonce = ?1",
            params![nonce],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Record a peer and how to reach it.
    pub fn upsert_peer(&mut self, peer: &PeerRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO peers (peer_id, label, addr_json, status, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(peer_id) DO UPDATE SET
                 label = COALESCE(excluded.label, peers.label),
                 addr_json = COALESCE(excluded.addr_json, peers.addr_json),
                 status = excluded.status,
                 last_seen = excluded.last_seen",
            params![
                peer.peer_id,
                peer.label,
                peer.addr_json,
                peer.status,
                peer.last_seen
            ],
        )?;
        Ok(())
    }

    /// Update the connection status of a known peer.
    pub fn set_peer_status(&mut self, peer_id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE peers SET status = ?2, last_seen = ?3 WHERE peer_id = ?1",
            params![peer_id, status, now_seconds()],
        )?;
        Ok(())
    }

    /// List known peers.
    pub fn list_peers(&self) -> Result<Vec<PeerRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_id, label, addr_json, status, last_seen FROM peers ORDER BY peer_id",
        )?;
        let peers = stmt
            .query_map([], |row| {
                Ok(PeerRecord {
                    peer_id: row.get(0)?,
                    label: row.get(1)?,
                    addr_json: row.get(2)?,
                    status: row.get(3)?,
                    last_seen: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(peers)
    }

    /// Replace the remote session table of one peer with a fresh snapshot.
    pub fn replace_remote_sessions(
        &mut self,
        peer_id: &str,
        sessions: &[RemoteSession],
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM remote_sessions WHERE peer_id = ?1",
            params![peer_id],
        )?;
        for session in sessions {
            self.conn.execute(
                "INSERT OR REPLACE INTO remote_sessions
                     (session_key, peer_id, name, agent_type, status, activity, last_seen, updated_at, repo)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    session.session_key,
                    peer_id,
                    session.name,
                    session.agent_type,
                    session.status,
                    session.activity,
                    session.last_seen,
                    now_seconds(),
                    session.repo
                ],
            )?;
        }
        Ok(())
    }

    /// Drop all sessions of a peer that is no longer reachable.
    pub fn clear_remote_sessions(&mut self, peer_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM remote_sessions WHERE peer_id = ?1",
            params![peer_id],
        )?;
        Ok(())
    }

    /// List synchronised sessions from other machines.
    pub fn list_remote_sessions(&self) -> Result<Vec<RemoteSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_key, peer_id, name, agent_type, status, activity, last_seen, repo
             FROM remote_sessions ORDER BY name",
        )?;
        let sessions = stmt
            .query_map([], |row| {
                Ok(RemoteSession {
                    session_key: row.get(0)?,
                    peer_id: row.get(1)?,
                    name: row.get(2)?,
                    agent_type: row.get(3)?,
                    status: row.get(4)?,
                    activity: row.get(5)?,
                    last_seen: row.get(6)?,
                    repo: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    /// Find which peer hosts a remote session.
    pub fn owner_of_session(&self, session_key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT peer_id FROM remote_sessions WHERE session_key = ?1",
                params![session_key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Store an outgoing remote message before it is sent.
    ///
    /// The row is the retry record: `delivery_state` moves from `pending` to
    /// `sent` and finally `acked`. The message id is reused on every retry, so
    /// the receiver can drop duplicates.
    pub fn queue_remote_message(
        &mut self,
        sender_key: &str,
        recipient_key: &str,
        body: &str,
        peer_id: &str,
        reply_to: Option<&str>,
    ) -> Result<Message> {
        if body.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "message body must not be empty".into(),
            ));
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(Error::InvalidRequest(format!(
                "message body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        if self.get_session(sender_key)?.is_none() {
            return Err(Error::NotFound(format!("sender session {sender_key}")));
        }
        let (reply_to, thread_id) = self.thread_for(reply_to)?;
        let id = new_id("msg");
        let message = Message {
            thread_id: Some(thread_id.unwrap_or_else(|| id.clone())),
            reply_to,
            id,
            sender_key: sender_key.to_string(),
            recipient_key: recipient_key.to_string(),
            body: body.to_string(),
            created_at: now_seconds(),
            state: "sent".to_string(),
            delivered: false,
        };
        self.conn.execute(
            "INSERT INTO messages
                 (id, sender_key, recipient_key, body, created_at, state, delivered,
                  origin, peer_id, remote_id, delivery_state, reply_to, thread_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 'local', ?7, NULL, 'pending', ?8, ?9)",
            params![
                message.id,
                message.sender_key,
                message.recipient_key,
                message.body,
                message.created_at,
                message.state,
                peer_id,
                message.reply_to,
                message.thread_id
            ],
        )?;
        Ok(message)
    }

    /// Outgoing messages that still need to reach a peer.
    pub fn pending_outbound(&self) -> Result<Vec<OutboundMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, peer_id, reply_to, thread_id
             FROM messages
             WHERE delivery_state IN ('pending', 'sent') AND peer_id IS NOT NULL
             ORDER BY created_at ASC",
        )?;
        let messages = stmt
            .query_map([], |row| {
                Ok(OutboundMessage {
                    id: row.get(0)?,
                    sender_key: row.get(1)?,
                    recipient_key: row.get(2)?,
                    body: row.get(3)?,
                    created_at: row.get(4)?,
                    peer_id: row.get(5)?,
                    reply_to: row.get(6)?,
                    thread_id: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// Update the delivery state of an outgoing message.
    pub fn set_delivery_state(&mut self, message_id: &str, state: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET delivery_state = ?2 WHERE id = ?1",
            params![message_id, state],
        )?;
        Ok(())
    }

    /// Recent outbound messages with their delivery state, for diagnostics.
    pub fn list_outbound(&self, limit: u32) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, recipient_key, peer_id, delivery_state, created_at
             FROM messages WHERE peer_id IS NOT NULL
             ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "recipient_key": row.get::<_, String>(1)?,
                    "peer_id": row.get::<_, Option<String>>(2)?,
                    "delivery_state": row.get::<_, String>(3)?,
                    "created_at": row.get::<_, i64>(4)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Store a message received from another machine.
    ///
    /// The argument list mirrors the wire frame one to one.
    ///
    /// Returns `Ok(None)` when the message was already stored, which is what
    /// makes retries safe: the receiver deduplicates on `(peer, remote_id)`.
    #[allow(clippy::too_many_arguments)]
    pub fn store_incoming_remote(
        &mut self,
        peer_id: &str,
        remote_id: &str,
        sender_key: &str,
        recipient_key: &str,
        body: &str,
        created_at: i64,
        reply_to: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<Option<Message>> {
        if sender_key.trim().eq_ignore_ascii_case(HUMAN_SENDER) {
            return Err(Error::InvalidRequest(format!(
                "a peer may not send as `{HUMAN_SENDER}`: that sender is reserved for the local human operator"
            )));
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(Error::InvalidRequest(
                "message exceeds the size limit".into(),
            ));
        }
        if self.get_session(recipient_key)?.is_none() {
            return Err(Error::NotFound(format!(
                "recipient session {recipient_key}"
            )));
        }
        // The sender's id is kept, so a reply can reference the original id on
        // either machine.
        // A colliding id (a different peer reusing an id) gets a fresh one.
        let taken: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM messages WHERE id = ?1",
                params![remote_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        let local_id = if taken && !self.has_remote(peer_id, remote_id)? {
            new_id("msg")
        } else {
            remote_id.to_string()
        };
        let message = Message {
            id: local_id,
            sender_key: sender_key.to_string(),
            recipient_key: recipient_key.to_string(),
            body: body.to_string(),
            created_at,
            state: "unread".to_string(),
            delivered: false,
            reply_to: reply_to.map(str::to_string),
            thread_id: thread_id
                .map(str::to_string)
                .or_else(|| Some(remote_id.to_string())),
        };
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO messages
                 (id, sender_key, recipient_key, body, created_at, state, delivered,
                  origin, peer_id, remote_id, delivery_state, reply_to, thread_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 'remote', ?7, ?8, 'local', ?9, ?10)",
            params![
                message.id,
                message.sender_key,
                message.recipient_key,
                message.body,
                message.created_at,
                message.state,
                peer_id,
                remote_id,
                message.reply_to,
                message.thread_id
            ],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        self.bump();
        Ok(Some(message))
    }

    /// True when a message from peer_id with this remote id is already stored.
    fn has_remote(&self, peer_id: &str, remote_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM messages WHERE peer_id = ?1 AND remote_id = ?2",
                params![peer_id, remote_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Sessions that must be advertised to peers.
    pub fn presence_snapshot(&self) -> Result<Vec<RemoteSession>> {
        let sessions = self.list_sessions()?;
        Ok(sessions
            .into_iter()
            .map(|session| RemoteSession {
                session_key: session.session_key,
                peer_id: String::new(),
                name: session.name,
                agent_type: session.agent_type,
                status: session.status,
                activity: session.activity,
                last_seen: session.last_seen,
                repo: session.repo,
            })
            .collect())
    }

    // -----------------------------------------------------------------------
    // Milestone 4: search, conversations, file-overlap activity
    // -----------------------------------------------------------------------

    /// Search stored messages by a case-insensitive substring of the body.
    ///
    /// Reuses the existing `messages` table; no separate search database is
    /// introduced. When `session_key` is set, only messages that session sent
    /// or received are returned. Newest first.
    pub fn search_messages(
        &self,
        query: &str,
        session_key: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        // Escape LIKE wildcards so a literal substring is matched.
        let escaped = needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let limit = limit.clamp(1, 500) as i64;

        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id
             FROM messages
             WHERE body LIKE ?1 ESCAPE '\\'
               AND (?2 IS NULL OR sender_key = ?2 OR recipient_key = ?2)
             ORDER BY created_at DESC, id DESC
             LIMIT ?3",
        )?;
        let messages = stmt
            .query_map(params![pattern, session_key, limit], row_to_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// The full conversation between two sessions, oldest first.
    ///
    /// Includes messages in both directions so the dashboard can render a
    /// single thread.
    pub fn conversation(&self, a: &str, b: &str, limit: u32) -> Result<Vec<Message>> {
        let limit = limit.clamp(1, 1000) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id
             FROM messages
             WHERE (sender_key = ?1 AND recipient_key = ?2)
                OR (sender_key = ?2 AND recipient_key = ?1)
             ORDER BY created_at ASC, id ASC
             LIMIT ?3",
        )?;
        let messages = stmt
            .query_map(params![a, b, limit], row_to_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// Every distinct session key this session has exchanged messages with,
    /// with the newest message and unread count per peer. Used to list
    /// conversation threads in the dashboard.
    pub fn conversation_partners(&self, session_key: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT partner,
                    MAX(created_at)        AS last_at,
                    SUM(CASE WHEN recipient_key = ?1 AND state = 'unread' THEN 1 ELSE 0 END) AS unread
             FROM (
                 SELECT CASE WHEN sender_key = ?1 THEN recipient_key ELSE sender_key END AS partner,
                        created_at, recipient_key, state
                 FROM messages
                 WHERE sender_key = ?1 OR recipient_key = ?1
             )
             GROUP BY partner
             ORDER BY last_at DESC",
        )?;
        let rows = stmt
            .query_map(params![session_key], |row| {
                Ok(serde_json::json!({
                    "partner": row.get::<_, String>(0)?,
                    "last_at": row.get::<_, i64>(1)?,
                    "unread": row.get::<_, i64>(2)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// A message from the human operator to a session on this machine.
    ///
    /// Only the local dashboard and IPC reach this. Agents cannot: the MCP
    /// server has no such tool, and `human` cannot be registered as a session.
    pub fn send_as_human(
        &mut self,
        recipient: &str,
        body: &str,
        reply_to: Option<&str>,
    ) -> Result<Message> {
        let recipient = recipient.trim();
        if body.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "message body must not be empty".into(),
            ));
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(Error::InvalidRequest(format!(
                "message body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        if self.get_session(recipient)?.is_none() {
            return Err(Error::NotFound(format!(
                "recipient session {recipient} (the human can only message sessions on this machine)"
            )));
        }
        let (reply_to, thread_id) = self.thread_for(reply_to)?;
        let id = new_id("msg");
        let message = Message {
            thread_id: Some(thread_id.unwrap_or_else(|| id.clone())),
            reply_to,
            id,
            sender_key: HUMAN_SENDER.to_string(),
            recipient_key: recipient.to_string(),
            body: body.to_string(),
            created_at: now_seconds(),
            state: "unread".to_string(),
            delivered: false,
        };
        self.conn.execute(
            "INSERT INTO messages
                 (id, sender_key, recipient_key, body, created_at, state, delivered, reply_to, thread_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)",
            params![
                message.id,
                message.sender_key,
                message.recipient_key,
                message.body,
                message.created_at,
                message.state,
                message.reply_to,
                message.thread_id,
            ],
        )?;
        self.bump();
        Ok(message)
    }

    /// Every pair of participants that exchanged messages, newest first, with
    /// the last message, a count and the unread count.
    pub fn conversation_pairs(&self, limit: u32) -> Result<Vec<serde_json::Value>> {
        let limit = limit.clamp(1, 500) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT a, b, MAX(created_at) AS last_at, COUNT(*) AS total,
                    SUM(CASE WHEN state = 'unread' THEN 1 ELSE 0 END) AS unread
             FROM (
                 SELECT CASE WHEN sender_key < recipient_key THEN sender_key ELSE recipient_key END AS a,
                        CASE WHEN sender_key < recipient_key THEN recipient_key ELSE sender_key END AS b,
                        created_at, state
                 FROM messages
             )
             GROUP BY a, b
             ORDER BY last_at DESC
             LIMIT ?1",
        )?;
        let pairs = stmt
            .query_map(params![limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for (a, b, last_at, total, unread) in pairs {
            let last: Option<(String, String)> = self
                .conn
                .query_row(
                    "SELECT sender_key, body FROM messages
                     WHERE (sender_key = ?1 AND recipient_key = ?2)
                        OR (sender_key = ?2 AND recipient_key = ?1)
                     ORDER BY created_at DESC, id DESC LIMIT 1",
                    params![a, b],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (last_sender, last_body) = last.unwrap_or_default();
            out.push(serde_json::json!({
                "a": a, "b": b, "last_at": last_at, "total": total, "unread": unread,
                "last_sender": last_sender,
                "last_body": snippet(&last_body, 140),
            }));
        }
        Ok(out)
    }

    /// A conversation with a derived delivery state per message.
    ///
    /// `delivery` is one of `queued` (waiting for a peer machine), `sent`
    /// (handed to the peer), `delivered` (stored in the recipient's inbox) or
    /// `read` (the recipient consumed it).
    pub fn conversation_view(
        &self,
        a: &str,
        b: &str,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        let limit = limit.clamp(1, 1000) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, state, reply_to, thread_id,
                    delivery_state, origin
             FROM messages
             WHERE (sender_key = ?1 AND recipient_key = ?2)
                OR (sender_key = ?2 AND recipient_key = ?1)
             ORDER BY created_at ASC, id ASC
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![a, b, limit], |row| {
                let state: String = row.get(5)?;
                let delivery_state: String = row.get(8)?;
                let origin: String = row.get(9)?;
                let delivery = match (origin.as_str(), delivery_state.as_str()) {
                    ("local", "pending") => "queued",
                    ("local", "sent") => "sent",
                    ("local", "acked") => "delivered",
                    _ if state == "read" => "read",
                    _ => "delivered",
                };
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "sender_key": row.get::<_, String>(1)?,
                    "recipient_key": row.get::<_, String>(2)?,
                    "body": row.get::<_, String>(3)?,
                    "created_at": row.get::<_, i64>(4)?,
                    "state": state,
                    "reply_to": row.get::<_, Option<String>>(6)?,
                    "thread_id": row.get::<_, Option<String>>(7)?,
                    "delivery": delivery,
                    "from_human": row.get::<_, String>(1)? == HUMAN_SENDER,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Search with filters and surrounding context.
    ///
    /// `sender` is `human` or `agent` to restrict who wrote the message;
    /// `since` is a unix time lower bound. Every hit carries the message
    /// before and after it in the same conversation.
    pub fn search_view(
        &self,
        query: &str,
        session_key: Option<&str>,
        sender: Option<&str>,
        since: Option<i64>,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let escaped = needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let limit = limit.clamp(1, 200) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, sender_key, recipient_key, body, created_at, state
             FROM messages
             WHERE body LIKE ?1 ESCAPE '\\'
               AND (?2 IS NULL OR sender_key = ?2 OR recipient_key = ?2)
               AND (?3 IS NULL OR (?3 = 'human' AND sender_key = 'human')
                               OR (?3 = 'agent' AND sender_key <> 'human'))
               AND (?4 IS NULL OR created_at >= ?4)
             ORDER BY created_at DESC, id DESC
             LIMIT ?5",
        )?;
        let hits = stmt
            .query_map(params![pattern, session_key, sender, since, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let neighbour = |sender: &str, recipient: &str, at: i64, id: &str, before: bool| {
            let sql = if before {
                "SELECT sender_key, body, created_at FROM messages
                 WHERE ((sender_key = ?1 AND recipient_key = ?2) OR (sender_key = ?2 AND recipient_key = ?1))
                   AND (created_at < ?3 OR (created_at = ?3 AND id < ?4))
                 ORDER BY created_at DESC, id DESC LIMIT 1"
            } else {
                "SELECT sender_key, body, created_at FROM messages
                 WHERE ((sender_key = ?1 AND recipient_key = ?2) OR (sender_key = ?2 AND recipient_key = ?1))
                   AND (created_at > ?3 OR (created_at = ?3 AND id > ?4))
                 ORDER BY created_at ASC, id ASC LIMIT 1"
            };
            self.conn
                .query_row(sql, params![sender, recipient, at, id], |row| {
                    Ok(serde_json::json!({
                        "sender_key": row.get::<_, String>(0)?,
                        "body": snippet(&row.get::<_, String>(1)?, 160),
                        "created_at": row.get::<_, i64>(2)?,
                    }))
                })
                .optional()
        };
        let mut out = Vec::new();
        for (id, sender, recipient, body, created_at, state) in hits {
            out.push(serde_json::json!({
                "id": id,
                "sender_key": sender,
                "recipient_key": recipient,
                "body": body,
                "created_at": created_at,
                "state": state,
                "from_human": sender == HUMAN_SENDER,
                "before": neighbour(&sender, &recipient, created_at, &id, true)?,
                "after": neighbour(&sender, &recipient, created_at, &id, false)?,
            }));
        }
        Ok(out)
    }

    /// Questions with the context an approver needs and whether the answer
    /// reached the asking agent.
    pub fn question_view(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        for question in self.list_questions(status)? {
            // What the asking session was dealing with just before it asked.
            let mut stmt = self.conn.prepare(
                "SELECT sender_key, recipient_key, body, created_at FROM messages
                 WHERE (sender_key = ?1 OR recipient_key = ?1) AND created_at <= ?2
                 ORDER BY created_at DESC, id DESC LIMIT 3",
            )?;
            let mut context = stmt
                .query_map(params![question.session_key, question.created_at], |row| {
                    Ok(serde_json::json!({
                        "sender_key": row.get::<_, String>(0)?,
                        "recipient_key": row.get::<_, String>(1)?,
                        "body": snippet(&row.get::<_, String>(2)?, 240),
                        "created_at": row.get::<_, i64>(3)?,
                    }))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            context.reverse();
            // Did the agent pick the answer up?
            let answer_state: Option<String> = self
                .conn
                .query_row(
                    "SELECT state FROM messages WHERE reply_to = ?1 AND sender_key = 'human'
                     ORDER BY created_at DESC LIMIT 1",
                    params![question.id],
                    |row| row.get(0),
                )
                .optional()?;
            let session = self.get_session(&question.session_key)?;
            let mut value = serde_json::to_value(&question)?;
            if let Some(object) = value.as_object_mut() {
                object.insert("context".into(), serde_json::Value::Array(context));
                object.insert(
                    "delivery".into(),
                    serde_json::json!(match answer_state.as_deref() {
                        Some("read") => "read by the agent",
                        Some(_) => "delivered, waiting for the agent",
                        None => "",
                    }),
                );
                object.insert(
                    "session".into(),
                    session
                        .map(|s| {
                            serde_json::json!({
                                "name": s.name, "agent_type": s.agent_type, "repo": s.repo,
                                "working_dir": s.working_dir, "status": s.status,
                            })
                        })
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            out.push(value);
        }
        Ok(out)
    }
    /// Record (or refresh) a file a session is working on.
    pub fn record_file_activity(&mut self, activity: &FileActivity) -> Result<()> {
        let path = activity.path.trim();
        if path.is_empty() {
            return Err(Error::InvalidRequest("file path must not be empty".into()));
        }
        self.conn.execute(
            "INSERT INTO file_activity (session_key, repo, path, action, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_key, path) DO UPDATE SET
                 repo       = COALESCE(excluded.repo, file_activity.repo),
                 action     = excluded.action,
                 updated_at = excluded.updated_at",
            params![
                activity.session_key,
                activity.repo,
                path,
                activity.action,
                activity.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Forget a session's file activity (e.g. when it goes offline).
    pub fn clear_file_activity(&mut self, session_key: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM file_activity WHERE session_key = ?1",
            params![session_key],
        )?;
        Ok(())
    }

    /// Detect files touched by two or more sessions within `window_seconds`.
    ///
    /// Only files edited (not merely read) are considered an overlap worth a
    /// warning. Sessions are compared within the same repository; a `NULL`
    /// repository only overlaps another `NULL` repository.
    pub fn file_overlaps(&self, window_seconds: i64) -> Result<Vec<FileOverlap>> {
        let cutoff = now_seconds() - window_seconds.max(0);
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(repo, '') AS r, path, session_key, updated_at
             FROM file_activity
             WHERE action IN ('edit', 'write') AND updated_at >= ?1
             ORDER BY r, path, updated_at DESC",
        )?;
        let rows = stmt
            .query_map(params![cutoff], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut overlaps: Vec<FileOverlap> = Vec::new();
        let mut index = 0;
        while index < rows.len() {
            let (repo, path, _, _) = &rows[index];
            let mut sessions = Vec::new();
            let mut last_activity = 0;
            let mut end = index;
            while end < rows.len() && &rows[end].0 == repo && &rows[end].1 == path {
                if !sessions.contains(&rows[end].2) {
                    sessions.push(rows[end].2.clone());
                }
                last_activity = last_activity.max(rows[end].3);
                end += 1;
            }
            if sessions.len() >= 2 {
                overlaps.push(FileOverlap {
                    repo: repo.clone(),
                    path: path.clone(),
                    sessions,
                    last_activity,
                });
            }
            index = end;
        }
        overlaps.sort_by_key(|overlap| std::cmp::Reverse(overlap.last_activity));
        Ok(overlaps)
    }

    /// Read a value from the local metadata table.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Write a value into the local metadata table.
    pub fn meta_set(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// Add a column to an existing table when it is missing.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !existing.iter().any(|name| name == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        session_key: row.get(0)?,
        name: row.get(1)?,
        agent_type: row.get(2)?,
        agent_ref: row.get(3)?,
        working_dir: row.get(4)?,
        repo: row.get(5)?,
        branch: row.get(6)?,
        status: row.get(7)?,
        activity: row.get(8)?,
        created_at: row.get(9)?,
        last_seen: row.get(10)?,
    })
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    let delivered: i64 = row.get(6)?;
    Ok(Message {
        id: row.get(0)?,
        sender_key: row.get(1)?,
        recipient_key: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
        state: row.get(5)?,
        delivered: delivered != 0,
        reply_to: row.get(7)?,
        thread_id: row.get(8)?,
    })
}

fn row_to_revocation(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRevocation> {
    Ok(StoredRevocation {
        team_id: row.get(0)?,
        peer_id: row.get(1)?,
        record_json: row.get(2)?,
        revoked_at: row.get(3)?,
    })
}

fn row_to_question(row: &rusqlite::Row<'_>) -> rusqlite::Result<Question> {
    Ok(Question {
        id: row.get(0)?,
        session_key: row.get(1)?,
        question: row.get(2)?,
        status: row.get(3)?,
        answer: row.get(4)?,
        created_at: row.get(5)?,
        answered_at: row.get(6)?,
    })
}

/// Shorten text to max characters for list views.
fn snippet(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}…")
    }
}

/// Current unix timestamp in seconds.
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Generate a unique, sortable id.
fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos:x}-{counter:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(store: &mut Store, key: &str, name: &str) -> Session {
        store
            .register(RegisterParams {
                session_key: Some(key.to_string()),
                name: name.to_string(),
                agent_type: Some("claude".into()),
                ..Default::default()
            })
            .expect("register")
    }

    #[test]
    fn registration_keeps_identity_and_is_idempotent() {
        let mut store = Store::open_in_memory().unwrap();
        let first = register(&mut store, "alice", "alice");
        assert_eq!(first.session_key, "alice");
        assert_eq!(first.status, "idle");

        let second = register(&mut store, "alice", "alice");
        assert_eq!(second.session_key, "alice");
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(store.list_sessions().unwrap().len(), 1);
    }

    #[test]
    fn duplicate_names_are_disambiguated() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "a", "worker");
        let second = register(&mut store, "b", "worker");
        assert_eq!(second.name, "worker-2");
    }

    #[test]
    fn messages_require_known_endpoints() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "a", "a");
        let err = store
            .send_message(SendMessageParams {
                reply_to: None,
                sender_key: "a".into(),
                recipient_key: "ghost".into(),
                body: "hi".into(),
            })
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn inbox_tracks_read_state() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "a", "a");
        register(&mut store, "b", "b");
        store
            .send_message(SendMessageParams {
                reply_to: None,
                sender_key: "a".into(),
                recipient_key: "b".into(),
                body: "hello".into(),
            })
            .unwrap();

        assert_eq!(store.unread_count("b").unwrap(), 1);
        let inbox = store
            .inbox(InboxParams {
                session_key: "b".into(),
                limit: None,
                unread_only: Some(true),
            })
            .unwrap();
        assert_eq!(inbox.len(), 1);

        let updated = store
            .mark_read(MarkReadParams {
                session_key: "b".into(),
                message_ids: None,
            })
            .unwrap();
        assert_eq!(updated, 1);
        assert_eq!(store.unread_count("b").unwrap(), 0);
    }

    // ---- Milestone 1 -----------------------------------------------------

    #[test]
    fn remote_messages_are_deduplicated_per_peer_and_id() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "bob", "bob");

        let first = store
            .store_incoming_remote("peer-1", "remote-1", "alice", "bob", "hi", 10, None, None)
            .unwrap();
        assert!(first.is_some());

        let again = store
            .store_incoming_remote("peer-1", "remote-1", "alice", "bob", "hi", 10, None, None)
            .unwrap();
        assert!(again.is_none(), "a retried message must be dropped");

        let inbox = store
            .inbox(InboxParams {
                session_key: "bob".into(),
                limit: None,
                unread_only: None,
            })
            .unwrap();
        assert_eq!(inbox.len(), 1);
    }

    #[test]
    fn the_same_remote_id_from_a_different_peer_is_a_separate_message() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "bob", "bob");
        assert!(
            store
                .store_incoming_remote("peer-1", "1", "a", "bob", "one", 10, None, None)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .store_incoming_remote("peer-2", "1", "b", "bob", "two", 10, None, None)
                .unwrap()
                .is_some()
        );
        assert_eq!(store.unread_count("bob").unwrap(), 2);
    }

    #[test]
    fn an_unknown_remote_recipient_is_rejected() {
        let mut store = Store::open_in_memory().unwrap();
        let error = store
            .store_incoming_remote("peer-1", "1", "a", "nobody", "hi", 10, None, None)
            .unwrap_err();
        assert!(matches!(error, Error::NotFound(_)));
    }

    #[test]
    fn outbound_messages_move_from_pending_to_acked() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "alice", "alice");
        let message = store
            .queue_remote_message("alice", "bob-agent", "hello", "peer-2", None)
            .unwrap();
        assert_eq!(store.pending_outbound().unwrap().len(), 1);

        store.set_delivery_state(&message.id, "acked").unwrap();
        assert!(store.pending_outbound().unwrap().is_empty());

        let outbox = store.list_outbound(10).unwrap();
        assert_eq!(outbox[0]["delivery_state"], "acked");
    }

    #[test]
    fn remote_sessions_are_replaced_per_peer() {
        let mut store = Store::open_in_memory().unwrap();
        let session = |key: &str, peer: &str| RemoteSession {
            repo: None,
            session_key: key.into(),
            peer_id: peer.into(),
            name: key.into(),
            agent_type: Some("claude".into()),
            status: "idle".into(),
            activity: String::new(),
            last_seen: 1,
        };

        store
            .replace_remote_sessions("peer-1", &[session("a", "peer-1"), session("b", "peer-1")])
            .unwrap();
        store
            .replace_remote_sessions("peer-2", &[session("c", "peer-2")])
            .unwrap();
        assert_eq!(store.list_remote_sessions().unwrap().len(), 3);
        assert_eq!(
            store.owner_of_session("a").unwrap().as_deref(),
            Some("peer-1")
        );

        // A fresh snapshot replaces the previous one for that peer only.
        store
            .replace_remote_sessions("peer-1", &[session("b", "peer-1")])
            .unwrap();
        let sessions = store.list_remote_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(store.owner_of_session("a").unwrap().is_none());
        assert!(store.owner_of_session("b").unwrap().is_some());
    }

    #[test]
    fn an_invitation_nonce_can_only_be_consumed_once() {
        let mut store = Store::open_in_memory().unwrap();
        assert!(store.consume_invite("nonce-1", "team-1").unwrap());
        assert!(!store.consume_invite("nonce-1", "team-1").unwrap());
        assert!(store.invite_used("nonce-1").unwrap());
        assert!(!store.invite_used("nonce-2").unwrap());
    }

    #[test]
    fn a_revocation_removes_the_member_and_is_recorded() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .upsert_team(&Team {
                team_id: "team-1".into(),
                name: "core".into(),
                admin_peer: "admin".into(),
                created_at: 1,
            })
            .unwrap();
        store
            .put_member(
                "{}",
                &StoredMember {
                    team_id: "team-1".into(),
                    peer_id: "peer-1".into(),
                    member_name: "laptop".into(),
                    record_json: "{}".into(),
                },
            )
            .unwrap();

        store
            .put_revocation(&StoredRevocation {
                team_id: "team-1".into(),
                peer_id: "peer-1".into(),
                record_json: "{}".into(),
                revoked_at: 2,
            })
            .unwrap();

        assert!(store.is_revoked("team-1", "peer-1").unwrap());
        assert!(store.is_revoked_anywhere("peer-1").unwrap());
        assert!(!store.is_revoked("team-1", "peer-2").unwrap());
        assert_eq!(store.list_revocations("team-1").unwrap().len(), 1);
        assert!(
            store.list_members("team-1").unwrap().is_empty(),
            "a revoked peer must leave the roster"
        );

        // Storing the same revocation twice is idempotent.
        store
            .put_revocation(&StoredRevocation {
                team_id: "team-1".into(),
                peer_id: "peer-1".into(),
                record_json: "{\"again\":true}".into(),
                revoked_at: 3,
            })
            .unwrap();
        assert_eq!(store.all_revocations().unwrap().len(), 1);
    }

    // ---- Milestone 4 -----------------------------------------------------

    fn send(store: &mut Store, from: &str, to: &str, body: &str) {
        store
            .send_message(SendMessageParams {
                reply_to: None,
                sender_key: from.into(),
                recipient_key: to.into(),
                body: body.into(),
            })
            .unwrap();
    }

    #[test]
    fn search_matches_body_substring_case_insensitively() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "a", "a");
        register(&mut store, "b", "b");
        send(&mut store, "a", "b", "Please review the Storage layer");
        send(&mut store, "b", "a", "done with tests");

        let hits = store.search_messages("storage", None, 50).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].body.contains("Storage"));

        // Scoped to a session that is not involved -> nothing.
        register(&mut store, "c", "c");
        assert!(
            store
                .search_messages("storage", Some("c"), 50)
                .unwrap()
                .is_empty()
        );
        // Scoped to an involved session -> found.
        assert_eq!(
            store
                .search_messages("storage", Some("a"), 50)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn search_treats_wildcards_as_literal() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "a", "a");
        register(&mut store, "b", "b");
        send(&mut store, "a", "b", "100% done");
        send(&mut store, "a", "b", "nothing here");

        // A bare `%` must not match every row.
        let hits = store.search_messages("100%", None, 50).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(store.search_messages("", None, 50).unwrap().is_empty());
    }

    #[test]
    fn conversation_returns_both_directions_in_order() {
        let mut store = Store::open_in_memory().unwrap();
        register(&mut store, "a", "a");
        register(&mut store, "b", "b");
        register(&mut store, "c", "c");
        send(&mut store, "a", "b", "one");
        send(&mut store, "b", "a", "two");
        send(&mut store, "a", "c", "unrelated");

        let thread = store.conversation("a", "b", 50).unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].body, "one");
        assert_eq!(thread[1].body, "two");

        let partners = store.conversation_partners("a").unwrap();
        assert_eq!(partners.len(), 2);
    }

    #[test]
    fn file_overlap_is_detected_only_for_concurrent_edits() {
        let mut store = Store::open_in_memory().unwrap();
        let now = now_seconds();
        let touch = |store: &mut Store, session: &str, path: &str, action: &str| {
            store
                .record_file_activity(&FileActivity {
                    session_key: session.into(),
                    repo: Some("app".into()),
                    path: path.into(),
                    action: action.into(),
                    updated_at: now,
                })
                .unwrap();
        };

        // Two sessions editing the same file -> overlap.
        touch(&mut store, "alice", "src/main.rs", "edit");
        touch(&mut store, "bob", "src/main.rs", "edit");
        // A file only one session edits -> no overlap.
        touch(&mut store, "alice", "src/lib.rs", "edit");
        // A read by a third session does not count as an overlapping edit.
        touch(&mut store, "carol", "src/util.rs", "read");
        touch(&mut store, "dave", "src/util.rs", "edit");

        let overlaps = store.file_overlaps(3600).unwrap();
        assert_eq!(overlaps.len(), 1, "only the shared edit overlaps");
        assert_eq!(overlaps[0].path, "src/main.rs");
        assert_eq!(overlaps[0].sessions.len(), 2);

        // Clearing one session removes the overlap.
        store.clear_file_activity("bob").unwrap();
        assert!(store.file_overlaps(3600).unwrap().is_empty());
    }

    #[test]
    fn file_overlap_respects_the_time_window() {
        let mut store = Store::open_in_memory().unwrap();
        let old = now_seconds() - 10_000;
        for session in ["alice", "bob"] {
            store
                .record_file_activity(&FileActivity {
                    session_key: session.into(),
                    repo: Some("app".into()),
                    path: "src/main.rs".into(),
                    action: "edit".into(),
                    updated_at: old,
                })
                .unwrap();
        }
        // The edits are outside a 60s window.
        assert!(store.file_overlaps(60).unwrap().is_empty());
        // But visible in a wide window.
        assert_eq!(store.file_overlaps(100_000).unwrap().len(), 1);
    }

    #[test]
    fn teams_and_members_round_trip() {
        let mut store = Store::open_in_memory().unwrap();
        let team = Team {
            team_id: "team-1".into(),
            name: "core".into(),
            admin_peer: "admin".into(),
            created_at: 1,
        };
        store.upsert_team(&team).unwrap();
        store
            .put_member(
                "{}",
                &StoredMember {
                    team_id: "team-1".into(),
                    peer_id: "peer-1".into(),
                    member_name: "laptop".into(),
                    record_json: "{}".into(),
                },
            )
            .unwrap();
        assert_eq!(store.list_teams().unwrap().len(), 1);
        assert_eq!(store.list_members("team-1").unwrap().len(), 1);

        assert!(store.delete_member("team-1", "peer-1").unwrap());
        assert!(store.list_members("team-1").unwrap().is_empty());
        assert!(store.delete_team("team-1").unwrap());
        assert!(store.get_team("team-1").unwrap().is_none());
    }
}
