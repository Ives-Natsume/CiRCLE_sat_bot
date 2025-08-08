use crate::{
    app_status::AppStatus,
    module::{
        amsat::official_report::query_satellite_status,
        amsat::user_report::{push_user_report, add_user_report, create_report_template},
    },
    msg::prelude::{BinMessageEvent, FromBinMessageEvent, MessageElement, MessageEvent},
    response::ApiResponse,
    socket::MsgContent
};
use std::{sync::Arc, vec};

pub async fn bot_message_handler(
    msg: MsgContent,
    app_status: Arc<AppStatus>,
) -> ApiResponse<Vec<String>> {
    #[allow(unused_mut)]
    let mut response: ApiResponse<Vec<String>> = ApiResponse {
        success: false,
        data: None,
        message: None,
    };
    if let Some(_message) = msg.message {
        // core端确保包含message的消息不会携带payload和command
        // 直接退出
        return response;
    }

    let Some(payload) = msg.payload else {
      return response;
    };
    let Some(command) = msg.command else {
      return response;
    };

    let payload = BinMessageEvent::from_bin_message_event(payload);
    router(command, payload, app_status).await
}

async fn router(
    _command: String,
    payload: MessageEvent,
    app_status: Arc<AppStatus>,
) -> ApiResponse<Vec<String>> {
    let mut response: ApiResponse<Vec<String>> = ApiResponse {
        success: false,
        data: None,
        message: None,
    };

    let mut message_text: String = String::new();
    for elem in &payload.message {
        if let MessageElement::Text { text } = elem {
            message_text.push_str(text);
        }
    }
    let text = message_text.trim().to_string();
    let (command, args) = get_command_and_args(&text);

    match command.as_str() {
        "q" | "query" => {
            response = query_satellite_status(&args, &app_status).await;
        }
        "s" | "sun" => {
            // let uri = match solar_image::get_image::file_uri("data/pic/solar_image_latest.png") {
            //     Ok(uri) => uri,
            //     Err(e) => {
            //         tracing::error!("Failed to encode solar image path: {}", e);
            //         return response;
            //     }
            // };
            let uri: String = "file:///server_data/pic/solar_image_latest.png".to_string();
            response = ApiResponse {
                success: true,
                data: Some(vec![uri]),
                message: Some("solar image".to_string()),
            };
        }
        "spot" => {
            let user_id = payload.user_id.clone();
            let admin_id = app_status.config.read().await.bot_config.admin_id.clone();
            if !admin_id.contains(&user_id) {
                return ApiResponse::error("测试阶段只开放给FNA测试喵".to_string());
            }
            response = push_user_report(&args).await;
        }
        "create" => {
            response = match create_report_template(&args, app_status).await {
                Ok(_) => ApiResponse::ok(vec!["报告模板创建成功喵".to_string()]),
                Err(e) => ApiResponse::error(format!("报告模板创建失败喵: {}", e)),
            };
        }
        "report" => {
            response = add_user_report(app_status, &args, &payload).await;
        }
        _ => {}
    }
    response
}

/// - Split by whitespace and normalize
/// - Keep CJK characters intact
pub fn _string_normalize(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .collect()
}

/// Get the command and arguments from the input string
/// - Supports commands like `/command args`
/// - Returns a tuple of (command, args)
pub fn get_command_and_args(input: &str) -> (String, String) {
    let re = regex::Regex::new(r"^\s*/(\w+)(?:\s+([\s\S]*))?$").unwrap();
    if let Some(caps) = re.captures(input) {
        let command = caps.get(1).map_or("", |m| m.as_str());
        let args = caps.get(2).map_or("", |m| m.as_str());
        (command.to_string(), args.to_string())
    } else {
        (String::new(), String::new())
    }
}