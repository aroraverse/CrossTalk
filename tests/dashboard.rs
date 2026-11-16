//! M4 tests: the local web dashboard, searchable history, conversations,
//! onboarding over the dashboard API, and file-overlap detection.
//!
//! The dashboard is exercised the way a browser would: real HTTP/1.1 requests
//! over a loopback TCP socket to the server the daemon runs. The point is to
//! prove the dashboard communicates *through the daemon* (its dispatch path),
//! not by touching SQLite on its own.

mod common;

use std::time::Duration;

use crosstalk::protocol::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use common::TestDaemon;

/// Minimal HTTP client: send one request, return (status_code, json_body).
async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.expect("connect dashboard");
    let payload = body.map(|value| serde_json::to_vec(&value).unwrap());
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Some(bytes) = &payload {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", bytes.len()));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    if let Some(bytes) = &payload {
        stream.write_all(bytes).await.unwrap();
    }
    stream.flush().await.unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body_text = text.split("\r\n\r\n").nth(1).unwrap_or("");
    let json = serde_json::from_str(body_text).unwrap_or(Value::Null);
    (status, json)
}

/// Convenience: POST /api/<name> and return the `result`, asserting ok.
async fn api(addr: std::net::SocketAddr, name: &str, params: Value) -> Value {
    let (status, body) = http(addr, "POST", &format!("/api/{name}"), Some(params)).await;
    assert_eq!(status, 200, "{name} -> {body}");
    assert_eq!(body["ok"], json!(true), "{name} failed: {body}");
    body["result"].clone()
}

#[tokio::test]
async fn dashboard_serves_the_single_page_app() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;

    let (status, _) = http(addr, "GET", "/", None).await;
    assert_eq!(status, 200);

    // The page must be HTML and mention the product; a raw text probe is
    // enough — we re-read as text here.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("text/html"), "page is served as HTML");
    assert!(text.contains("Crosstalk"), "page names the product");
    daemon.stop().await;
}

#[tokio::test]
async fn dashboard_reads_and_writes_through_the_daemon() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;

    // Register two sessions via the dashboard API.
    api(
        addr,
        "register",
        json!({ "session_key": "alice", "name": "alice", "agent_type": "codex" }),
    )
    .await;
    api(
        addr,
        "register",
        json!({ "session_key": "bob", "name": "bob", "agent_type": "claude" }),
    )
    .await;

    // Send a message through the dashboard.
    api(addr, "send-message", json!({ "sender_key": "alice", "recipient_key": "bob", "body": "hello from the dashboard" })).await;

    // The daemon (over IPC, a different client) must see the same state —
    // proving the dashboard wrote through the daemon, not a private store.
    let inbox = daemon
        .call(Method::Inbox, json!({ "session_key": "bob" }))
        .await
        .unwrap();
    let messages = inbox["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["body"], "hello from the dashboard");

    // The overview aggregates everything the dashboard renders.
    let overview = api(addr, "overview", json!({})).await;
    assert!(overview["status"].is_object());
    assert_eq!(overview["sessions"].as_array().unwrap().len(), 2);
    daemon.stop().await;
}

#[tokio::test]
async fn dashboard_refuses_unsafe_methods() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;

    // `shutdown` is not in the curated method map -> 404, daemon stays up.
    let (status, _) = http(addr, "POST", "/api/shutdown", Some(json!({}))).await;
    assert_eq!(status, 404);

    // The daemon is still answering.
    let probe = daemon.call(Method::Status, json!({})).await;
    assert!(probe.is_ok(), "daemon must survive a rejected shutdown");
    daemon.stop().await;
}

#[tokio::test]
async fn search_and_conversation_use_the_existing_history() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "alice", "name": "alice" }),
    )
    .await;
    api(
        addr,
        "register",
        json!({ "session_key": "bob", "name": "bob" }),
    )
    .await;

    api(addr, "send-message", json!({ "sender_key": "alice", "recipient_key": "bob", "body": "review the Storage layer" })).await;
    api(
        addr,
        "send-message",
        json!({ "sender_key": "bob", "recipient_key": "alice", "body": "on it" }),
    )
    .await;

    // Search is a substring over the existing messages table.
    let hits = api(addr, "search", json!({ "query": "storage" })).await;
    let messages = hits["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0]["body"].as_str().unwrap().contains("Storage"));

    // The conversation thread shows both directions.
    let thread = api(
        addr,
        "conversation",
        json!({ "session_key": "alice", "with": "bob" }),
    )
    .await;
    assert_eq!(thread["messages"].as_array().unwrap().len(), 2);

    // Conversation partners power the thread list.
    let threads = api(addr, "conversations", json!({ "session_key": "alice" })).await;
    assert_eq!(threads["conversations"].as_array().unwrap().len(), 1);
    daemon.stop().await;
}

#[tokio::test]
async fn onboarding_flow_over_the_dashboard() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;

    // Create a team (this machine is admin) and issue an invitation — the
    // two actions the onboarding "Create a team" panel performs.
    let team = api(addr, "team-create", json!({ "name": "core" })).await;
    let team_id = team["team_id"].as_str().unwrap().to_string();
    let invite = api(
        addr,
        "team-invite",
        json!({ "team_id": team_id, "ttl_seconds": 3600 }),
    )
    .await;
    assert!(invite["token"].as_str().unwrap().starts_with("rn1:"));

    // Register a session, then verify connectivity (the final onboarding step).
    api(
        addr,
        "register",
        json!({ "name": "alice", "session_key": "alice" }),
    )
    .await;
    let diag = api(addr, "diagnostics", json!({})).await;
    assert_eq!(diag["sessions"], json!(1));
    assert!(diag["network"]["running"].as_bool().unwrap());

    // The teams list reflects the new team with admin role.
    let teams = api(addr, "teams", json!({})).await;
    let list = teams["teams"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["is_admin"], json!(true));
    daemon.stop().await;
}

#[tokio::test]
async fn file_overlap_is_reported_through_the_daemon() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "alice", "name": "alice" }),
    )
    .await;
    api(
        addr,
        "register",
        json!({ "session_key": "bob", "name": "bob" }),
    )
    .await;

    // Two sessions edit the same file in the same repo (as the hook would
    // report on PostToolUse).
    api(addr, "overview", json!({})).await; // warm up
    let one = daemon
        .call(
            Method::RecordFileActivity,
            json!({ "session_key": "alice", "repo": "app", "path": "src/main.rs", "action": "edit" }),
        )
        .await
        .unwrap();
    assert_eq!(one["recorded"], json!(true));
    daemon
        .call(
            Method::RecordFileActivity,
            json!({ "session_key": "bob", "repo": "app", "path": "src/main.rs", "action": "edit" }),
        )
        .await
        .unwrap();

    // The dashboard sees the overlap through the API.
    let overlaps = api(addr, "overlaps", json!({ "window_seconds": 900 })).await;
    let list = overlaps["overlaps"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["path"], "src/main.rs");
    assert_eq!(list[0]["sessions"].as_array().unwrap().len(), 2);

    // It also surfaces in the aggregated overview the dashboard polls.
    let overview = api(addr, "overview", json!({})).await;
    assert_eq!(overview["overlaps"].as_array().unwrap().len(), 1);
    daemon.stop().await;
}

#[tokio::test]
async fn dashboard_can_be_disabled() {
    // A daemon started without a dashboard address exposes none.
    let daemon = TestDaemon::start().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(daemon.daemon.dashboard_addr().is_none());
    daemon.stop().await;
}

#[tokio::test]
async fn approvals_list_answer_and_route_to_the_agent() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;

    // An agent session asks a question (as the ask_user MCP tool does).
    api(
        addr,
        "register",
        json!({ "session_key": "alice", "name": "alice", "agent_type": "codex" }),
    )
    .await;
    let asked = api(
        addr,
        "ask-user",
        json!({ "session_key": "alice", "question": "Deploy to production?" }),
    )
    .await;
    let question_id = asked["id"].as_str().unwrap().to_string();

    // The Approvals view lists it as pending with who asked and when.
    let pending = api(addr, "questions", json!({ "status": "pending" })).await;
    let list = pending["questions"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["session_key"], "alice");
    assert_eq!(list[0]["question"], "Deploy to production?");
    assert_eq!(list[0]["status"], "pending");
    assert!(list[0]["created_at"].as_i64().unwrap() > 0);

    // Answered list is empty until we respond.
    let answered_before = api(addr, "questions", json!({ "status": "answered" })).await;
    assert!(answered_before["questions"].as_array().unwrap().is_empty());

    // Approve it through the dashboard.
    let result = api(
        addr,
        "answer",
        json!({ "question_id": question_id, "answer": "Approved" }),
    )
    .await;
    assert_eq!(result["status"], "answered");

    // The status flips immediately: it leaves pending, enters answered.
    let pending_after = api(addr, "questions", json!({ "status": "pending" })).await;
    assert!(pending_after["questions"].as_array().unwrap().is_empty());
    let answered_after = api(addr, "questions", json!({ "status": "answered" })).await;
    let answered = answered_after["questions"].as_array().unwrap();
    assert_eq!(answered.len(), 1);
    assert_eq!(answered[0]["answer"], "Approved");

    // The answer reaches the requesting agent through its existing inbox
    // (verified over IPC, a separate client).
    let inbox = daemon
        .call(Method::Inbox, json!({ "session_key": "alice" }))
        .await
        .unwrap();
    let messages = inbox["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["body"], "Approved");

    daemon.stop().await;
}

#[tokio::test]
async fn approvals_support_reject_and_custom_responses() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "bob", "name": "bob" }),
    )
    .await;

    // Two pending questions from the same agent.
    let q1 = api(
        addr,
        "ask-user",
        json!({ "session_key": "bob", "question": "Delete the table?" }),
    )
    .await;
    let q2 = api(
        addr,
        "ask-user",
        json!({ "session_key": "bob", "question": "Which region?" }),
    )
    .await;

    // Reject one, answer the other with a custom response.
    api(
        addr,
        "answer",
        json!({ "question_id": q1["id"], "answer": "Rejected" }),
    )
    .await;
    api(
        addr,
        "answer",
        json!({ "question_id": q2["id"], "answer": "Use ap-south-1" }),
    )
    .await;

    let answered = api(addr, "questions", json!({ "status": "answered" })).await;
    let bodies: Vec<String> = answered["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["answer"].as_str().unwrap().to_string())
        .collect();
    assert!(bodies.contains(&"Rejected".to_string()));
    assert!(bodies.contains(&"Use ap-south-1".to_string()));

    // Both answers landed in the agent's inbox.
    let inbox = daemon
        .call(Method::Inbox, json!({ "session_key": "bob" }))
        .await
        .unwrap();
    assert_eq!(inbox["messages"].as_array().unwrap().len(), 2);

    daemon.stop().await;
}

#[tokio::test]
async fn pending_questions_appear_in_the_overview() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "alice", "name": "alice" }),
    )
    .await;
    api(
        addr,
        "ask-user",
        json!({ "session_key": "alice", "question": "Proceed?" }),
    )
    .await;

    // The overview the dashboard polls carries pending questions, which drives
    // the sidebar badge.
    let overview = api(addr, "overview", json!({})).await;
    let questions = overview["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0]["question"], "Proceed?");

    daemon.stop().await;
}

#[tokio::test]
async fn anchor_role_toggles_and_persists() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;

    // Off by default.
    let status = api(addr, "anchor-status", json!({})).await;
    assert_eq!(status["anchor"], json!(false));

    // Enable it through the dashboard.
    let enabled = api(addr, "set-anchor", json!({ "enabled": true })).await;
    assert_eq!(enabled["anchor"], json!(true));

    // The daemon (over IPC) agrees, and it is reflected in the overview.
    let via_ipc = daemon.call(Method::AnchorStatus, json!({})).await.unwrap();
    assert_eq!(via_ipc["anchor"], json!(true));
    let overview = api(addr, "overview", json!({})).await;
    assert_eq!(overview["anchor"]["anchor"], json!(true));

    // It survives a daemon restart (stored in the existing meta table).
    let restarted = daemon.restart().await;
    let after = restarted
        .call(Method::AnchorStatus, json!({}))
        .await
        .unwrap();
    assert_eq!(after["anchor"], json!(true));
    restarted.stop().await;
}

// ---- v2: product dashboard ----------------------------------------------------

/// Raw request with arbitrary headers, returns (status, body text).
async fn raw(addr: std::net::SocketAddr, request: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let text = format!(
        "{request}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(text.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (
        status,
        text.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
    )
}

#[tokio::test]
async fn cross_site_and_rebinding_requests_cannot_answer_approvals() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "alice", "name": "alice" }),
    )
    .await;
    let asked = api(
        addr,
        "ask-user",
        json!({ "session_key": "alice", "question": "Deploy?" }),
    )
    .await;
    let body = json!({ "question_id": asked["id"], "answer": "Approved" }).to_string();
    let head = |host: &str, extra: &str| {
        format!(
            "POST /api/answer HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n{extra}"
        )
    };

    // A web page on another site (Origin differs).
    let (status, _) = raw(
        addr,
        &head(&addr.to_string(), "Origin: https://evil.example\r\n"),
        &body,
    )
    .await;
    assert_eq!(status, 403);
    // A page on another local port.
    let (status, _) = raw(
        addr,
        &head(&addr.to_string(), "Origin: http://localhost:3000\r\n"),
        &body,
    )
    .await;
    assert_eq!(status, 403);
    // DNS rebinding: a non-loopback Host name.
    let (status, _) = raw(addr, &head("attacker.example:7777", ""), &body).await;
    assert_eq!(status, 403);
    // Browser says it is cross-site.
    let (status, _) = raw(
        addr,
        &head(&addr.to_string(), "Sec-Fetch-Site: cross-site\r\n"),
        &body,
    )
    .await;
    assert_eq!(status, 403);
    // A "simple" form post (no JSON content type).
    let simple =
        format!("POST /api/answer HTTP/1.1\r\nHost: {addr}\r\nContent-Type: text/plain\r\n");
    let (status, _) = raw(addr, &simple, &body).await;
    assert_eq!(status, 403);

    // None of it changed anything.
    let pending = api(addr, "questions", json!({ "status": "pending" })).await;
    assert_eq!(pending["questions"].as_array().unwrap().len(), 1);

    // The dashboard page itself (same origin) works.
    let (status, text) = raw(
        addr,
        &head(
            &addr.to_string(),
            &format!("Origin: http://{addr}\r\nSec-Fetch-Site: same-origin\r\n"),
        ),
        &body,
    )
    .await;
    assert_eq!(status, 200, "{text}");
    daemon.stop().await;
}

#[tokio::test]
async fn forgetting_a_session_removes_only_the_registration() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "old", "name": "old", "agent_type": "codex" }),
    )
    .await;
    api(
        addr,
        "register",
        json!({ "session_key": "live", "name": "live", "agent_type": "claude-code" }),
    )
    .await;
    api(
        addr,
        "send-message",
        json!({ "sender_key": "live", "recipient_key": "old", "body": "history stays" }),
    )
    .await;

    let result = api(addr, "forget-session", json!({ "session_key": "old" })).await;
    assert_eq!(result["removed"], json!(true));
    let overview = api(addr, "overview", json!({})).await;
    let keys: Vec<&str> = overview["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["session_key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, vec!["live"]);
    // Messages survive in history.
    let hits = api(addr, "search", json!({ "query": "history stays" })).await;
    assert_eq!(hits["messages"].as_array().unwrap().len(), 1);
    // The raw method name is not reachable.
    let (status, _) = http(
        addr,
        "POST",
        "/api/remove-session",
        Some(json!({ "session_key": "live" })),
    )
    .await;
    assert_eq!(status, 404);
    daemon.stop().await;
}

#[tokio::test]
async fn the_human_can_message_agents_and_conversations_show_delivery() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(
        addr,
        "register",
        json!({ "session_key": "a", "name": "a", "agent_type": "codex" }),
    )
    .await;
    api(
        addr,
        "register",
        json!({ "session_key": "b", "name": "b", "agent_type": "claude-code" }),
    )
    .await;

    let sent = api(
        addr,
        "send-message",
        json!({ "sender_key": "a", "recipient_key": "b", "body": "agent to agent" }),
    )
    .await;
    let human = api(
        addr,
        "send-human",
        json!({ "recipient_key": "b", "body": "please hold", "reply_to": sent["id"] }),
    )
    .await;
    assert_eq!(human["sender_key"], "human");
    assert_eq!(human["reply_to"], sent["id"]);

    // Unknown recipients are refused.
    let (_, refused) = http(
        addr,
        "POST",
        "/api/send-human",
        Some(json!({ "recipient_key": "nobody", "body": "x" })),
    )
    .await;
    assert_eq!(refused["ok"], json!(false));

    let all = api(addr, "all-conversations", json!({})).await;
    let pairs = all["conversations"].as_array().unwrap();
    assert_eq!(pairs.len(), 2);
    assert!(pairs.iter().any(|p| p["a"] == "b" && p["b"] == "human"));
    assert!(pairs[0]["unread"].as_i64().unwrap() >= 1);

    let thread = api(
        addr,
        "conversation",
        json!({ "session_key": "a", "with": "b" }),
    )
    .await;
    let msg = &thread["messages"].as_array().unwrap()[0];
    assert_eq!(msg["delivery"], "delivered");
    // Once the recipient reads it, the state moves to read.
    daemon
        .call(Method::MarkRead, json!({ "session_key": "b" }))
        .await
        .unwrap();
    let thread = api(
        addr,
        "conversation",
        json!({ "session_key": "a", "with": "b" }),
    )
    .await;
    assert_eq!(thread["messages"][0]["delivery"], "read");
    daemon.stop().await;
}

#[tokio::test]
async fn history_search_filters_and_returns_context() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    for key in ["a", "b", "c"] {
        api(addr, "register", json!({ "session_key": key, "name": key })).await;
    }
    api(
        addr,
        "send-message",
        json!({ "sender_key": "a", "recipient_key": "b", "body": "before the plan" }),
    )
    .await;
    api(
        addr,
        "send-message",
        json!({ "sender_key": "b", "recipient_key": "a", "body": "the migration plan is ready" }),
    )
    .await;
    api(
        addr,
        "send-message",
        json!({ "sender_key": "a", "recipient_key": "b", "body": "thanks" }),
    )
    .await;
    api(
        addr,
        "send-message",
        json!({ "sender_key": "a", "recipient_key": "c", "body": "migration plan for c" }),
    )
    .await;
    api(
        addr,
        "send-human",
        json!({ "recipient_key": "c", "body": "human says migration plan ok" }),
    )
    .await;

    let all = api(
        addr,
        "search",
        json!({ "query": "migration plan", "context": true }),
    )
    .await;
    assert_eq!(all["messages"].as_array().unwrap().len(), 3);
    let by_session = api(
        addr,
        "search",
        json!({ "query": "migration plan", "session_key": "b", "context": true }),
    )
    .await;
    let hits = by_session["messages"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["before"]["body"], "before the plan");
    assert_eq!(hits[0]["after"]["body"], "thanks");
    let humans = api(
        addr,
        "search",
        json!({ "query": "migration plan", "sender": "human", "context": true }),
    )
    .await;
    assert_eq!(humans["messages"].as_array().unwrap().len(), 1);
    let agents = api(
        addr,
        "search",
        json!({ "query": "migration plan", "sender": "agent", "context": true }),
    )
    .await;
    assert_eq!(agents["messages"].as_array().unwrap().len(), 2);
    let future = api(
        addr,
        "search",
        json!({ "query": "migration", "since": 4102444800i64, "context": true }),
    )
    .await;
    assert!(future["messages"].as_array().unwrap().is_empty());
    daemon.stop().await;
}

#[tokio::test]
async fn approvals_show_context_and_whether_the_agent_received_the_answer() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    api(addr, "register", json!({ "session_key": "claude", "name": "claude", "agent_type": "claude-code", "repo": "app" })).await;
    api(
        addr,
        "register",
        json!({ "session_key": "codex", "name": "codex", "agent_type": "codex" }),
    )
    .await;
    api(
        addr,
        "send-message",
        json!({ "sender_key": "codex", "recipient_key": "claude", "body": "ready to deploy v2" }),
    )
    .await;
    let asked = api(
        addr,
        "ask-user",
        json!({ "session_key": "claude", "question": "Deploy v2?" }),
    )
    .await;

    let pending = api(addr, "questions", json!({ "status": "pending" })).await;
    let q = &pending["questions"][0];
    assert_eq!(q["session"]["repo"], "app");
    assert!(
        q["context"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == "ready to deploy v2")
    );

    api(
        addr,
        "answer",
        json!({ "question_id": asked["id"], "answer": "Approved" }),
    )
    .await;
    let answered = api(addr, "questions", json!({ "status": "answered" })).await;
    assert_eq!(
        answered["questions"][0]["delivery"],
        "delivered, waiting for the agent"
    );
    daemon
        .call(Method::MarkRead, json!({ "session_key": "claude" }))
        .await
        .unwrap();
    let answered = api(addr, "questions", json!({ "status": "answered" })).await;
    assert_eq!(answered["questions"][0]["delivery"], "read by the agent");
    daemon.stop().await;
}

#[tokio::test]
async fn setup_status_reports_each_agent() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    let status = api(addr, "setup-status", json!({})).await;
    let agents = status["agents"].as_array().expect("agents array");
    let ids: Vec<&str> = agents
        .iter()
        .map(|a| a["agent"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["claude", "codex", "cursor", "kiro", "opencode"]);
    for entry in agents {
        assert!(entry["state"].is_string());
    }
    daemon.stop().await;
}

#[tokio::test]
async fn the_page_is_responsive_and_accessible_by_construction() {
    let daemon = TestDaemon::start_with_dashboard().await;
    let addr = daemon.dashboard_addr().await;
    let (status, html) = raw(addr, "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n", "").await;
    assert_eq!(status, 200);
    for needle in [
        r#"<html lang="en">"#,
        r#"name="viewport""#,
        "class=\"skip\"",
        r#"aria-live="polite""#,
        r#"aria-current"#,
        ":focus-visible",
        "prefers-reduced-motion",
        "prefers-color-scheme: dark",
        "@media (max-width: 860px)",
        "<dialog",
        "overflow-x: auto",
    ] {
        assert!(html.contains(needle), "missing `{needle}`");
    }
    // Every page of the product exists, and nothing is loaded from the network.
    for page in [
        "overview",
        "sessions",
        "messages",
        "approvals",
        "history",
        "teams",
        "setup",
    ] {
        assert!(
            html.contains(&format!("pages.{page} =")),
            "missing page {page}"
        );
    }
    assert!(!html.contains("src=\"http"), "no external scripts");
    assert!(!html.contains("href=\"http"), "no external assets");
    // No emoji navigation.
    assert!(
        !html
            .chars()
            .any(|c| ('\u{1F300}'..='\u{1FAFF}').contains(&c)),
        "no emoji"
    );
    daemon.stop().await;
}
