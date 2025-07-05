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
                let utc_time = Utc.timestamp_opt(pass.startUTC, 0).single().unwrap_or(Utc::now());
                let bjt_time = utc_time + Duration::hours(8);
                let hour = bjt_time.hour();
                let (_is_pm, _hour12) = bjt_time.hour12();
                let _am_pm = if _is_pm { "下午" } else { "上午" };
    
                result.push(format!(
                    "[提醒]\n卫星 {} 即将过境，预计时间为北京时间{}:{:02}",
                    sat.satname,
                    hour,
                    bjt_time.minute()
                ));
            }
        }
    }

    result
}
