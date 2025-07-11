use crate::pass_query::satellites::SATELLITE_LIST;
use std::collections::{HashMap, HashSet};
use std::fs;
use crate::pass_query::satellites::AllSatInfo;


const TEMP_FILE: &str = "temp_sat_cache.toml";

pub fn list_sat_list() -> Vec<String> {
    let all_map = SATELLITE_LIST.read().unwrap();

    let temp_content = fs::read_to_string(TEMP_FILE).unwrap_or_default();
    let temp_map: HashMap<String, AllSatInfo> = toml::from_str(&temp_content).unwrap_or_default();
    let temp_names: HashSet<_> = temp_map.keys().cloned().collect();

    let mut result = vec!["[列表]".to_string()];
    let mut main_entries = Vec::new();
    let mut temp_entries = Vec::new();

    for (name, info) in all_map.iter() {
        if let Some(id) = info.id {
            if info.track || info.notify {
                let prefix = if temp_names.contains(name) {
                    format!("{} -> {}", id, name)
                } else {
                    format!("[{} -> {}]", id, name)
                };

                let track_mark = if info.track { "√" } else { "×" };
                let notify_mark = if info.notify { "√" } else { "×" };

                let line = format!(
                    "{} | t: {} | n: {}",
                    prefix, track_mark, notify_mark
                );

                if temp_names.contains(name) {
                    temp_entries.push(line);
                } else {
                    main_entries.push(line);
                }
            }
        }
    }

    result.extend(main_entries);
    result.extend(temp_entries);
    result
}
