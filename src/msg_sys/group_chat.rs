use reqwest;
use serde_json;
use tracing::info;
use crate::{query, response::ApiResponse};
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

pub async fn send_group_msg_from_request(
    message_raw_text: String,
) {
    let payload = match parse_message_event(&message_raw_text) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::error!("Failed to parse message event: {}", e);
            return ;
        }
    };
    
    let mut response_data: Vec<String> = Vec::new();
    let mut response_msg: String = String::new();
    let mut success: bool = true;
    let mut message_text: String = String::new();

    let mut ated = false;
    for (_, elem) in payload.message.iter().enumerate() {
        match elem {
            MessageElement::Text { text } => {
                if ated {
                    message_text.push_str(text);
                }
            }
            MessageElement::At { qq, .. } => {
                if qq == "3906406150" {
                    ated = true;
                }
                continue;
            }
            _ => {
                // Ignore
            }
        }
    }
    if ated == false {
        return ;
    }

    info!("Received query from user: {}, group_id: {}",
          payload.sender.user_id, payload.group_id);

    // query key words:
    // `/query <sat_name>`: look up for AMSAT data by satellite name
    match message_text.find(" /query") {
        Some(idx) => {
            if idx != 0 {
                response_msg = "Invalid command format. Use /query <sat_name>".to_string();
                success = false;
            }
        }
        None => {
            response_msg = "Invalid command format. Use /query <sat_name>".to_string();
            success = false;
        }
    }

    if success {
        let query_sat_name = message_text.trim_start_matches(" /query").trim();
        if query_sat_name.trim().is_empty() {
            response_msg = "Message should not be empty".to_string();
            success = false;
        }

        let json_file_path = "amsat_status.json";
        let toml_file_path = "satellites.toml";

        let query_response = query::sat_query::look_up_sat_status_from_json(
            json_file_path,
            toml_file_path,
            query_sat_name,
        );

        match query_response {
            ApiResponse { success: true, data: Some(results), message: None } => {
                response_data = results;
                if response_data.is_empty() {
                    response_msg = format!("Internal error occurred while looking up for{}", query_sat_name);
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

    send_group_msg(response, payload.group_id).await;

    return ;
}