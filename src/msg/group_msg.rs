use reqwest;
use serde_json;
use std::sync::Arc;
use crate::{
    response::ApiResponse,
    i18n,
    app_status::AppStatus,
};
use crate::msg::prelude::*;

#[allow(unused)]
pub async fn send_group_msg(
    response: ApiResponse<Vec<String>>,
    payload: &MessageEvent,
    url: &String,
) {
    let message_text: String = response
        .data
        .map(|data| data.join("\n"))
        .unwrap_or_else(|| response.message.unwrap_or_else(|| i18n::text("no_response_data")));

    let group_id = payload.group_id;
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

    let endpoint_url = format!("{}/send_group_msg", url);
    let client = reqwest::Client::new();
    let response = client
        .post(endpoint_url)
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

pub async fn send_group_message_to_multiple_groups(
    response: ApiResponse<Vec<String>>,
    app_status: &Arc<AppStatus>,
) {
    // TODO: message type auto, banning bot from sending auto system msg
    if response == ApiResponse::empty() {
        return ;
    };
    let config = app_status.config.read().await;
    let url = config.bot_config.sse_url.clone();
    let groups = &config.bot_config.group_id;
    let msg_text = response
        .data
        .map(|data| data.join("\n"))
        .unwrap_or_else(|| response.message.unwrap_or_else(|| i18n::text("no_response_data")));
    for group_id in groups {
        let msg_body = serde_json::json!({
            "group_id": group_id,
            "message": [
                {
                    "type": "text",
                    "data": {
                        "text": msg_text
                    }
                }
            ]
        });

        let endpoint_url = format!("{}/send_group_msg", url);
        let client = reqwest::Client::new();
        let response = client
            .post(endpoint_url)
            .json(&msg_body)
            .send()
            .await;

        match response {
            Ok(res) => {
                let status = res.status();
                let body = res.text().await.unwrap_or_else(|_| "<Failed to read body>".to_string());
                if !status.is_success() {
                    tracing::error!("{}: {}", i18n::text("send_group_msg_err"), body);
                }
                if body.contains("error") {
                    tracing::error!("{}: {}", i18n::text("send_group_msg_err"), body);
                }
            }
            Err(err) => {
                tracing::error!("{}: {}", i18n::text("send_group_msg_err"), err);
            }
        }
    }
}

pub async fn _send_picture_to_group(
    response: ApiResponse<Vec<String>>,
    payload: &MessageEvent,
    app_status: &Arc<AppStatus>,
) {
    let config = app_status.config.read().await;
    let url = config.bot_config.sse_url.clone();
    let image_path = match response.data {
        Some(data) if !data.is_empty() => data[0].clone(),
        _ => {
            tracing::error!("No image data provided in response");
            return;
        }
    };

    let msg_body = serde_json::json!({
        "group_id": payload.group_id,
        "message": [
            {
                "type": "image",
                "data": {
                    "file": image_path
                }
            }
        ]
    });

    let endpoint_url = format!("{}/send_group_msg", url);
    let client = reqwest::Client::new();
    let response = client
        .post(endpoint_url)
        .json(&msg_body)
        .send()
        .await;

    match response {
        Ok(res) => {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<Failed to read body>".to_string());
            if !status.is_success() {
                tracing::error!("{}: {}", i18n::text("send_group_msg_err"), body);
            }
            if body.contains("error") {
                tracing::error!("{}: {}", i18n::text("send_group_msg_err"), body);
            }
        }
        Err(err) => {
            tracing::error!("{}: {}", i18n::text("send_group_msg_err"), err);
        }
    }
}
