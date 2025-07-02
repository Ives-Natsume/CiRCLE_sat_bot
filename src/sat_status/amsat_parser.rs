use reqwest;
use scraper::{Html, Selector};
use serde::{Serialize, Deserialize};
use tokio;
use crate::config::Config;
use crate::msg_sys;
use crate::response::ApiResponse;

const AMSAT_URL: &str = "https://www.amsat.org/status/";
const SATELLITE_STATUS_FILE: &str = "amsat_status.json";
const AMSAT_HTML_FILE: &str = "amsat_status.html";
const SATELLITE_STATUS_CACHE_FILE: &str = "amsat_status_cache.json";
const BLUE_STATUS: &str = "Transponder/Repeater Active";
const YELLOW_STATUS: &str = "Telemetry/Beacon Only";
const RED_STATUS: &str = "No Signal";
const ORANGE_STATUS: &str = "Conflictng Reports";
const PURPLE_STATUS: &str = "ISS Crew(Voice) Active";
const UNKNOWN_STATUS: &str = "Unknown Status";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StatusFlag {
    pub report_nums: u8,
    pub description: String,
}

impl StatusFlag {
    pub fn match_status_with_color(color: &str, nums: u8) -> Option<StatusFlag> {
        match color {
            "#4169E1" => Some(StatusFlag { report_nums: nums, description: BLUE_STATUS.to_string() }),
            "yellow" => Some(StatusFlag { report_nums: nums, description: YELLOW_STATUS.to_string() }),
            "red" => Some(StatusFlag { report_nums: nums, description: RED_STATUS.to_string() }),
            "orange" => Some(StatusFlag { report_nums: nums, description: ORANGE_STATUS.to_string() }),
            "#9900FF" => Some(StatusFlag { report_nums: nums, description: PURPLE_STATUS.to_string() }),
            _ => Some(StatusFlag { report_nums: 0, description: UNKNOWN_STATUS.to_string() }),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SatelliteStatus {
    pub name: String,
    pub status: Vec<Vec<StatusFlag>>,
}

impl SatelliteStatus {
    pub fn new(name: String, status: Vec<Vec<StatusFlag>>) -> Self {
        SatelliteStatus { name, status }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct SatelliteStatusCache {
    name: String,
    status: String,
}

impl SatelliteStatusCache {
    pub fn new(name: String, status: String) -> Self {
        SatelliteStatusCache { name, status }
    }
}

/// Run the amsat_module, with html parser and status watchdog
pub async fn run_amsat_module(config: &Config) -> anyhow::Result<()> {
    let response = reqwest::get(AMSAT_URL).await?;
    let html_content = response.text().await?;
    let broadcast_group_ids = config.bot_config.group_id.clone();

    // parse the HTML content to extract satellite status
    let satellite_status = get_satellite_status(&html_content);

    // save files
    let json_content = serde_json::to_string_pretty(&satellite_status).expect("Unable to serialize satellite status to JSON");
    tokio::fs::write(SATELLITE_STATUS_FILE, json_content).await?;
    tokio::fs::write(AMSAT_HTML_FILE, html_content).await?;

    tracing::info!("Satellite status saved to amsat_status.json");
    tracing::info!("HTML content saved to amsat_status.html");

    // monitor satellite status
    let api_response = monitor_satellite_status(
        satellite_status.clone(),
    ).await;

    for id in broadcast_group_ids {
        msg_sys::group_chat::send_group_msg(
            api_response.clone(),
            id,
        ).await;
    }

    Ok(())
}

// Get all Satellite name
pub fn get_satellite_names(html: &str) -> Vec<String> {
    let mut names = Vec::new();
    let document = Html::parse_document(&html);

    // selector to find the <select> element with name "SatName"
    let select_selector = Selector::parse(r#"select[name="SatName"] option"#).unwrap();

    // extract all satellite names from the <option> elements
    for option in document.select(&select_selector) {
        if let Some(value) = option.value().attr("value") {
            if !value.is_empty() {
                //println!("{}", value);
                names.push(value.to_string());
            }
        }
    }

    names
}

// get all Satellite status
pub fn get_satellite_status(html: &str) -> Vec<SatelliteStatus> {
    let document = Html::parse_document(&html);

    let tr_sel = Selector::parse("tr").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let sat_sel = Selector::parse(r#"td[align="right"] > a"#).unwrap();

    // all satellite names
    let all_sat_list = get_satellite_names(html);

    let mut current_sat = String::new();
    let mut groups: Vec<SatelliteStatus> = Vec::new();

    for tr in document.select(&tr_sel) {
        // get all <td> elements in the current <tr>
        let tds: Vec<_> = tr.select(&td_sel).collect();

        // get the first <td> element as the satellite name
        if let Some(sat_name_elem) = tds[0].select(&sat_sel).next() {
            let sat_name = sat_name_elem.text().collect::<Vec<_>>().join(" ");
            //check if the satellite name is valid
            if all_sat_list.contains(&sat_name) {
                current_sat = sat_name;
                // add the satellite if it not exists
                if !groups.iter().any(|g| g.name == current_sat) {
                    groups.push(SatelliteStatus::new(current_sat.clone(), Vec::new()));
                }
            }
        }
        else {
            // skip if the first <td> is not a satellite name
            continue;
        }

        // The AMSAT page devides a day to 12 time blocks,
        // each block is 2 hours long, and blocks on the future are not shown.
        // Calculate the valid time blocks to skip not shown blocks.
        let blocks_to_skip = calculate_valid_time_blocks();

        // get the rest of the <td> elements as the status
        // extract the status colors to match with the status flags
        let status_colors: Vec<String> = tds.iter()
            .skip(blocks_to_skip) // skip the first <td> which is the satellite name
            .filter_map(|td| td.value().attr("bgcolor").map(|s| s.to_string()))
            .collect();
        // get the report numbers
        let report_nums: Vec<String> = tds.iter()
            .skip(blocks_to_skip) // skip the first <td> which is the satellite name
            .filter_map(|td| td.text().next().map(|s| s.to_string()))
            .collect();
        // map the status colors and report numbers to StatusFlag
        let status_flags: Vec<Vec<StatusFlag>> = status_colors.iter()
            .zip(report_nums.iter())
            .map(|(color, nums)| {
                if let Some(flag) = StatusFlag::match_status_with_color(color, nums.parse().unwrap_or(0)) {
                    vec![flag]
                } else {
                    vec![StatusFlag { report_nums: 0, description: UNKNOWN_STATUS.to_string() }]
                }
            })
            .collect();
        // if the current satellite is not empty, add the status flags to the current group
        if !current_sat.is_empty() {
            // find the group with the current satellite name
            if let Some(group) = groups.iter_mut().find(|g| g.name == current_sat) {
                group.status = status_flags;
            }
        }
    }

    // return the groups
    groups
}

/// Calculate the valid time blocks to skip not shown blocks.
/// All time need to be UTC time
pub fn calculate_valid_time_blocks() -> usize {
    use chrono::{Utc, Timelike};
    let now = Utc::now();
    let current_hour = now.hour();

    let valid_blocks = if current_hour < 2 {
        1
    } else {
        (current_hour / 2) as usize
    };

    12 - valid_blocks
}

async fn monitor_satellite_status(
    satellite_status: Vec<SatelliteStatus>,
) -> ApiResponse<Vec<String>> {
    let _monitored_sats = vec![
        "ISS-FM",
        "AO-123",
        "SO-50",
        "SO-124",
        "SO-125",
        "PO-101[FM]",
        "AO-91",
        "RS-44"
    ];
    let mut data: Vec<String> = Vec::new();
    let mut success = true;
    let mut msg: String = String::new();

    // Check if the satellite status cache file exists
    if !tokio::fs::metadata(SATELLITE_STATUS_CACHE_FILE).await.is_ok() {
        // If the file does not exist, create it
        tokio::fs::File::create(SATELLITE_STATUS_CACHE_FILE).await.expect("Failed to create cache file");
    }

    // check if status cache is empty or not exists
    if let Ok(cache_content) = tokio::fs::read_to_string(SATELLITE_STATUS_CACHE_FILE).await {
        if !cache_content.is_empty() {
            // Deserialize the cached status
            let cached_status: Vec<SatelliteStatusCache> = serde_json::from_str(&cache_content)
                .expect("Failed to deserialize satellite status cache");

            // Get current time in UTC
            let _now = chrono::Utc::now();
            data.push(format!("数据已更新喵~"));

            // Analyse status changes
            for sat in &satellite_status {
                if let Some(cached_sat) = cached_status.iter().find(|c| c.name == sat.name) {
                    // Check if the status has changed
                    if let Some(latest_status) = get_latest_valid_status(sat) {
                        if cached_sat.status != latest_status && cached_sat.status != UNKNOWN_STATUS {
                            // Update the cache with the new status
                            let mut updated_cache = cached_status.clone();
                            if let Some(entry) = updated_cache.iter_mut().find(|c| c.name == sat.name) {
                                entry.status = latest_status.clone();
                            } else {
                                updated_cache.push(SatelliteStatusCache::new(
                                    sat.name.clone(),
                                    latest_status.clone()
                                ));
                            }
                            // Write the updated cache back to the file
                            if let Ok(json_content) = serde_json::to_string_pretty(&updated_cache) {
                                tokio::fs::write(SATELLITE_STATUS_CACHE_FILE, json_content)
                                    .await.expect("Failed to write updated cache file");
                            } else {
                                tracing::error!("Failed to serialize updated satellite status cache");
                            }
                            tracing::info!("Status change detected for {}: {} -> {}", sat.name, cached_sat.status, latest_status);
                            data.push(format!("{}: {}", sat.name, latest_status));
                        }
                    } else {
                        tracing::warn!("No valid status found for {}", sat.name);
                        data.push(format!("找不到{}的数据喵，换个卫星试试吧", sat.name));
                    }
                } else {
                    tracing::warn!("Satellite {} not found in cache", sat.name);
                }
            }
        } else {
            let mut cache_content: Vec<SatelliteStatusCache> = Vec::new();
            for sat in &satellite_status {
                match get_latest_valid_status(sat) {
                    Some(status) => {
                        let cache_entry = SatelliteStatusCache::new(
                            sat.name.clone(),
                            status
                        );
                        cache_content.push(cache_entry);
                    }
                    None => {
                        let cache_entry = SatelliteStatusCache::new(
                            sat.name.clone(),
                            UNKNOWN_STATUS.to_string()
                        );
                        cache_content.push(cache_entry);
                    }
                }
            }
            if let Ok(json_content) = serde_json::to_string_pretty(&cache_content) {
                tokio::fs::write(SATELLITE_STATUS_CACHE_FILE, json_content).await.expect("Failed to write cache file");
                tracing::info!("Satellite status cache file created successfully");
                data.push("数据保存成功~".to_string());
            } else {
                tracing::error!("Failed to serialize satellite status cache");
                msg = "卫星数据序列化失败:(".to_string();
                success = false;
            }
        }
    } else {
        tracing::error!("Failed to read satellite status cache file");
        msg = "卫星数据读取失败:(".to_string();
        success = false;
    }

    ApiResponse::new(success, data, msg)
}

fn get_latest_valid_status(
    satellite_status: &SatelliteStatus
) -> Option<String> {
    // Get the latest valid status for the satellite
    for status in &satellite_status.status {
        if let Some(flag) = status.first() {
            if flag.description != UNKNOWN_STATUS {
                return Some(flag.description.clone());
            }
        }
    }
    None
}