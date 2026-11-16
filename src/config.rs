use std::path::{Path, PathBuf};

/// Environment variable that overrides the application data directory.
pub const ENV_DATA_DIR: &str = "CROSSTALK_DATA_DIR";
/// Environment variable carrying the stable session key of the calling agent.
pub const ENV_SESSION_KEY: &str = "CROSSTALK_SESSION_KEY";
/// Environment variable overriding the `claude` binary used for push delivery.
pub const ENV_CLAUDE_BIN: &str = "CROSSTALK_CLAUDE_BIN";
/// Environment variable overriding the session name used for auto registration.
pub const ENV_SESSION_NAME: &str = "CROSSTALK_SESSION_NAME";
/// Environment variable naming the project directory of an agent session,
/// set by agents whose MCP configuration can interpolate it (Cursor writes
/// `${workspaceFolder}`). Used to derive the session key of the MCP server.
pub const ENV_WORKSPACE: &str = "CROSSTALK_WORKSPACE";
/// Environment variable overriding the dashboard HTTP bind address.
///
/// Defaults to `127.0.0.1:7777`. Set to an empty value or `off` to keep the
/// dashboard server from starting.
pub const ENV_DASHBOARD_ADDR: &str = "CROSSTALK_DASHBOARD_ADDR";

/// Default local address the dashboard HTTP server binds to.
pub const DEFAULT_DASHBOARD_ADDR: &str = "127.0.0.1:7777";

/// Name of the SQLite database file inside the data directory.
pub const DB_FILE_NAME: &str = "crosstalk.db";

/// The local endpoint clients use to reach the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Windows named pipe, e.g. `\\.\pipe\crosstalk-1a2b3c4d`.
    #[cfg(windows)]
    NamedPipe(String),
    /// A Unix domain socket path.
    #[cfg(unix)]
    UnixSocket(PathBuf),
}

impl Endpoint {
    /// Human readable form used in diagnostics and error messages.
    pub fn display(&self) -> String {
        match self {
            #[cfg(windows)]
            Endpoint::NamedPipe(name) => name.clone(),
            #[cfg(unix)]
            Endpoint::UnixSocket(path) => path.display().to_string(),
        }
    }
}

/// Resolved runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Directory holding the database and any local runtime files.
    pub data_dir: PathBuf,
    /// Full path of the SQLite database.
    pub db_path: PathBuf,
    /// Local endpoint the daemon listens on.
    pub endpoint: Endpoint,
    /// Local address for the dashboard HTTP server, when enabled.
    pub dashboard_addr: Option<String>,
    /// Peer-to-peer networking options.
    pub net: crate::net::NetOptions,
}

impl Config {
    /// Build the configuration from the environment.
    ///
    /// `CROSSTALK_DATA_DIR` wins when set, which is what the test suite
    /// uses to stay away from real user data.
    pub fn from_env() -> Self {
        let data_dir = match std::env::var_os(ENV_DATA_DIR) {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => default_data_dir(),
        };
        Self::with_data_dir(data_dir)
    }

    /// Build the configuration for an explicit data directory.
    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let db_path = data_dir.join(DB_FILE_NAME);
        let endpoint = endpoint_for(&data_dir);
        Self {
            data_dir,
            db_path,
            endpoint,
            dashboard_addr: dashboard_addr_from_env(),
            net: crate::net::NetOptions::from_env(),
        }
    }

    /// Override the networking options, used by tests.
    pub fn with_net_options(mut self, net: crate::net::NetOptions) -> Self {
        self.net = net;
        self
    }

    /// Override the dashboard bind address, used by tests.
    pub fn with_dashboard_addr(mut self, addr: Option<String>) -> Self {
        self.dashboard_addr = addr;
        self
    }

    /// Create the data directory if it does not exist yet.
    pub fn ensure_data_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)
    }
}

/// Resolve the dashboard bind address from the environment.
///
/// Unset uses the default; an empty value or `off`/`0`/`false` disables it.
fn dashboard_addr_from_env() -> Option<String> {
    match std::env::var(ENV_DASHBOARD_ADDR) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case("off")
                || trimmed == "0"
                || trimmed.eq_ignore_ascii_case("false")
            {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(_) => Some(DEFAULT_DASHBOARD_ADDR.to_string()),
    }
}

/// Platform specific default location for application data.
fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("crosstalk");
        }
        let profile = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        profile.join("AppData").join("Roaming").join("crosstalk")
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("crosstalk");
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".local").join("share").join("crosstalk")
    }
}

/// Derive a per-data-directory endpoint so parallel test runs never collide.
fn endpoint_for(data_dir: &Path) -> Endpoint {
    #[cfg(windows)]
    {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let canonical = data_dir.to_string_lossy().to_lowercase();
        let mut hasher = DefaultHasher::new();
        canonical.hash(&mut hasher);
        let digest = hasher.finish();
        Endpoint::NamedPipe(format!(r"\\.\pipe\crosstalk-{digest:016x}"))
    }
    #[cfg(not(windows))]
    {
        Endpoint::UnixSocket(data_dir.join("daemon.sock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_override_changes_database_and_endpoint() {
        let config = Config::with_data_dir("/tmp/rn-test-dir");
        assert!(config.db_path.ends_with(DB_FILE_NAME));
        assert_ne!(
            config.endpoint,
            Config::with_data_dir("/tmp/rn-other").endpoint
        );
    }

    #[test]
    fn same_data_dir_yields_same_endpoint() {
        assert_eq!(
            Config::with_data_dir("/tmp/rn-same").endpoint,
            Config::with_data_dir("/tmp/rn-same").endpoint
        );
    }
}
