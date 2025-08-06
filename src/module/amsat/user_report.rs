#![allow(unused)]
use crate::{
    fs::handler::{FileData, FileFormat, FileRequest},
    module::{amsat::prelude::*, prelude::*},
    response::ApiResponse,
};
use tokio::{
    sync::RwLock,
};
use std::sync::Arc;
use chrono::{DateTime, Utc, Datelike, Timelike};
use reqwest;

// TODO：unfinished
pub async fn data_parser(
    args: &String,
) -> anyhow::Result<SatStatus> {
    // Args: Callsign Grid Sat-name Report Report-time
    let args: Vec<&str> = args.split_whitespace().collect();
    if args.len() < 5 {
        // abort if not enough arguments
        return Err(anyhow::anyhow!("Not enough arguments"));
    }

    let callsign = args[0].to_string();
    let grid = args[1].to_string();
    let sat_name = args[2].to_string();
    let report = args[3].to_string();
    let reported_time = args[4].to_string();

    if !is_valid_callsign(&callsign) {
        return Err(anyhow::anyhow!("Invalid callsign"));
    }
    if !is_valid_grid(&grid) {
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

pub async fn save_user_report(
    report: SatStatus,
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
) -> anyhow::Result<()> {
    // load data
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: _USER_REPORT_DATA.into(),
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
    let period = (reported_time.minute() / 15);
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