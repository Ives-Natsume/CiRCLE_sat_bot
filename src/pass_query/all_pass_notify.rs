use chrono::{Utc, TimeZone, Duration, Timelike, Local};
use std::{collections::HashMap};
use super::sat_pass_predict::SatPassData;
use tokio::fs;

const CACHE_FILE: &str = "sat_pass_cache.json";

pub async fn get_all_sats_pass() -> Vec<String> {
    let content = match fs::read_to_string(CACHE_FILE).await {
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

    let mut active_passes = Vec::new();
    let mut upcoming_passes: Vec<(i64, String)> = Vec::new();

    for sat in data.values() {
        if let Some(p) = sat
            .passes
            .iter()
            .find(|p| p.startUTC <= now && p.endUTC > now)
        {
            let remaining = p.endUTC - now;
            let minutes = remaining / 60;
            let seconds = remaining % 60;
            active_passes.push(format!(
                "{} | 过境中 | 剩{}m{}s",
                sat.satname, minutes, seconds
            ));
        } else if let Some(p) = sat
            .passes
            .iter()
            .filter(|p| p.startUTC > now)
            .min_by_key(|p| p.startUTC)
        {
            let countdown = p.startUTC - now;
            let hours = countdown / 3600;
            let minutes = (countdown % 3600) / 60;
    
            let utc_time = Utc.timestamp_opt(p.startUTC, 0).single().unwrap_or(Utc::now());
            let bjt_time = utc_time + Duration::hours(8);
            let bjt_formatted = format!("{:02}:{:02}", bjt_time.hour(), bjt_time.minute());
    
            upcoming_passes.push((
                countdown,
                format!(
                    "{} | {}过境 | {}h{}m后",
                    sat.satname, bjt_formatted, hours, minutes
                ),
            ));
        }
    }
    

    upcoming_passes.sort_by_key(|(countdown, _)| *countdown);

    if active_passes.is_empty() && upcoming_passes.is_empty() {
        Vec::new()
    } else {
        let mut result = vec!["[预告]".to_string()];
        result.extend(active_passes.into_iter());
        result.extend(upcoming_passes.into_iter().map(|(_, msg)| msg));
        result
    }
}