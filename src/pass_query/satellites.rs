use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs};
use once_cell::sync::Lazy;
use std::sync::RwLock;

const MAIN_FILE: &str = "satellites.toml";
const TEMP_FILE: &str = "temp_sat_cache.toml";

#[derive(Debug, Clone, Deserialize)]
pub struct AllSatInfo {
    pub aliases: Option<Vec<String>>,
    pub id: Option<u32>,
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

pub fn get_satellite_aliases() -> HashMap<String, Vec<String>> {
    use crate::query::sat_query::sat_name_normalize;
    
    let satellite_list = SATELLITE_LIST.read().unwrap();
    let mut map = HashMap::new();

    for (name, info) in satellite_list.iter() {
        let mut aliases = Vec::new();

        if let Some(alias_list) = &info.aliases {
            aliases.extend(alias_list.iter().map(|s| sat_name_normalize(s)));
        }

        if let Some(id) = info.id {
            aliases.push(id.to_string());
        }

        aliases.push(sat_name_normalize(name));

        map.insert(name.clone(), aliases);
    }

    map
}

pub fn refresh_satellite_list() {
    let new_map = load_combined_satellites();
    let mut map = SATELLITE_LIST.write().unwrap();
    *map = new_map;
}

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

pub fn get_notify_id_list(
    sat_map: &HashMap<String, AllSatInfo>
) -> Vec<u32> {
    sat_map
        .values()
        .filter(|s| s.notify)
        .filter_map(|s| s.id)
        .collect()
}