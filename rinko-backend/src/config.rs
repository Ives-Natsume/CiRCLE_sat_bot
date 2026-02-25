use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// gRPC server listening configuration — maps to the `[server]` table in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerListenConfig {
    /// IP address to bind (e.g. "127.0.0.1" for loopback, "0.0.0.0" for all interfaces)
    #[serde(default = "default_host")]
    pub host: String,

    /// TCP port for the gRPC server
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerListenConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// Top-level backend configuration — mirrors the structure of config.toml.
///
/// Unknown TOML keys/tables (e.g. `[qq]`, `[backend]`, `[media_server]`) are
/// silently ignored by serde, so the backend can share the same config.toml as
/// the frontend without error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Tracing log level: "trace" | "debug" | "info" | "warn" | "error"
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// gRPC server listening address (see `[server]` in config.toml)
    #[serde(default)]
    pub server: ServerListenConfig,

    /// Optional base URL of a co-hosted media server (e.g. "media.example.com").
    /// When present the backend probes `https://<url>/health` before embedding
    /// image URLs in bot replies.  Maps to the top-level `media_server_url` key
    /// in config.toml.
    #[serde(default)]
    pub media_server_url: Option<String>,

    /// Optional ASRTU telemetry API endpoint URL.
    ///
    /// When configured, the backend will query this endpoint during each
    /// satellite update cycle and merge ASRTU-1 (AO-123) repeater status into
    /// satellite search results.
    #[serde(default)]
    pub asrtu_api_url: Option<String>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            server: ServerListenConfig::default(),
            media_server_url: None,
            asrtu_api_url: None,
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    50051
}

fn default_log_level() -> String {
    "info".to_string()
}

pub static CONFIG: OnceLock<BackendConfig> = OnceLock::new();

/// Read and parse the configuration file.
///
/// B-4 fixes applied (mirroring the same fixes made to rinko-frontend):
/// - E-1: propagate all errors via `?` instead of calling `panic!()`
/// - E-2: use `map_err` on `OnceLock::set` so double-init is a real error
/// - E-3: resolve path from `$RINKO_CONFIG` env var, falling back to `../config.toml`
pub fn read_config() -> anyhow::Result<()> {
    let path = std::env::var("RINKO_CONFIG")
        .unwrap_or_else(|_| "../config.toml".to_string());

    let config_str = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;

    let config: BackendConfig = toml::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse config file '{}': {}", path, e))?;

    CONFIG
        .set(config)
        .map_err(|_| anyhow::anyhow!("Config is already initialized (read_config called twice)"))?;

    Ok(())
}

impl BackendConfig {
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}
