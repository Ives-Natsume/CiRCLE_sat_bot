use crate::{
    app_status::AppStatus, 
    fs::handler::{FileData, FileFormat, FileRequest},
    module::amsat::prelude::*,
    msg::group_msg::send_group_message_to_multiple_groups,
    response::ApiResponse,
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::RwLock,
};
use chrono::{DateTime, Utc, Timelike, Duration};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SatStatusCache {
    name: String,
    status: ReportStatus,
    report_num: u64,
    report_time: String,    // format rfc3339
}

const SAT_STATUS_CACHE: &str = "data/sat_status_cache.json";

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

async fn load_official_report_data(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    path: String
) -> anyhow::Result<Vec<SatelliteFileFormat>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: path.into(),
        format: FileFormat::Json,
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    }

    let file_result = match rx.await {
        Ok(result) => result,
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    let file_data_raw = match file_result {
        Ok(FileData::Json(data)) => data,
        Ok(_) => return Err(anyhow::anyhow!("Unexpected file format received")),
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    let file_data: Vec<SatelliteFileFormat> = match serde_json::from_value(file_data_raw) {
        Ok(data) => data,
        Err(e) => return Err(anyhow::anyhow!("Failed to parse JSON data: {}", e)),
    };

    Ok(file_data)
}

async fn check_official_data_file_exist(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>
) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Exists {
        path: OFFICIAL_REPORT_DATA.into(),
        responder: tx
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("{}", e);
        return false;
    }

    let result = match rx.await {
        Ok(sth) => sth,
        Err(e) => {
            tracing::error!("{}", e);
            return false;
        }
    };

    match result {
        Ok(value) => value,
        Err(e) => {
            tracing::error!("{}", e);
            return false;
        }
    }
}

pub async fn write_report_data(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    file_data: &Vec<SatelliteFileFormat>,
    path: String
) -> anyhow::Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    let data = serde_json::to_value(&file_data)
        .map_err(|e| anyhow::anyhow!("Failed to convert data to JSON: {}", e))?;
    let request = FileRequest::Write {
        path: path.into(),
        format: FileFormat::Json,
        data: FileData::Json(data),
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("Failed to send file write request: {}", e);
        return Err(anyhow::anyhow!("Failed to send file write request: {}", e));
    }

    match rx.await {
        Ok(result) => match result {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Failed to write file: {}", e)),
        },
        Err(e) => Err(anyhow::anyhow!("Failed to receive file write response: {}", e)),
    }
}

pub async fn load_satellites_list(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>
) -> anyhow::Result<SatelliteList> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: SATELLITES_TOML.into(),
        format: FileFormat::Toml,
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("Failed to send file read request: {}", e);
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    }

    let file_result = match rx.await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Failed to receive file data: {}", e);
            return Err(anyhow::anyhow!("Failed to receive file data: {}", e));
        },
    };

    let file_data_raw = match file_result {
        Ok(FileData::Toml(data)) => data,
        Ok(_) => return Err(anyhow::anyhow!("Unexpected file format received")),
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    let file_data_raw_str = match toml::to_string(&file_data_raw) {
        Ok(data) => data,
        Err(e) => return Err(anyhow::anyhow!("Failed to convert TOML data to string: {}", e)),
    };

    let file_data: SatelliteList = match toml::from_str(&file_data_raw_str) {
        Ok(data) => data,
        Err(e) => return Err(anyhow::anyhow!("Failed to parse TOML data: {}", e)),
    };

    Ok(file_data)
}

async fn create_offficial_data_file(
    app_status: &Arc<AppStatus>,
) {
    let tx_filerequest = app_status.file_tx.clone();
    let satellite_list = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to load satellite list: {}", e);
            return ;
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

    let _ = write_report_data(tx_filerequest.clone(), &file_data, OFFICIAL_REPORT_DATA.into()).await;
    let _ = write_report_data(tx_filerequest.clone(), &file_data, OFFICIAL_STATUS_CACHE.into()).await;
}

pub fn pack_satellite_data(reports: Vec<SatStatus>) -> Option<SatelliteFileFormat> {
    if reports.is_empty() {
        return None;
    }

    let name = reports[0].name.clone();
    let mut grouped: BTreeMap<String, Vec<SatStatus>> = BTreeMap::new();

    for report in reports {
        // filter callsign
        if report.callsign.contains("BH6BMJ") {
            continue;
        }

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

    Some(SatelliteFileFormat { name, data })
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
        // Filter out reports that are too old
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
        data: sorted_data,
    }
}

async fn load_sat_status_cache(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    path: String
) -> anyhow::Result<Vec<SatStatusCache>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: path.into(),
        format: FileFormat::Json,
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("Failed to send file read request: {}", e);
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    }

    let read_result = match rx.await {
        Ok(data) => data,
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to receive file read response: {}", e));
        }
    };

    let status_cache = match read_result {
        Ok(FileData::Json(data)) => match serde_json::from_value::<Vec<SatStatusCache>>(data) {
            Ok(data) => data,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to parse satellite status cache: {}", e));
            }
        },
        _ => {
            return Err(anyhow::anyhow!("Unexpected file format received"));
        }
    };

    Ok(status_cache)
}

async fn write_sat_status_cache(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    status_cache: &Vec<SatStatusCache>,
    path: String
) -> anyhow::Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let data = serde_json::to_value(status_cache).map_err(|e| {
        tracing::error!("Failed to serialize satellite status cache: {}", e);
        anyhow::anyhow!("Failed to serialize satellite status cache: {}", e)
    })?;
    let request = FileRequest::Write {
        path: path.into(),
        format: FileFormat::Json,
        data: FileData::Json(data),
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("Failed to send file write request: {}", e);
        return Err(anyhow::anyhow!("Failed to send file write request: {}", e));
    }

    match rx.await {
        Ok(result) => match result {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Failed to write file: {}", e)),
        },
        Err(e) => Err(anyhow::anyhow!("Failed to receive file write response: {}", e)),
    }
}

/// sat_status_cache.json, stores shortcuts about satellite status
pub async fn sat_status_cache_handler(
    app_status: &Arc<AppStatus>,
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::empty();
    let mut response_data = Vec::new();
    let tx_filerequest = app_status.file_tx.clone();

    // load satellite status cache
    let status_cache = match load_sat_status_cache(tx_filerequest.clone(), SAT_STATUS_CACHE.into()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let mut new_cache = Vec::new();

    // load official report
    let official_report = match load_official_report_data(tx_filerequest.clone(), OFFICIAL_REPORT_DATA.into()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    // compare satellite status cache with official report
    for report_format in official_report {
        let sat_name = &report_format.name;
        if report_format.data.is_empty() {
            let time: String = Utc::now().to_rfc3339();
            let cache_entry = SatStatusCache {
                name: sat_name.clone(),
                status: ReportStatus::Grey,
                report_time: time,
                report_num: 0,
            };
            new_cache.push(cache_entry);
        }
        for data in report_format.data {
            let time_block = &data.time;
            let reports = &data.report;
            if reports.is_empty() {
                continue;
            }

            // find the corresponding status cache entry
            if let Some(cache_entry) = status_cache.iter().find(|entry: &&SatStatusCache| &entry.name == sat_name) {
                // update latest data only
                let report_time = DateTime::parse_from_rfc3339(&time_block.clone())
                    .expect("Invalid time format in official report")
                    .with_timezone(&Utc);
                let cache_time = DateTime::parse_from_rfc3339(&cache_entry.report_time)
                    .expect("Invalid time format in status cache")
                    .with_timezone(&Utc);
                if report_time > cache_time {
                    // we need to update the status cache
                    let mut status_count: HashMap<ReportStatus, usize> = HashMap::new();
                    for report in reports {
                        let status = ReportStatus::from_string(&report.report);
                        *status_count.entry(status).or_insert(0) += 1;
                    }
                    let new_status = determine_report_status(&status_count);
                    if new_status != cache_entry.status {
                        response_data.push(format!(
                            "{}: {}",
                            sat_name, new_status.to_string()
                        ));
                    }
                    // update the cache entry
                    let mut cache_entry = cache_entry.clone();
                    cache_entry.status = new_status;
                    cache_entry.report_time = time_block.clone();
                    new_cache.push(cache_entry);
                }
                else {
                    // keep the cache entry
                    new_cache.push(cache_entry.clone());
                }
            }
            else {
                let new_status = {
                    let mut status_count: HashMap<ReportStatus, usize> = HashMap::new();
                    for report in reports {
                        let status = ReportStatus::from_string(&report.report);
                        *status_count.entry(status).or_insert(0) += 1;
                    }
                    determine_report_status(&status_count)
                };
                new_cache.push(SatStatusCache {
                    name: sat_name.clone(),
                    status: new_status,
                    report_time: time_block.clone(),
                    report_num: 1,
                });
            }

            break; // only process the latest time block
        }
    }

    // write the updated cache back to the file
    tracing::info!("Writing updated satellite status cache...");
    if let Err(e) = write_sat_status_cache(tx_filerequest.clone(), &new_cache, SAT_STATUS_CACHE.into()).await {
        response.message = Some(format!("{}", e));
        return response;
    }

    if !response_data.is_empty() {
        response_data.insert(0, "卫星状态更新了喵~".to_string());
    }
    response.success = true;
    response.data = Some(response_data);
    response
}

/// Scheduled task
pub async fn amsat_data_handler(
    app_status: &Arc<AppStatus>,
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::empty();
    let tx_filerequest = app_status.file_tx.clone();

    match check_official_data_file_exist(tx_filerequest.clone()).await {
        true => {},
        false => {
            tracing::info!("Creating AMSAT data file...");
            create_offficial_data_file(&app_status).await;
            return response;
        }
    }

    let mut file_data: Vec<SatelliteFileFormat> = match load_official_report_data(tx_filerequest.clone(), OFFICIAL_REPORT_DATA.into()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let satellite_list: SatelliteList = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let mut response_data = Vec::new();
    for sat in satellite_list.satellites {
        let sat_name = &sat.official_name;
        let data = match get_amsat_data(sat_name, 1, &app_status).await {
            Ok(data) => data,
            Err(e) => {
                response.message = Some(format!("{}", e));
                continue;
            }
        };
        if data.is_empty() {
            continue;
        }

        if let Some(exist_data) = file_data.iter_mut().find(|f| f.name == *sat_name) {
            let updated_data = update_satellite_data(exist_data.clone(), data, 48);
            *exist_data = updated_data;
        } else {
            let new_data = pack_satellite_data(data);
            if let Some(new_data) = new_data {
                file_data.push(new_data);
            }
        }
    }

    // write the updated data back to the file
    if let Err(e) = write_report_data(tx_filerequest.clone(), &file_data, OFFICIAL_REPORT_DATA.into()).await {
        response.message = Some(format!("{}", e));
        return response;
    }
    if let Err(e) = write_report_data(tx_filerequest.clone(), &file_data, OFFICIAL_STATUS_CACHE.into()).await {
        response.message = Some(format!("{}", e));
        return response;
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

    let satellite_lists = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    // let latest_data = match load_sat_status_cache(tx_filerequest.clone(), SAT_STATUS_CACHE.into()).await {
    //     Ok(data) => data,
    //     Err(e) => {
    //         response.message = Some(format!("{}", e));
    //         return response;
    //     }
    // };

    let latest_data = match load_official_report_data(tx_filerequest.clone(), OFFICIAL_STATUS_CACHE.into()).await {
        Ok(data) => data,
        Err(e) => {
            response.message = Some(format!("{}", e));
            return response;
        }
    };

    let inputs: Vec<&str> = input.split('/').collect();
    let mut match_sat = Vec::new();
    let mut response_data = Vec::new();
    for sat in inputs {
        let match_sat_raw = search_satellites(sat, &satellite_lists, 0.95);
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

    // for official_name in match_sat {
    //     let sat_data_cache = latest_data.iter().find(|f| f.name == official_name);
    //     response_data.push(format!(
    //         "{}吗，交给Rinko喵~",
    //         official_name
    //     ));
    //     if let Some(sat_record) = sat_data_cache {
    //         // get latest report
    //         if sat_record.status == ReportStatus::Grey {
    //             response_data.push(format!("过去两天没有{}的报告呢，去上传报告吧", official_name));
    //             continue;
    //         }
    //         let report_time = DateTime::parse_from_rfc3339(&sat_record.report_time)
    //             .expect("Invalid time format in data element")
    //             .with_timezone(&Utc);
    //         let now = Utc::now();
    //         let time_diff = (now - report_time).num_hours();

    //         response_data.push(format!(
    //             "大约{}小时前有{}个报告，{}的说",
    //             time_diff,
    //             sat_record.report_num,
    //             sat_record.status.to_chinese_string()
    //         ));
    //     } else {
    //         response_data.push(format!("过去两天没有{}的报告呢，去上传报告吧", official_name));
    //     }
    //     response_data.push("\n".to_string());
    // }

    for official_name in match_sat {
        let sat_data = latest_data.iter().find(|f| f.name == official_name);
        response_data.push(format!(
            "{}吗，交给Rinko喵~",
            official_name
        ));
        if let Some(sat_record) = sat_data {
            // get latest report
            for data_element in &sat_record.data {
                if data_element.report.is_empty() {
                    continue;
                }
                let mut report_status_count: HashMap<ReportStatus, usize> = HashMap::new();
                let mut report_total_count = 0;
                for report in &data_element.report {
                    let status = ReportStatus::from_string(&report.report.clone());
                    *report_status_count.entry(status).or_default() += 1;
                    report_total_count += 1;
                }
                let report_status = determine_report_status(&report_status_count);
                let report_timeblock = DateTime::parse_from_rfc3339(&data_element.time)
                    .expect("Invalid time format in data element")
                    .with_timezone(&Utc);
                let now = Utc::now();
                let time_diff = (now - report_timeblock).num_hours();

                response_data.push(format!(
                    "大约{}小时前有{}个报告，{}的说",
                    time_diff,
                    report_total_count,
                    report_status.to_chinese_string()
                ));
                break;
            }
        } else {
            response_data.push(format!("过去两天没有{}的报告呢，去上传报告吧", official_name));
        }
        response_data.push("\n".to_string());
    }

    if response_data.iter().all(|s| s.trim().is_empty()) {
        response.message = Some("^ ^)/".to_string());
        return response;
    }

    while let Some(last) = response_data.last() {
        if last.trim().is_empty() {
            response_data.pop();
        } else {
            break;
        }
    }

    response.success = true;
    response.data = Some(response_data);
    response
}