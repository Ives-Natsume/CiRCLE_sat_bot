use crate::{config::QQConfig, utils::BotAdapter};
use crate::utils::*;
use crate::backend::connection_manager::BackendConnectionManager;
use rinko_common::proto::MessageResponse;
use rinko_common::proto::ContentType;
use uuid::Uuid;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::{sleep, Duration};
use serde::{Deserialize, Deserializer, Serialize};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use axum::{
    extract::State,
    routing::post,
    Json,
    Router,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

const QQ_ACCESS_TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
const QQ_AUTHORIZE_URL: &str = "https://api.sgroup.qq.com";

/// Accept QQ's `timestamp` field whether it's an integer epoch or an RFC-3339
/// string (the API is inconsistent across message types).
fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!(
            "expected string or number for timestamp, got: {}",
            other
        ))),
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct WebhookPayload {
    op: u8,
    d: serde_json::Value,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    s: Option<u32>,
}

#[derive(Deserialize)]
struct ValidationRequest {
    plain_token: String,
    event_ts: String,
}

#[derive(Serialize)]
struct ValidationResponse {
    plain_token: String,
    signature: String,
}

#[allow(unused)]
#[derive(Deserialize, Debug)]
struct GroupMessageEvent {
    id: String,
    group_openid: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    timestamp: String,
}

#[derive(Serialize, Debug)]
struct SendGroupMessageRequest {
    content: Option<String>,
    msg_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg_seq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media: Option<MediaInfo>,
}

#[derive(Serialize, Debug)]
struct MediaInfo {
    file_info: String,
}

#[derive(Serialize, Debug)]
struct UploadMediaRequest {
    file_type: u8,
    url: String,
    srv_send_msg: bool,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct UploadMediaResponse {
    file_uuid: String,
    file_info: String,
    ttl: u32,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct SendMessageResponse {
    pub id: String,
    /// QQ may return either an integer epoch or an RFC-3339 string depending
    /// on the API version / message type, so we accept both by treating it as
    /// a raw JSON value and converting to string for display.
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub timestamp: String,
}

#[derive(Clone)]
struct WebhookState {
    client_secret: String,  // used as bot_secret for signature verification
    qq_config: Arc<RwLock<QQConfig>>,
    backend_manager: Option<Arc<BackendConnectionManager>>,
    /// Set of recently handled message IDs (message_id or event_id), used to
    /// deduplicate repeated QQ webhook deliveries.  Entries older than 2 minutes
    /// are purged on each insertion.  Protected by a std Mutex (never held
    /// across an await point).
    recent_event_ids: Arc<Mutex<HashMap<String, Instant>>>,
    /// Limits the number of concurrent media uploads to avoid hammering QQ's
    /// upload API and triggering its rate limiter / fetch-timeout errors.
    upload_semaphore: Arc<Semaphore>,
}

/// Webhook handler for QQ bot events
async fn handle_webhook(
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    tracing::debug!("Received webhook request");
    tracing::debug!("Headers: {:#?}", headers);
    tracing::debug!("Body: {:#?}", body);

    // Parse payload to check op code
    let payload: WebhookPayload = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to parse payload: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
        }
    };

    // Handle different operation codes
    match payload.op {
        13 => {
            // op=13: Configuration validation (when setting up webhook URL)
            tracing::info!("Handling webhook configuration validation (op=13)");
            handle_validation(&state.client_secret, payload.d).await
        }
        0 => {
            // op=0: Runtime event dispatch
            tracing::debug!("Handling runtime event (op=0)");

            // Verify signature before doing anything else
            let sig_header = headers.get("X-Signature-Ed25519")
                .and_then(|v| v.to_str().ok());
            let timestamp_header = headers.get("X-Signature-Timestamp")
                .and_then(|v| v.to_str().ok());

            let (sig_hex, timestamp) = match (sig_header, timestamp_header) {
                (Some(s), Some(t)) => (s, t),
                _ => {
                    tracing::warn!("Missing signature headers");
                    return (StatusCode::UNAUTHORIZED, "Missing signature headers").into_response();
                }
            };

            if let Err(e) = verify_runtime_signature(&state.client_secret, sig_hex, timestamp, &body) {
                tracing::error!("Signature verification failed: {}", e);
                return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
            }

            tracing::debug!("Signature verified");

            if let Some(event_type) = payload.t.clone() {
                let event_id = payload.d.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                let msg_id   = payload.d.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

                // ----------------------------------------------------------
                // Deduplication: QQ retries webhook delivery if it doesn't
                // receive a timely response.  We track message IDs so that a
                // second delivery of the same event is silently dropped.
                // ----------------------------------------------------------
                let dedup_key = format!("{}:{}", event_type, msg_id);
                {
                    let mut seen = state.recent_event_ids.lock().unwrap();
                    // Purge entries older than 2 minutes
                    let cutoff = Instant::now();
                    seen.retain(|_, t| cutoff.duration_since(*t).as_secs() < 120);

                    if seen.contains_key(&dedup_key) {
                        tracing::warn!(
                            "Duplicate webhook delivery for event '{}' id='{}', ignoring",
                            event_type, msg_id
                        );
                        // Return 204 so QQ stops retrying this delivery
                        return (StatusCode::NO_CONTENT, "").into_response();
                    }
                    seen.insert(dedup_key, Instant::now());
                }

                tracing::info!("Event type: {}", event_type);

                // ----------------------------------------------------------
                // Return 204 to QQ *immediately* so it stops retrying, then
                // process the event in a background task.
                // ----------------------------------------------------------
                let state_bg = state.clone();
                let data_bg  = payload.d.clone();
                tokio::spawn(async move {
                    handle_event(
                        &state_bg.qq_config,
                        &state_bg.backend_manager,
                        &state_bg.upload_semaphore,
                        &event_type,
                        &data_bg,
                        event_id,
                    ).await;
                });
            }

            (StatusCode::NO_CONTENT, "").into_response()
        }
        _ => {
            tracing::warn!("Unknown op code: {}", payload.op);
            (StatusCode::BAD_REQUEST, "Unknown operation").into_response()
        }
    }
}

/// Handle webhook configuration validation (op=13)
async fn handle_validation(
    client_secret: &str,
    data: serde_json::Value,
) -> Response {
    let validation: ValidationRequest = match serde_json::from_value(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to parse validation request: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid validation data").into_response();
        }
    };

    // Generate signature for validation
    let signature = match generate_validation_signature(client_secret, &validation.event_ts, &validation.plain_token) {
        Ok(sig) => sig,
        Err(e) => {
            tracing::error!("Failed to generate signature: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate signature").into_response();
        }
    };

    let response = ValidationResponse {
        plain_token: validation.plain_token,
        signature,
    };

    tracing::info!("Validation response generated successfully");
    Json(response).into_response()
}

/// Handle event processing
async fn handle_event(
    qq_config: &Arc<RwLock<QQConfig>>,
    backend_manager: &Option<Arc<BackendConnectionManager>>,
    upload_semaphore: &Arc<Semaphore>,
    event_type: &str,
    data: &serde_json::Value,
    event_id: Option<String>,
) {
    match event_type {
        "READY" => tracing::info!("Bot is ready"),
        "GROUP_AT_MESSAGE_CREATE" => {
            handle_group_at_message(qq_config, backend_manager, upload_semaphore, data, event_id).await;
        }
        "MESSAGE_CREATE" | "C2C_MESSAGE_CREATE" => {
            tracing::debug!("Message received: {:#?}", data);
            // TODO: Process other message types
        }
        "GUILD_CREATE" => tracing::info!("Joined a guild"),
        "FRIEND_ADD" => tracing::info!("Friend added"),
        "GROUP_ADD_ROBOT" => {
            tracing::info!("Added to group: {:#?}", data);
            // Could send a welcome message here
        }
        _ => tracing::debug!("Unhandled event type: {}", event_type),
    }
}

/// Handle GROUP_AT_MESSAGE_CREATE event
async fn handle_group_at_message(
    qq_config: &Arc<RwLock<QQConfig>>,
    backend_manager: &Option<Arc<BackendConnectionManager>>,
    upload_semaphore: &Arc<Semaphore>,
    data: &serde_json::Value,
    event_id: Option<String>,
) {
    tracing::debug!("Group @ message received: {:#?}", data);
    
    // Parse message event
    let msg_event = match serde_json::from_value::<GroupMessageEvent>(data.clone()) {
        Ok(event) => event,
        Err(e) => {
            tracing::error!("Failed to parse GROUP_AT_MESSAGE_CREATE event: {}", e);
            return;
        }
    };

    tracing::debug!(
        "Parsed message - ID: {}, Group: {}, Content: '{}'",
        msg_event.id,
        msg_event.group_openid,
        msg_event.content
    );

    let content_trimmed = msg_event.content.trim();
    
    // Parse command from message
    // let parsed_cmd = ParsedCommand::parse(content_trimmed);
    
    // Report message to backend if connected
    if let Some(manager) = backend_manager {
        let state = manager.state().await;
        
        if state == crate::backend::connection_manager::ConnectionState::Connected {
            let unified_msg = UnifiedMessage {
                event_id: Uuid::now_v7(),
                content: content_trimmed.to_string(),
                platform: Platform::QQ,
            };
            
            // Create metadata with command info and message context
            let mut metadata = HashMap::new();
            metadata.insert("group_openid".to_string(), msg_event.group_openid.clone());
            metadata.insert("message_id".to_string(), msg_event.id.clone());
            if let Some(ref eid) = event_id {
                metadata.insert("event_id".to_string(), eid.clone());
            }
            
            // Try to report to backend
            let client_lock = manager.client();
            if let Some(client) = &mut *client_lock.write().await {
                match client.report_message(unified_msg, metadata.clone()).await {
                    Ok(response) => {
                        tracing::debug!(
                            "Message reported to backend",
                        );
                        
                        // If backend returns a response message, send it
                        if !response.message.is_empty() && response.message != "OK" {
                            let config = qq_config.read().await;
                            if let Err(e) = config.send_message(response, &msg_event, upload_semaphore).await {
                                tracing::error!("Failed to send backend response to QQ: {}", e);
                                // Fall back to text reply
                                let fallback_reply = format!("\nMedia upload failed >_\nPlease retry later.\nMessage received:\n「{}」", content_trimmed);
                                if let Err(e) = config.send_group_message(
                                    &msg_event.group_openid,
                                    &fallback_reply,
                                    Some(msg_event.id.clone()),
                                    event_id.clone(),
                                    Some(1),
                                ).await {
                                    tracing::error!("Failed to send fallback reply to QQ: {}", e);
                                }
                            }
                        }
                        
                        // Backend handled the message, return early
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to report message to backend: {}. Marking as disconnected.", e);
                        manager.mark_disconnected().await;
                    }
                }
            }
        } else {
            tracing::debug!("Backend offline, using local processing");
        }
    }
    
    // Local fallback processing if backend is disabled or failed
    let config = qq_config.read().await;

    let reply_content = format!("\nRinko backend offline >_\nMessage received: {}", content_trimmed);
    
    if let Err(e) = config.send_group_message(
        &msg_event.group_openid,
        &reply_content,
        Some(msg_event.id),
        event_id,
        Some(1),
    ).await {
        tracing::error!("Failed to send reply: {}", e);
    }
}

/// Generate signature for configuration validation (op=13)
fn generate_validation_signature(
    client_secret: &str,
    event_ts: &str,
    plain_token: &str,
) -> anyhow::Result<String> {
    // Generate signing key from client_secret
    let mut seed = client_secret.to_string();
    while seed.len() < 32 {
        seed.push_str(client_secret);
    }
    let seed_bytes: [u8; 32] = seed.as_bytes()[0..32]
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to create seed"))?;

    let signing_key = SigningKey::from_bytes(&seed_bytes);

    // Construct message: event_ts + plain_token
    let mut message = Vec::new();
    message.extend_from_slice(event_ts.as_bytes());
    message.extend_from_slice(plain_token.as_bytes());

    // Sign and encode as hex
    let signature = signing_key.sign(&message);
    Ok(hex::encode(signature.to_bytes()))
}

/// Verify Ed25519 signature for runtime events (op=0)
fn verify_runtime_signature(
    client_secret: &str,  // client_secret is used as bot_secret
    sig_hex: &str,
    timestamp: &str,
    body: &str,
) -> anyhow::Result<()> {
    // 1. Generate public key from client_secret (used as bot_secret)
    // According to docs: seed = secret + secret (until >= 32 bytes)
    let mut seed = client_secret.to_string();
    while seed.len() < 32 {
        seed.push_str(client_secret);
    }
    let seed_bytes: [u8; 32] = seed.as_bytes()[0..32]
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to create seed"))?;

    // Generate keypair to extract public key
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed_bytes);
    let verifying_key: VerifyingKey = (&signing_key).into();

    // 2. Decode signature from hex
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| anyhow::anyhow!("Failed to decode signature hex: {}", e))?;
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid signature format: {}", e))?;

    // 3. Construct message: timestamp + body
    let mut message = Vec::new();
    message.extend_from_slice(timestamp.as_bytes());
    message.extend_from_slice(body.as_bytes());

    // 4. Verify signature
    verifying_key
        .verify(&message, &signature)
        .map_err(|e| anyhow::anyhow!("Signature verification failed: {}", e))?;

    Ok(())
}

#[async_trait]
impl BotAdapter for QQConfig {
    async fn process_message(&self) -> anyhow::Result<UnifiedMessage> {
        // Implementation for processing a message from QQ
        let message = UnifiedMessage {
            event_id: Uuid::now_v7(),
            content: "Sample QQ message".to_string(),
            platform: Platform::QQ,
        };
        Ok(message)
    }

    async fn send_message(&self, message: &UnifiedMessage) -> anyhow::Result<()> {
        // Implementation for sending a message via QQ
        println!("Sending message to QQ: {:#?}", message);
        Ok(())
    }
}

impl QQConfig {
    pub async fn init(&mut self) -> anyhow::Result<()> {
        self.client = reqwest::Client::new();
        self.get_access_token().await
    }

    /// Start webhook server to receive QQ bot events
    pub async fn start_webhook_server(
        qq_config: Arc<RwLock<Self>>,
        backend_manager: Option<Arc<BackendConnectionManager>>,
        port: u16,
    ) -> anyhow::Result<()> {
        let client_secret = qq_config.read().await.client_secret.clone();
        
        let state = Arc::new(WebhookState {
            client_secret,
            qq_config: qq_config.clone(),
            backend_manager,
            recent_event_ids: Arc::new(Mutex::new(HashMap::new())),
            // Allow at most 2 concurrent media uploads.  QQ's fetch-timeout
            // errors (err_code 40034003) become much less frequent when we
            // avoid sending parallel upload requests.
            upload_semaphore: Arc::new(Semaphore::new(2)),
        });

        let app = Router::new()
            .route("/webhook", post(handle_webhook))
            .with_state(state);

        let addr = format!("127.0.0.1:{}", port);
        tracing::info!("Starting QQ webhook server on {}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }

    pub async fn get_access_token(&mut self) -> anyhow::Result<()> {
        let resp = self.client
            .post(QQ_ACCESS_TOKEN_URL)
            .json(&serde_json::json!({
                "appId": self.app_id,
                "clientSecret": self.client_secret
            }))
            .send()
            .await?;

        let status = resp.status();
        let raw = resp.bytes().await?;
        tracing::debug!(
            "get_access_token response  status={} body={}",
            status,
            String::from_utf8_lossy(&raw)
        );
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "get_access_token HTTP {}: {}",
                status,
                String::from_utf8_lossy(&raw)
            ));
        }

        let resp_json: serde_json::Value = serde_json::from_slice(&raw)?;
        
        let token = resp_json
            .get("access_token")
            .and_then(|v| v.as_str());

        let expire = resp_json
            .get("expires_in")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());

        if let (Some(token), Some(expire)) = (token, expire) {
            // S-1 fix: log only prefix to avoid leaking the full credential
            let masked = format!("{}…", &token[..token.len().min(8)]);
            tracing::debug!("Obtained QQ access token prefix={} expires_in={}", masked, expire);
            self.access_token = token.to_string();
            self.token_expires_in = expire;
            self.token_fetched_at = Some(tokio::time::Instant::now());
            Ok(())
        } else {
            Err(anyhow::anyhow!("Failed to get access token from QQ: {:?}", resp_json))
        }
    }

    /// Starts a background task that automatically renews the access token before it expires
    pub fn start_token_renewal_task(config: Arc<RwLock<Self>>) {
        tokio::spawn(async move {
            loop {
                let refresh_delay = {
                    let cfg = config.read().await;
                    if let Some(fetched_at) = cfg.token_fetched_at {
                        let elapsed = fetched_at.elapsed().as_secs();
                        let lifetime = cfg.token_expires_in;
                        // refresh 50 seconds before expiry
                        let refresh_after = lifetime.saturating_sub(50);
                        if elapsed >= refresh_after {
                            Duration::from_secs(0)
                        } else {
                            Duration::from_secs(refresh_after - elapsed)
                        }
                    } else {
                        Duration::from_secs(0)
                    }
                };
                
                tracing::debug!("QQ token renewal scheduled in {:?}", refresh_delay);
                sleep(refresh_delay).await;
                
                tracing::info!("Attempting to renew QQ access token...");
                let mut cfg = config.write().await;
                if let Err(e) = cfg.get_access_token().await {
                    tracing::error!("Failed to renew QQ access token: {}. Retrying in 30 seconds.", e);
                    drop(cfg); // Release lock before sleeping
                    sleep(Duration::from_secs(30)).await;
                } else {
                    tracing::info!("QQ access token renewed successfully.");
                }
            }
        });
    }

    /// Upload media file and get file_info, with automatic retry.
    ///
    /// QQ's media upload endpoint is occasionally flaky (transient 5xx or
    /// empty/malformed responses). We retry up to 3 times with exponential
    /// back-off (1 s, 2 s, 4 s) before giving up.
    ///
    /// # Parameters
    /// - `group_openid`: The openid of the target group
    /// - `file_type`: 1=image, 2=video, 3=voice, 4=file
    /// - `url`: URL of the media resource (must be accessible by QQ servers)
    /// - `srv_send_msg`: Whether to send message directly (not recommended)
    async fn upload_group_media(
        &self,
        group_openid: &str,
        file_type: u8,
        url: &str,
        srv_send_msg: bool,
    ) -> anyhow::Result<UploadMediaResponse> {
        const MAX_ATTEMPTS: u32 = 3;
        let api_url = format!("{}/v2/groups/{}/files", QQ_AUTHORIZE_URL, group_openid);

        let payload = UploadMediaRequest {
            file_type,
            url: url.to_string(),
            srv_send_msg,
        };

        tracing::info!(
            "upload_group_media  group={} file_type={} url={}",
            group_openid, file_type, url
        );

        let mut last_err = anyhow::anyhow!("upload_group_media: no attempts made");

        for attempt in 1..=MAX_ATTEMPTS {
            if attempt > 1 {
                let backoff = Duration::from_secs(1u64 << (attempt - 2)); // 1 s, 2 s
                tracing::warn!(
                    "upload_group_media attempt {}/{} after {:?} backoff (prev error: {})",
                    attempt, MAX_ATTEMPTS, backoff, last_err
                );
                sleep(backoff).await;
            }

            let send_result = self.client
                .post(&api_url)
                .header("Authorization", format!("QQBot {}", self.access_token))
                .json(&payload)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await;

            let resp = match send_result {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("upload_group_media attempt {} network error: {}", attempt, e);
                    last_err = e.into();
                    continue;
                }
            };

            let status = resp.status();
            let raw = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("upload_group_media attempt {} body read error: {}", attempt, e);
                    last_err = e.into();
                    continue;
                }
            };

            tracing::debug!(
                "upload_group_media attempt {} response  status={} body={}",
                attempt, status, String::from_utf8_lossy(&raw)
            );

            if status.is_server_error() {
                // 5xx — transient, worth retrying
                last_err = anyhow::anyhow!(
                    "upload_group_media HTTP {} (attempt {}): {}",
                    status, attempt, String::from_utf8_lossy(&raw)
                );
                tracing::warn!("upload_group_media 5xx on attempt {}: {}", attempt, last_err);
                continue;
            }

            if !status.is_success() {
                // For 4xx responses, check whether QQ's err_code indicates a
                // server-side transient failure that is worth retrying:
                //   40034003 — QQ server timed out fetching the media URL
                //   40034002 — QQ server failed to download the media URL
                // These look like client errors (400) but the fault is on QQ's
                // side; a short wait is usually enough to recover.
                let qq_err_code = serde_json::from_slice::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|v| v.get("err_code").and_then(|c| c.as_u64()));

                const RETRYABLE_QQ_CODES: &[u64] = &[
                    40034003, // media fetch timeout on QQ side
                    40034002, // media download failed on QQ side
                ];

                if let Some(code) = qq_err_code {
                    if RETRYABLE_QQ_CODES.contains(&code) {
                        last_err = anyhow::anyhow!(
                            "upload_group_media QQ err_code={} (attempt {}): {}",
                            code, attempt, String::from_utf8_lossy(&raw)
                        );
                        tracing::warn!(
                            "upload_group_media retryable QQ error on attempt {}: {}",
                            attempt, last_err
                        );
                        continue;
                    }
                }

                // Permanent 4xx (bad request, auth failure, etc.)
                let err = anyhow::anyhow!(
                    "upload_group_media HTTP {}: {}",
                    status, String::from_utf8_lossy(&raw)
                );
                tracing::error!("{}", err);
                return Err(err);
            }

            match serde_json::from_slice::<UploadMediaResponse>(&raw) {
                Ok(response) => {
                    tracing::info!(
                        "upload_group_media success (attempt {})  UUID={} TTL={}s",
                        attempt, response.file_uuid, response.ttl
                    );
                    return Ok(response);
                }
                Err(e) => {
                    // Parse failure on a 200 — log and retry; may be a partially
                    // flushed response.
                    last_err = anyhow::anyhow!(
                        "upload_group_media parse error (attempt {}): {}  body={}",
                        attempt, e, String::from_utf8_lossy(&raw)
                    );
                    tracing::warn!("{}", last_err);
                    continue;
                }
            }
        }

        tracing::error!("upload_group_media failed after {} attempts: {}", MAX_ATTEMPTS, last_err);
        Err(last_err)
    }

    /// Send rich media message to group
    /// 
    /// # Parameters
    /// - `group_openid`: The openid of the target group
    /// - `file_info`: The file_info obtained from upload_group_media
    /// - `msg_id`: Optional message ID for passive reply
    /// - `event_id`: Optional event ID for passive message
    /// - `msg_seq`: Optional message sequence number
    async fn send_group_media_message(
        &self,
        group_openid: &str,
        file_info: &str,
        msg_id: Option<String>,
        event_id: Option<String>,
        msg_seq: Option<u32>,
    ) -> anyhow::Result<SendMessageResponse> {
        let url = format!("{}/v2/groups/{}/messages", QQ_AUTHORIZE_URL, group_openid);
        
        let payload = SendGroupMessageRequest {
            content: None,
            msg_type: 7, // 7 = rich media
            msg_id,
            event_id,
            msg_seq,
            media: Some(MediaInfo {
                file_info: file_info.to_string(),
            }),
        };

        tracing::debug!("Sending media message to group {}: {:?}", group_openid, payload);

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("QQBot {}", self.access_token))
            .json(&payload)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;

        let status = resp.status();
        let raw = resp.bytes().await?;
        tracing::debug!(
            "send_group_media_message response  status={} body={}",
            status,
            String::from_utf8_lossy(&raw)
        );
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "send_group_media_message HTTP {}: {}",
                status,
                String::from_utf8_lossy(&raw)
            ));
        }

        let response: SendMessageResponse = serde_json::from_slice(&raw)
            .map_err(|e| anyhow::anyhow!(
                "send_group_media_message parse error: {}  body={}",
                e,
                String::from_utf8_lossy(&raw)
            ))?;
        tracing::info!(
            "Media message sent successfully - ID: {}, Timestamp: {}",
            response.id,
            response.timestamp
        );

        Ok(response)
    }

    /// Send image from local file to group
    /// This is a high-level function that handles the complete workflow
    /// 
    /// # Parameters
    /// - `group_openid`: The openid of the target group
    /// - `local_path`: Path to the local image file (e.g., "../rinko-backend/data/satellite_cache/rendered_images/sat_123.png")
    /// - `msg_id`: Optional message ID for passive reply
    /// - `event_id`: Optional event ID for passive message
    /// - `msg_seq`: Optional message sequence number
    /// 
    /// # Notes
    /// - Requires `media_base_url` to be configured in config.toml
    /// - Extracts filename from local_path and constructs public URL
    pub async fn send_group_image(
        &self,
        group_openid: &str,
        local_path: &str,
        msg_id: Option<String>,
        event_id: Option<String>,
        msg_seq: Option<u32>,
        upload_semaphore: &Semaphore,
    ) -> anyhow::Result<SendMessageResponse> {
        // Extract filename from local path
        let filename = std::path::Path::new(local_path)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid file path: {}", local_path))?;
        
        // Construct public URL using media_base_url from config
        let image_url = if let Some(base_url) = &self.media_base_url {
            format!("{}/{}", base_url.trim_end_matches('/'), filename)
        } else {
            // Fallback: use placeholder if media_base_url is not configured
            tracing::warn!(
                "media_base_url not configured in config.toml. Using placeholder URL."
            );
            "https://metasequoiani.com/_astro/image.BnwkcnDf_Z2vIi4L.webp".to_string()
        };
        
        tracing::info!(
            "Sending image '{}' to group via URL: {}",
            filename,
            image_url
        );

        // Acquire semaphore permit before uploading.  This caps concurrent
        // uploads globally so we don't saturate QQ's media-fetch pipeline.
        let _permit = upload_semaphore
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("upload semaphore closed"))?;
        tracing::debug!("upload semaphore acquired for '{}'", filename);

        // Step 1: Upload media and get file_info
        let upload_response = self.upload_group_media(
            group_openid,
            1, // 1 = image
            &image_url,
            false, // Don't send directly, get file_info for flexible usage
        ).await?;

        // Step 2: Send media message using file_info
        // (_permit is still held here; released when this fn returns)
        self.send_group_media_message(
            group_openid,
            &upload_response.file_info,
            msg_id,
            event_id,
            msg_seq,
        ).await
    }

    /// Send message to a group chat
    /// 
    /// # Parameters
    /// - `group_openid`: The openid of the target group
    /// - `content`: Message content
    /// - `msg_id`: Optional message ID for passive reply (within 5 minutes)
    /// - `event_id`: Optional event ID for passive message
    /// - `msg_seq`: Optional message sequence number (default: 1)
    pub async fn send_group_message(
        &self,
        group_openid: &str,
        content: &str,
        msg_id: Option<String>,
        event_id: Option<String>,
        msg_seq: Option<u32>,
    ) -> anyhow::Result<SendMessageResponse> {
        let url = format!("{}/v2/groups/{}/messages", QQ_AUTHORIZE_URL, group_openid);
        
        let payload = SendGroupMessageRequest {
            content: Some(content.to_string()),
            msg_type: 0, // 0 = text message
            msg_id,
            event_id,
            msg_seq,
            media: None,
        };

        tracing::debug!("Sending message to group {}: {:#?}", group_openid, payload);

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("QQBot {}", self.access_token))
            .json(&payload)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;

        let status = resp.status();
        let raw = resp.bytes().await
            .map_err(|e| anyhow::anyhow!("Failed to read send_group_message response body: {}", e))?;
        tracing::debug!(
            "send_group_message response  status={} body={}",
            status,
            String::from_utf8_lossy(&raw)
        );
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "send_group_message HTTP {}: {}",
                status,
                String::from_utf8_lossy(&raw)
            ));
        }

        let response: SendMessageResponse = match serde_json::from_slice(&raw) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(
                    "Failed to parse send_group_message response: {}  body={}",
                    e,
                    String::from_utf8_lossy(&raw)
                );
                return Err(anyhow::anyhow!(
                    "Failed to parse send_group_message response: {}  body={}",
                    e,
                    String::from_utf8_lossy(&raw)
                ));
            }
        };
        tracing::info!(
            "Message sent successfully - ID: {}, Timestamp: {}",
            response.id,
            response.timestamp
        );

        Ok(response)
    }

    async fn send_message(&self, resp: MessageResponse, msg_event: &GroupMessageEvent, upload_semaphore: &Semaphore) -> anyhow::Result<()> {
        match resp.content_type {
            ct if ct == ContentType::Text as i32 => {
                self.send_group_message(
                    &msg_event.group_openid,
                    &resp.message,
                    Some(msg_event.id.clone()),
                    None,
                    Some(1),
                ).await?;
            }
            ct if ct == ContentType::Image as i32 => {
                // For image messages, the message field contains the file path
                let local_path = resp.message.strip_prefix("file:///").unwrap_or(&resp.message);
                self.send_group_image(
                    &msg_event.group_openid,
                    local_path,
                    Some(msg_event.id.clone()),
                    None,
                    Some(1),
                    upload_semaphore,
                ).await?;
            }
            _ => {
                tracing::warn!("Unsupported content type: {}", resp.content_type);
                // Fallback to sending as text
                self.send_group_message(
                    &msg_event.group_openid,
                    "[Unsupported content type]",
                    Some(msg_event.id.clone()),
                    None,
                    Some(1),
                ).await?;
            }
        }
        Ok(())
    }
}