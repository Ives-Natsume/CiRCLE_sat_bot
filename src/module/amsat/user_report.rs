use crate::{
    fs::handler::{FileRequest, FileFormat, FileData},
    module::prelude::*,
    module::amsat::prelude::*,
};
use tokio::{
    sync::{oneshot, RwLock}
};
use std::sync::Arc;

pub async fn data_parser(
    args: &String,
) -> anyhow::Result<UserReport> {
    // Args: Callsign Grid Sat-name Report
    let args: Vec<&str> = args.split_whitespace().collect();
    if args.len() < 4 {
        // abort if not enough arguments
        return Err(anyhow::anyhow!("Not enough arguments"));
    }

    let callsign = args[0].to_string();
    let grid = args[1].to_string();
    let sat_name = args[2].to_string();
    let report = args[3].to_string();

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

    Ok(UserReport {
        callsign,
        grid,
        sat_name,
        report,
    })
}

pub async fn save_user_report(
    report: UserReport,
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

    Ok(())
}