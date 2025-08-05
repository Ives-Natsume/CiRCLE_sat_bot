#![allow(unused)]
use serde::{Serialize, Deserialize};
use channels::channel;
use tokio::{
    net::TcpStream,
    sync::{
        oneshot,
        Mutex,
        RwLock,
    },
};
use std::{
    sync::Arc,
    net::SocketAddr,
    time::{Duration, Instant},
    path::PathBuf,
};
use crate::{
    app_status::AppStatus,
    config::Config,
    fs,
    i18n,
    router,
    msg::prelude::BinMessageEvent,
    response::ApiResponse,
    CONFIG_FILE_PATH
};
use tokio::sync::OnceCell;

pub static GLOBAL_APP_STATUS: OnceCell<AppStatus> = OnceCell::const_new();
const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize, Debug)]
pub enum BotMessage {
    Heartbeat,
    Pong,
    Chat { from: String, to: String, content: MsgContent },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MsgContent {
    pub command: Option<String>,
    pub payload: Option<BinMessageEvent>,
    pub message: Option<String>,
    pub api_response: Option<ApiResponse<Vec<String>>>,
}
impl MsgContent {
    pub fn msg_only(message: String) -> Self {
        Self {
            command: None,
            payload: None,
            message: Some(message),
            api_response: None,
        }
    }
}

pub async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    app_status: Arc<AppStatus>,
) -> anyhow::Result<()> {
    tracing::info!("New connection from {}", addr);

    let (r, w) = stream.into_split();
    let (tx, mut rx) = channel::<BotMessage, _, _>(r, w);

    let tx = Arc::new(Mutex::new(tx));
    let last_seen = Arc::new(Mutex::new(Instant::now()));

    app_status.update_bot_connection(tx.clone()).await;

    loop {
        match rx.recv().await {
            Ok(msg) => {
                // pass app_status to the task
                let app_status_clone = Arc::clone(&app_status);
                let tx_clone = Arc::clone(&tx);
                let last_seen_clone = Arc::clone(&last_seen);
                tokio::spawn(async move {
                    *last_seen_clone.lock().await = Instant::now();

                    match msg {
                        BotMessage::Heartbeat => {
                            let mut last_seen_guard = last_seen_clone.lock().await;
                            *last_seen_guard = Instant::now();
                            let mut tx_guard = tx_clone.lock().await;
                            if let Err(e) = tx_guard.send(BotMessage::Pong).await {
                                tracing::error!("Failed to send Pong: {}", e);
                            }
                        }
                        BotMessage::Pong => {
                            // No action needed for Pong
                        }
                        BotMessage::Chat { from, to, content } => {
                            let response = router::bot_message_handler(
                                content.clone(),
                                app_status_clone.clone()
                            ).await;
                            let response_content = MsgContent {
                                command: None,
                                payload: content.payload.clone(),
                                message: content.message.clone(),
                                api_response: Some(response),
                            };
                            let mut tx_guard = tx_clone.lock().await;
                            if let Err(e) = tx_guard.send(BotMessage::Chat {
                                from,
                                to,
                                content: response_content,
                            }).await {
                                tracing::error!("Failed to send Chat response: {}", e);
                            }
                        }
                    }
                });
            }
            Err(e) => {
                tracing::error!("Error receiving message from {}: {}", addr, e);
                break;
            }
        }
    }

    app_status.clear_bot_connection().await;
    tracing::info!("Connection closed: {}", addr);

    Ok(())
}

pub async fn initialize_app_status() -> AppStatus {
    let (tx_filerequest, rx_filerequest) = tokio::sync::mpsc::channel(100);
    let _file_manager_handle = tokio::spawn(async move {
        fs::handler::file_manager(rx_filerequest).await;
    });

    // Read initial configuration
    let config_file_path = PathBuf::from(CONFIG_FILE_PATH);
    let (conf_resp_tx, conf_resp_rx) = oneshot::channel();
    let config_read_request = fs::handler::FileRequest::Read {
        path: config_file_path,
        format: fs::handler::FileFormat::Json,
        responder: conf_resp_tx,
    };
    let _ = tx_filerequest.send(config_read_request).await;
    let initial_config: Config = match conf_resp_rx.await {
        Ok(Ok(data)) => {
            match data {
                fs::handler::FileData::Json(json_data) => {
                    serde_json::from_value(json_data)
                        .map_err(|e| anyhow::Error::new(e)).expect("Failed to parse config JSON")
                },
                _ => {
                    tracing::error!("{}: Expected JSON data", i18n::text("config_read_error"));
                    std::process::exit(1);
                }
            }
        },
        Ok(Err(e)) => {
            tracing::error!("{}: {}", i18n::text("config_read_error"), e);
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!("{}: {}", i18n::text("config_read_timeout"), e);
            std::process::exit(1);
        }
    };

    let app_config = Arc::new(RwLock::new(initial_config.clone()));
    let tx_filerequest = Arc::new(RwLock::new(tx_filerequest));
    let app_status = AppStatus {
        config: app_config,
        file_tx: tx_filerequest,
        botmsg_tx: Arc::new(RwLock::new(None)),
    };

    GLOBAL_APP_STATUS.set(app_status.clone()).expect("Failed to set global app status");

    app_status
}