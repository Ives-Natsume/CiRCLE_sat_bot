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

use notify::{Watcher, RecursiveMode, RecommendedWatcher};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::path::Path;
pub static CONFIG_PATH: &str = "config.json";
type SharedConfig = Arc<Mutex<Config>>;
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
}

use once_cell::sync::Lazy;
use std::collections::HashMap;
static I18N_MAP: Lazy<HashMap<(&'static str, &'static str), &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();

    m.insert(("status_title", "en"), "Status:");
    m.insert(("status_title", "cn"), "状态：");

    m.insert(("no_reports", "en"), "No reports for last one and half days");
    m.insert(("no_reports", "cn"), "最近amsat没人报告");

    m.insert(("unknown_status", "en"), "Unknown Status");
    m.insert(("unknown_status", "ja"), "希腊奶~");

    m.insert(("no_data", "en"), "Status: Unknown\n- no status data");
    m.insert(("no_data", "ja"), "没说");

    m.insert(("not_found", "en"), "No satellites found matching the query: ");
    m.insert(("not_found", "cn"), "amsat没有");

    // status translate
    m.insert(("Transponder/Repeater Active", "en"), "Transponder/Repeater active");
    m.insert(("Transponder/Repeater Active", "cn"), "转发器已开机");

    m.insert(("Telemetry/Beacon Only", "en"), "Telemetry/Beacon only");
    m.insert(("Telemetry/Beacon Only", "cn"), "只有信标");

    m.insert(("No Signal", "en"), "No signal");
    m.insert(("No Signal", "cn"), "无信号");

    m.insert(("Conflicting Reports", "en"), "Conflicting reports");
    m.insert(("Conflicting Reports", "cn"), "冲突报告");

    m.insert(("ISS Crew (Voice) Active", "en"), "ISS Crew (Voice) Active");
    m.insert(("ISS Crew (Voice) Active", "cn"), "乘组语音活跃");

    m.insert(("Unknown Status", "en"), "Unknown Status");
    m.insert(("Unknown Status", "cn"), "未知状态");

    m.insert(("no_data_available", "en"), "No data available");
    m.insert(("no_data_available", "cn"), "Rinko不知道呢...");

    m
});

pub fn t(key: &str, lang: &str) -> &'static str {
    I18N_MAP.get(&(key, lang)).copied().unwrap_or("[Untranslated]")
}