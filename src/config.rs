#![allow(unused)]
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc
};
use crate::i18n;
use crate::response::ApiResponse;

pub const CONFIG_PATH: &str = "config.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub bot_config: BotConfig,
    pub backend_config: BackendConfig,
    pub pass_api_config: PassApiConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BotConfig {
    pub url: String,
    pub listen_addr: String,
    pub qq_id: String,
    pub group_id: Vec<u64>,
    pub admin_id: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendConfig {
    pub timeout: u64,
    pub concurrent_limit: u64,
    /// 该参数中的群聊开放过境查询相关模块
    pub pass_predict_group_id: Option<Vec<u64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PassApiConfig {
    pub host: String,
    pub api_key: String,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub day: u32,
    pub min_elevation: u32,
}

pub trait ConfigProvider: Send + Sync + 'static {
    fn get_config(&self) -> ApiResponse<Config>;
}

pub struct FileConfigProvider {
    path: String,
}

impl FileConfigProvider {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    fn load_config(&self) -> ApiResponse<Config> {
        let config_str = match std::fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(e) => {
                let error_msg = format!("{} {}: {}", self.path, i18n::text("config_file_read_error"), e);
                tracing::error!("{}", error_msg);
                return ApiResponse::error(error_msg);
            }
        };

        if config_str.is_empty() {
            let error_msg = format!("{}: {}", i18n::text("config_file_empty"), self.path);
            tracing::error!("{}", error_msg);
            return ApiResponse::error(error_msg);
        }

        match serde_json::from_str(&config_str) {
            Ok(config) => ApiResponse::ok(config),
            Err(e) => {
                let error_msg = format!("{}: {}", i18n::text("config_file_parse_error"), e);
                tracing::error!("{}", error_msg);
                ApiResponse::error(error_msg)
            }
        }
    }
}

impl ConfigProvider for FileConfigProvider {
    fn get_config(&self) -> ApiResponse<Config> {
        self.load_config()
    }
}

lazy_static::lazy_static! {
    static ref CONFIG_PROVIDER: Arc<dyn ConfigProvider> =
        Arc::new(FileConfigProvider::new(CONFIG_PATH));
}

pub fn get_config() -> ApiResponse<Config> {
    CONFIG_PROVIDER.get_config()
}

// Documentation structure for commands and help
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Doc {
    pub help: Vec<String>,
    pub about: Vec<String>,
}