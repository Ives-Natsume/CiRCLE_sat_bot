use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub bot_config: BotConfig,
    pub backend_config: BackendConfig,
    pub n2yo_config: N2yoConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BotConfig {
    pub url: String,
    pub qq_id: String,
    pub group_id: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendConfig {
    pub about: Option<Vec<String>>,
    pub help: Option<Vec<String>>,
    pub timeout: u64,
    pub concurrent_limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct N2yoConfig {
    pub api_key: String,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub day: u32,
    pub min_elevation: u32,
}

pub fn load_config(config_path: &str) -> Config {
    let config_str = std::fs::read_to_string(config_path)
        .unwrap_or_else(|_| {
            tracing::warn!("Failed to read config file.");
            tracing::warn!("Exit.");
            panic!()
        });

    if config_str.is_empty() {
        tracing::warn!("No config data found or file is empty.");
        tracing::warn!("Exit.");
        panic!()
    }

    serde_json::from_str(&config_str).unwrap_or_else(|err| {
        tracing::error!("Failed to parse config file: {}", err);
        tracing::warn!("Exit.");
        panic!()
    })
}