use crate::{
    app_status::AppStatus, fs::handler::{FileData, FileFormat, FileRequest}, module::{prelude::*}, msg::prelude::MessageEvent, response::ApiResponse
};
use std::{clone, sync::Arc};
use regex::Regex;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};

const USER_ROAMING_DATA: &str = "data/user_roaming_data.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoamingData {
    pub callsign: String,
    pub grid: String,
    pub info: Option<String>,
}

async fn write_back_to_file(
    tx_filerequest: &Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    roaming_data: Vec<RoamingData>,
) -> anyhow::Result<()> {
    let tx_filerequest = tx_filerequest.write().await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let file_request = FileRequest::Write {
        path: USER_ROAMING_DATA.into(),
        format: FileFormat::Json,
        data: FileData::Json(serde_json::to_value(roaming_data).unwrap()),
        responder: tx,
    };

    if let Err(e) = tx_filerequest.send(file_request).await {
        tracing::error!("Failed to send file write request: {}", e);
        return Err(anyhow::anyhow!("Failed to send file write request: {}", e));
    };

    let write_response = match rx.await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to receive file write response: {}", e);
            return Err(anyhow::anyhow!("Failed to receive file write response: {}", e));
        }
    };

    match write_response {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Failed to write roaming data: {}", e)),
    }
}

async fn read_roaming_data(
    tx_filerequest: &Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>
) -> anyhow::Result<Vec<RoamingData>> {
    let tx_filerequest = tx_filerequest.write().await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let file_request = FileRequest::Read {
        path: USER_ROAMING_DATA.into(),
        format: FileFormat::Json,
        responder: tx,
    };

    if let Err(e) = tx_filerequest.send(file_request).await {
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    };

    let read_result = match rx.await {
        Ok(data) => data,
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to receive file read response: {}", e));
        }
    };

    let roaming_data = match read_result {
        Ok(FileData::Json(data)) => match serde_json::from_value::<Vec<RoamingData>>(data) {
            Ok(data) => data,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to parse roaming data: {}", e));
            }
        },
        _ => {
            return Err(anyhow::anyhow!("Unexpected file format received"));
        }
    };

    Ok(roaming_data)
}

pub async fn add_roaming(
    app_status: &Arc<AppStatus>,
    args: &String,
    payload: &MessageEvent
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::<Vec<String>>::empty();

    let args = match parse_input_flexible(args) {
        Some(parsed) => parsed,
        None => {
            return ApiResponse::<Vec<String>>::error("无法解析输入喵，请确保格式为：<呼号> <网格1> [网格2 ...] [备注]，呼号和网格间用空格分隔，多个网格间也用空格分隔，呼号可以使用'/'喵\n但是由于目前验证机制不成熟，需要确保你输入的呼号包含在你的群昵称内".to_string());
        }
    };

    let callsign = args.callsign;
    let grid = args.grids;
    let info = args.remark;

    tracing::info!("Adding roaming data: {} {} {:?}", callsign, grid, info);

    let admin_id = {
        let config_guard = app_status.config.read().await;
        config_guard.bot_config.admin_id.clone()
    };

    if !callsign_auth(&callsign, payload, &admin_id) {
        return ApiResponse::error("Rinko无法验证你的身份喵");
    }

    let grids = grid.split_whitespace().collect::<Vec<&str>>();
    if grids.is_empty() {
        return ApiResponse::<Vec<String>>::error("请提供漫游网格喵".to_string());
    }
    for g in &grids {
        if !is_valid_maidenhead_grid(g) {
            return ApiResponse::<Vec<String>>::error(format!("{}不是有效的梅登黑格网格喵", g));
        }
    }

    let tx_filerequest = app_status.file_tx.clone();

    let mut roaming_data = match read_roaming_data(&tx_filerequest).await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to read roaming data: {}", e);
            return ApiResponse::error(format!("Failed to read roaming data: {}", e));
        }
    };

    if let Some(existing) = roaming_data.iter_mut().find(|r| r.callsign.contains(&callsign)) {
        existing.grid = grid.clone().into();
        existing.info = info.map(|s| s.into());
        response.data = Some(vec![format!("{}的漫游信息已更新为: {}", callsign, grid)]);
    } else {
        let new_data = RoamingData {
            callsign: callsign.clone(),
            grid: grid.clone().into(),
            info: info.map(|s| s.into()),
        };
        roaming_data.push(new_data);
        response.data = Some(vec![format!("{}的漫游信息已添加: {}", callsign, grid)]);
    }

    match write_back_to_file(&tx_filerequest, roaming_data).await {
        Ok(_) => {}
        Err(e) => {
            tracing::error!("Failed to write roaming data: {}", e);
            return ApiResponse::error(format!("Failed to write roaming data: {}", e));
        }
    }

    tracing::info!("漫游信息已更新: {}: {}", callsign, grid);
    response.success = true;
    response
}

pub async fn remove_roaming(
    app_status: &Arc<AppStatus>,
    args: &String,
    payload: &MessageEvent
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::<Vec<String>>::empty();

    // Args: remove <callsign>
    let callsign = match args.split_whitespace().nth(1).map(|s| s.to_uppercase()) {
        Some(callsign) => callsign,
        None => {
            return ApiResponse::error("请提供要删除的呼号喵".to_string());
        }
    };
    let callsign_filter = Some(callsign.clone());

    let admin_id = {
        let config_guard = app_status.config.read().await;
        config_guard.bot_config.admin_id.clone()
    };

    if !callsign_auth(&callsign, payload, &admin_id) {
        return ApiResponse::error("Rinko无法验证你的身份喵");
    }

    let tx_filerequest = app_status.file_tx.clone();
    let roaming_data = match read_roaming_data(&tx_filerequest).await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to read roaming data: {}", e);
            return ApiResponse::error(format!("文件读取失败: {}", e));
        }
    };

    // filter roaming data by callsign
    let filtered_data = if let Some(callsign_to_remove) = callsign_filter {
        let original_len = roaming_data.len();
        let filtered: Vec<RoamingData> = roaming_data
            .into_iter()
            .filter(|r| !r.callsign.contains(&callsign_to_remove))
            .collect();
        
        if filtered.len() == original_len {
            return ApiResponse::error(format!("没有找到呼号为 {} 的漫游信息喵", callsign_to_remove));
        }
        filtered
    } else {
        roaming_data
    };

    match write_back_to_file(&tx_filerequest, filtered_data).await {
        Ok(_) => {
            response.success = true;
            response.data = Some(vec![format!("{}的漫游信息已删除喵", callsign)]);
        }
        Err(e) => {
            tracing::error!("Failed to write roaming data: {}", e);
            return ApiResponse::error(format!("文件写入失败: {}", e));
        }
    }

    response
}

pub async fn list_roaming(
    app_status: &Arc<AppStatus>,
    args: &String,
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::<Vec<String>>::empty();

    // Args: list [Callsign]
    let callsign_filter = args.split_whitespace().nth(1).map(|s| s.to_uppercase());

    let tx_filerequest = app_status.file_tx.clone();
    let roaming_data = match read_roaming_data(&tx_filerequest).await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to read roaming data: {}", e);
            return ApiResponse::error(format!("文件读取失败: {}", e));
        }
    };

    // filter roaming data by callsign
    let filtered_data = if let Some(callsign) = callsign_filter {
        roaming_data.into_iter().filter(|r| r.callsign.contains(&callsign)).collect()
    } else {
        roaming_data
    };

    // response.data = Some(filtered_data.into_iter().map(|r| format!("{}:\n网格: {}\n备注: {:?}", r.callsign, r.grid, r.info)).collect());
    let mut data = Vec::new();
    for r in filtered_data {
        let formated_string = format!(
            "{}:\n网格: {}\n",
            r.callsign,
            r.grid,
        );
        if let Some(info) = r.info {
            data.push(format!("{}备注: {}\n", formated_string, info));
        } else {
            data.push(formated_string);
        }
    }

    if data.is_empty() {
        data.push("没有找到任何漫游信息喵".to_string());
    }

    response.data = Some(data);
    response
}

#[derive(Debug)]
struct ParsedInput {
    callsign: String,
    grids: String,
    remark: Option<String>,
}

fn parse_input_flexible(input: &str) -> Option<ParsedInput> {
    // 宽容匹配：
    // - 呼号：不含空白和CJK的字母数字及/
    // - 网格：至少一个合法的（A-R a-r）(2) + 数字(2) + 后续字母数字（可空）
    // - 备注：可有可无
    let re = Regex::new(
        r"(?xi)                     # (?x) 忽略空白，(?i) 不区分大小写
        ^\s*
        (?P<callsign>[A-Za-z0-9/]+)   # 呼号
        \s+
        (?P<grids>
            (?:[A-R]{2}[0-9]{2}[A-Za-z0-9]*)   # 第一个网格
            (?:\s+[A-R]{2}[0-9]{2}[A-Za-z0-9]*)* # 后续网格
        )
        (?:\s+(?P<remark>.+))?        # 可选备注
        \s*$
        "
    ).unwrap();

    if let Some(caps) = re.captures(input) {
        // 规范化网格部分：多个空格压缩成一个空格
        let grids_clean = caps["grids"]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        Some(ParsedInput {
            callsign: caps["callsign"].to_uppercase(), // 呼号统一成大写
            grids: grids_clean,
            remark: caps.name("remark").map(|m| m.as_str().trim().to_string()),
        })
    } else {
        None
    }
}
