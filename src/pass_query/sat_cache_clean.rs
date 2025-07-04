use chrono::Utc;
use std::{collections::HashMap, fs};
use super::sat_pass_predict::SatPassData;

const CACHE_FILE: &str = "sat_pass_cache.json";

pub async fn clean_expired_cache() -> anyhow::Result<()> {
    let content = match fs::read_to_string(CACHE_FILE) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("读取缓存文件失败: {}", e);
            return Ok(());
        }
    };

    let mut data: HashMap<String, SatPassData> = match serde_json::from_str(&content) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("解析缓存数据失败: {}", e);
            return Ok(());
        }
    };

    let now = Utc::now().timestamp();
    let mut expired_confirm = false;

    for sat_data in data.values_mut() {
        let original_len = sat_data.passes.len();
        sat_data.passes.retain(|p| p.startUTC > now);
        if sat_data.passes.len() < original_len {
            expired_confirm = true;
        }
    }

    if expired_confirm {
        let serialized = serde_json::to_string_pretty(&data)?;
        fs::write(CACHE_FILE, serialized)?;
        tracing::info!("已清除过时缓存");
    }

    Ok(())
}
