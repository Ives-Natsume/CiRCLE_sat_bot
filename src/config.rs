use serde::{Deserialize, Serialize};
use once_cell::sync::Lazy;
use std::fs;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub bot_config: BotConfig,
    pub backend_config: BackendConfig,
    pub pass_api_config: PassApiConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BotConfig {
    pub url: String,
    pub qq_id: String,
    pub group_id: Vec<u64>,
    pub admin_id: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendConfig {
    pub about: Option<Vec<String>>,
    pub help: Option<Vec<String>>,
    pub timeout: u64,
    pub concurrent_limit: u64,
    pub special_group_id: Option<Vec<u64>>,
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

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let content = fs::read_to_string("config.json").expect("无法读取配置文件");
    serde_json::from_str::<Config>(&content).expect("配置解析失败")
});

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

//use notify::{Watcher, RecursiveMode, RecommendedWatcher};
//use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
//use std::path::Path;
pub static CONFIG_PATH: &str = "config.json";
type SharedConfig = Arc<Mutex<Config>>;

/*
pub fn spawn_config_watcher(shared_config: SharedConfig) {
    std::thread::spawn(move || {
        let (tx, rx) = channel();

        let mut watcher = RecommendedWatcher::new(tx, notify::Config::default())
            .expect("Failed to create watcher");

        watcher
            .watch(Path::new(CONFIG_PATH), RecursiveMode::NonRecursive)
            .expect("Failed to watch config file");

        while let Ok(_event) = rx.recv() {
            tracing::info!("Config file change detected. Reloading...");
            match std::fs::read_to_string(CONFIG_PATH) {
                Ok(new_config_str) => match serde_json::from_str::<Config>(&new_config_str) {
                    Ok(new_config) => {
                        let mut config_guard = shared_config.lock().unwrap();
                        *config_guard = new_config;
                        tracing::info!("Config reloaded successfully.");
                    }
                    Err(e) => {
                        tracing::error!("Failed to parse updated config file: {}", e);
                    }
                },
                Err(e) => {
                    tracing::error!("Failed to read updated config file: {}", e);
                }
            }
        }
    });
*/

pub fn spawn_config_watcher(shared_config: SharedConfig) {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(new_config_str) => {
            match serde_json::from_str::<Config>(&new_config_str) {
                Ok(new_config) => {
                    let mut config_guard = shared_config.lock().unwrap();
                    *config_guard = new_config;
                    tracing::info!("Config loaded successfully.");
                }
                Err(e) => {
                    tracing::error!("Failed to parse config file: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to read config file: {}", e);
        }
    }
}

use std::collections::HashMap;
static I18N_MAP: Lazy<HashMap<(&'static str, &'static str), &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();

    // status translate
    m.insert(("Transponder/Repeater Active", "en"), "Transponder/Repeater active");
    m.insert(("Transponder/Repeater Active", "cn"), "转发器已开机");

    m.insert(("Telemetry/Beacon Only", "en"), "Telemetry/Beacon only");
    m.insert(("Telemetry/Beacon Only", "cn"), "只有信标回波");

    m.insert(("No Signal", "en"), "No signal");
    m.insert(("No Signal", "cn"), "完全没有信号");

    m.insert(("Conflicting Reports", "en"), "Conflicting reports");
    m.insert(("Conflicting Reports", "cn"), "报告有冲突");

    m.insert(("ISS Crew(Voice) Active", "en"), "ISS Crew(Voice) Active");
    m.insert(("ISS Crew(Voice) Active", "cn"), "乘组语音活跃中");

    m.insert(("Unknown Status", "en"), "Unknown Status");
    m.insert(("Unknown Status", "cn"), "罕见的未知状态");

    m.insert(("no_data_available", "en"), "No data available");
    m.insert(("no_data_available", "cn"), "Rinko不知道呢...");

    m
});

pub fn t(key: &str, lang: &str) -> &'static str {
    I18N_MAP.get(&(key, lang)).copied().unwrap_or("[Untranslated]")
}
