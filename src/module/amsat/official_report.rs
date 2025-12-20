use crate::{
    app_status::AppStatus, 
    fs::{self, handler::*},
    module::{amsat::{amsat_scraper, prelude::*}, tools::render::{render_satstatus_data, render_satstatus_query_handler}},
    msg::{group_msg::send_group_message_to_multiple_groups, prelude::MessageEvent},
    response::ApiResponse,
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::RwLock,
};
use chrono::{DateTime, Duration, Timelike, Utc};
use std::{collections::{BTreeMap, HashSet}};
use std::sync::Arc;
use std::collections::HashMap;

async fn get_amsat_data(
    sat_name: &str,
    hours: u64,
    app_status: &Arc<AppStatus>,
) -> anyhow::Result<Vec<SatStatus>> {
    tracing::debug!("Fetching AMSAT data for {}", sat_name);
    let api_url = format!(
        "https://www.amsat.org/status/api/v1/sat_info.php?name={}&hours={}",
        sat_name, hours
    );

    const MAX_RETRIES: u64 = 3;
    for attempt in 1..=MAX_RETRIES {
        if attempt > 1 {
            tokio::time::sleep(tokio::time::Duration::from_secs(2 * attempt)).await;
        }
        let response = reqwest::get(&api_url).await;
        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Vec<SatStatus> = resp.json().await?;
                    return Ok(data);
                } else {
                    tracing::error!(
                        "{} 获取 AMSAT 数据失败: HTTP {}\n重试次数 {}/{}",
                        sat_name,
                        resp.status(),
                        attempt,
                        MAX_RETRIES
                    );
                    let response_msg = format!(
                        "{} 获取 AMSAT 数据失败，重试次数 {}/{}",
                        sat_name,
                        attempt,
                        MAX_RETRIES
                    );
                    let response: ApiResponse<Vec<String>> = ApiResponse::error(response_msg);
                    send_group_message_to_multiple_groups(response, &app_status).await;
                }
            }
            Err(e) => {
                if attempt == MAX_RETRIES {
                    return Err(anyhow::anyhow!("获取 AMSAT 数据失败: {}", e));
                }
            }
        }
    }

    Ok(vec![SatStatus::default()])
}

pub async fn load_satellites_list(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>
) -> anyhow::Result<SatelliteList> {
    let file_data_raw = match fs::handler::read_file(
        tx_filerequest.clone(),
        SATELLITES_TOML.into(),
        FileFormat::Toml,
    ).await {
        Ok(FileData::Toml(data)) => data,
        Ok(_) => {
            return Err(anyhow::anyhow!("Unexpected file format received when loading satellite list"));
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to load satellite list: {}", e));
        }
    };

    let file_data: SatelliteList = match toml::from_str(&toml::to_string(&file_data_raw).unwrap()) {
        Ok(data) => data,
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to parse satellite list: {}", e));
        }
    };

    Ok(file_data)
}

pub async fn write_satellite_list(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    satellite_list: &SatelliteList,
) -> anyhow::Result<()> {
    let toml_data = toml::to_string(&satellite_list)
        .map_err(|e| anyhow::anyhow!("Failed to convert satellite list to TOML: {}", e))?;
    let toml_value: toml::Value = toml::from_str(&toml_data)
        .map_err(|e| anyhow::anyhow!("Failed to parse TOML data: {}", e))?;

    match fs::handler::write_file(
        tx_filerequest,
        SATELLITES_TOML.into(),
        &FileData::Toml(toml_value)
    ).await {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Failed to write satellite list: {}", e)),
    }
}

async fn create_satellite_list_file(
    app_status: &Arc<AppStatus>,
) {
    let tx_filerequest = app_status.file_tx.clone();
    let satellite_names = match amsat_scraper::fetch_satellite_names().await {
        Ok(names) => names,
        Err(e) => {
            tracing::error!("Failed to fetch satellite names: {}", e);
            return;
        }
    };

    let mut satellite_list = SatelliteList { satellites: Vec::new() };
    for name in satellite_names {
        let sat = SatelliteName {
            official_name: name,
            aliases: Vec::new(),
        };
        satellite_list.satellites.push(sat);
    }

    match write_satellite_list(tx_filerequest.clone(), &satellite_list).await {
        Ok(_) => {},
        Err(e) => {
            tracing::error!("Failed to write satellite list file: {}", e);
            return;
        }
    }
}

async fn create_offficial_data_file(
    app_status: &Arc<AppStatus>,
) {
    let tx_filerequest = app_status.file_tx.clone();
    let satellite_list_raw = match read_file(
        tx_filerequest.clone(),
        SATELLITES_TOML.into(),
        FileFormat::Toml,
    ).await {
        Ok(FileData::Toml(data)) => data,
        Ok(_) => {
            tracing::error!("Unexpected file format received when loading satellite list");
            return;
        }
        Err(e) => {
            tracing::error!("Failed to load satellite list: {}", e);
            return;
        }
    };

    // check if satellite list exists
    match check_file_exists(tx_filerequest.clone(), SATELLITES_TOML.into()).await {
        true => {},
        false => {
            tracing::info!("Satellite list file not found, creating...");
            create_satellite_list_file(&app_status).await;
        }
    }

    let satellite_list: SatelliteList = match toml::from_str(&toml::to_string(&satellite_list_raw).unwrap()) {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to parse satellite list: {}", e);
            return;
        }
    };

    let mut file_data: Vec<SatelliteFileFormat> = Vec::new();
    for sat in satellite_list.satellites {
        let sat_name = &sat.official_name;
        let vec_satstatus = match get_amsat_data(sat_name, 48, &app_status).await {
            Ok(data) => data,
            Err(_) => {
                continue;
            }
        };
        if vec_satstatus.is_empty() {
            continue;
        }

        if let Some(data) = pack_satellite_data(vec_satstatus) {
            file_data.push(data);
        }
    }

    let file_data = FileData::Json(serde_json::to_value(&file_data).unwrap());

    match write_file(tx_filerequest.clone(), OFFICIAL_REPORT_DATA.into(), &file_data).await {
        Ok(_) => {},
        Err(e) => {
            tracing::error!("Failed to write official report data file: {}", e);
            return;
        }
    }
    match write_file(tx_filerequest.clone(), OFFICIAL_STATUS_CACHE.into(), &file_data).await {
        Ok(_) => {},
        Err(e) => {
            tracing::error!("Failed to write official status cache file: {}", e);
            return;
        }
    }
}

pub fn pack_satellite_data(reports: Vec<SatStatus>) -> Option<SatelliteFileFormat> {
    if reports.is_empty() {
        return None;
    }

    let name = reports[0].name.clone();
    let mut grouped: BTreeMap<String, Vec<SatStatus>> = BTreeMap::new();

    for report in reports {
        // parse time string to chrono DateTime, UTC zone
        let datetime = match DateTime::parse_from_rfc3339(&report.reported_time) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                tracing::error!("Failed to parse reported time: {}", e);
                continue;
            }
        };

        // ensure report time should not larger than now
        let offset = Duration::minutes(5);
        if datetime.with_timezone(&Utc) > Utc::now() + offset {
            // tracing::warn!("Report time {} is in the future, skipping", datetime);
            continue;
        }

        let hour_block = datetime
            .with_minute(0).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap()
            .to_rfc3339();

        grouped.entry(hour_block).or_default().push(report);
    }

    // Sort the grouped map by time, descending
    let mut sorted: Vec<_> = grouped.into_iter().collect();
    sorted.sort_by(|a, b| b.0.cmp(&a.0));

    let data: Vec<SatelliteFileElement> = sorted.into_iter()
        .map(|(time, report)| SatelliteFileElement { time, report })
        .collect();

    let last_update_time = Utc::now().to_rfc3339();

    Some(SatelliteFileFormat { name, last_update_time, data, amsat_update_status: true })
}

pub fn update_satellite_data(
    existing: SatelliteFileFormat,
    new_reports: Vec<SatStatus>,
    retain_hours: i64,
) -> SatelliteFileFormat {
    let now = Utc::now();
    let mut grouped: BTreeMap<String, Vec<SatStatus>> = BTreeMap::new();

    let new_element = match pack_satellite_data(new_reports) {
        Some(data) => data,
        None => return existing, // If no new data, return existing
    };

    // Group new reports by time block
    for report in new_element.data {
        // Filter out reports that are outdated
        let report_time = DateTime::parse_from_rfc3339(&report.time)
            .expect("Invalid time format in report")
            .with_timezone(&Utc);
        
        if (now - report_time).num_hours() > retain_hours {
            continue;
        }

        grouped.entry(report.time).or_default().extend(report.report);
    }

    // remove old data from existing
    let mut existing_data: BTreeMap<String, Vec<SatStatus>> = BTreeMap::new();
    for element in existing.data {
        let report_time = DateTime::parse_from_rfc3339(&element.time)
            .expect("Invalid time format in existing data")
            .with_timezone(&Utc);
        
        if (now - report_time).num_hours() <= retain_hours {
            existing_data.entry(element.time).or_default().extend(element.report);
        }
    }

    // merge new data into existing data
    // we just need to think about the latest time block, so we can ignore older reports
    for (time, reports) in grouped {
        // pass the old block, for we don't want to have duplicate data
        if existing_data.contains_key(&time) {
            // check if current time sits in the time block
            let time_block = DateTime::parse_from_rfc3339(&time)
                .expect("Invalid time format in existing data")
                .with_timezone(&Utc);
            // map now to the time block
            let now_mapped = now.with_hour(time_block.hour()).unwrap()
                .with_minute(time_block.minute()).unwrap()
                .with_second(time_block.second()).unwrap()
                .with_nanosecond(0).unwrap();
            if now_mapped > time_block {
                continue; // skip this block, we already have it
            }
            // otherwise, we need to merge the reports
            let existing_reports = existing_data.get_mut(&time).unwrap();
            existing_reports.extend(reports);
        } else {
            existing_data.insert(time, reports);
        }
    }

    // check for duplicate reports in the same time block
    for (_, reports) in existing_data.iter_mut() {
        let mut seen: HashSet<String> = HashSet::new();
        reports.retain(|report| {
            if seen.contains(&report.callsign) {
                false // remove duplicate
            } else {
                seen.insert(report.callsign.clone());
                true // keep unique report
            }
        });
    }

    // Convert back to Vec<SatelliteFileElement>
    let updated_data: Vec<SatelliteFileElement> = existing_data.into_iter()
        .map(|(time, report)| SatelliteFileElement { time, report })
        .collect();

    // sort the updated data by time, descending
    let mut sorted_data = updated_data;
    sorted_data.sort_by(|a, b| b.time.cmp(&a.time));

    SatelliteFileFormat {
        name: existing.name,
        last_update_time: now.to_rfc3339(),
        amsat_update_status: true,
        data: sorted_data,
    }
}

/// Scheduled task
pub async fn amsat_data_handler(
    app_status: &Arc<AppStatus>,
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::empty();
    let tx_filerequest = app_status.file_tx.clone();

    match check_file_exists(
        tx_filerequest.clone(),
        SATELLITES_TOML.into()
    ).await {
        true => {},
        false => {
            tracing::info!("Satellite list file not found, creating...");
            create_satellite_list_file(&app_status).await;
            return response;
        }
    }

    match check_file_exists(
        tx_filerequest.clone(),
        OFFICIAL_REPORT_DATA.into()
    ).await {
        true => {},
        false => {
            tracing::info!("Creating AMSAT data file...");
            create_offficial_data_file(&app_status).await;
            return response;
        }
    }

    let official_report_raw = match fs::handler::read_file(
        tx_filerequest.clone(),
        OFFICIAL_REPORT_DATA.into(),
        FileFormat::Json).await {
        Ok(FileData::Json(data)) => data,
        Ok(_) => {
            response.message = Some("Unexpected file format received".to_string());
            return response;
        }
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let mut official_report_data: Vec<SatelliteFileFormat> = match serde_json::from_value(official_report_raw) {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    // update satellite names
    let satellite_names = match amsat_scraper::fetch_satellite_names().await {
        Ok(names) => names,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let mut satellite_list: SatelliteList = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    // iterate through satellite list
    for sat_name in satellite_names {
        // add new satellite if not exist
        if !satellite_list.satellites.iter().any(|s| s.official_name == sat_name) {
            tracing::info!("New satellite found: {}", sat_name);
            let new_sat = SatelliteName {
                official_name: sat_name.clone(),
                aliases: vec![],
            };
            satellite_list.satellites.push(new_sat);
        }
    }

    let mut response_data = Vec::new();
    for sat in &satellite_list.satellites {
        let sat_name = &sat.official_name;
        let mut amsat_update_status = false;
        let data = match get_amsat_data(sat_name, 1, &app_status).await {
            Ok(data) => {
                amsat_update_status = true;
                Some(data)
            },
            Err(e) => {
                response.message = Some(format!("{}", e));
                None
            }
        };

        if let Some(exist_data) = official_report_data.iter_mut().find(|f| f.name == *sat_name) {
            if amsat_update_status {
                let data = match data {
                    Some(d) => d,
                    None => continue,
                };
                let updated_data = update_satellite_data(exist_data.clone(), data, 48);
                *exist_data = updated_data;
            } else {
                // keep existing data and just mark amsat_update_status as false
                exist_data.amsat_update_status = false;
            }
        } else {
            if amsat_update_status {
                let data = match data {
                    Some(d) => d,
                    None => continue,
                };
                let new_data = pack_satellite_data(data);
                if let Some(new_data) = new_data {
                    official_report_data.push(new_data);
                }
                continue;
            } else {
                // create empty record
                let empty_record = SatelliteFileFormat {
                    name: sat_name.clone(),
                    last_update_time: Utc::now().to_rfc3339(),
                    amsat_update_status: false,
                    data: Vec::new(),
                };
                official_report_data.push(empty_record);
            }
        }
    }

    let file_data = FileData::Json(serde_json::to_value(&official_report_data).unwrap());
    match write_file(tx_filerequest.clone(), OFFICIAL_REPORT_DATA.into(), &file_data).await {
        Ok(_) => {},
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    }
    match write_file(tx_filerequest.clone(), OFFICIAL_STATUS_CACHE.into(), &file_data).await {
        Ok(_) => {},
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    }
    match write_satellite_list(tx_filerequest.clone(), &satellite_list).await {
        Ok(_) => {},
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    }

    response.success = true;
    if !response_data.is_empty() {
        response_data.insert(0, "卫星状态更新了喵~".to_string());
    }
    response.data = Some(response_data);
    response
}

pub fn determine_report_status(
    data: &HashMap<ReportStatus, usize>
) -> ReportStatus {
    if data.is_empty() {
        return ReportStatus::Grey; // 规则 4
    }

    // let present: Vec<ReportStatus> = data.iter()
    //     .filter_map(|(status, cnt)| if *cnt > 0 { Some(status.clone()) } else { None })
    //     .collect();

    // --- 1. 数据聚合 ---
    let mut count_map: HashMap<ReportStatus, u32> = HashMap::new();
    for (status, count) in data {
        *count_map.entry(status.clone()).or_insert(0) += *count as u32;
    }

    let get_count = |status: &ReportStatus| count_map.get(status).cloned().unwrap_or(0);

    let blue_count = get_count(&ReportStatus::Blue);
    let purple_count = get_count(&ReportStatus::Purple);
    let yellow_count = get_count(&ReportStatus::Yellow);
    let red_count = get_count(&ReportStatus::Red);
    let orange_count = get_count(&ReportStatus::Orange);

    let active_group_count = blue_count + purple_count;
    let weak_signal_group_count = yellow_count + red_count;
    let total_main_reports = active_group_count + weak_signal_group_count;

    if total_main_reports == 0 {
        // 如果主要分组都没有报告，则只可能是 Grey 或 Orange
        return if orange_count > 0 { ReportStatus::Orange } else { ReportStatus::Grey };
    }

    // --- 2. 智能冲突检测 ---
    // 定义冲突阈值：当两个对立组的报告数都超过总报告数的 20% 时，视为冲突。
    const CONFLICT_THRESHOLD_PERCENT: f32 = 0.20;
    
    // 如果Orange报告本身就很多，也应视为冲突
    if orange_count as f32 / total_main_reports as f32 > CONFLICT_THRESHOLD_PERCENT {
        return ReportStatus::Orange;
    }

    let active_ratio = active_group_count as f32 / total_main_reports as f32;
    let weak_signal_ratio = weak_signal_group_count as f32 / total_main_reports as f32;

    if active_ratio > CONFLICT_THRESHOLD_PERCENT && weak_signal_ratio > CONFLICT_THRESHOLD_PERCENT {
        return ReportStatus::Orange;
    }

    // --- 3. 确定主导分组 ---
    if weak_signal_group_count > active_group_count {
        // --- 4. 在 {Yellow, Red} 组内确定最终状态 (Red 优先) ---
        if red_count > 0 {
            ReportStatus::Red
        } else {
            ReportStatus::Yellow
        }
    } else {
        // --- 4. 在 {Blue, Purple} 组内确定最终状态 (Purple 优先) ---
        if purple_count > 0 {
            ReportStatus::Purple
        } else {
            ReportStatus::Blue
        }
    }
}

pub async fn query_satellite_status(
    input: &str,
    app_status: &Arc<AppStatus>,
) -> ApiResponse<Vec<String>> {
    tracing::debug!("Querying satellite status for input: {}", input);
    let mut response = ApiResponse::empty();
    let tx_filerequest = app_status.file_tx.clone();

    let inputs: Vec<&str> = input.split('/').collect();
    if inputs.len() > 7 {
        response.message = Some("干嘛，，，".to_string());
        return response;
    }

    let satellite_lists = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };


    let official_report_raw = match fs::handler::read_file(
        tx_filerequest.clone(),
        OFFICIAL_REPORT_DATA.into(),
        FileFormat::Json).await {
        Ok(FileData::Json(data)) => data,
        Ok(_) => {
            response.message = Some("Unexpected file format received".to_string());
            return response;
        }
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let latest_data: Vec<SatelliteFileFormat> = match serde_json::from_value(official_report_raw) {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let mut sat_query_list: Vec<String> = Vec::new();
    let mut match_sat: Vec<String> = Vec::new();

    // check if input contains `fm`
    if inputs.iter().any(|&s| s.to_ascii_lowercase() == "fm") {
        sat_query_list = vec!["AO-91", "PO-101[FM]", "ISS-FM", "SO-50", "AO-123", "SO-124", "SO-125"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    } else if inputs.iter().any(|&s| s.to_ascii_lowercase() == "lin" || s.to_ascii_lowercase() == "linear") {
        sat_query_list = vec!["AO-7", "AO-27", "FO-29", "RS-44", "QO-100", "JO-97"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    } else {
        // for sat in inputs {
        //     let match_sat_raw = search_satellites(sat, &satellite_lists, 0.95);
        //     for sat in match_sat_raw {
        //         if !match_sat.contains(&sat) {
        //             match_sat.push(sat);
        //         }
        //     }
        // }
        // if match_sat.is_empty() {
        //     response.message = Some("^ ^)/".to_string());
        //     return response;
        // }
        for sat in inputs {
            sat_query_list.push(sat.to_string());
        }
    }

    for query in sat_query_list {
        let match_sat_raw = search_satellites(&query, &satellite_lists, 0.95);
        for sat in match_sat_raw {
            if !match_sat.contains(&sat) {
                match_sat.push(sat);
            }
        }
    }

    if match_sat.is_empty() {
        response.message = Some("^ ^)/".to_string());
        return response;
    }

    let mut matched_sat_data: Vec<SatelliteFileFormat> = Vec::new();
    for official_name in match_sat {
        let sat_data = latest_data.iter().find(|f| f.name == official_name);
        if let Some(sat_record) = sat_data {
            matched_sat_data.push(sat_record.clone());
        }
        else {
            // build a empty record
            let empty_record = SatelliteFileFormat {
                name: official_name.clone(),
                last_update_time: Utc::now().to_rfc3339(),
                amsat_update_status: false,
                data: Vec::new(),
            };
            matched_sat_data.push(empty_record);
        }
    }

    response = render_satstatus_query_handler(&matched_sat_data).await;
    response
}