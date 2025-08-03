mod logger;
mod i18n;
mod socket;
mod app_status;
mod fs;
mod config;
use tokio::net::TcpStream;
use std::sync::Arc;
use channels::channel;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    i18n::I18N.set_lang("zh");

    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot");
    tracing::info!("{}", i18n::text("log_initialized"));

    let stream = TcpStream::connect("127.0.0.1:3310").await?;
    tracing::info!("Connected to bot core server at {}", stream.peer_addr()?);
    let (r, w) = stream.into_split();
    let (mut tx, mut rx) = channel::<socket::BotMessage, _, _>(r, w);

    let tx = Arc::new(tokio::sync::Mutex::new(tx));

    // let tx_clone = tx.clone();
    // tokio::spawn(async move {
    //     loop {
    //         tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    //         if tx_clone.lock().await.send(socket::BotMessage::Heartbeat).await.is_err() {
    //             tracing::error!("{}", i18n::text("heartbeat_failed"));
    //             break;
    //         }
    //     }
    // });

    let tx_clone2 = tx.clone();
    tokio::spawn(async move {
        tx_clone2.lock().await.send(socket::BotMessage::Chat {
            from: "bot_server_client".to_string(),
            to: "bot_core_server".to_string(),
            content: "Hello from Rinko".to_string(),
        }).await.unwrap();
    });

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            if let Err(e) = tx_clone.lock().await.send(socket::BotMessage::Heartbeat).await {
                tracing::error!("Failed to send Heartbeat: {}", e);
            }
            tracing::info!("Sent Heartbeat");
        }
    });

    // Handle incoming messages
    loop {
        match rx.recv().await {
            Ok(msg) => {
                tracing::info!("Received message: {:?}", msg);
                match msg {
                    socket::BotMessage::Heartbeat => tracing::info!("Received Heartbeat"),
                    socket::BotMessage::Pong => tracing::info!("Received Pong"),
                    socket::BotMessage::Chat { from, to, content } => {
                        tracing::info!("Chat from {} to {}: {}", from, to, content);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Error receiving message: {}", e);
                break;
            }
        }
    }

    Ok(())
}
