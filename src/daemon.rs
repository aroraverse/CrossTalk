//! The local daemon.
//!
//! The daemon is the single owner of application state. It listens on the
//! local endpoint, dispatches protocol requests, runs the push delivery
//! worker, and hosts the peer-to-peer network node. No other component
//! touches the database.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use crate::config::Config;
use crate::delivery::{ClaudeBridge, ClaudeEvent};
use crate::error::{Error, Result};
use crate::identity::MachineIdentity;
use crate::ipc::{Connection, Listener};
use crate::net::{NetOptions, Network, NetworkDeps, NetworkEvent};
use crate::protocol::{
    AnswerQuestionParams, AskUserParams, ClaimMessagesParams, ConversationParams,
    ConversationsParams, FileOverlapsParams, GetMessageParams, InboxParams, ListQuestionsParams,
    MarkReadParams, Method, NetConnectParams, RecordFileActivityParams, RegisterParams, Request,
    ResolveRecipientParams, Response, SearchMessagesParams, SendMessageParams, SessionKeyParams,
    TeamCreateParams, TeamInviteParams, TeamJoinParams, TeamLeaveParams, TeamMembersParams,
    TeamRevokeParams, UpdateParams, WaitReplyParams,
};
use crate::resolve::{self, Candidate};
use crate::store::{FileActivity, Message, Store};

/// Version reported by the daemon.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How often the network worker retries outbound messages and reconnects.
const NET_TICK: Duration = Duration::from_secs(5);

/// Per-peer reconnection bookkeeping.
///
/// A dial can take longer than one tick (the handshake timeout is 15s), so an
/// `in_flight` flag is required to stop overlapping attempts to the same peer.
/// Failed attempts back off exponentially instead of hammering a dead address.
#[derive(Debug, Default)]
struct ReconnectState {
    in_flight: bool,
    failures: u32,
    next_attempt: Option<Instant>,
}

impl ReconnectState {
    /// True when a new dial may start now.
    fn should_attempt(&self, now: Instant) -> bool {
        if self.in_flight {
            return false;
        }
        match self.next_attempt {
            Some(next) => now >= next,
            None => true,
        }
    }

    /// Mark a dial as running.
    fn start(&mut self) {
        self.in_flight = true;
    }

    /// The peer answered: clear the backoff.
    fn succeeded(&mut self) {
        self.in_flight = false;
        self.failures = 0;
        self.next_attempt = None;
    }

    /// The dial failed: schedule the next attempt with exponential backoff.
    fn failed(&mut self, now: Instant) -> Duration {
        self.in_flight = false;
        self.failures = self.failures.saturating_add(1);
        let delay = reconnect_backoff(self.failures);
        self.next_attempt = Some(now + delay);
        delay
    }
}

/// Delay before the next dial attempt after `failures` consecutive failures.
///
/// 5s, 10s, 20s, 40s, then 60s for every further failure.
fn reconnect_backoff(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(4);
    let seconds = 5u64.saturating_mul(1u64 << exponent).min(60);
    Duration::from_secs(seconds)
}

/// The daemon.
pub struct Daemon {
    config: Config,
    store: Arc<Mutex<Store>>,
    /// The delivery bridge, owned by the push worker once the daemon serves.
    delivery: Mutex<Option<ClaudeBridge>>,
    /// Live agent sessions, for diagnostics.
    live_sessions: Arc<AtomicUsize>,
    events: Mutex<Option<mpsc::UnboundedReceiver<ClaudeEvent>>>,
    shutdown: tokio::sync::watch::Sender<bool>,
    notify: Arc<Notify>,
    started_at: Instant,
    identity: Arc<MachineIdentity>,
    /// The peer-to-peer node, once the daemon serves.
    network: Mutex<Option<Arc<Network>>>,
    network_events: Mutex<Option<mpsc::UnboundedReceiver<NetworkEvent>>>,
    /// One entry per peer being reconnected.
    reconnect: Mutex<HashMap<String, ReconnectState>>,
    /// The address the dashboard HTTP server actually bound to, once serving.
    dashboard_addr: Mutex<Option<std::net::SocketAddr>>,
}

impl Daemon {
    /// Open the database and prepare the daemon.
    pub fn new(config: Config) -> Result<Arc<Self>> {
        config.ensure_data_dir()?;
        let store = Store::open(&config.db_path)?;
        let identity = MachineIdentity::load_or_create(&config.data_dir)?;
        let (bridge, events) = ClaudeBridge::new();
        let live_sessions = bridge.live_counter();
        let (shutdown, _) = tokio::sync::watch::channel(false);

        Ok(Arc::new(Self {
            config,
            store: Arc::new(Mutex::new(store)),
            delivery: Mutex::new(Some(bridge)),
            live_sessions,
            events: Mutex::new(Some(events)),
            shutdown,
            notify: Arc::new(Notify::new()),
            started_at: Instant::now(),
            identity: Arc::new(identity),
            network: Mutex::new(None),
            network_events: Mutex::new(None),
            reconnect: Mutex::new(HashMap::new()),
            dashboard_addr: Mutex::new(None),
        }))
    }

    /// This machine's peer id.
    pub fn peer_id(&self) -> String {
        self.identity.peer_id_string()
    }

    /// The networking options in use.
    pub fn net_options(&self) -> NetOptions {
        self.config.net.clone()
    }

    /// The configuration in use.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The address the dashboard HTTP server bound to, once it is serving.
    pub fn dashboard_addr(&self) -> Option<std::net::SocketAddr> {
        *self
            .dashboard_addr
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Run the daemon until it is asked to stop.
    pub async fn serve(self: Arc<Self>) -> Result<()> {
        let mut listener = Listener::bind(&self.config.endpoint)?;
        let mut shutdown = self.shutdown.subscribe();

        eprintln!(
            "crosstalk daemon {VERSION} listening on {}",
            self.config.endpoint.display()
        );
        eprintln!("  data directory: {}", self.config.data_dir.display());
        eprintln!("  machine peer id: {}", self.identity.peer_id_string());

        let worker = {
            let bridge = self
                .delivery
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            let events = self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            match (bridge, events) {
                (Some(bridge), Some(events)) => {
                    Some(tokio::spawn(Arc::clone(&self).push_worker(bridge, events)))
                }
                _ => None,
            }
        };

        let net_worker = self.start_network().await;
        let dashboard_worker = self.start_dashboard().await;

        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                accepted = listener.accept() => {
                    let connection = accepted?;
                    let daemon = Arc::clone(&self);
                    tokio::spawn(async move {
                        daemon.handle_connection(connection).await;
                    });
                }
            }
        }

        eprintln!("crosstalk daemon stopped");
        if let Some(worker) = worker {
            worker.abort();
        }
        if let Some(worker) = net_worker {
            worker.abort();
        }
        if let Some(worker) = dashboard_worker {
            worker.abort();
        }
        if let Some(network) = self
            .network
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            network.shutdown();
        }
        Ok(())
    }

    /// Bind the peer-to-peer node and start its worker.
    async fn start_network(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.net.enabled {
            eprintln!("  peer-to-peer networking: disabled");
            return None;
        }

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let deps = Arc::new(NetworkDeps {
            store: Arc::clone(&self.store),
            identity: Arc::clone(&self.identity),
            machine_name: default_machine_name(),
        });

        let network = match Network::bind(deps, self.config.net.clone(), events_tx).await {
            Ok(network) => network,
            Err(error) => {
                eprintln!("  peer-to-peer networking: unavailable ({error})");
                return None;
            }
        };

        eprintln!(
            "  peer-to-peer endpoint: {}",
            serde_json::to_string(&network.addr()).unwrap_or_default()
        );
        *self
            .network
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(network);
        *self
            .network_events
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(events_rx);

        let receiver = self
            .network_events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();

        receiver.map(|receiver| tokio::spawn(Arc::clone(self).net_worker(receiver)))
    }

    /// Bind the local web dashboard and start serving it.
    ///
    /// Loopback-only HTTP. A failure to bind (for example the port is already
    /// taken by another daemon) is logged and the daemon keeps running without
    /// the dashboard.
    async fn start_dashboard(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let Some(addr) = self.config.dashboard_addr.clone() else {
            eprintln!("  dashboard: disabled");
            return None;
        };
        match crate::dashboard::bind(&addr).await {
            Ok((listener, local)) => {
                eprintln!("  dashboard: http://{local}");
                *self
                    .dashboard_addr
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(local);
                let daemon = Arc::clone(self);
                Some(tokio::spawn(crate::dashboard::serve(daemon, listener)))
            }
            Err(error) => {
                eprintln!(
                    "  dashboard: unavailable ({error}) — set {} to change the address",
                    crate::config::ENV_DASHBOARD_ADDR
                );
                None
            }
        }
    }

    /// The network node, when networking is running.
    fn network(&self) -> Option<Arc<Network>> {
        self.network
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Signal the daemon to stop.
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
        self.notify.notify_one();
    }

    /// Serve one client connection until it closes.
    async fn handle_connection(self: Arc<Self>, mut connection: Connection) {
        loop {
            match connection.read_frame::<Request>().await {
                Ok(Some(request)) => {
                    let method = request.method;
                    let response = self.dispatch(request).await;
                    if method == Method::Shutdown && response.ok {
                        self.stop();
                    }
                    if connection.write_frame(&response).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let response = Response::fail(None, error.code(), error.to_string());
                    let _ = connection.write_frame(&response).await;
                    break;
                }
            }
        }
    }

    /// Execute a single request.
    pub async fn dispatch(&self, request: Request) -> Response {
        let id = request.id.clone();
        let params = request.params.clone();
        match self.execute(request.method, params).await {
            Ok(result) => Response::ok(id, result),
            Err(error) => Response::fail(id, error.code(), error.to_string()),
        }
    }

    async fn execute(&self, method: Method, params: Value) -> Result<Value> {
        match method {
            Method::Status => self.status(),
            Method::Register => {
                let params: RegisterParams = parse_params(params)?;
                let session = self.with_store(|store| store.register(params))?;
                self.notify.notify_one();
                self.broadcast_presence_if_connected();
                Ok(serde_json::to_value(session)?)
            }
            Method::Update => {
                let params: UpdateParams = parse_params(params)?;
                let session = self.with_store(|store| store.update(params))?;
                self.broadcast_presence_if_connected();
                Ok(serde_json::to_value(session)?)
            }
            Method::ListSessions => {
                let sessions = self.with_store(|store| store.list_sessions())?;
                Ok(json!({ "sessions": sessions }))
            }
            Method::GetSession => {
                let params: SessionKeyParams = parse_params(params)?;
                let session = self
                    .with_store(|store| store.get_session(&params.session_key))?
                    .ok_or_else(|| Error::NotFound(format!("session {}", params.session_key)))?;
                Ok(serde_json::to_value(session)?)
            }
            Method::RemoveSession => {
                let params: SessionKeyParams = parse_params(params)?;
                let removed = self.with_store(|store| store.remove_session(&params.session_key))?;
                self.broadcast_presence_if_connected();
                Ok(json!({ "removed": removed }))
            }
            Method::SendMessage => {
                let params: SendMessageParams = parse_params(params)?;
                self.send_message(params)
            }
            Method::Inbox => {
                let params: InboxParams = parse_params(params)?;
                let key = params.session_key.clone();
                let messages = self.with_store(|store| {
                    let messages = store.inbox(params)?;
                    // Reading the inbox is also a heartbeat.
                    store.touch(&key)?;
                    Ok(messages)
                })?;
                Ok(json!({ "messages": messages }))
            }
            Method::UnreadCount => {
                let params: SessionKeyParams = parse_params(params)?;
                let unread = self.with_store(|store| store.unread_count(&params.session_key))?;
                Ok(json!({ "unread": unread }))
            }
            Method::MarkRead => {
                let params: MarkReadParams = parse_params(params)?;
                let updated = self.with_store(|store| store.mark_read(params))?;
                Ok(json!({ "updated": updated }))
            }
            Method::AskUser => {
                let params: AskUserParams = parse_params(params)?;
                let question = self.with_store(|store| store.ask_user(params))?;
                Ok(serde_json::to_value(question)?)
            }
            Method::ListQuestions => {
                let params: ListQuestionsParams = parse_params(params)?;
                let questions =
                    self.with_store(|store| store.question_view(params.status.as_deref()))?;
                Ok(json!({ "questions": questions }))
            }
            Method::AnswerQuestion => {
                let params: AnswerQuestionParams = parse_params(params)?;
                let question = self.with_store(|store| store.answer_question(params))?;
                self.notify.notify_one();
                Ok(serde_json::to_value(question)?)
            }
            Method::Shutdown => Ok(json!({ "stopping": true })),
            Method::Diagnostics => self.diagnostics(),
            Method::NetInfo => self.net_info(),
            Method::TeamCreate => {
                let params: TeamCreateParams = parse_params(params)?;
                let network = self.require_network()?;
                let team = network.create_team(&params.name)?;
                Ok(serde_json::to_value(team)?)
            }
            Method::TeamInvite => {
                let params: TeamInviteParams = parse_params(params)?;
                let network = self.require_network()?;
                let invitation =
                    network.invite(&params.team_id, params.ttl_seconds.unwrap_or(3600))?;
                Ok(json!({
                    "token": invitation.to_token()?,
                    "team_id": invitation.team_id,
                    "expires_at": invitation.expires_at,
                    "admin_peer": invitation.admin_peer_id,
                }))
            }
            Method::TeamJoin => {
                let params: TeamJoinParams = parse_params(params)?;
                let network = self.require_network()?;
                let team = network.join_team(&params.token).await?;
                network.broadcast_presence();
                Ok(serde_json::to_value(team)?)
            }
            Method::TeamList => {
                let teams = self.with_store(|store| store.list_teams())?;
                let me = self.peer_id();
                let mut out = Vec::new();
                for team in teams {
                    let members = self.with_store(|store| store.list_members(&team.team_id))?;
                    out.push(json!({
                        "team_id": team.team_id,
                        "name": team.name,
                        "admin_peer": team.admin_peer,
                        "created_at": team.created_at,
                        "is_admin": team.admin_peer == me,
                        "members": members.len(),
                    }));
                }
                Ok(json!({ "teams": out }))
            }
            Method::TeamMembers => {
                let params: TeamMembersParams = parse_params(params)?;
                let members = self.with_store(|store| store.list_members(&params.team_id))?;
                let me = self.peer_id();
                let network = self.network();
                let out: Vec<Value> = members
                    .into_iter()
                    .map(|member| {
                        let connected = network
                            .as_ref()
                            .map(|network| network.is_connected(&member.peer_id))
                            .unwrap_or(false);
                        json!({
                            "peer_id": member.peer_id,
                            "member_name": member.member_name,
                            "self": member.peer_id == me,
                            "connected": connected,
                        })
                    })
                    .collect();
                let revoked: Vec<String> = self
                    .with_store(|store| store.list_revocations(&params.team_id))?
                    .into_iter()
                    .map(|record| record.peer_id)
                    .collect();
                Ok(json!({
                    "team_id": params.team_id,
                    "members": out,
                    "revoked": revoked,
                }))
            }
            Method::TeamLeave => {
                let params: TeamLeaveParams = parse_params(params)?;
                let me = self.peer_id();
                let members = self.with_store(|store| store.list_members(&params.team_id))?;
                let removed = self.with_store(|store| store.delete_team(&params.team_id))?;
                // Former teammates are no longer part of the directory.
                for member in members.iter().filter(|member| member.peer_id != me) {
                    self.with_store(|store| store.clear_remote_sessions(&member.peer_id))?;
                    self.with_store(|store| store.set_peer_status(&member.peer_id, "left"))?;
                }
                Ok(json!({ "removed": removed }))
            }
            Method::TeamRevoke => {
                let params: TeamRevokeParams = parse_params(params)?;
                let network = self.require_network()?;
                let record = network.revoke(&params.team_id, &params.peer_id)?;
                self.with_store(|store| store.clear_remote_sessions(&params.peer_id))?;
                self.with_store(|store| store.set_peer_status(&params.peer_id, "revoked"))?;
                Ok(json!({
                    "team_id": record.team_id,
                    "peer_id": record.peer_id,
                    "issued_at": record.issued_at,
                    "revoked": true,
                }))
            }
            Method::PeerList => {
                let peers = self.with_store(|store| store.list_peers())?;
                let network = self.network();
                let out: Vec<Value> = peers
                    .into_iter()
                    .map(|peer| {
                        let connected = network
                            .as_ref()
                            .map(|network| network.is_connected(&peer.peer_id))
                            .unwrap_or(false);
                        let status = if connected {
                            "connected"
                        } else if matches!(peer.status.as_str(), "left" | "connecting" | "revoked")
                        {
                            peer.status.as_str()
                        } else {
                            "offline"
                        };
                        json!({
                            "peer_id": peer.peer_id,
                            "label": peer.label,
                            "status": status,
                            "connected": connected,
                            "last_seen": peer.last_seen,
                        })
                    })
                    .collect();
                Ok(json!({ "peers": out }))
            }
            Method::NetConnect => {
                let params: NetConnectParams = parse_params(params)?;
                let addr: iroh::EndpointAddr =
                    serde_json::from_str(&params.addr).map_err(|error| {
                        Error::InvalidRequest(format!("invalid peer address: {error}"))
                    })?;
                let network = self.require_network()?;
                let peer = network.connect(addr).await?;
                Ok(json!({ "peer_id": peer, "connected": true }))
            }
            Method::Directory => self.directory(),
            Method::Outbox => {
                let messages = self.with_store(|store| store.list_outbound(100))?;
                Ok(json!({ "messages": messages }))
            }
            Method::SearchMessages => {
                let params: SearchMessagesParams = parse_params(params)?;
                if params.context.unwrap_or(false) {
                    let hits = self.with_store(|store| {
                        store.search_view(
                            &params.query,
                            params.session_key.as_deref(),
                            params.sender.as_deref(),
                            params.since,
                            params.limit.unwrap_or(100),
                        )
                    })?;
                    return Ok(json!({ "messages": hits }));
                }
                let messages = self.with_store(|store| {
                    store.search_messages(
                        &params.query,
                        params.session_key.as_deref(),
                        params.limit.unwrap_or(100),
                    )
                })?;
                Ok(json!({ "messages": messages }))
            }
            Method::Conversation => {
                let params: ConversationParams = parse_params(params)?;
                let messages = self.with_store(|store| {
                    store.conversation_view(
                        &params.session_key,
                        &params.with,
                        params.limit.unwrap_or(500),
                    )
                })?;
                Ok(json!({ "messages": messages }))
            }
            Method::Conversations => {
                let params: ConversationsParams = parse_params(params)?;
                let threads =
                    self.with_store(|store| store.conversation_partners(&params.session_key))?;
                Ok(json!({ "conversations": threads }))
            }
            Method::RecordFileActivity => {
                let params: RecordFileActivityParams = parse_params(params)?;
                let activity = FileActivity {
                    session_key: params.session_key,
                    repo: params.repo,
                    path: params.path,
                    action: params.action.unwrap_or_else(|| "edit".to_string()),
                    updated_at: crate::store::now_seconds(),
                };
                self.with_store(|store| store.record_file_activity(&activity))?;
                let overlaps = self.with_store(|store| store.file_overlaps(900))?;
                Ok(json!({ "recorded": true, "overlaps": overlaps }))
            }
            Method::FileOverlaps => {
                let params: FileOverlapsParams = parse_params(params)?;
                let overlaps = self.with_store(|store| {
                    store.file_overlaps(params.window_seconds.unwrap_or(900))
                })?;
                Ok(json!({ "overlaps": overlaps }))
            }
            Method::Overview => self.overview(),
            Method::SendAsHuman => {
                let params: crate::protocol::SendAsHumanParams = parse_params(params)?;
                let message = self.with_store(|store| {
                    store.send_as_human(
                        &params.recipient_key,
                        &params.body,
                        params.reply_to.as_deref(),
                    )
                })?;
                self.notify.notify_one();
                Ok(message_with_delivery(&message, "local", None))
            }
            Method::AllConversations => {
                let pairs = self.with_store(|store| store.conversation_pairs(200))?;
                Ok(json!({ "conversations": pairs }))
            }
            Method::SetupStatus => self.setup_report(None),
            Method::SetupApply => {
                let params: crate::protocol::SetupApplyParams = parse_params(params)?;
                let agent = crate::agents::Agent::parse(&params.agent)?;
                self.setup_report(Some(agent))
            }
            Method::ResolveRecipient => {
                let params: ResolveRecipientParams = parse_params(params)?;
                let (candidate, scope) =
                    self.resolve_recipient(params.session_key.as_deref(), &params.selector)?;
                Ok(json!({ "session": candidate, "matched_by": scope }))
            }
            Method::WaitReply => {
                let params: WaitReplyParams = parse_params(params)?;
                self.wait_reply(params).await
            }
            Method::ClaimMessages => {
                let params: ClaimMessagesParams = parse_params(params)?;
                let messages = self.with_store(|store| {
                    let limit = params.limit.unwrap_or(5).clamp(1, 50);
                    let messages = store.inbox(InboxParams {
                        session_key: params.session_key.clone(),
                        limit: Some(limit),
                        unread_only: Some(true),
                    })?;
                    let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
                    if !ids.is_empty() {
                        store.mark_read(MarkReadParams {
                            session_key: params.session_key.clone(),
                            message_ids: Some(ids),
                        })?;
                    }
                    store.touch(&params.session_key)?;
                    Ok(messages)
                })?;
                let remaining = self.with_store(|store| store.unread_count(&params.session_key))?;
                Ok(json!({ "messages": messages, "remaining_unread": remaining }))
            }
            Method::GetMessage => {
                let params: GetMessageParams = parse_params(params)?;
                let message = self
                    .with_store(|store| store.get_message(&params.message_id))?
                    .ok_or_else(|| Error::NotFound(format!("message {}", params.message_id)))?;
                Ok(serde_json::to_value(message)?)
            }
            Method::AnchorStatus => self.anchor_status(),
            Method::SetAnchor => {
                let params: crate::protocol::SetAnchorParams = parse_params(params)?;
                self.with_store(|store| {
                    store.meta_set("anchor", if params.enabled { "1" } else { "0" })
                })?;
                self.anchor_status()
            }
        }
    }

    /// The anchor role of this machine.
    ///
    /// An anchor is a machine designated to stay on (through the background
    /// service) so the team always has one member holding the shared message
    /// history and able to receive mail while others are offline. The messages
    /// themselves are already persisted by the store and retried by the
    /// network worker; the anchor flag records the intent and lets the
    /// dashboard and `doctor` point at the machine the team can rely on.
    fn anchor_status(&self) -> Result<Value> {
        let enabled = self
            .with_store(|store| store.meta_get("anchor"))?
            .map(|value| value == "1")
            .unwrap_or(false);
        let (sessions, messages) = self.with_store(|store| store.counts())?;
        // The service keeps the daemon alive at login; report whether it is
        // installed so users know the anchor will actually stay on.
        Ok(json!({
            "anchor": enabled,
            "peer_id": self.peer_id(),
            "messages_retained": messages,
            "sessions": sessions,
            "note": if enabled {
                "this machine is the team anchor: it retains team history and receives mail while \
        other machines are offline. Keep the background service installed so it stays on."
            } else {
                "this machine is not an anchor. Designate one always-on machine as the anchor so \
        the team keeps a mailbox and searchable history when others are offline."
            },
        }))
    }

    /// A single aggregated snapshot for the dashboard: status, teams, peers,
    /// the session directory, recent outbound messages and file overlaps. One
    /// call keeps the dashboard's polling cheap.
    fn overview(&self) -> Result<Value> {
        let status = self.status()?;
        let net = self.net_info()?;
        let directory = self.directory()?;
        let teams = self.execute_sync(Method::TeamList)?;
        let peers = self.execute_sync(Method::PeerList)?;
        let outbox = self.with_store(|store| store.list_outbound(50))?;
        let overlaps = self.with_store(|store| store.file_overlaps(900))?;
        let questions = self.with_store(|store| store.list_questions(Some("pending")))?;
        let anchor = self.anchor_status()?;
        Ok(json!({
            "status": status,
            "network": net,
            "sessions": directory.get("sessions").cloned().unwrap_or(Value::Null),
            "teams": teams.get("teams").cloned().unwrap_or(Value::Null),
            "peers": peers.get("peers").cloned().unwrap_or(Value::Null),
            "outbox": outbox,
            "overlaps": overlaps,
            "questions": questions,
            "anchor": anchor,
        }))
    }

    /// Run one of the synchronous, side-effect-free read methods the overview
    /// aggregates. Keeps the overview from duplicating their JSON shaping.
    fn execute_sync(&self, method: Method) -> Result<Value> {
        match method {
            Method::TeamList => {
                let teams = self.with_store(|store| store.list_teams())?;
                let me = self.peer_id();
                let mut out = Vec::new();
                for team in teams {
                    let members = self.with_store(|store| store.list_members(&team.team_id))?;
                    out.push(json!({
                        "team_id": team.team_id,
                        "name": team.name,
                        "admin_peer": team.admin_peer,
                        "created_at": team.created_at,
                        "is_admin": team.admin_peer == me,
                        "members": members.len(),
                    }));
                }
                Ok(json!({ "teams": out }))
            }
            Method::PeerList => {
                let peers = self.with_store(|store| store.list_peers())?;
                let network = self.network();
                let out: Vec<Value> = peers
                    .into_iter()
                    .map(|peer| {
                        let connected = network
                            .as_ref()
                            .map(|network| network.is_connected(&peer.peer_id))
                            .unwrap_or(false);
                        let status = if connected {
                            "connected"
                        } else if matches!(peer.status.as_str(), "left" | "connecting" | "revoked")
                        {
                            peer.status.as_str()
                        } else {
                            "offline"
                        };
                        json!({
                            "peer_id": peer.peer_id,
                            "label": peer.label,
                            "status": status,
                            "connected": connected,
                            "last_seen": peer.last_seen,
                        })
                    })
                    .collect();
                Ok(json!({ "peers": out }))
            }
            other => Err(Error::Other(format!("{other:?} is not an aggregate read"))),
        }
    }
    /// Resolve a message: locally when the recipient is local, otherwise
    /// through the team directory to the owning machine.
    fn send_message(&self, mut params: SendMessageParams) -> Result<Value> {
        // The recipient may be an exact session key or a looser selector; it is
        // resolved to exactly one session or the send fails explicitly.
        let (resolved, scope) =
            self.resolve_recipient(Some(&params.sender_key), &params.recipient_key)?;
        params.recipient_key = resolved.session_key.clone();
        let mut result = self.deliver_message(params)?;
        if scope != crate::resolve::Scope::ExactKey
            && let Some(object) = result.as_object_mut()
        {
            object.insert("resolved_recipient".into(), json!(resolved));
            object.insert("matched_by".into(), json!(scope));
        }
        Ok(result)
    }

    fn deliver_message(&self, params: SendMessageParams) -> Result<Value> {
        let local = self.with_store(|store| store.get_session(&params.recipient_key))?;
        if local.is_some() {
            let message = self.with_store(|store| {
                store.send_message(SendMessageParams {
                    sender_key: params.sender_key.clone(),
                    recipient_key: params.recipient_key.clone(),
                    body: params.body.clone(),
                    reply_to: params.reply_to.clone(),
                })
            })?;
            self.notify.notify_one();
            return Ok(message_with_delivery(&message, "local", None));
        }

        let owner = self
            .with_store(|store| store.owner_of_session(&params.recipient_key))?
            .ok_or_else(|| {
                Error::NotFound(format!("recipient session {}", params.recipient_key))
            })?;

        let message = self.with_store(|store| {
            store.queue_remote_message(
                &params.sender_key,
                &params.recipient_key,
                &params.body,
                &owner,
                params.reply_to.as_deref(),
            )
        })?;

        let state = match self.network() {
            Some(network) if network.is_connected(&owner) => {
                match self.transmit(&network, &message, &owner) {
                    Ok(()) => "sent",
                    Err(error) => {
                        eprintln!("[daemon] message {} queued: {error}", message.id);
                        "pending"
                    }
                }
            }
            Some(_) => "pending",
            None => "pending",
        };
        if state != "pending" {
            self.with_store(|store| store.set_delivery_state(&message.id, state))?;
        }
        Ok(message_with_delivery(&message, state, Some(&owner)))
    }

    /// Push one message onto a peer connection.
    fn transmit(&self, network: &Arc<Network>, message: &Message, peer: &str) -> Result<()> {
        use crate::net::proto::{Body, Frame, MsgFrame};
        network.send(
            peer,
            Frame::new(Body::Msg(MsgFrame {
                id: message.id.clone(),
                sender_session: message.sender_key.clone(),
                recipient_session: message.recipient_key.clone(),
                body: message.body.clone(),
                created_at: message.created_at,
                reply_to: message.reply_to.clone(),
                thread_id: message.thread_id.clone(),
            })),
        )
    }

    /// Push the local session directory to peers, when connected.
    fn broadcast_presence_if_connected(&self) {
        if let Some(network) = self.network() {
            network.broadcast_presence();
        }
    }

    fn require_network(&self) -> Result<Arc<Network>> {
        self.network()
            .ok_or_else(|| Error::Other("peer-to-peer networking is disabled".into()))
    }

    fn net_info(&self) -> Result<Value> {
        let network = self.network();
        let teams = self.with_store(|store| store.list_teams())?;
        let reconnecting = self
            .reconnect
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|state| state.in_flight)
            .count();
        Ok(json!({
            "enabled": self.config.net.enabled,
            "running": network.is_some(),
            "peer_id": self.peer_id(),
            "addr": network.as_ref().map(|network| network.addr()),
            "connected": network
                .as_ref()
                .map(|network| network.connected_peers())
                .unwrap_or_default(),
            "teams": teams.len(),
            "relay": self.config.net.relay,
            "discovery": self.config.net.discovery,
            "replacements": network.as_ref().map(|network| network.replacements()).unwrap_or(0),
            "reconnecting": reconnecting,
            "protocol": {
                "alpn": String::from_utf8_lossy(crate::net::proto::ALPN),
                "version": crate::net::proto::PROTOCOL_VERSION,
            },
        }))
    }

    /// Inspect (`apply = None`) or apply agent configuration. Applying always
    /// backs the existing files up first; it only touches the files setup
    /// manages.
    fn setup_report(&self, apply: Option<crate::agents::Agent>) -> Result<Value> {
        let pinned = std::env::var_os(crate::config::ENV_DATA_DIR)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from);
        let host = crate::agents::Host::from_env(pinned)?;
        let selected: Vec<crate::agents::Agent> = match apply {
            Some(agent) => vec![agent],
            None => crate::agents::Agent::ALL.to_vec(),
        };
        let report = crate::agents::run_setup(&host, &selected, apply.is_none());
        Ok(report.to_json())
    }

    /// Every addressable session, local and remote, with derived liveness.
    fn candidates(&self) -> Result<Vec<Candidate>> {
        let now = crate::store::now_seconds();
        let me = self.peer_id();
        let mut out: Vec<Candidate> = self
            .with_store(|store| store.list_sessions())?
            .into_iter()
            .map(|session| Candidate {
                liveness: resolve::liveness(&session.status, session.last_seen, now),
                provider: resolve::provider_of(session.agent_type.as_deref()),
                session_key: session.session_key,
                name: session.name,
                agent_type: session.agent_type,
                machine: me.clone(),
                local: true,
                working_dir: session.working_dir,
                repo: session.repo,
                branch: session.branch,
                status: session.status,
                activity: session.activity,
                last_seen: session.last_seen,
                reachable: true,
            })
            .collect();
        let network = self.network();
        for session in self.with_store(|store| store.list_remote_sessions())? {
            let reachable = network
                .as_ref()
                .map(|network| network.is_connected(&session.peer_id))
                .unwrap_or(false);
            out.push(Candidate {
                liveness: resolve::liveness(&session.status, session.last_seen, now),
                provider: resolve::provider_of(session.agent_type.as_deref()),
                session_key: session.session_key,
                name: session.name,
                agent_type: session.agent_type,
                machine: session.peer_id,
                local: false,
                working_dir: None,
                repo: session.repo,
                branch: None,
                status: session.status,
                activity: session.activity,
                last_seen: session.last_seen,
                reachable,
            });
        }
        Ok(out)
    }

    /// Resolve a recipient selector for `caller` (a session key), or fail with
    /// a precise error: not found, or ambiguous.
    fn resolve_recipient(
        &self,
        caller: Option<&str>,
        selector: &str,
    ) -> Result<(Candidate, resolve::Scope)> {
        let candidates = self.candidates()?;
        let caller = match caller.map(str::trim).filter(|key| !key.is_empty()) {
            Some(key) => {
                self.with_store(|store| store.get_session(key))?
                    .map(|s| resolve::Caller {
                        provider: resolve::provider_of(s.agent_type.as_deref()),
                        session_key: s.session_key,
                        working_dir: s.working_dir,
                        repo: s.repo,
                    })
            }
            None => None,
        };
        match resolve::resolve(selector, caller.as_ref(), &candidates) {
            resolve::Resolution::Unique(found, scope) => Ok((*found, scope)),
            resolve::Resolution::Ambiguous(found) => Err(Error::Ambiguous(
                resolve::ambiguity_message(selector, &found),
            )),
            resolve::Resolution::NoMatch(reason) => Err(Error::NotFound(format!(
                "recipient session {selector}: {reason}"
            ))),
        }
    }

    /// Local and remote sessions in one directory.
    fn directory(&self) -> Result<Value> {
        let entries: Vec<Value> = self
            .candidates()?
            .into_iter()
            .map(|c| {
                json!({
                    "session_key": c.session_key,
                    "name": c.name,
                    "agent_type": c.agent_type,
                    "provider": c.provider,
                    "status": c.status,
                    "liveness": c.liveness,
                    "activity": c.activity,
                    "last_seen": c.last_seen,
                    "local": c.local,
                    "machine": c.machine,
                    "working_dir": c.working_dir,
                    "repo": c.repo,
                    "branch": c.branch,
                    "reachable": c.reachable,
                })
            })
            .collect();
        Ok(json!({ "sessions": entries }))
    }

    /// Wait for the reply to a message, without polling the database.
    ///
    /// Subscribes to the store's message events first, then checks, then
    /// sleeps until a message arrives or the deadline passes. Only a message
    /// whose `reply_to` is the awaited id is returned; everything else is
    /// ignored. A timeout is reported as such, with what is known about the
    /// original message and the recipient.
    async fn wait_reply(&self, params: WaitReplyParams) -> Result<Value> {
        let timeout = Duration::from_secs(params.timeout_seconds.unwrap_or(30).clamp(1, 900));
        let deadline = tokio::time::Instant::now() + timeout;
        let mark_read = params.mark_read.unwrap_or(true);
        let mut events = self.with_store(|store| Ok(store.subscribe()))?;
        loop {
            let found = self.with_store(|store| {
                store.find_reply(
                    &params.session_key,
                    &params.message_id,
                    params.from.as_deref(),
                )
            })?;
            if let Some(reply) = found {
                if mark_read {
                    let id = reply.id.clone();
                    self.with_store(|store| {
                        store.mark_read(MarkReadParams {
                            session_key: params.session_key.clone(),
                            message_ids: Some(vec![id]),
                        })
                    })?;
                }
                let sender = self
                    .candidates()?
                    .into_iter()
                    .find(|c| c.session_key == reply.sender_key);
                return Ok(json!({
                    "status": "reply",
                    "reply": reply,
                    "from": sender,
                    "in_reply_to": params.message_id,
                    "waited_seconds": (timeout - deadline.saturating_duration_since(tokio::time::Instant::now())).as_secs(),
                }));
            }
            match tokio::time::timeout_at(deadline, events.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => break,
                Err(_) => break,
            }
        }
        let original = self.with_store(|store| store.get_message(&params.message_id))?;
        let recipient = match &original {
            Some(message) => self
                .candidates()?
                .into_iter()
                .find(|c| c.session_key == message.recipient_key),
            None => None,
        };
        Ok(json!({
            "status": "timeout",
            "in_reply_to": params.message_id,
            "waited_seconds": timeout.as_secs(),
            "original_message": original,
            "recipient": recipient,
            "note": "No reply arrived. The message was stored; the recipient may not have read it yet \
        (see original_message.state and recipient.liveness). Do not assume agreement.",
        }))
    }
    fn status(&self) -> Result<Value> {
        let (sessions, messages) = self.with_store(|store| store.counts())?;
        Ok(json!({
            "daemon": "crosstalk",
            "version": VERSION,
            "pid": std::process::id(),
            "endpoint": self.config.endpoint.display(),
            "uptime_seconds": self.started_at.elapsed().as_secs(),
            "sessions": sessions,
            "messages": messages,
        }))
    }

    fn diagnostics(&self) -> Result<Value> {
        let (sessions, messages) = self.with_store(|store| store.counts())?;
        let binary = std::env::var_os(crate::config::ENV_CLAUDE_BIN)
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "claude".to_string());
        let network = self.network();
        Ok(json!({
            "daemon": "crosstalk",
            "version": VERSION,
            "pid": std::process::id(),
            "endpoint": self.config.endpoint.display(),
            "data_dir": self.config.data_dir.display().to_string(),
            "db_path": self.config.db_path.display().to_string(),
            "uptime_seconds": self.started_at.elapsed().as_secs(),
            "sessions": sessions,
            "messages": messages,
            "delivery": {
                "provider": "claude",
                "binary": binary,
                "live_sessions": self.live_sessions.load(Ordering::Relaxed),
            },
            "network": {
                "enabled": self.config.net.enabled,
                "running": network.is_some(),
                "peer_id": self.peer_id(),
                "addr": network.as_ref().map(|network| network.addr()),
                "connected": network
                    .as_ref()
                    .map(|network| network.connected_peers())
                    .unwrap_or_default(),
                "relay": self.config.net.relay,
                "discovery": self.config.net.discovery,
            },
        }))
    }

    /// Run a closure with the store locked.
    fn with_store<T>(&self, action: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        let mut store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        action(&mut store)
    }

    /// React to network events and keep remote delivery moving.
    async fn net_worker(self: Arc<Self>, mut events: mpsc::UnboundedReceiver<NetworkEvent>) {
        let mut ticker = tokio::time::interval(NET_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut ticks: u64 = 0;

        loop {
            tokio::select! {
                event = events.recv() => {
                    match event {
                        Some(event) => self.handle_network_event(&event),
                        None => return,
                    }
                }
                _ = ticker.tick() => {
                    ticks = ticks.wrapping_add(1);
                    self.retry_outbound();
                    self.reconnect_peers();
                    if ticks.is_multiple_of(3)
                        && let Some(network) = self.network()
                    {
                        network.broadcast_presence();
                    }
                }
            }
        }
    }

    fn handle_network_event(&self, event: &NetworkEvent) {
        match event {
            NetworkEvent::PeerUp { peer_id, machine } => {
                self.reset_reconnect_state(peer_id);
                eprintln!("[daemon] peer {peer_id} ({machine}) connected");
            }
            NetworkEvent::PeerDown { peer_id } => {
                eprintln!("[daemon] peer {peer_id} disconnected");
            }
            NetworkEvent::Presence { peer_id, sessions } => {
                eprintln!(
                    "[daemon] peer {peer_id} reported {} session(s)",
                    sessions.len()
                );
            }
            NetworkEvent::Message {
                peer_id,
                message_id,
                recipient_session,
                duplicate,
            } => {
                if *duplicate {
                    return;
                }
                eprintln!("[daemon] received {message_id} from {peer_id} for {recipient_session}");
                // Hand the message to the local push delivery worker.
                self.notify.notify_one();
            }
            NetworkEvent::Ack {
                peer_id,
                message_id,
                accepted,
                error,
            } => {
                let state = if *accepted { "acked" } else { "failed" };
                if let Err(store_error) =
                    self.with_store(|store| store.set_delivery_state(message_id, state))
                {
                    eprintln!("[daemon] cannot record acknowledgement: {store_error}");
                }
                match error {
                    Some(error) => {
                        eprintln!("[daemon] {peer_id} rejected message {message_id}: {error}")
                    }
                    None => eprintln!("[daemon] {peer_id} acknowledged {message_id}"),
                }
            }
            NetworkEvent::Error { peer_id, message } => {
                eprintln!(
                    "[daemon] network error{}: {message}",
                    peer_id
                        .as_ref()
                        .map(|peer| format!(" ({peer})"))
                        .unwrap_or_default()
                );
            }
        }
    }

    /// Re-send outgoing messages that have not been acknowledged yet.
    fn retry_outbound(&self) {
        let Some(network) = self.network() else {
            return;
        };
        let pending = match self.with_store(|store| store.pending_outbound()) {
            Ok(pending) => pending,
            Err(error) => {
                eprintln!("[daemon] cannot read outbound queue: {error}");
                return;
            }
        };

        for outbound in pending {
            let Some(peer) = outbound.peer_id.as_deref() else {
                continue;
            };
            if !network.is_connected(peer) {
                continue;
            }
            let message = Message {
                id: outbound.id.clone(),
                sender_key: outbound.sender_key.clone(),
                recipient_key: outbound.recipient_key.clone(),
                body: outbound.body.clone(),
                created_at: outbound.created_at,
                state: "sent".into(),
                delivered: false,
                reply_to: outbound.reply_to.clone(),
                thread_id: outbound.thread_id.clone(),
            };
            match self.transmit(&network, &message, peer) {
                Ok(()) => {
                    self.with_store(|store| store.set_delivery_state(&outbound.id, "sent"))
                        .ok();
                }
                Err(error) => {
                    eprintln!("[daemon] retry for {} failed: {error}", outbound.id);
                }
            }
        }
    }

    /// Try to reach known team peers that are currently disconnected.
    ///
    /// One attempt per peer at a time, spaced by [`reconnect_backoff`].
    fn reconnect_peers(self: &Arc<Self>) {
        let Some(network) = self.network() else {
            return;
        };
        let now = Instant::now();

        for (peer_id, addr) in network.known_peer_addresses() {
            if network.is_connected(&peer_id) {
                continue;
            }
            {
                let mut states = self
                    .reconnect
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let state = states.entry(peer_id.clone()).or_default();
                if !state.should_attempt(now) {
                    continue;
                }
                state.start();
            }

            let daemon = Arc::clone(self);
            let network = Arc::clone(&network);
            let peer = peer_id.clone();
            tokio::spawn(async move {
                let outcome = network.connect(addr).await;
                daemon.record_reconnect_result(&peer, outcome.err());
            });
        }
    }

    /// Record the outcome of one dial and schedule the next one on failure.
    fn record_reconnect_result(&self, peer_id: &str, error: Option<Error>) {
        let mut states = self
            .reconnect
            .lock()
            .unwrap_or_else(|lock_error| lock_error.into_inner());
        let state = states.entry(peer_id.to_string()).or_default();
        match error {
            None => state.succeeded(),
            Some(error) => {
                let delay = state.failed(Instant::now());
                eprintln!(
                    "[daemon] reconnect to {peer_id} failed: {error} (next attempt in {}s)",
                    delay.as_secs()
                );
            }
        }
    }

    /// Forget the backoff for a peer that connected (either direction).
    fn reset_reconnect_state(&self, peer_id: &str) {
        self.reconnect
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(peer_id);
    }

    /// Run the push delivery worker.
    async fn push_worker(
        self: Arc<Self>,
        mut bridge: ClaudeBridge,
        mut events: mpsc::UnboundedReceiver<ClaudeEvent>,
    ) {
        loop {
            tokio::select! {
                event = events.recv() => {
                    match event {
                        Some(event) => self.handle_delivery_event(event),
                        None => {
                            // The bridge was dropped; keep serving without live events.
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
            self.push_pending(&mut bridge).await;
        }
    }

    /// Hand every pending message to its push target, if there is one.
    async fn push_pending(&self, bridge: &mut ClaudeBridge) {
        let pending = match self.with_store(|store| store.undelivered_messages()) {
            Ok(pending) => pending,
            Err(error) => {
                eprintln!("[daemon] cannot read pending messages: {error}");
                return;
            }
        };

        for message in pending {
            let recipient = match self.with_store(|store| store.get_session(&message.recipient_key))
            {
                Ok(recipient) => recipient,
                Err(error) => {
                    eprintln!("[daemon] cannot resolve recipient: {error}");
                    continue;
                }
            };
            let Some(session) = recipient else {
                continue;
            };
            if !ClaudeBridge::supports(&session) {
                // Polling delivery only.
                continue;
            }

            match bridge.deliver(&session, &message).await {
                Ok(()) => {
                    if let Err(error) = self.with_store(|store| store.mark_delivered(&message.id)) {
                        eprintln!("[daemon] cannot mark message delivered: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("[daemon] push delivery failed: {error}");
                }
            }
        }
    }

    /// React to an event coming from a live agent session.
    fn handle_delivery_event(&self, event: ClaudeEvent) {
        match event {
            ClaudeEvent::TurnComplete {
                session_key,
                reply_to,
                text,
            } => {
                if text.trim().is_empty() {
                    return;
                }
                let Some(reply_to) = reply_to else {
                    return;
                };
                let sender_exists = self
                    .with_store(|store| store.get_session(&reply_to))
                    .ok()
                    .flatten()
                    .is_some();
                if !sender_exists {
                    return;
                }
                let result = self.with_store(|store| {
                    store.send_message(SendMessageParams {
                        sender_key: session_key,
                        recipient_key: reply_to,
                        body: text,
                        reply_to: None,
                    })
                });
                if let Err(error) = result {
                    eprintln!("[daemon] cannot store agent reply: {error}");
                }
            }
            ClaudeEvent::SessionId {
                session_key,
                conversation_id,
            } => {
                let result = self.with_store(|store| {
                    store.update(UpdateParams {
                        session_key: session_key.clone(),
                        agent_ref: Some(conversation_id.clone()),
                        ..Default::default()
                    })
                });
                if let Err(error) = result {
                    eprintln!("[daemon] cannot record agent conversation id: {error}");
                } else {
                    eprintln!(
                        "[daemon] agent session {session_key} uses conversation {conversation_id}"
                    );
                }
            }
            ClaudeEvent::Failed { session_key, error } => {
                eprintln!("[daemon] agent session {session_key} failed: {error}");
            }
        }
    }
}

/// Deserialize the parameters of a request.
fn parse_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T> {
    if params.is_null() {
        return serde_json::from_value(Value::Object(Default::default()))
            .map_err(|error| Error::InvalidRequest(error.to_string()));
    }
    serde_json::from_value(params).map_err(|error| Error::InvalidRequest(error.to_string()))
}

/// Serialize a message together with its delivery state.
fn message_with_delivery(message: &Message, state: &str, peer: Option<&str>) -> Value {
    let mut value = serde_json::to_value(message).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.insert("delivery_state".to_string(), json!(state));
        object.insert("peer_id".to_string(), json!(peer));
    }
    value
}

/// A human readable name for this machine.
fn default_machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "machine".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{InboxParams, RegisterParams};

    #[test]
    fn reconnect_backoff_grows_and_is_capped() {
        assert_eq!(reconnect_backoff(1), Duration::from_secs(5));
        assert_eq!(reconnect_backoff(2), Duration::from_secs(10));
        assert_eq!(reconnect_backoff(3), Duration::from_secs(20));
        assert_eq!(reconnect_backoff(4), Duration::from_secs(40));
        assert_eq!(reconnect_backoff(5), Duration::from_secs(60));
        assert_eq!(reconnect_backoff(50), Duration::from_secs(60));
    }

    #[test]
    fn a_dial_in_flight_blocks_another_one() {
        let now = Instant::now();
        let mut state = ReconnectState::default();
        assert!(state.should_attempt(now));

        state.start();
        assert!(
            !state.should_attempt(now + Duration::from_secs(60)),
            "a dial that is still running must not be doubled"
        );
    }

    #[test]
    fn failures_back_off_before_the_next_attempt() {
        let now = Instant::now();
        let mut state = ReconnectState::default();
        state.start();

        let delay = state.failed(now);
        assert_eq!(delay, Duration::from_secs(5));
        assert!(
            !state.should_attempt(now + Duration::from_secs(4)),
            "must wait out the backoff"
        );
        assert!(state.should_attempt(now + Duration::from_secs(5)));

        // A second failure waits longer.
        state.start();
        let delay = state.failed(now);
        assert_eq!(delay, Duration::from_secs(10));
        assert!(!state.should_attempt(now + Duration::from_secs(9)));
        assert!(state.should_attempt(now + Duration::from_secs(10)));
    }

    #[test]
    fn a_successful_dial_clears_the_backoff() {
        let now = Instant::now();
        let mut state = ReconnectState::default();
        for _ in 0..3 {
            state.start();
            state.failed(now);
        }
        assert!(!state.should_attempt(now));

        state.succeeded();
        assert!(state.should_attempt(now));
        assert_eq!(state.failures, 0);
    }

    #[tokio::test]
    async fn an_agent_turn_is_stored_as_a_reply_message() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::new(Config::with_data_dir(dir.path())).unwrap();
        for key in ["agent", "asker"] {
            daemon
                .with_store(|store| {
                    store.register(RegisterParams {
                        session_key: Some(key.to_string()),
                        name: key.to_string(),
                        ..Default::default()
                    })
                })
                .unwrap();
        }

        daemon.handle_delivery_event(ClaudeEvent::TurnComplete {
            session_key: "agent".into(),
            reply_to: Some("asker".into()),
            text: "the answer is 4".into(),
        });

        let inbox = daemon
            .with_store(|store| {
                store.inbox(InboxParams {
                    session_key: "asker".into(),
                    limit: None,
                    unread_only: None,
                })
            })
            .unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].sender_key, "agent");
        assert_eq!(inbox[0].body, "the answer is 4");
    }

    #[tokio::test]
    async fn a_turn_without_a_sender_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::new(Config::with_data_dir(dir.path())).unwrap();
        daemon
            .with_store(|store| {
                store.register(RegisterParams {
                    session_key: Some("agent".into()),
                    name: "agent".into(),
                    ..Default::default()
                })
            })
            .unwrap();

        daemon.handle_delivery_event(ClaudeEvent::TurnComplete {
            session_key: "agent".into(),
            reply_to: None,
            text: "nobody asked".into(),
        });

        let (sessions, messages) = daemon.with_store(|store| store.counts()).unwrap();
        assert_eq!(sessions, 1);
        assert_eq!(messages, 0);
    }
}
