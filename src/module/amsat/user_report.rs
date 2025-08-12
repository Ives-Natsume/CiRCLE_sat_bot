use crate::{
    app_status::AppStatus, fs::handler::{FileData, FileFormat, FileRequest}, i18n, module::{amsat::{official_report, prelude::*}, prelude::*}, msg::prelude::MessageEvent, response::ApiResponse
};
use tokio::{
    sync::RwLock,
};
use std::sync::Arc;
use std::collections::HashMap;
use chrono::{DateTime, Utc, Datelike, Timelike};
use reqwest;
use crate::module::amsat::official_report::{load_satellites_list, write_report_data};

const USER_REPORT_DATA: &str = "data/user_report_data.json";

// TODO：unfinished
pub async fn data_parser(
    args: &String,
) -> anyhow::Result<SatStatus> {
    // Args: Callsign Grid Sat-name Report-time (Optional: Report-status, default for status Blue)
    let args: Vec<&str> = args.split_whitespace().collect();
    if args.len() < 5 {
        // abort if not enough arguments
        return Err(anyhow::anyhow!("参数不足喵"));
    }

    let callsign = args[0].to_string();
    let grid = args[1].to_string();
    let sat_name = args[2].to_string();
    let reported_time = args[3].to_string();
    let report = args[4].to_string();

    if !is_valid_callsign(&callsign) {
        return Err(anyhow::anyhow!("Invalid callsign"));
    }
    if !is_valid_maidenhead_grid(&grid) {
        return Err(anyhow::anyhow!("Invalid grid"));
    }

    let report = ReportStatus::status_mapper(&report);
    if report == ReportStatus::Grey {
        return Err(anyhow::anyhow!("Invalid report status"));
    }

    Ok(SatStatus {
        name: sat_name,
        callsign,
        grid_square: grid,
        reported_time,
        report: report.to_string(),
    })
}

#[allow(unused)]
pub async fn save_user_report(
    report: SatStatus,
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
) -> anyhow::Result<()> {
    // load data
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: USER_REPORT_DATA.into(),
        format: FileFormat::Json,
        responder: tx
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    }

    let data = match rx.await {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => return Err(anyhow::anyhow!("Failed to read file: {}", e)),
        Err(e) => return Err(anyhow::anyhow!("File read request timed out: {}", e)),
    };

    let mut user_reports: SatStatus = match data {
        FileData::Json(json_data) => serde_json::from_value(json_data)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize user reports: {}", e))?,
        _ => return Err(anyhow::anyhow!("Invalid file format, expected JSON")),
    };

    Ok(())
}

pub async fn read_user_report_file(
    app_status: &AppStatus,
) -> anyhow::Result<Vec<SatelliteFileFormat>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx_filerequest = app_status.file_tx.clone();

    let file_request = FileRequest::Read {
        path: USER_REPORT_DATA.into(),
        format: FileFormat::Json,
        responder: tx
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(file_request).await {
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    };

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

pub async fn create_report_template(
    args: &String,
    app_status: Arc<AppStatus>
) -> anyhow::Result<()> {
    // Args: Sat-name Report-time (rfc3339)
    let args: Vec<&str> = args.split_whitespace().collect();

    // let reported_time = match parse_user_datetime(&args[1]) {
    //     Ok(d) => d,
    //     Err(e) => return Err(anyhow::anyhow!("时间设定失败: {}", e)),
    // };

    if args.len() < 2 {
        return Err(anyhow::anyhow!("参数不足喵"));
    }
    let _reported_time = match DateTime::parse_from_rfc3339(args[1]) {
        Ok(datetime) => datetime,
        Err(e) => return Err(anyhow::anyhow!("时间设定失败: {}\n时间格式为 2025-01-30T12:34:00Z 喵", e)),
    };

    // let year = reported_time.year();
    // let month = reported_time.month();
    // let day = reported_time.day();
    // let hour = reported_time.hour();
    // // an hour is divided into 4 periods of 15 minutes each
    // let period = (reported_time.minute() / 15);

    // let report_time_block: String = format!(
    //     "{:04} {:02} {:02} {:02} {}",
    //     year, month, day, hour, period
    // );

    // let report_time = reported_time.to_string();

    let tx_filerequest = app_status.file_tx.clone();
    let satellite_lists = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            return Err(anyhow::anyhow!("可用卫星列表加载失败: {}", e));
        }
    };

    let match_sat = search_satellites(args[0], &satellite_lists, 0.95);
    if match_sat.is_empty() || match_sat.len() != 1 {
        return Err(anyhow::anyhow!("无法选中卫星"))
    }
    let match_sat = match_sat[0];

    // TODO: finish the file read/write
    let mut user_report_data = match read_user_report_file(&app_status).await {
        Ok(data) => data,
        Err(e) => return Err(anyhow::anyhow!("{}", e)),
    };

    let new_element: SatelliteFileElement = SatelliteFileElement {
        time: args[1].to_string(),
        report: Vec::new()
    };

    let mut matched = false;
    for item in user_report_data.iter_mut() {
        if item.name == match_sat {
            // check if two template has little delta time
            if let Some(last_time) = item.data.last().map(|e| e.time.clone()) {
                let last_time: DateTime<Utc> = match DateTime::parse_from_rfc3339(&last_time) {
                    Ok(dt) => dt.with_timezone(&Utc),
                    Err(e) => {
                        tracing::error!("Failed to parse last_time: {}", e);
                        continue;
                    }
                };
                let new_time: DateTime<Utc> = match DateTime::parse_from_rfc3339(&new_element.time) {
                    Ok(dt) => dt.with_timezone(&Utc),
                    Err(e) => {
                        tracing::error!("Failed to parse new_element time: {}", e);
                        continue;
                    }
                };
                if (last_time - new_time).abs() < chrono::Duration::minutes(15) {
                    return Err(anyhow::anyhow!("本次过境的模板已经被创建了喵"));
                }
            }
            // reset the old data
            item.data = vec![new_element.clone()];
            matched = true;
            break;
        }
    }

    if !matched {
        user_report_data.push(SatelliteFileFormat {
            name: match_sat.to_string(),
            data: vec![new_element],
        });
    }

    if let Err(e) = write_report_data(
        tx_filerequest.clone(),
        &user_report_data,
        USER_REPORT_DATA.into(),
    ).await {
        return Err(anyhow::anyhow!("{}", e));
    }

    Ok(())
}

pub async fn add_user_report(
    app_status: Arc<AppStatus>,
    args: &String,
    payload: &MessageEvent,
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::<Vec<String>>::empty();
    let mut response_data = Vec::new();

    // Args: Sat-name Callsign Grid Status
    let args: Vec<&str> = args.split_whitespace().collect();

    if args.len() < 4 {
        return ApiResponse::<Vec<String>>::error("参数不足喵".to_string());
    }

    let callsign = args[1].to_uppercase().to_string();
    let sat_name = args[0].to_string();
    let grid = args[2].to_string();
    let status = args[3].to_lowercase().to_string();

    let nickname = payload.sender.card.clone();
    if !nickname.to_uppercase().contains(&callsign) {
        return ApiResponse::error("无法验证你的身份喵".to_string());
    }

    if !is_valid_maidenhead_grid(grid.as_str()) {
        return ApiResponse::error("网格参数非法喵".to_string());
    }

    let status: String = match status.as_str() {
        "blue" | "b" | "蓝" => ReportStatus::Blue.to_string_report_format(),
        "yellow" | "y" | "黄" => ReportStatus::Yellow.to_string_report_format(),
        "red" | "r" | "红" => ReportStatus::Red.to_string_report_format(),
        "purple" | "p" | "紫" => ReportStatus::Purple.to_string_report_format(),
        _ => return ApiResponse::<Vec<String>>::error("Rinko不能解析你报告的状态呢")
    };

    let tx_filerequest = app_status.file_tx.clone();
    let satellite_lists = match load_satellites_list(tx_filerequest.clone()).await {
        Ok(data) => data,
        Err(e) => {
            return ApiResponse::<Vec<String>>::error(format!("可用卫星列表加载失败: {}", e))
        }
    };

    let match_sat = search_satellites(&sat_name, &satellite_lists, 0.95);
    if match_sat.is_empty() || match_sat.len() != 1 {
        return ApiResponse::<Vec<String>>::error("无法选中卫星喵>_");
    }
    let match_sat = match_sat[0];

    let mut user_report_data = match read_user_report_file(&app_status).await {
        Ok(data) => data,
        Err(e) => return ApiResponse::<Vec<String>>::error(format!("{}", e)),
    };

    let mut found = false;
    for item in user_report_data.iter_mut() {
        if item.name == match_sat && item.data.len() > 0 {
            found = true;
            let mut element = item.data[0].clone();
            let time = element.time.clone();
            let report = SatStatus {
                name: match_sat.to_string(),
                reported_time: time,
                callsign: callsign.clone(),
                report: status.clone(),
                grid_square: grid.clone(),
            };
            // check for duplicate reports
            if element.report.iter().any(|r| r.callsign == callsign) {
                // replace the old report for the callsign
                element.report.retain(|r| r.callsign != callsign);
                response_data.push(format!("{} 的报告已更新喵", callsign));
            }
            element.report.push(report);
            item.data = vec![element];
            break;
        }
    }

    // return warn if the satellite is not found
    if !found {
        return ApiResponse::<Vec<String>>::error(format!("{}", i18n::text("cmd_report_user_no_template")));
    }

    // check if reports have conflicts
    let mut report_status_count: HashMap<ReportStatus, usize> = HashMap::new();
    for item in user_report_data.iter() {
        if item.name == match_sat {
            for data in &item.data {
                for report in &data.report {
                    let status = ReportStatus::from_string(&report.report);
                    *report_status_count.entry(status).or_insert(0) += 1;
                }
            }
        }
    }
    let report_status = official_report::determine_report_status(&report_status_count);
    if report_status == ReportStatus::Orange {
        response_data.push(i18n::text("cmd_report_user_conflict_report"));
    }

    if let Err(e) = write_report_data(
        tx_filerequest.clone(),
        &user_report_data,
        USER_REPORT_DATA.into(),
    ).await {
        return ApiResponse::<Vec<String>>::error(format!("{}", e));
    }

    response.success = true;
    response_data.push(format!(
        "{} 的报告已添加喵",
        callsign
    ));
    response.data = Some(response_data);
    response
}

pub async fn push_user_report(
    report: &String,
) -> ApiResponse<Vec<String>> {
    let report = match data_parser(report).await {
        Ok(r) => r,
        Err(e) => return ApiResponse::error(format!("Failed to parse user report: {}", e)),
    };
    // report time user input format: "YYYY-MM-DDTHH:MM:SSZ"
    let reported_time: DateTime<Utc> = match DateTime::parse_from_rfc3339(&report.reported_time) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return ApiResponse::error(
            "Invalid reported time format, expected RFC3339".to_string(),
        ),
    };
    let year = reported_time.year();
    let month = reported_time.month();
    let day = reported_time.day();
    let hour = reported_time.hour();
    // an hour is divided into 4 periods of 15 minutes each
    let period = reported_time.minute() / 15;
    let get_url = format!("https://www.amsat.org/status/submit.php?SatSubmit=yes&Confirm=yes&SatName={}&SatYear={:04}&SatMonth={:02}&SatDay={:02}&SatHour={:02}&SatPeriod={}&SatCall={}&SatReport={}&SatGridSquare={}",
        report.name, year, month, day, hour, period, report.callsign, report.report, report.grid_square);

    let client = reqwest::Client::new();
    let response = client.get(&get_url).send().await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                tracing::info!("{}'s report submitted successfully for {}", report.callsign, report.name);
            } else {
                return ApiResponse::error(format!("Failed to submit user report, status: {}", resp.status()));
            }
        },
        Err(e) => return ApiResponse::error(format!("Error submitting user report: {}", e)),
    }

    ApiResponse::ok(vec!["User report submitted successfully".to_string()])
}

#[allow(non_snake_case)]
pub async fn push_user_report_from_SatStatus(
    report: &SatStatus,
) -> anyhow::Result<()> {
    // report time user input format: "YYYY-MM-DDTHH:MM:SSZ"
    let reported_time: DateTime<Utc> = match DateTime::parse_from_rfc3339(&report.reported_time) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return Err(anyhow::anyhow!("Invalid reported time format, expected RFC3339")),
    };
    let year = reported_time.year();
    let month = reported_time.month();
    let day = reported_time.day();
    let hour = reported_time.hour();
    // an hour is divided into 4 periods of 15 minutes each
    let period = reported_time.minute() / 15;
    let get_url = format!("https://www.amsat.org/status/submit.php?SatSubmit=yes&Confirm=yes&SatName={}&SatYear={:04}&SatMonth={:02}&SatDay={:02}&SatHour={:02}&SatPeriod={}&SatCall={}&SatReport={}&SatGridSquare={}",
        report.name, year, month, day, hour, period, report.callsign, report.report, report.grid_square);

    let client = reqwest::Client::new();
    let response = client.get(&get_url).send().await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                tracing::info!("{}'s report submitted successfully for {}", report.callsign, report.name);
            } else {
                return Err(anyhow::anyhow!("Failed to submit user report, status: {}", resp.status()));
            }
        },
        Err(e) => return Err(anyhow::anyhow!("Error submitting user report: {}", e)),
    }

    Ok(())
}