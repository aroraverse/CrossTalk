//! Agent coordination: project-specific discovery, ambiguity, reply
//! correlation, waiting for replies, duplicate prevention and the human
//! approval path, against a real daemon on a temporary data directory.

mod common;

use std::time::{Duration, Instant};

use crosstalk::agents::{self, Agent};
use crosstalk::hook;
use crosstalk::protocol::Method;
use serde_json::{Value, json};

use common::TestDaemon;

async fn register(daemon: &TestDaemon, key: &str, agent_type: &str, dir: &str, status: &str) {
    daemon
        .call(
            Method::Register,
            json!({
                "session_key": key,
                "name": key,
                "agent_type": agent_type,
                "working_dir": dir,
                "repo": dir.rsplit(['/', '\\']).next().unwrap(),
                "status": status,
            }),
        )
        .await
        .unwrap();
}

async fn send(
    daemon: &TestDaemon,
    from: &str,
    to: &str,
    body: &str,
    reply_to: Option<&str>,
) -> Value {
    daemon
        .call(
            Method::SendMessage,
            json!({ "sender_key": from, "recipient_key": to, "body": body, "reply_to": reply_to }),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn selector_resolves_to_the_codex_of_the_same_project() {
    let daemon = TestDaemon::start().await;
    register(
        &daemon,
        "claude-proj",
        "claude-code",
        r"D:\Work\proj",
        "idle",
    )
    .await;
    register(&daemon, "codex-proj", "codex", "D:/work/proj", "idle").await;
    register(
        &daemon,
        "codex-system32",
        "codex",
        r"C:\Windows\System32",
        "idle",
    )
    .await;

    let sent = send(&daemon, "claude-proj", "codex", "hello", None).await;
    assert_eq!(sent["recipient_key"], "codex-proj");
    assert_eq!(sent["resolved_recipient"]["session_key"], "codex-proj");
    assert_eq!(sent["matched_by"], "same_directory");

    // The unrelated session received nothing.
    let other = daemon
        .call(
            Method::UnreadCount,
            json!({ "session_key": "codex-system32" }),
        )
        .await
        .unwrap();
    assert_eq!(other["unread"], 0);
}

#[tokio::test]
async fn unrelated_sessions_are_never_chosen_automatically() {
    let daemon = TestDaemon::start().await;
    register(
        &daemon,
        "claude-proj",
        "claude-code",
        r"D:\Work\proj",
        "idle",
    )
    .await;
    register(
        &daemon,
        "codex-system32",
        "codex",
        r"C:\Windows\System32",
        "idle",
    )
    .await;

    let error = daemon
        .call(
            Method::SendMessage,
            json!({ "sender_key": "claude-proj", "recipient_key": "codex", "body": "x" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("not_found"), "{error}");
    assert!(error.contains("this project"), "{error}");

    // An exact session key is still honoured, deliberately.
    let sent = send(&daemon, "claude-proj", "codex-system32", "on purpose", None).await;
    assert_eq!(sent["recipient_key"], "codex-system32");
}

#[tokio::test]
async fn ambiguous_recipients_are_an_explicit_error() {
    let daemon = TestDaemon::start().await;
    register(
        &daemon,
        "claude-proj",
        "claude-code",
        r"D:\work\proj",
        "idle",
    )
    .await;
    register(&daemon, "codex-a", "codex", r"D:\work\proj", "idle").await;
    register(&daemon, "codex-b", "codex", r"D:\work\proj", "idle").await;

    let error = daemon
        .call(
            Method::ResolveRecipient,
            json!({ "session_key": "claude-proj", "selector": "codex" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("ambiguous_recipient"), "{error}");
    assert!(
        error.contains("codex-a") && error.contains("codex-b"),
        "{error}"
    );

    // Exact keys disambiguate.
    let ok = daemon
        .call(
            Method::ResolveRecipient,
            json!({ "session_key": "claude-proj", "selector": "codex-b" }),
        )
        .await
        .unwrap();
    assert_eq!(ok["session"]["session_key"], "codex-b");
}

#[tokio::test]
async fn offline_sessions_are_not_selected_but_are_reported() {
    let daemon = TestDaemon::start().await;
    register(
        &daemon,
        "claude-proj",
        "claude-code",
        r"D:\work\proj",
        "idle",
    )
    .await;
    register(&daemon, "codex-proj", "codex", r"D:\work\proj", "offline").await;
    let error = daemon
        .call(
            Method::ResolveRecipient,
            json!({ "session_key": "claude-proj", "selector": "codex" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("offline"), "{error}");

    let directory = daemon.call(Method::Directory, json!({})).await.unwrap();
    let entry = directory["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["session_key"] == "codex-proj")
        .unwrap();
    assert_eq!(entry["liveness"], "offline");
    assert_eq!(entry["provider"], "codex");
    assert_eq!(entry["working_dir"], r"D:\work\proj");
}

#[tokio::test]
async fn replies_are_correlated_and_share_a_thread() {
    let daemon = TestDaemon::start().await;
    register(&daemon, "claude-proj", "claude-code", "/p/proj", "idle").await;
    register(&daemon, "codex-proj", "codex", "/p/proj", "idle").await;

    let first = send(&daemon, "claude-proj", "codex-proj", "proposal", None).await;
    let id = first["id"].as_str().unwrap();
    assert_eq!(first["thread_id"], id);

    let reply = send(&daemon, "codex-proj", "claude-proj", "looks good", Some(id)).await;
    assert_eq!(reply["reply_to"], id);
    assert_eq!(reply["thread_id"], id);

    let bad = daemon
        .call(
            Method::SendMessage,
            json!({ "sender_key": "codex-proj", "recipient_key": "claude-proj",
                    "body": "x", "reply_to": "msg-nope" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(bad.contains("not a known message"), "{bad}");
}

#[tokio::test]
async fn wait_for_reply_returns_the_actual_reply_and_ignores_unrelated_messages() {
    let daemon = TestDaemon::start().await;
    register(&daemon, "claude-proj", "claude-code", "/p/proj", "idle").await;
    register(&daemon, "codex-proj", "codex", "/p/proj", "idle").await;
    register(&daemon, "noise", "codex", "/p/other", "idle").await;

    let first = send(&daemon, "claude-proj", "codex-proj", "proposal", None).await;
    let id = first["id"].as_str().unwrap().to_string();

    let waiter = {
        let endpoint = daemon.config.endpoint.clone();
        let id = id.clone();
        tokio::spawn(async move {
            let request = crosstalk::protocol::Request {
                id: Some("w".into()),
                method: Method::WaitReply,
                params: json!({ "session_key": "claude-proj", "message_id": id, "timeout_seconds": 20 }),
            };
            let started = Instant::now();
            let response: crosstalk::protocol::Response =
                crosstalk::ipc::request_once(&endpoint, &request)
                    .await
                    .unwrap();
            (response, started.elapsed())
        })
    };

    // Unrelated traffic first: it must neither satisfy nor wake-and-return.
    tokio::time::sleep(Duration::from_millis(300)).await;
    send(&daemon, "noise", "claude-proj", "unrelated chatter", None).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiter.is_finished(), "an unrelated message ended the wait");

    send(
        &daemon,
        "codex-proj",
        "claude-proj",
        "LGTM, ship it",
        Some(&id),
    )
    .await;
    let (response, waited) = waiter.await.unwrap();
    assert!(response.ok);
    let result = response.result.unwrap();
    assert_eq!(result["status"], "reply");
    assert_eq!(result["reply"]["body"], "LGTM, ship it");
    assert_eq!(result["reply"]["sender_key"], "codex-proj");
    assert_eq!(result["from"]["provider"], "codex");
    assert!(
        waited < Duration::from_secs(5),
        "woke by event, not by timeout: {waited:?}"
    );

    // The reply was marked read; the unrelated message is still unread.
    let unread = daemon
        .call(Method::UnreadCount, json!({ "session_key": "claude-proj" }))
        .await
        .unwrap();
    assert_eq!(unread["unread"], 1);
}

#[tokio::test]
async fn wait_for_reply_reports_a_timeout_honestly() {
    let daemon = TestDaemon::start().await;
    register(&daemon, "claude-proj", "claude-code", "/p/proj", "idle").await;
    register(&daemon, "codex-proj", "codex", "/p/proj", "idle").await;
    let first = send(&daemon, "claude-proj", "codex-proj", "ping", None).await;

    let started = Instant::now();
    let result = daemon
        .call(
            Method::WaitReply,
            json!({ "session_key": "claude-proj", "message_id": first["id"], "timeout_seconds": 1 }),
        )
        .await
        .unwrap();
    assert_eq!(result["status"], "timeout");
    assert_eq!(result["original_message"]["state"], "unread");
    assert_eq!(result["recipient"]["session_key"], "codex-proj");
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn claimed_messages_are_handed_over_exactly_once() {
    let daemon = TestDaemon::start().await;
    register(&daemon, "a", "codex", "/p/proj", "idle").await;
    register(&daemon, "b", "codex", "/p/proj2", "idle").await;
    send(&daemon, "a", "b", "one", None).await;

    let (first, second) = tokio::join!(
        daemon.call(Method::ClaimMessages, json!({ "session_key": "b" })),
        daemon.call(Method::ClaimMessages, json!({ "session_key": "b" })),
    );
    let total = first.unwrap()["messages"].as_array().unwrap().len()
        + second.unwrap()["messages"].as_array().unwrap().len();
    assert_eq!(
        total, 1,
        "two concurrent claims must not both get the message"
    );
}

#[tokio::test]
async fn nobody_can_impersonate_the_human() {
    let daemon = TestDaemon::start().await;
    let error = daemon
        .call(
            Method::Register,
            json!({ "session_key": "human", "name": "human" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("reserved"), "{error}");
    let error = daemon
        .call(
            Method::Register,
            json!({ "session_key": "Human", "name": "Human" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("reserved"), "{error}");
}

#[tokio::test]
async fn a_human_answer_is_correlated_to_the_question() {
    let daemon = TestDaemon::start().await;
    register(&daemon, "claude-proj", "claude-code", "/p/proj", "idle").await;
    let question = daemon
        .call(
            Method::AskUser,
            json!({ "session_key": "claude-proj", "question": "Deploy to production?" }),
        )
        .await
        .unwrap();
    let qid = question["id"].as_str().unwrap().to_string();

    let waiter = {
        let endpoint = daemon.config.endpoint.clone();
        let qid = qid.clone();
        tokio::spawn(async move {
            let request = crosstalk::protocol::Request {
                id: None,
                method: Method::WaitReply,
                params: json!({ "session_key": "claude-proj", "message_id": qid, "timeout_seconds": 20 }),
            };
            let response: crosstalk::protocol::Response =
                crosstalk::ipc::request_once(&endpoint, &request)
                    .await
                    .unwrap();
            response.result.unwrap()
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    daemon
        .call(
            Method::AnswerQuestion,
            json!({ "question_id": qid, "answer": "Approved" }),
        )
        .await
        .unwrap();
    let result = waiter.await.unwrap();
    assert_eq!(result["status"], "reply");
    assert_eq!(result["reply"]["sender_key"], "human");
    assert_eq!(result["reply"]["body"], "Approved");
    assert_eq!(result["reply"]["reply_to"], qid);
}

// ---- hooks: documented response formats --------------------------------------

fn payload(name: &str, cwd: &std::path::Path, extra: Value) -> String {
    let mut value = json!({
        "hook_event_name": name,
        "session_id": "conv-1",
        "cwd": cwd.display().to_string(),
    });
    for (k, v) in extra.as_object().unwrap() {
        value[k] = v.clone();
    }
    value.to_string()
}

#[tokio::test]
async fn codex_event_responses_follow_the_documented_formats() {
    let daemon = TestDaemon::start().await;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let key = agents::session_key(Agent::Codex, &repo);
    let config = &daemon.config;

    // SessionStart: JSON additionalContext.
    let out = hook::run(
        config,
        Agent::Codex,
        &payload("SessionStart", &repo, json!({"source":"startup"})),
    )
    .await;
    let v: Value = serde_json::from_str(&out).expect("SessionStart output must be JSON");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains(&key)
    );

    register(&daemon, "peer", "claude-code", "/elsewhere", "idle").await;
    let sent = send(&daemon, "peer", &key, "please review the plan", None).await;

    // Stop: documented continuation, and only once per turn.
    let out = hook::run(
        config,
        Agent::Codex,
        &payload("Stop", &repo, json!({"stop_hook_active": true})),
    )
    .await;
    assert!(
        out.is_empty(),
        "a Stop hook that already continued must let the agent stop: {out}"
    );
    let out = hook::run(
        config,
        Agent::Codex,
        &payload("Stop", &repo, json!({"stop_hook_active": false})),
    )
    .await;
    let v: Value = serde_json::from_str(&out).expect("Stop output must be JSON");
    assert_eq!(v["decision"], "block");
    let reason = v["reason"].as_str().unwrap();
    assert!(
        reason.contains("please review the plan") && reason.contains("UNTRUSTED"),
        "{reason}"
    );
    assert!(reason.contains(sent["id"].as_str().unwrap()), "{reason}");

    // Delivered once: the next Stop has nothing to hand over (empty output).
    let out = hook::run(config, Agent::Codex, &payload("Stop", &repo, json!({}))).await;
    assert!(out.is_empty(), "{out}");

    // PostToolUse: JSON additionalContext (plain text is ignored by Codex).
    send(&daemon, "peer", &key, "second message", None).await;
    let out = hook::run(
        config,
        Agent::Codex,
        &payload(
            "PostToolUse",
            &repo,
            json!({"tool_name":"Bash","tool_input":{"command":"ls"}}),
        ),
    )
    .await;
    let v: Value = serde_json::from_str(&out).expect("PostToolUse output must be JSON");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("second message")
    );

    // SessionEnd: no output at all.
    let out = hook::run(
        config,
        Agent::Codex,
        &payload("SessionEnd", &repo, json!({"reason":"other"})),
    )
    .await;
    assert!(out.is_empty());
}

#[test]
fn payload_decoding_tolerates_bom_and_bad_bytes() {
    let json = r#"{"hook_event_name":"Stop"}"#;
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(json.as_bytes());
    assert!(hook::HookEvent::parse(&hook::decode_payload(&utf8_bom)).is_some());

    let mut utf16 = vec![0xFF, 0xFE];
    for unit in json.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    assert!(hook::HookEvent::parse(&hook::decode_payload(&utf16)).is_some());

    // Invalid bytes never panic or fail.
    let _ = hook::decode_payload(&[0xFF, 0x00, 0xC3, 0x28]);
}

/// The Windows failure: Codex runs `pwsh -NoProfile -Command "<command>"`.
/// Run the generated command exactly that way, from an executable path that
/// contains spaces and a non-ASCII character, and require exit code 0.
/// Claude Code on Windows runs a hook `command` through Git Bash when it finds
/// it and through PowerShell otherwise. The old shell-form command (a quoted
/// path followed by arguments) is a PowerShell parse error, which is the
/// "hook exited with code 1" users saw. Setup now writes the exec form, which
/// Claude Code spawns directly without any shell.
#[cfg(windows)]
#[test]
fn the_generated_claude_hook_runs_without_a_shell_from_a_path_with_spaces() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let spaced = dir.path().join("Reason – Net work");
    std::fs::create_dir_all(&spaced).unwrap();
    let exe = spaced.join("crosstalk.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_crosstalk"), &exe).unwrap();
    let data = dir.path().join("data dir");

    let host = agents::Host {
        home: dir.path().to_path_buf(),
        path: vec![],
        claude_config_dir: None,
        codex_home: None,
        exe: exe.clone(),
        data_dir: Some(data.clone()),
    };

    // The root cause, kept as evidence: the old form fails under PowerShell.
    let old = host.hook_command(Agent::Claude);
    let shell = ["pwsh", "powershell"]
        .into_iter()
        .find(|name| {
            Command::new(name)
                .args(["-NoProfile", "-Command", "exit 0"])
                .status()
                .is_ok()
        })
        .expect("a PowerShell is required on Windows");
    let parse = Command::new(shell)
        .args(["-NoProfile", "-Command", &old])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_ne!(
        parse.status.code(),
        Some(0),
        "old form under PowerShell: {old}"
    );

    let handler = host.claude_hook_handler(10);
    let command = handler["command"].as_str().unwrap().to_string();
    let args: Vec<String> = handler["args"]
        .as_array()
        .expect("exec form on Windows")
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect();
    assert_eq!(command, exe.display().to_string());

    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "SessionEnd",
    ] {
        let mut child = Command::new(&command)
            .args(&args)
            .current_dir(&spaced)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let body = json!({
            "hook_event_name": event,
            "session_id": "s",
            "cwd": spaced.display().to_string(),
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "stop_hook_active": false,
        });
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{event}: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let log = std::fs::read_to_string(data.join("hook.log")).expect("hook.log is written");
    assert!(log.contains("agent=claude event=Stop"), "{log}");
}

#[cfg(windows)]
#[test]
fn the_generated_codex_command_runs_under_powershell_from_a_path_with_spaces() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let spaced = dir.path().join("Reason – Net work");
    std::fs::create_dir_all(&spaced).unwrap();
    let exe = spaced.join("crosstalk.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_crosstalk"), &exe).unwrap();
    let data = dir.path().join("data dir");

    let host = agents::Host {
        home: dir.path().to_path_buf(),
        path: vec![],
        claude_config_dir: None,
        codex_home: None,
        exe: exe.clone(),
        data_dir: Some(data.clone()),
    };
    let command = host.hook_command(Agent::Codex);
    assert!(
        command.starts_with("& '"),
        "must use the call operator: {command}"
    );

    let shell = ["pwsh", "powershell"]
        .into_iter()
        .find(|name| {
            Command::new(name)
                .arg("-NoProfile")
                .arg("-Command")
                .arg("exit 0")
                .status()
                .is_ok()
        })
        .expect("a PowerShell is required on Windows");
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "SessionEnd",
    ] {
        let mut child = Command::new(shell)
            .args(["-NoProfile", "-Command", &command])
            .current_dir(&spaced)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let body = json!({
            "hook_event_name": event,
            "session_id": "s",
            "cwd": spaced.display().to_string(),
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "stop_hook_active": false,
        });
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        // No daemon is running on that data dir: the hook must still succeed.
        assert_eq!(
            out.status.code(),
            Some(0),
            "{event}: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let log = std::fs::read_to_string(data.join("hook.log")).expect("hook.log is written");
    assert!(log.contains("event=Stop"), "{log}");
    assert!(
        !log.contains("please"),
        "no payload content in the default log"
    );
}
