use std::sync::{Mutex, Arc};
use once_cell::sync::Lazy;
use reqwest::Client;
use tracing::error;
use crate::config::Config;

pub static EXTRA_IDS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

pub async fn add_extra_id(new_id: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut ids = EXTRA_IDS.lock().unwrap();

    if ids.contains(&new_id.to_string()) {
        result.push(format!("{}已在临时列表中喵~", new_id));
        return;
    }

    ids.push(new_id.to_string());

    let url = format!(
        "{}?extra_ids={}&apikey={}",
        conf.host,ids.join(","),conf.api_key
    );

    match Client::new().get(&url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                result.push(format!("{}添加成功喵~", new_id))
            } else {
                result.push(format!("API请求失败了呢，返回了{}...", resp.status()))
            }
        }
        Err(e) => {
            error!("请求失败: {}", e);
            result.push("请求API失败了喵...".to_string())
        }
    }
}

pub fn delete_extra_id(target_id: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut ids = EXTRA_IDS.lock().unwrap();

    if let Some(pos) = ids.iter().position(|x| x == target_id) {
        ids.remove(pos);
        result.push(format!("成功移除{}喵~", target_id))
    } else {
        result.push(format!("{}不在临时列表中喵...", target_id))
    }
}

pub fn list_extra_ids() -> Vec<String> {
    EXTRA_IDS.lock().unwrap().clone()
}
