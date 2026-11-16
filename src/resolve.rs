//! Recipient resolution and session liveness.
//!
//! Agents address each other by *exact session key*. A caller may also give a
//! looser selector (`codex`, `claude`, a session name); the resolver turns it
//! into exactly one live session of the caller's own project, or into an
//! explicit error. It never guesses across projects and never relies on a
//! display name alone: provider, project (canonical working directory and
//! repository), machine and liveness all take part.

use std::path::Path;

use serde::Serialize;

use crate::util;

/// A session with no sign of life for this long is `stale`.
pub const STALE_AFTER_SECS: i64 = 6 * 60 * 60;
/// A session seen within this window counts as `active` even if its last
/// reported status was `idle`: a live MCP server heartbeats every minute.
pub const ACTIVE_WINDOW_SECS: i64 = 150;
/// A `busy` session is considered active for this long without a heartbeat.
pub const BUSY_WINDOW_SECS: i64 = 15 * 60;

/// How alive a registration looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Liveness {
    /// Recently heard from, or working.
    Active,
    /// Registered and quiet, but not long enough to be considered abandoned.
    Idle,
    /// The agent reported that its session ended.
    Offline,
    /// Registered a long time ago and never heard from since.
    Stale,
}

impl Liveness {
    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Liveness::Active => "active",
            Liveness::Idle => "idle",
            Liveness::Offline => "offline",
            Liveness::Stale => "stale",
        }
    }

    /// True when a message can reasonably be expected to be read soon.
    pub fn is_live(self) -> bool {
        matches!(self, Liveness::Active | Liveness::Idle)
    }
}

/// Classify a registration from its reported status and `last_seen`.
pub fn liveness(status: &str, last_seen: i64, now: i64) -> Liveness {
    let age = (now - last_seen).max(0);
    if status == "offline" {
        return Liveness::Offline;
    }
    if age > STALE_AFTER_SECS {
        return Liveness::Stale;
    }
    if age <= ACTIVE_WINDOW_SECS || (status == "busy" && age <= BUSY_WINDOW_SECS) {
        return Liveness::Active;
    }
    Liveness::Idle
}

/// The provider family of an `agent_type`: `claude-code` and `claude` are both
/// Claude.
pub fn provider_of(agent_type: Option<&str>) -> String {
    match agent_type.unwrap_or("").to_ascii_lowercase().as_str() {
        "claude" | "claude-code" => "claude".to_string(),
        "codex" | "codex-cli" => "codex".to_string(),
        "" => "unknown".to_string(),
        other => other.to_string(),
    }
}

/// One addressable session, local or remote.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    /// Exact session key.
    pub session_key: String,
    /// Display name. Never used on its own to identify a session.
    pub name: String,
    /// `claude`, `codex`, ...
    pub provider: String,
    /// Raw agent type as registered.
    pub agent_type: Option<String>,
    /// Peer id of the owning machine.
    pub machine: String,
    /// True when the session lives on this machine.
    pub local: bool,
    /// Canonical working directory, local sessions only.
    pub working_dir: Option<String>,
    /// Repository name.
    pub repo: Option<String>,
    /// Branch, local sessions only.
    pub branch: Option<String>,
    /// Reported status.
    pub status: String,
    /// Short activity description.
    pub activity: String,
    /// Last activity (unix seconds).
    pub last_seen: i64,
    /// Derived liveness.
    pub liveness: Liveness,
    /// For remote sessions: whether the owning machine is connected now.
    pub reachable: bool,
}

/// What is known about the asking session.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    /// Caller's own session key (excluded from the result).
    pub session_key: String,
    /// Caller's provider.
    pub provider: String,
    /// Caller's working directory.
    pub working_dir: Option<String>,
    /// Caller's repository.
    pub repo: Option<String>,
}

/// Why a project match was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// An exact session key was given.
    ExactKey,
    /// Same canonical working directory.
    SameDirectory,
    /// Same repository.
    SameRepo,
    /// No project information on the caller; matched on selector alone.
    Unscoped,
}

/// The outcome of resolving a selector.
#[derive(Debug, Clone)]
pub enum Resolution {
    /// Exactly one session.
    Unique(Box<Candidate>, Scope),
    /// Several sessions matched and nothing separates them.
    Ambiguous(Vec<Candidate>),
    /// Nothing usable; the message explains why.
    NoMatch(String),
}

fn norm_dir(dir: &str) -> String {
    let trimmed = dir.strip_prefix(r"\\?\").unwrap_or(dir);
    util::normalize_path(Path::new(trimmed))
}

fn same_dir(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => norm_dir(a) == norm_dir(b),
        _ => false,
    }
}

fn same_repo(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => !a.is_empty() && a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

fn describe(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|c| {
            format!(
                "{} ({}, {}, {})",
                c.session_key,
                c.provider,
                c.liveness.as_str(),
                c.working_dir
                    .as_deref()
                    .or(c.repo.as_deref())
                    .unwrap_or("unknown project")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Resolve `selector` against `candidates` on behalf of `caller`.
pub fn resolve(selector: &str, caller: Option<&Caller>, candidates: &[Candidate]) -> Resolution {
    let selector = selector.trim();
    if selector.is_empty() {
        return Resolution::NoMatch("recipient must not be empty".into());
    }

    // 1. An exact session key always wins, whatever its liveness.
    if let Some(found) = candidates.iter().find(|c| c.session_key == selector) {
        return Resolution::Unique(Box::new(found.clone()), Scope::ExactKey);
    }

    // 2. Otherwise the selector is a provider or an exact session name.
    let wanted = provider_of(Some(selector));
    let lower = selector.to_ascii_lowercase();
    let matching: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| caller.is_none_or(|me| me.session_key != c.session_key))
        .filter(|c| c.provider == wanted || c.name.to_ascii_lowercase() == lower)
        .collect();
    if matching.is_empty() {
        return Resolution::NoMatch(format!(
            "no session matches `{selector}`: use list_team and pass an exact session key"
        ));
    }

    // 3. Only sessions that can plausibly answer are ever chosen automatically.
    let live: Vec<&Candidate> = matching
        .iter()
        .copied()
        .filter(|c| c.liveness.is_live() && (c.local || c.reachable))
        .collect();
    if live.is_empty() {
        let all: Vec<Candidate> = matching.into_iter().cloned().collect();
        return Resolution::NoMatch(format!(
            "`{selector}` only matches sessions that are offline, stale or unreachable: {}. \
Start the agent in the project, or pass an exact session key to queue a message anyway",
            describe(&all)
        ));
    }

    // 4. Prefer the caller's own project; never cross into unrelated ones.
    let (pool, scope): (Vec<&Candidate>, Scope) = match caller {
        Some(me) if me.working_dir.is_some() || me.repo.is_some() => {
            let dir: Vec<&Candidate> = live
                .iter()
                .copied()
                .filter(|c| c.local && same_dir(&c.working_dir, &me.working_dir))
                .collect();
            if !dir.is_empty() {
                (dir, Scope::SameDirectory)
            } else {
                let repo: Vec<&Candidate> = live
                    .iter()
                    .copied()
                    .filter(|c| same_repo(&c.repo, &me.repo))
                    .collect();
                if repo.is_empty() {
                    let others: Vec<Candidate> = live.into_iter().cloned().collect();
                    return Resolution::NoMatch(format!(
                        "no live `{selector}` session belongs to this project; refusing to pick an \
unrelated one. Sessions elsewhere: {}. Pass an exact session key to message one of them",
                        describe(&others)
                    ));
                }
                (repo, Scope::SameRepo)
            }
        }
        _ => (live, Scope::Unscoped),
    };

    // 5. A single candidate is the answer; otherwise liveness may separate them.
    if pool.len() == 1 {
        return Resolution::Unique(Box::new(pool[0].clone()), scope);
    }
    let active: Vec<&Candidate> = pool
        .iter()
        .copied()
        .filter(|c| c.liveness == Liveness::Active)
        .collect();
    if active.len() == 1 {
        return Resolution::Unique(Box::new(active[0].clone()), scope);
    }
    let narrowed = if active.is_empty() { pool } else { active };
    Resolution::Ambiguous(narrowed.into_iter().cloned().collect())
}

/// The error text for an ambiguous resolution.
pub fn ambiguity_message(selector: &str, candidates: &[Candidate]) -> String {
    format!(
        "`{selector}` is ambiguous: {} sessions match and nothing distinguishes them. \
Choose one and pass its exact session key: {}",
        candidates.len(),
        describe(candidates)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn cand(key: &str, provider: &str, dir: &str, status: &str, age: i64) -> Candidate {
        Candidate {
            session_key: key.into(),
            name: key.split('-').take(2).collect::<Vec<_>>().join("-"),
            provider: provider.into(),
            agent_type: Some(provider.into()),
            machine: "m1".into(),
            local: true,
            working_dir: Some(dir.into()),
            repo: Some(dir.rsplit(['/', '\\']).next().unwrap_or("").into()),
            branch: None,
            status: status.into(),
            activity: String::new(),
            last_seen: NOW - age,
            liveness: liveness(status, NOW - age, NOW),
            reachable: true,
        }
    }

    fn caller(dir: &str) -> Caller {
        Caller {
            session_key: "claude-proj-1".into(),
            provider: "claude".into(),
            working_dir: Some(dir.into()),
            repo: Some(dir.rsplit(['/', '\\']).next().unwrap_or("").into()),
        }
    }

    #[test]
    fn liveness_classes() {
        assert_eq!(liveness("idle", NOW - 10, NOW), Liveness::Active);
        assert_eq!(liveness("idle", NOW - 600, NOW), Liveness::Idle);
        assert_eq!(liveness("busy", NOW - 600, NOW), Liveness::Active);
        assert_eq!(
            liveness("idle", NOW - STALE_AFTER_SECS - 1, NOW),
            Liveness::Stale
        );
        assert_eq!(liveness("offline", NOW, NOW), Liveness::Offline);
    }

    #[test]
    fn picks_the_codex_of_the_same_project_not_system32() {
        let cands = vec![
            cand(
                "codex-system32-aa",
                "codex",
                r"C:\Windows\System32",
                "idle",
                30,
            ),
            cand("codex-proj-bb", "codex", r"D:\Work\proj", "idle", 30),
        ];
        match resolve("codex", Some(&caller("D:/work/proj")), &cands) {
            Resolution::Unique(found, scope) => {
                assert_eq!(found.session_key, "codex-proj-bb");
                assert_eq!(scope, Scope::SameDirectory);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn refuses_unrelated_sessions() {
        let cands = vec![cand(
            "codex-system32-aa",
            "codex",
            r"C:\Windows\System32",
            "idle",
            30,
        )];
        assert!(matches!(
            resolve("codex", Some(&caller("D:/work/proj")), &cands),
            Resolution::NoMatch(_)
        ));
    }

    #[test]
    fn ambiguity_is_reported_not_guessed() {
        let cands = vec![
            cand("codex-proj-a1", "codex", r"D:\work\proj", "idle", 30),
            cand("codex-proj-a2", "codex", r"D:\work\proj", "idle", 40),
        ];
        assert!(matches!(
            resolve("codex", Some(&caller(r"D:\work\proj")), &cands),
            Resolution::Ambiguous(found) if found.len() == 2
        ));
    }

    #[test]
    fn liveness_breaks_a_tie_and_offline_is_never_chosen() {
        let cands = vec![
            cand("codex-proj-a1", "codex", r"D:\work\proj", "idle", 4000),
            cand("codex-proj-a2", "codex", r"D:\work\proj", "busy", 20),
            cand("codex-proj-a3", "codex", r"D:\work\proj", "offline", 5),
        ];
        match resolve("codex", Some(&caller(r"D:\work\proj")), &cands) {
            Resolution::Unique(found, _) => assert_eq!(found.session_key, "codex-proj-a2"),
            other => panic!("unexpected {other:?}"),
        }
        let only_dead = vec![cands[2].clone()];
        assert!(matches!(
            resolve("codex", Some(&caller(r"D:\work\proj")), &only_dead),
            Resolution::NoMatch(_)
        ));
    }

    #[test]
    fn exact_key_targets_any_session() {
        let cands = vec![cand(
            "codex-system32-aa",
            "codex",
            r"C:\Windows\System32",
            "offline",
            99,
        )];
        assert!(matches!(
            resolve("codex-system32-aa", Some(&caller("D:/work/proj")), &cands),
            Resolution::Unique(_, Scope::ExactKey)
        ));
    }
}
