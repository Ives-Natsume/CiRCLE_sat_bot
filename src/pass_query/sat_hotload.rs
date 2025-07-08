use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tokio::fs;
use toml;
use tracing::error;
use reqwest::Client;
use crate::config::Config;

const TEMP_LIST_FILE: &str = "temp_sat_cache.toml";

#[derive(Serialize, Deserialize, Debug)]
struct TempSatList(HashMap<String, TempSatInfo>);

#[derive(Serialize, Deserialize, Debug)]
pub struct TempSatInfo {
    pub id: u32,
}

#[derive(Deserialize)]
struct SatApiResponse {
    id: u32,
    name: String,
}

pub async fn add_to_temp_list(id: u32) -> Vec<String> {
    let mut result = Vec::new();
    let conf = config.pass_api_config.clone();

    let url = format!("{}/search?id={}&apikey={}", conf.host, id, conf.api_key);

    match Client::new().get(&url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<SatApiResponse>().await {
                    Ok(sat_info) => {
                        let name = sat_info.name.clone();
                        let mut cache = load_temp_list().await.unwrap_or_else(|_| TempSatList(HashMap::new()));

                        if cache.0.contains_key(&name) {
                            result.push(format!("{}->{} 已在缓存列表中喵~", id, name));
                            return result;
                        }

                        cache.0.insert(name.clone(), TempSatInfo { id });

                        match toml::to_string_pretty(&cache) {
                            Ok(toml_string) => {
                                if let Err(e) = fs::write(TEMP_LIST_FILE, toml_string).await {
                                    error!("写入缓存失败: {}", e);
                                    result.push("写入缓存失败喵...".to_string());
                                } else {
                                    result.push(format!("{}->{} 添加成功喵~", id, name));
                                }
                            }
                            Err(e) => {
                                error!("序列化失败: {}", e);
                                result.push("缓存序列化失败喵...".to_string());
                            }
                        }
                    }
                    Err(e) => {
                        error!("解析 JSON 失败: {}", e);
                        result.push("返回的消息看不懂喵...".to_string());
                    }
                }
            } else {
                result.push(format!("API{}了喵...", resp.status()));
            }
        }
        Err(e) => {
            error!("请求失败: {}", e);
            result.push("请求API失败了喵...".to_string());
        }
    }

    result
}

pub async fn remove_from_temp_list(name: &str) -> Vec<String> {
    let mut result = Vec::new();

    match load_temp_list().await {
        Ok(mut cache) => {
            if cache.0.remove(name).is_some() {
                match toml::to_string_pretty(&cache) {
                    Ok(toml_string) => {
                        if let Err(e) = fs::write(TEMP_LIST_FILE, toml_string).await {
                            error!("写入失败: {}", e);
                            result.push(format!("写入失败喵..."));
                        } else {
                            result.push(format!("成功移除{}喵~", name));
                        }
                    }
                    Err(e) => {
                        error!("序列化失败: {}", e);
                        result.push(format!("缓存序列化失败喵..."));
                    }
                }
            } else {
                result.push(format!("{}不在缓存列表中喵...", name));
            }
        }
        Err(e) => {
            error!("读取缓存失败: {}", e);
            result.push("读取缓存失败喵...".to_string());
        }
    }

    result
}

async fn load_temp_list() -> Result<TempSatList, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(TEMP_LIST_FILE).await?;
    let cache: TempSatList = toml::from_str(&content)?;
    Ok(cache)
}
