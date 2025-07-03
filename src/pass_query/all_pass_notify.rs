use chrono::{Utc, TimeZone};
use std::{collections::HashMap, fs};
use super::sat_pass_predict::SatPassData;

const CACHE_FILE: &str = "sat_pass_cache.json";

pub async fn get_all_sats_pass() -> Vec<String> {
    let content = match fs::read_to_string(CACHE_FILE) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("读取缓存文件失败: {}", e);
            return Vec::new();
        }
    };

    let data: HashMap<String, SatPassData> = match serde_json::from_str(&content) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("解析缓存数据失败: {}", e);
            return Vec::new();
        }
    };

    let now = Utc::now().timestamp();
    let mut passes = Vec::new();

    for sat in data.values() {
        if let Some(next_pass) = sat.passes.iter().filter(|p| p.startUTC > now).min_by_key(|p| p.startUTC) {
            let countdown = next_pass.startUTC - now;
            let utc_time = Utc.timestamp_opt(next_pass.startUTC, 0).single().unwrap_or(Utc::now());

            passes.push((
                countdown,
                format!(
                    "[预告] 卫星 {} 距离过境还有 {} 秒，过境时间为 {} UTC",
                    sat.satname,
                    countdown,
                    utc_time.format("%Y-%m-%d %H:%M:%S")
                ),
            ));
        }
    }
    passes.sort_by_key(|(countdown, _)| *countdown);
    passes.into_iter().map(|(_, msg)| msg).collect()
}
