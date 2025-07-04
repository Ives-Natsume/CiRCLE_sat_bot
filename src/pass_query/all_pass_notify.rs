use chrono::{Utc, TimeZone, Duration, Timelike};
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
        if let Some(next_pass) = sat
            .passes
            .iter()
            .filter(|p| p.startUTC > now)
            .min_by_key(|p| p.startUTC)
        {
            let countdown = next_pass.startUTC - now;
            let minutes = countdown / 60;
            let seconds = countdown % 60;

            let utc_time = Utc.timestamp_opt(next_pass.startUTC, 0).single().unwrap_or(Utc::now());
            let bjt_time = utc_time + Duration::hours(8);
            let (is_pm, hour12) = bjt_time.hour12();
            let am_pm = if is_pm { "下午" } else { "上午" };

            let bjt_formatted = format!("{}{}点{}分", am_pm, hour12, bjt_time.minute());

            passes.push((
                countdown,
                format!(
                    "{}: {}分{}秒后，{}过境",
                    sat.satname, minutes, seconds, bjt_formatted
                ),
            ));
        }
    }
    passes.sort_by_key(|(countdown, _)| *countdown);

    if passes.is_empty() {
        Vec::new()
    } else {
        let mut result = vec!["[预告]".to_string()];
        result.extend(passes.into_iter().map(|(_, msg)| msg + "；"));
        result
    }
}
