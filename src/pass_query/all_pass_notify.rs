use chrono::{Utc, TimeZone, Duration, Timelike};
use std::collections::HashMap;
use super::sat_pass_predict::SatPassData;
use super::satellites::{SATELLITE_LIST, get_notify_id_list};
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

    let sat_map = SATELLITE_LIST.read().unwrap();
    let notify_ids = get_notify_id_list(&sat_map);

    let now = Utc::now().timestamp();

    let mut active_passes = Vec::new();
    let mut upcoming_passes: Vec<(i64, String)> = Vec::new();
    let mut no_pass_info = Vec::new();
    let mut no_cache_info = Vec::new(); 

    let mut found_ids = std::collections::HashSet::new();

    for sat in data.values() {
        if !notify_ids.contains(&sat.satid) {
            continue;
        }
    
        if sat.passes.is_empty() {
            no_pass_info.push(format!("{} | 无过境信息...", sat.satname));
            found_ids.insert(sat.satid);
            continue;
        }
    
        found_ids.insert(sat.satid);
    
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

    for (name, info) in sat_map.iter() {
        if info.notify {
            if let Some(id) = info.id {
                if !found_ids.contains(&id) {
                    no_cache_info.push(format!("{} | 未缓存信息...", name));
                }
            }
        }
    }

    upcoming_passes.sort_by_key(|(countdown, _)| *countdown);

    if active_passes.is_empty() && upcoming_passes.is_empty() {
        Vec::new()
    } else {
        let mut result = vec!["[预测]".to_string()];
        result.extend(active_passes);
        result.extend(upcoming_passes.into_iter().map(|(_, msg)| msg));
        result.extend(no_pass_info);
        result.extend(no_cache_info);
        result
    }
}