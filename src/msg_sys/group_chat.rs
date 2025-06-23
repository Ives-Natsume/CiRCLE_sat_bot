use reqwest;
use serde_json;
use crate::response::ApiResponse;

const ENDPOINT_URL: &str = "http://localhost:3300";

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