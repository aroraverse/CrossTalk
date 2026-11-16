//! Local web dashboard (M4).
//!
//! A tiny, dependency-free HTTP/1.1 server bound to loopback. It serves one
//! static single-page application and a thin JSON API. Every API request is
//! turned into a [`Request`] and handed to the daemon's own
//! [`Daemon::dispatch`], so the dashboard never touches SQLite directly and
//! reuses the exact same validation, routing and networking as the CLI and
//! the MCP server.
//!
//! The server speaks just enough HTTP for a browser on the same machine:
//! `GET` for the page and the aggregated reads, `POST` with a JSON body for
//! actions. It is loopback-only and local-first; no cloud component exists.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::daemon::Daemon;
use crate::protocol::{Method, Request};

/// The single-page dashboard, embedded in the binary.
const INDEX_HTML: &str = include_str!("dashboard/index.html");

/// Maximum size of a request body the dashboard will read (256 KiB).
const MAX_BODY: usize = 256 * 1024;

/// Bind the dashboard HTTP server and serve until the task is dropped.
///
/// Returns the actual bound address (useful when the configured port was 0).
/// Errors from binding are returned so the daemon can log and carry on without
/// the dashboard.
pub async fn bind(addr: &str) -> std::io::Result<(TcpListener, std::net::SocketAddr)> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    Ok((listener, local))
}

/// Accept connections on `listener` and serve the dashboard, forever.
pub async fn serve(daemon: Arc<Daemon>, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    if let Err(error) = handle(daemon, stream).await {
                        // A broken pipe from a browser tab closing is normal.
                        if error.kind() != std::io::ErrorKind::BrokenPipe {
                            eprintln!("[dashboard] connection error: {error}");
                        }
                    }
                });
            }
            Err(error) => {
                eprintln!("[dashboard] accept error: {error}");
                return;
            }
        }
    }
}

/// A parsed HTTP request line plus body.
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
    fetch_site: Option<String>,
}

/// Read and parse one HTTP/1.1 request from the stream.
async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];

    // Read until we have the full header block.
    let header_end = loop {
        if let Some(pos) = find_double_crlf(&buf) {
            break pos;
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.len() > MAX_BODY {
            return Ok(None);
        }
    };

    let header_text = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0usize;
    let (mut host, mut origin, mut content_type, mut fetch_site) = (None, None, None, None);
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            match name.as_str() {
                "content-length" => content_length = value.parse().unwrap_or(0),
                "host" => host = Some(value),
                "origin" => origin = Some(value),
                "content-type" => content_type = Some(value),
                "sec-fetch-site" => fetch_site = Some(value),
                _ => {}
            }
        }
    }
    content_length = content_length.min(MAX_BODY);

    // The body may already be partly in `buf`.
    let mut body = buf[(header_end + 4).min(buf.len())..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(Some(HttpRequest {
        method,
        path,
        body,
        host,
        origin,
        content_type,
        fetch_site,
    }))
}

/// Locate the end of the HTTP header block.
fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Serve one connection: one request, one response, then close.
async fn handle(daemon: Arc<Daemon>, mut stream: TcpStream) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream).await? else {
        return Ok(());
    };

    let (status, content_type, body) = route(&daemon, &request).await;
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

/// Map an HTTP request onto either the static page or a daemon method.
async fn route(
    daemon: &Arc<Daemon>,
    request: &HttpRequest,
) -> (&'static str, &'static str, Vec<u8>) {
    let path = request.path.split('?').next().unwrap_or("/");

    // Static page.
    if request.method == "GET" && (path == "/" || path == "/index.html") {
        return (
            "200 OK",
            "text/html; charset=utf-8",
            INDEX_HTML.as_bytes().to_vec(),
        );
    }

    // JSON API under /api/<method>.
    if let Some(rest) = path.strip_prefix("/api/") {
        if let Err(reason) = check_origin(request) {
            return json_response("403 Forbidden", &json!({ "error": reason }));
        }
        let payload: Value = if request.body.is_empty() {
            Value::Null
        } else {
            match serde_json::from_slice(&request.body) {
                Ok(value) => value,
                Err(error) => {
                    return json_response(
                        "400 Bad Request",
                        &json!({ "error": error.to_string() }),
                    );
                }
            }
        };
        return dispatch_api(daemon, rest, payload).await;
    }

    json_response("404 Not Found", &json!({ "error": "not found" }))
}

/// Reject requests that did not come from this dashboard page.
///
/// The server only listens on loopback, but a loopback port is still
/// reachable from any web page the user visits (cross-site requests and DNS
/// rebinding). Since the API can answer approvals, the request must:
///
/// * carry a loopback `Host` (defeats DNS rebinding);
/// * if it carries an `Origin`, be exactly `http://<Host>` (defeats
///   cross-site and cross-port pages);
/// * not be marked `Sec-Fetch-Site: cross-site` / `same-site`;
/// * send a JSON content type when it has a body (forces a CORS preflight
///   for anything but this page).
fn check_origin(request: &HttpRequest) -> Result<(), String> {
    let host = request.host.as_deref().unwrap_or("");
    let name = match host.strip_prefix('[') {
        Some(rest) => format!("[{}]", rest.split(']').next().unwrap_or("")),
        None => host.split(':').next().unwrap_or("").to_string(),
    };
    if !matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        return Err(format!("host `{host}` is not a loopback address"));
    }
    if let Some(origin) = request.origin.as_deref()
        && origin != format!("http://{host}")
    {
        return Err(format!("origin `{origin}` is not this dashboard"));
    }
    if let Some(site) = request.fetch_site.as_deref()
        && !matches!(site, "same-origin" | "none")
    {
        return Err(format!("cross-site request refused ({site})"));
    }
    if !request.body.is_empty() {
        let json = request
            .content_type
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
        if !json {
            return Err("API bodies must be application/json".into());
        }
    }
    Ok(())
}

/// Translate an API path into a daemon [`Method`] and dispatch it.
async fn dispatch_api(
    daemon: &Arc<Daemon>,
    name: &str,
    params: Value,
) -> (&'static str, &'static str, Vec<u8>) {
    let Some(method) = method_for(name) else {
        return json_response(
            "404 Not Found",
            &json!({ "error": format!("unknown endpoint {name}") }),
        );
    };

    let request = Request {
        id: Some(format!("dashboard-{name}")),
        method,
        params,
    };
    let response = daemon.dispatch(request).await;
    if response.ok {
        json_response("200 OK", &json!({ "ok": true, "result": response.result }))
    } else {
        let error = response
            .error
            .map(|error| json!({ "code": error.code, "message": error.message }))
            .unwrap_or(Value::Null);
        json_response("200 OK", &json!({ "ok": false, "error": error }))
    }
}

/// The dashboard only exposes a curated, safe subset of daemon methods.
fn method_for(name: &str) -> Option<Method> {
    Some(match name {
        "overview" => Method::Overview,
        "status" => Method::Status,
        "diagnostics" => Method::Diagnostics,
        "net-info" => Method::NetInfo,
        "directory" => Method::Directory,
        "sessions" => Method::ListSessions,
        "register" => Method::Register,
        "update-session" => Method::Update,
        "send-message" => Method::SendMessage,
        "inbox" => Method::Inbox,
        "unread" => Method::UnreadCount,
        "mark-read" => Method::MarkRead,
        "search" => Method::SearchMessages,
        "conversation" => Method::Conversation,
        "conversations" => Method::Conversations,
        "ask-user" => Method::AskUser,
        "questions" => Method::ListQuestions,
        "answer" => Method::AnswerQuestion,
        "outbox" => Method::Outbox,
        "overlaps" => Method::FileOverlaps,
        "teams" => Method::TeamList,
        "team-create" => Method::TeamCreate,
        "team-invite" => Method::TeamInvite,
        "team-join" => Method::TeamJoin,
        "team-members" => Method::TeamMembers,
        "team-leave" => Method::TeamLeave,
        "team-revoke" => Method::TeamRevoke,
        "peers" => Method::PeerList,
        "anchor-status" => Method::AnchorStatus,
        "set-anchor" => Method::SetAnchor,
        "resolve" => Method::ResolveRecipient,
        "send-human" => Method::SendAsHuman,
        "all-conversations" => Method::AllConversations,
        "forget-session" => Method::RemoveSession,
        "setup-status" => Method::SetupStatus,
        "setup-apply" => Method::SetupApply,
        "message" => Method::GetMessage,
        _ => return None,
    })
}

/// Build a JSON HTTP response.
fn json_response(status: &'static str, value: &Value) -> (&'static str, &'static str, Vec<u8>) {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    (status, "application/json; charset=utf-8", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_curated_methods_are_exposed() {
        assert_eq!(method_for("overview"), Some(Method::Overview));
        assert_eq!(method_for("send-message"), Some(Method::SendMessage));
        assert_eq!(method_for("search"), Some(Method::SearchMessages));
        // Forgetting a session goes through `forget-session`, which the page
        // confirms first; the raw method name is not exposed.
        assert_eq!(method_for("remove-session"), None);
        assert_eq!(method_for("forget-session"), Some(Method::RemoveSession));
        // Shutdown must never be reachable from the browser.
        assert_eq!(method_for("shutdown"), None);
        assert_eq!(method_for("net-connect"), None);
        assert_eq!(method_for("nonsense"), None);
    }

    #[test]
    fn double_crlf_marks_the_header_end() {
        let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody";
        let pos = find_double_crlf(buf).unwrap();
        assert_eq!(&buf[pos..pos + 4], b"\r\n\r\n");
    }
}
