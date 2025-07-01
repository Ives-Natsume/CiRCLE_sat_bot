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
            "query" => {
                response = query_handler(&args).await;
            },
            "help" | "h" => {
                response.success = true;
                response.data = Some(vec![
                    "Available commands:".to_string(),
                    "/query <sat_name>\n- Look up AMSAT data by satellite name\n".to_string(),
                    "/about\n- About me\n".to_string(),
                    "/help\n- Show this help message".to_string(),
                ]);
            },
            "about" => {
                response.success = true;
                response.data = config.backend_config.about.clone();
            }
            _ => {
                response.message = Some(format!("Unknown command: {}\nUse /help for available commands", command));
            }
        }
    } else {
        response.message = Some("gsm!".to_string());
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
                response_msg = format!("Internal error occurred while looking up for{}", args);
                success = false;
            }
        }
        ApiResponse { success: false, data: None, message: Some(msg) } => {
            response_msg = msg;
            success = false;
        }
        _ => {
            response_msg = "Unexpected response format".to_string();
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
        },
        Err(_) => {
            //tracing::error!("Failed to parse message event: {}", e);
        }
    };
}