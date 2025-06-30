use reqwest;
use serde_json;
use tracing::info;
use crate::{query, response:: ApiResponse};
use axum::{
    extract::Json,
    response::IntoResponse,
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
        .unwrap_or_else(|| "No message provided".to_string());

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
    Json(payload) : Json<IncomingRequest>,
) -> impl IntoResponse {
    let message_text = payload.message;
    let group_id = payload.group_id;
    info!("Received query from user: {}, group_id: {}, message: {}",
          payload.sender.user_id, group_id, message_text);
    
    let mut response_data: Vec<String> = Vec::new();
    let mut response_msg: String = String::new();
    let mut success: bool = true;

    // query key words:
    // `/query <sat_name>`: look up for AMSAT data by satellite name
    match message_text.find("/query") {
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
        let query_sat_name = message_text.trim_start_matches("/query").trim();
        if query_sat_name.trim().is_empty() {
            tracing::warn!("Received empty message from user: {}, group_id: {}", payload.sender.user_id, group_id);
            response_msg = "Empty message received".to_string();
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
                    response_msg = format!("Internal error occurred while looking up: {}", query_sat_name);
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

    let response_text = if success {
        response_data.join("\n")
    } else {
        response_msg
    };

    let msg_body = serde_json::json!({
        "group_id": group_id,
        "message": [
            {
                "type": "text",
                "data": {
                    "text": response_text
                }
            }
        ]
    });

    Json(msg_body)
}