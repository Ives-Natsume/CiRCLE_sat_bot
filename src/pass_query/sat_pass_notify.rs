use chrono::{Utc, TimeZone, Duration, Timelike};
use std::{collections::HashMap, fs};
use super::sat_pass_predict::SatPassData;

const CACHE_FILE: &str = "sat_pass_cache.json";

pub async fn check_upcoming_passes() -> Vec<String> {
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
    let mut result = Vec::new();

    for sat in data.values() {
        for pass in &sat.passes {
            let countdown = pass.startUTC - now;
    
            if countdown <= 3600 && countdown > 3540 {
                result.push(format!(
                    "[提醒]\n卫星 {} 预计将在1h后过境喵...",
                    sat.satname
                ));
            } else if countdown <= 60 && countdown > 0 {
                let start_bjt = Utc.timestamp_opt(pass.startUTC, 0).single().unwrap_or(Utc::now()) + Duration::hours(8);
                let end_bjt = Utc.timestamp_opt(pass.endUTC, 0).single().unwrap_or(Utc::now()) + Duration::hours(8);
            
                result.push(format!(
                    "[提醒]\n>>> 卫星过境中 >>>\n{:02}:{:02} -> [{}] -> {:02}:{:02}\n速来建工楼顶喵！",
                    start_bjt.hour(),
                    start_bjt.minute(),
                    sat.satname,
                    end_bjt.hour(),
                    end_bjt.minute()
                ));
            }
            
        }

    result
}
