use serde::Deserialize;
use std::{collections::HashMap, fs};
use once_cell::sync::Lazy;
use std::sync::RwLock;

pub type SatelliteMap = HashMap<String, AllSatInfo>;


const MAIN_FILE: &str = "satellites.toml";
const TEMP_FILE: &str = "temp_sat_cache.toml";

#[derive(Debug, Clone, Deserialize)]
pub struct AllSatInfo {
    pub aliases: Option<Vec<String>>,
    pub id: u32,
    #[serde(default)]
    pub track: bool,
    #[serde(default)]
    pub notify: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SatInfo {
    pub id: u32,
}

fn load_combined_satellites() -> HashMap<String, AllSatInfo> {
    let main_content = fs::read_to_string(MAIN_FILE).unwrap_or_else(|_| {
        eprintln!("无法读取 {}", MAIN_FILE);
        String::new()
    });

    let mut main_map: HashMap<String, AllSatInfo> =
        toml::from_str(&main_content).unwrap_or_else(|_| {
            eprintln!("解析 {} 失败", MAIN_FILE);
            HashMap::new()
        });

    let temp_content = fs::read_to_string(TEMP_FILE).unwrap_or_else(|_| {
        eprintln!("无法读取 {}", TEMP_FILE);
        String::new()
    });

    let temp_map: HashMap<String, AllSatInfo> =
        toml::from_str(&temp_content).unwrap_or_else(|_| {
            eprintln!("解析 {} 失败", TEMP_FILE);
            HashMap::new()
        });

    for (name, info) in temp_map {
        main_map.insert(name, info);
    }

    main_map
}

pub static SATELLITE_LIST: Lazy<RwLock<HashMap<String, AllSatInfo>>> = Lazy::new(|| {
    RwLock::new(load_combined_satellites())
});

pub static SATELLITE_ALIASES: Lazy<HashMap<String, Vec<String>>> = Lazy::new(|| {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (name, info) in SATELLITE_LIST.iter() {
        map.insert(name.clone(), info.aliases.clone());
    }
    map
});

pub fn get_track_sat_list(
    satellites: &HashMap<String, AllSatInfo>
) -> HashMap<String, SatInfo> {
    satellites
        .iter()
        .filter_map(|(name, info)| {
            if info.track {
                info.id.map(|id| (name.clone(), SatInfo { id }))
            } else {
                None
            }
        })
        .collect()
}

pub fn get_notify_id_list() -> Vec<u32> {
    SATELLITE_LIST
        .values()
        .filter(|s| s.notify)
        .map(|s| s.id)
        .collect()
}