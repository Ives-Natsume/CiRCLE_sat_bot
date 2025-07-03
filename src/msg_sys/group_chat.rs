use reqwest;
use serde_json;
use crate::{
    query,
    response::ApiResponse,
    config
};
use crate::msg_sys::prelude::*;

const ENDPOINT_URL: &str = "http://localhost:3300/send_group_msg";

/// Sends a group message to the specified group ID using the provided API response.
/// Send `ApiResponse.message` if no valid data is provided.
pub async fn send_group_msg(
    response: ApiResponse<Vec<String>>,
    group_id: u64,
) {
    let message_text: String = response
        .data
        .map(|data| data.join("\n"))
        .unwrap_or_else(|| response.message.unwrap_or_else(|| "No data available".to_string()));

    let msg_body = serde_json::json!({
        "group_id": group_id,
        "message": [
            {
                "type": "text",
                "data": {
                    "text": message_text
                }
            }
        ]
    });

    let client = reqwest::Client::new();
    let response = client
        .post(ENDPOINT_URL)
        .json(&msg_body)
        .send()
        .await;

    match response {
        Ok(res) => {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<Failed to read body>".to_string());
            tracing::info!("Group message sent. Status: {}, Response: {}", status, body);
        }
        Err(err) => {
            tracing::error!("Failed to send group message: {}", err);
        }
    }
}

async fn router(
    payload: &MessageEvent,
    config: &config::Config,
) {
    let mut message_text: String = String::new();
    let mut response: ApiResponse<Vec<String>> = ApiResponse {
        success: false,
        data: None,
        message: None,
    };

    for elem in &payload.message {
        match elem {
            MessageElement::Text { text } => {
                message_text.push_str(text);
            }
            _ => {}
        }
    }

    let re = regex::Regex::new(r"^\s*/(\w+)(?:\s+([\s\S]*))?$").unwrap();

    if let Some(caps) = re.captures(message_text.as_str()) {
        let command = caps.get(1).unwrap().as_str().to_string();
        let args = caps.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();

        match command.as_str() {
            "query" | "q"=> {
                response = query_handler(&args).await;
            },
            "help" | "h" => {
                response.success = true;
                response.data = config.backend_config.help.clone();
            },
            "pass" | "p" => {
                if payload.group_id != 965954401 {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }
                if args.is_empty() {
                    response.message = Some("没有卫星名无法查询喵！".to_string());
                } else {
                    let query_response = crate::pass_query::sat_pass_predict::query_satellite(Some(args));
                    if query_response.is_empty() {
                        response.message = Some("没有找到这个名字的卫星喵...".to_string());
                    } else {
                        response.success = true;
                        response.data = Some(query_response);
                    }
                }
            },
            "all" | "a" => {
                if payload.group_id != 965954401 {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }
                let query_response = crate::pass_query::all_pass_notify::get_all_sats_pass().await;
                if query_response.is_empty() {
                    response.message = Some("没有找到卫星经过的信息呢，是哪出错了呢？QWQ".to_string());
                } else {
                    response.success = true;
                    response.data = Some(query_response);
                }
            },
            "about" => {
                response.success = true;
                response.data = config.backend_config.about.clone();
            }
            _ => {
                response.message = Some(format!("说了这些难懂的话，你也有责任吧？"));
            }
        }
    } else {
        response.message = Some("干什么！".to_string());
    }

    let group_id = payload.group_id;
    send_group_msg(response, group_id).await;
}

async fn query_handler(
    args: &str,
) -> ApiResponse<Vec<String>> {
    let mut response_data: Vec<String> = Vec::new();
    let mut response_msg: String = String::new();
    let mut success: bool = true;

    // query key words:
    // `/query <sat_name>`: look up for AMSAT data by satellite name
    let json_file_path = "amsat_status.json";
    let toml_file_path = "satellites.toml";

    let query_response = query::sat_query::look_up_sat_status_from_json(
        json_file_path,
        toml_file_path,
        args,
    );

    match query_response {
        ApiResponse { success: true, data: Some(results), message: None } => {
            response_data = results;
            if response_data.is_empty() {
                response_msg = format!("Rinko宕机了喵...重新试试吧");
                success = false;
            }
        }
        ApiResponse { success: false, data: None, message: Some(msg) } => {
            response_msg = msg;
            success = false;
        }
        _ => {
            response_msg = "Ako酱...总感觉...有什么不好的事情发生了".to_string();
            success = false;
        }
    }

    let response = ApiResponse {
        success,
        data: if response_data.is_empty() {
            None
        } else {
            Some(response_data)
        },
        message: if response_msg.is_empty() {
            None
        } else {
            Some(response_msg)
        },
    };

    response
}

pub async fn message_handler(
    message_raw_text: String,
    config: &config::Config,
) {
    match parse_message_event(&message_raw_text) {
        Ok(payload) => {
            // check if message contains AT element
            if payload.message.iter().any(|elem| {
                matches!(elem, MessageElement::At { qq, .. } if *qq == config.bot_config.qq_id)
            }) {
                router(&payload, &config).await;
            }
            else if payload.message.iter().any(|elem| {
                matches!(elem, MessageElement::Text { text } if text.starts_with("/q") || text.starts_with("/h") || text.starts_with("/p") || text.starts_with("/a")) &&
                !matches!(elem, MessageElement::At { .. })
            }){
                router(&payload, &config).await;
            }
        },
        Err(_) => {
            //tracing::error!("Failed to parse message event: {}", e);
        }
    };
}