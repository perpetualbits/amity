// config.rs — service configuration loading.
//
// Configuration is read from a TOML file on startup. The canonical location is:
//   Linux:   $XDG_CONFIG_HOME/amity/config.toml  (usually ~/.config/amity/)
//   macOS:   ~/Library/Application Support/amity/config.toml
//   Windows: %APPDATA%\amity\config.toml
//
// The `directories` crate handles platform differences transparently.
//
// If the config file does not exist, built-in defaults are used — the service
// is runnable out of the box without any configuration ceremony.
//
// All fields have `#[serde(default)]` so a config file that only overrides one
// value is valid; missing fields use the defaults below.

// `PathBuf` is used for config and data file paths resolved at runtime.
use std::path::PathBuf;

// `ProjectDirs` resolves XDG/macOS/Windows platform directories transparently.
use directories::ProjectDirs;
// Serde: Deserialize to parse TOML; Serialize for symmetry and potential write-back.
use serde::{Deserialize, Serialize};

/// Top-level service configuration.
///
/// Loaded from `$XDG_CONFIG_HOME/amity/config.toml` (or platform equivalent).
/// All fields are optional in the file; missing fields use the values returned
/// by their `Default` implementations.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Database configuration.
    #[serde(default)]
    pub database: DatabaseConfig,

    /// HTTP server configuration.
    #[serde(default)]
    pub server: ServerConfig,
}

/// Database connection settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    /// `SQLite` database URL.
    ///
    /// Defaults to a file in the platform data directory:
    ///   Linux:   `$XDG_DATA_HOME/amity/amity.db`
    ///   macOS:   `~/Library/Application Support/amity/amity.db`
    ///   Windows: `%APPDATA%\amity\amity.db`
    ///
    /// For tests or development, pass `"sqlite::memory:"`.
    #[serde(default = "default_database_url")]
    pub url: String,
}

// The `Default` impl delegates to `default_database_url()` so the serde
// `#[serde(default)]` path and the programmatic `DatabaseConfig::default()`
// path always produce the same value.
impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_database_url(),
        }
    }
}

/// HTTP server bind settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// IP address to bind.
    ///
    /// Defaults to `127.0.0.1` (loopback only). The service must never bind
    /// to `0.0.0.0` by default — it exposes an unauthenticated API and must
    /// only be reachable from the local machine. Remote access is post-MVP
    /// and requires auth first.
    #[serde(default = "default_bind_address")]
    pub bind_address: String,

    /// TCP port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
}

// Mirror of the DatabaseConfig default — keeps the serde and programmatic
// defaults in sync so there is only one source of truth for each value.
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
        }
    }
}

// ─── Default value functions ─────────────────────────────────────────────────
// These are separate functions (not inline closures) because serde requires
// `#[serde(default = "fn_name")]` to name a function, not an expression.

fn default_bind_address() -> String {
    // Loopback only — see ServerConfig::bind_address doc.
    "127.0.0.1".to_owned()
}

fn default_port() -> u16 {
    // 7890 — chosen to avoid conflicts with common development ports.
    7890
}

fn default_database_url() -> String {
    // Resolve the platform data directory at runtime. Falls back to a local
    // file if the platform dirs crate cannot determine the directory (unusual).
    // Falls back to a local file if the platform dirs crate cannot determine
    // the directory — unusual but possible in restricted environments.
    resolve_default_database_path().map_or_else(
        || "sqlite://amity.db".to_owned(),
        |p| format!("sqlite://{}", p.display()),
    )
}

/// Resolve the default database file path for the current platform.
///
/// Returns `None` if the platform data directory cannot be determined.
fn resolve_default_database_path() -> Option<PathBuf> {
    // `ProjectDirs::from` takes (qualifier, organisation, application).
    let dirs = ProjectDirs::from("", "amity", "amity")?;
    Some(dirs.data_dir().join("amity.db"))
}

// ─── Config loading ───────────────────────────────────────────────────────────

/// Load the service configuration from the platform config file, or return
/// defaults if the file does not exist.
///
/// # Errors
///
/// Returns an error if the config file exists but cannot be read or parsed.
/// A missing file is not an error — it means "use defaults".
pub fn load_config() -> anyhow::Result<ServiceConfig> {
    let config_path = resolve_config_path();

    match config_path {
        // Config path resolved; check whether the file actually exists.
        Some(path) if path.exists() => {
            // `read_to_string` is used instead of a BufReader because config files
            // are always small enough to fit in memory without streaming.
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("failed to read config at {}: {e}", path.display()))?;

            // `toml::from_str` fills only the fields present in the file; missing
            // fields fall back to their `#[serde(default)]` functions defined above.
            let config: ServiceConfig = toml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("invalid config at {}: {e}", path.display()))?;

            tracing::info!(path = %path.display(), "loaded config file");
            Ok(config)
        }

        // Config path resolved but file does not exist — use defaults silently.
        Some(path) => {
            tracing::info!(path = %path.display(), "config file not found, using defaults");
            Ok(ServiceConfig::default())
        }

        // Platform dirs unavailable (unusual) — use defaults.
        None => {
            tracing::warn!("could not determine config directory, using defaults");
            Ok(ServiceConfig::default())
        }
    }
}

/// Resolve the platform config file path without reading it.
///
/// Linux:   `$XDG_CONFIG_HOME/amity/config.toml`
/// macOS:   `~/Library/Preferences/amity/config.toml`
/// Windows: `%APPDATA%\amity\config.toml`
fn resolve_config_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "amity", "amity")?;
    Some(dirs.config_dir().join("config.toml"))
}
