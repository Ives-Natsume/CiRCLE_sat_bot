use std::sync::OnceLock;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub guild_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QQConfig {
    pub app_id: String,
    pub client_secret: String,       // also used as bot_secret for webhook signature verification
    pub access_token: String,
    #[serde(default)]
    pub media_base_url: Option<String>,  // Base URL for media server (e.g., "https://media.rinkosoft.me/media")
    #[serde(skip)]
    pub token_expires_in: u64,       // expire time in seconds
    #[serde(skip)]
    pub client: reqwest::Client,
    #[serde(skip)]
    pub token_fetched_at: Option<tokio::time::Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub token: String,
    pub chat_id: i64,
    #[serde(default)]
    pub media_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterpriseWeChatConfig {
    pub corp_id: String,
    pub agent_id: u32,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub enable: bool,
    pub url: String,
    pub frontend_id: String,
    pub heartbeat_interval: u64,  // in seconds
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfigs {
    pub backend: Option<BackendConfig>,
    pub discord: Option<DiscordConfig>,
    pub qq: Option<QQConfig>,
    pub telegram: Option<TelegramConfig>,
    pub enterprise_wechat: Option<EnterpriseWeChatConfig>,
    pub log_level: String,
}

pub static CONFIG: OnceLock<BotConfigs> = OnceLock::new();
    
pub fn read_config() -> anyhow::Result<()> {
    // E-3: path is resolved from $RINKO_CONFIG env var, falling back to "../config.toml".
    // The fallback is relative to the current working directory, so prefer setting
    // RINKO_CONFIG to an absolute path in production deployments.
    let path = std::env::var("RINKO_CONFIG")
        .unwrap_or_else(|_| "../config.toml".to_string());

    // E-1: propagate I/O errors instead of panicking so callers can handle them.
    let config_str = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;

    // E-1: propagate parse errors via '?' instead of panicking.
    let config: BotConfigs = toml::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse config file '{}': {}", path, e))?;

    // E-2: no unnecessary clone; map the OnceLock error to a proper anyhow error.
    CONFIG
        .set(config)
        .map_err(|_| anyhow::anyhow!("Config is already initialized (read_config called twice)"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_config() {
        read_config().unwrap();
        let config = CONFIG.get().unwrap();
        println!("Loaded config: {:#?}", config);
    }
}