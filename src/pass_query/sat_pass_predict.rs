use chrono::{Duration, Local, TimeZone};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};
use super::satellites::{SATELLITE_LIST, get_track_sat_list};
use crate::config::Config;
use crate::query::sat_query::sat_name_normalize;

const CACHE_FILE: &str = "sat_pass_cache.json";

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PassInfo {
    pub startUTC: i64,
    pub maxEl: f64,
    pub maxUTC: i64,
    pub endUTC: i64,
    pub duration: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SatPassData {
    pub satid: u32,
    pub satname: String,
    pub passes: Vec<PassInfo>,
    pub last_update: i64,
}

pub async fn update_sat_pass_cache(config: &Config) -> anyhow::Result<()> {
    let client = Client::new();
    let mut cache: HashMap<String, SatPassData> = HashMap::new();
    let conf = config.pass_api_config.clone();

    let track_sat_list = {
        let sat_list = SATELLITE_LIST.read().unwrap();
        get_track_sat_list(&sat_list)
    };

    for (name, sat_info) in track_sat_list.iter() {
        let url = format!(
            "{}/{}/{}/{}/{}/{}/{}&apikey={}",
            conf.host, sat_info.id, conf.lat, conf.lon, conf.alt, conf.day, conf.min_elevation, conf.api_key
        );

        match client.get(&url).send().await {
            Ok(response) => match response.text().await {
                Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(json) => {
                        let info = &json["info"];
                        let defaut_vec = Vec::new();
                        let passes = json["passes"].as_array().unwrap_or(&defaut_vec);

                        let parsed_passes = passes
                            .iter()
                            .map(|p| {
                                let start = p["startUTC"].as_i64().unwrap_or(0);
                                let end = p["endUTC"].as_i64().unwrap_or(0);
                                let duration = if end > start {(end - start) as u64} else {0};

                                PassInfo {
                                    startUTC: start,
                                    maxEl: p["maxEl"].as_f64().unwrap_or(0.0),
                                    maxUTC: p["maxUTC"].as_i64().unwrap_or(0),
                                    endUTC: end,
                                    duration,
                                }
                            })
                            .collect();

                        cache.insert(
                            name.clone(),
                            SatPassData {
                                satid: info["satid"].as_u64().unwrap_or(0) as u32,
                                satname: info["satname"]
                                    .as_str()
                                    .unwrap_or(name)
                                    .to_string(),
                                passes: parsed_passes,
                                last_update: chrono::Utc::now().timestamp(),
                            },
                        );
                    }
                    Err(e) => {
                        tracing::error!("解析 JSON 失败：{} ({})", name, e);
                    }
                },
                Err(e) => {
                    tracing::error!("读取响应正文失败：{} ({})", name, e);
                }
            },
            Err(e) => {
                tracing::error!("请求失败：{} ({})", name, e);
            }
        }
    }

    let serialized = serde_json::to_string_pretty(&cache)?;
    fs::write(CACHE_FILE, serialized)?;

    let bj_now = chrono::Utc::now() + Duration::hours(8);
    let fmt_time = bj_now.format("%Y年%m月%d日%H时%M分").to_string();
    tracing::info!("卫星预测信息更新时间: {}", fmt_time);

    Ok(())
}

fn _need_update_cache() -> bool {
    if !Path::new(CACHE_FILE).exists() {
        return true;
    }
    let content = match fs::read_to_string(CACHE_FILE) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("读取缓存文件失败: {}", e);
            return true;
        }
    };

    // if let Ok(data): Result<HashMap<String, SatPassData>, _> = serde_json::from_str(&content) {
    //     let latest = data.values().map(|d| d.last_update).max().unwrap_or(0);
    //     let now = chrono::Utc::now().timestamp();
    //     return now - latest > 60 * 60 * 24 * 2;
    // }

    let data: HashMap<String, SatPassData> = match serde_json::from_str(&content) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("解析缓存数据失败: {}", e);
            return true;
        }
    };
    let latest = data.values().map(|d| d.last_update).max().unwrap_or(0);
    let now = chrono::Utc::now().timestamp();

    return now - latest > 60 * 60 * 24 * 2;
}

pub fn query_satellite(name: Option<String>) -> Vec<String> {
    let content = fs::read_to_string(CACHE_FILE).unwrap_or_default();
    let data: HashMap<String, SatPassData> = serde_json::from_str(&content).unwrap_or_default();
    let mut result = Vec::new();
    let now = chrono::Utc::now().timestamp();

    match name {
        Some(n) => {
            let match_name = find_alias_match(&n);
            if let Some(key) = match_name {
                if let Some(sat) = data.get(&key) {
                    if let Some(p) = sat.passes.iter().find(|p| p.endUTC > now) {
                        let start = Local.timestamp_opt(p.startUTC, 0)
                            .unwrap();
                        let end = Local.timestamp_opt(p.endUTC, 0)
                            .unwrap();
                        result.push(format!(
                            "[预测]\n{} 下一次过境将在{}开始，最高仰角{:.1}°，{}结束喵",
                            sat.satname,
                            start.format("%m-%d %H:%M"),
                            p.maxEl,
                            end.format("%m-%d %H:%M")
                        ));
                    } else {
                        result.push("没有即将过境的信息呢".to_string());
                    }
                } else {
                    result.push("缓存里没有它的数据呢".to_string());
                }
            } else {
                result.push("这是什么？能吃吗".to_string());
            }
        }
        None => {
            result.push("Rinko不知道你要查什么喵...".to_string());
        }
    }

    result
}

pub fn find_alias_match(query: &str) -> Option<String> {
    use crate::pass_query::satellites::get_satellite_aliases;

    let norm_query = sat_name_normalize(query);
    let alias_map = get_satellite_aliases();

    for (key, aliases) in alias_map.iter() {
        if sat_name_normalize(key) == norm_query || aliases.iter().any(|a| a == &norm_query || a == query) {
            return Some(key.to_string());
        }
    }

    None
}