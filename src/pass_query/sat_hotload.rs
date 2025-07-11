use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tokio::fs;
use toml;
use tracing::error;
use reqwest::Client;
use anyhow::Result;
use crate::config::Config;
use super::sat_pass_predict::{update_sat_pass_cache, find_alias_match};
use super::satellites::refresh_satellite_list;

const TEMP_FILE: &str = "temp_sat_cache.toml";

#[derive(Serialize, Deserialize, Debug)]
struct TempSatList(HashMap<String, TempSatInfo>);

#[derive(Serialize, Deserialize, Debug)]
pub struct TempSatInfo {
    pub id: u32,
    #[serde(default)]
    pub track: bool,
    #[serde(default)]
    pub notify: bool,
}

#[derive(Deserialize)]
struct SatApiResponse {
    name: String,
}

async fn update_remote_tle(extra_ids: &[u32], config: &Config) -> Result<(), String> {
    if extra_ids.is_empty() {
        return Ok(());
    }

    let conf = &config.pass_api_config;
    let id_str = extra_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
    let url = format!("{}/update_tle?extra_ids={}&apikey={}", conf.host, id_str, conf.api_key);

    match Client::new().get(&url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("更新 TLE 失败，状态码: {}", resp.status()))
            }
        }
        Err(e) => Err(format!("更新 TLE 请求失败: {}", e)),
    }
}

pub async fn add_to_temp_list(id: u32, config: &Config) -> Vec<String> {
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

                        cache.0.insert(name.clone(), TempSatInfo {
                            id,
                            track: true,
                            notify: false,
                        });

                        match toml::to_string_pretty(&cache) {
                            Ok(toml_string) => {
                                if let Err(e) = fs::write(TEMP_FILE, toml_string).await {
                                    error!("写入缓存失败: {}", e);
                                    result.push("写入缓存失败喵...".to_string());
                                } else {
                                    result.push(format!("{}->{} 添加成功喵~", id, name));

                                    refresh_satellite_list();
                                    let all_ids: Vec<u32> = cache.0.values().map(|info| info.id).collect();
                                    if let Err(e) = update_remote_tle(&all_ids, config).await {
                                        tracing::error!("更新远程 TLE 失败: {}", e);
                                    }

                                    if let Err(e) = update_sat_pass_cache(config).await {
                                        error!("更新主缓存失败: {}", e);
                                        result.push("同步主缓存失败喵...".to_string());
                                    }
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
            result.push("请求失败了喵...".to_string());
        }
    }

    result
}

pub async fn remove_from_temp_list(name_or_id: &str, config: &Config) -> Vec<String> {
    let mut result = Vec::new();

    match load_temp_list().await {
        Ok(mut cache) => {
            let key_to_remove_opt = if let Ok(id) = name_or_id.parse::<u32>() {
                cache.0.iter()
                    .find(|(_, info)| info.id == id)
                    .map(|(k, _)| k.clone())
            } else {
                cache.0.get_key_value(name_or_id).map(|(k, _)| k.clone())
            };

            if let Some(key_to_remove) = key_to_remove_opt {
                let removed = cache.0.remove(&key_to_remove);
                if let Some(info) = removed {
                    match toml::to_string_pretty(&cache) {
                        Ok(toml_string) => {
                            if let Err(e) = fs::write(TEMP_FILE, toml_string).await {
                                error!("写入失败: {}", e);
                                result.push("写入缓存失败喵...".to_string());
                            } else {
                                result.push(format!("{}->{} 移除成功喵~", info.id, key_to_remove));

                                refresh_satellite_list();
                                let all_ids: Vec<u32> = cache.0.values().map(|info| info.id).collect();
                                if let Err(e) = update_remote_tle(&all_ids, config).await {
                                    tracing::error!("更新远程 TLE 失败: {}", e);
                                }

                                if let Err(e) = update_sat_pass_cache(config).await {
                                    error!("更新主缓存失败: {}", e);
                                    result.push("同步主缓存失败喵...".to_string());
                                }
                            }
                        }
                        Err(e) => {
                            error!("序列化失败: {}", e);
                            result.push("缓存序列化失败喵...".to_string());
                        }
                    }
                } else {
                    result.push("移除失败了喵...".to_string());
                }
            } else {
                result.push(format!("没有找到{}喵...", name_or_id));
            }
        }
        Err(e) => {
            error!("请求失败: {}", e);
            result.push("请求失败了喵...".to_string());
        }
    }

    result
}

pub async fn set_temp_sat_permission(
    name_or_id: &str,
    field: &str,
    value: u8,
    config: &Config,
) -> Vec<String> {
    let mut result = Vec::new();

    match load_temp_list().await {
        Ok(mut cache) => {
            let key_opt = if let Ok(id) = name_or_id.parse::<u32>() {
                cache.0.iter()
                    .find(|(_, info)| info.id == id)
                    .map(|(k, _)| k.clone())
            } else {
                let name = find_alias_match(name_or_id).unwrap_or_else(|| name_or_id.to_string());
                cache.0.get_key_value(&name).map(|(k, _)| k.clone())
            };

            if let Some(key) = key_opt {
                let sat_info = cache.0.get_mut(&key).unwrap();

                match field {
                    "track" | "t" => {
                        sat_info.track = value != 0;
                        result.push(format!(
                            "{}->{} 预测功能已{}喵~",
                            sat_info.id,
                            key,
                            if value != 0 { "开启" } else { "关闭" }
                        ));
                    }
                    "notify" | "n" => {
                        sat_info.notify = value != 0;
                        result.push(format!(
                            "{}->{} 播报功能已{}喵~",
                            sat_info.id,
                            key,
                            if value != 0 { "开启" } else { "关闭" }
                        ));
                    }
                    _ => {
                        result.push("找不到这个参数喵...".to_string());
                        return result;
                    }
                }

                match toml::to_string_pretty(&cache) {
                    Ok(toml_string) => {
                        if let Err(e) = fs::write(TEMP_FILE, toml_string).await {
                            error!("写入失败: {}", e);
                            result.push("写入缓存失败喵...".to_string());
                        } else {
                            refresh_satellite_list();
                            let all_ids: Vec<u32> = cache.0.values().map(|info| info.id).collect();
                            if let Err(e) = update_remote_tle(&all_ids, config).await {
                                tracing::error!("更新远程 TLE 失败: {}", e);
                            }

                            if let Err(e) = update_sat_pass_cache(config).await {
                                error!("更新主缓存失败: {}", e);
                                result.push("同步主缓存失败喵...".to_string());
                            }
                        }
                    }
                    Err(e) => {
                        error!("序列化失败: {}", e);
                        result.push("缓存序列化失败喵...".to_string());
                    }
                }
            } else {
                result.push(format!("没有找到{}喵...", name_or_id));
            }
        }
        Err(e) => {
            error!("缓存加载失败: {}", e);
            result.push("缓存加载失败了喵...".to_string());
        }
    }

    result
}

async fn load_temp_list() -> Result<TempSatList> {
    let content = fs::read_to_string(TEMP_FILE).await?;
    let cache: TempSatList = toml::from_str(&content)?;
    Ok(cache)
}