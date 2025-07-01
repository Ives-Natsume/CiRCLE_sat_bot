use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};
use tokio::time::Instant;
use crate::satellites::SATELLITE_LIST;

const CACHE_FILE: &str = "sat_pass_cache.json";
const LAT: &str = "34.3242";
const LON: &str = "108.8750";
const ALT: &str = "200";
const MIN_ELEVATION: &str = "10";
const DAY: &str = "3";
const API_KEY: &str = "xxx";

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

pub async fn update_sat_pass_cache() -> anyhow::Result<()> {
    let client = Client::new();
    let mut cache: HashMap<String, SatPassData> = HashMap::new();

    for (name, sat_info) in SATELLITE_LIST.iter() {
        let url = format!(
            "https://api.n2yo.com/rest/v1/satellite/visualpasses/{}/{}/{}/{}/{}/{}/&apiKey={}",
            sat_info.id, LAT, LON, ALT, DAY, MIN_ELEVATION, API_KEY
        );

        //请求api
        match client.get(&url).send().await {
            Ok(response) => match response.text().await {
                Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(json) => {
                        let info = &json["info"];
                        let passes = json["passes"].as_array().unwrap_or(&vec![]);

                        let parsed_passes = passes
                            .iter()
                            .map(|p| PassInfo {
                                startUTC: p["startUTC"].as_i64().unwrap_or(0),
                                maxEl: p["maxEl"].as_f64().unwrap_or(0.0),
                                maxUTC: p["maxUTC"].as_i64().unwrap_or(0),
                                endUTC: p["endUTC"].as_i64().unwrap_or(0),
                                duration: p["duration"].as_u64().unwrap_or(0),
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
                        eprintln!("解析 JSON 失败：{} ({})", name, e);
                    }
                },
                Err(e) => {
                    eprintln!("读取响应正文失败：{} ({})", name, e);
                }
            },
            Err(e) => {
                eprintln!("请求失败：{} ({})", name, e);
            }
        }
    }

    // 缓存api数据并推送缓存时间
    let serialized = serde_json::to_string_pretty(&cache)?;
    fs::write(CACHE_FILE, serialized)?;

    let bj_now = chrono::Utc::now() + Duration::hours(8);
    let fmt_time = bj_now.format("%Y年%m月%d日%H时%M分").to_string();
    println!("卫星预测信息更新时间: {}", fmt_time);

    Ok(())
}

// 缓存过期(2days)判定
pub fn need_update_cache() -> bool {
    if !Path::new(CACHE_FILE).exists() {
        return true;
    }
    let content = match fs::read_to_string(CACHE_FILE) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("读取缓存文件失败: {}", e);
            return true;
        }
    };

    if let Ok(data): Result<HashMap<String, SatPassData>, _> = serde_json::from_str(&content) {
        let latest = data.values().map(|d| d.last_update).max().unwrap_or(0);
        let now = chrono::Utc::now().timestamp();
        return now - latest > 60 * 60 * 24 * 2;
    }
    true
}

// 数据可视化并推送
pub fn query_satellite(name: Option<String>) -> Vec<String> {
    let content = fs::read_to_string(CACHE_FILE).unwrap_or_default();
    let data: HashMap<String, SatPassData> = serde_json::from_str(&content).unwrap_or_default();
    let mut result = Vec::new();

    match name {
        Some(n) => {
            let match_name = find_alias_match(&n);
            if let Some(key) = match_name {
                if let Some(sat) = data.get(&key) {
                    if let Some(p) = sat.passes.first() {
                        let start = Local.timestamp_opt(p.startUTC, 0).unwrap_or_else(|| Local::now());
                        let end = Local.timestamp_opt(p.endUTC, 0).unwrap();
                        let max = Local.timestamp_opt(p.maxUTC, 0).unwrap();
                        result.push(format!(
                            "{} 过境：起始 {}，最高仰角 {:.1}°，结束 {}，持续 {} 秒",
                            sat.satname,
                            start.format("%m-%d %H:%M"),
                            p.maxEl,
                            end.format("%m-%d %H:%M"),
                            p.duration
                        ));
                    }
                } else {
                    result.push("无对应卫星缓存数据".to_string());
                }
            } else {
                result.push("未识别的卫星名".to_string());
            }
        }
        None => {
            for sat in data.values() {
                if let Some(p) = sat.passes.first() {
                    let max_time = Local.timestamp_opt(p.maxUTC, 0).unwrap();
                    let delta = max_time - Local::now();
                    result.push(format!(
                        "{} 下一次过境时间: {}，倒计时: {} 分钟",
                        sat.satname,
                        max_time.format("%m-%d %H:%M"),
                        delta.num_minutes()
                    ));
                }
            }
        }
    }

    result
}

fn find_alias_match(query: &str) -> Option<String> {
    use crate::satellites::SATELLITE_ALIASES;
    for (key, aliases) in SATELLITE_ALIASES.iter() {
        if key.eq_ignore_ascii_case(query) || aliases.iter().any(|a| a.eq_ignore_ascii_case(query)) {
            return Some(key.clone());
        }
    }
    None
}
