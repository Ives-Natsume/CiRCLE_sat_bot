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
    i18n,
    fs,
    config::Config,
    app_status::AppStatus,
    CONFIG_FILE_PATH,
};
use tokio::sync::OnceCell;

static GLOBAL_APP_STATUS: OnceCell<AppStatus> = OnceCell::const_new();
pub const LISTEN_ADDR: &str = "127.0.0.1:3310";
const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize, Debug)]
pub enum BotMessage {
    Heartbeat,
    Pong,
    Chat { from: String, to: String, content: String },
}

pub async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
) -> anyhow::Result<()> {
    tracing::info!("New connection from {}", addr);

    let (r, w) = stream.into_split();
    let (tx, mut rx) = channel::<BotMessage, _, _>(r, w);

    let tx = Arc::new(Mutex::new(tx));
    let last_seen = Arc::new(Mutex::new(Instant::now()));

    let tx_clone = tx.clone();
    let last_seen_clone = last_seen.clone();
    let heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            interval.tick().await;
            let last_seen_guard = last_seen_clone.lock().await;
            if last_seen_guard.elapsed() > CLIENT_TIMEOUT {
                tracing::warn!("Connection timeout with {}", addr);
                break;
            }
            drop(last_seen_guard);

            let mut tx_guard = tx_clone.lock().await;
            if let Err(e) = tx_guard.send(BotMessage::Heartbeat).await {
                tracing::error!("Heartbeat error to {}: {}", addr, e);
                break;
            }
        }
    });

    if let Some(app_status) = GLOBAL_APP_STATUS.get() {
        app_status.update_bot_connection(tx.clone()).await;
    }

    loop {
        match rx.recv().await {
            Ok(msg) => {
                match msg {
                    BotMessage::Heartbeat => {
                        if let Err(e) = tx.lock().await.send(BotMessage::Pong).await {
                            tracing::error!("Error sending Pong to {}: {}", addr, e);
                            break;
                        }
                    }
                    BotMessage::Pong => {
                        *last_seen.lock().await = Instant::now();
                    }
                    BotMessage::Chat { from, to, content } => {
                        tracing::info!("Chat message from {} to {}: {}", from, to, content);
                        // Handle chat message logic here
                    }
                }
            }
            Err(e) => {
                tracing::error!("Error receiving message from {}: {}", addr, e);
                break;
            }
        }
    }

    heartbeat_handle.abort();

    if let Some(app_status) = GLOBAL_APP_STATUS.get() {
        app_status.clear_bot_connection().await;
    }
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
    let app_status = AppStatus {
        config: app_config,
        file_tx: tx_filerequest,
        botmsg_tx: Arc::new(RwLock::new(None)),
    };

    GLOBAL_APP_STATUS.set(app_status.clone()).expect("Failed to set global app status");

    app_status
}