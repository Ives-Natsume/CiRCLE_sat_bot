extern crate lazy_static;
mod app_status;
mod i18n;
mod config;
mod logger;
mod response;
mod msg;
mod socket;
mod fs;
mod module;
use std::{
    sync::Arc,
};
use channels::channel;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use crate::fs::handler::{FileRequest, FileFormat, FileData};
use crate::socket::{BotMessage, MsgContent};
use crate::module::handler::router;
use crate::response::ApiResponse;

pub const CONFIG_FILE_PATH: &str = "config.json";
pub const DOC_FILE_PATH: &str = "locales/doc.json";
pub const COMMAND_TOML_PATH: &str = "commands.toml";

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    i18n::I18N.set_lang("zh");
    
    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot_server");
    tracing::info!("{}", i18n::text("log_initialized"));

    // connect to bot core at 3310 port
    let stream = TcpStream::connect("127.0.0.1:3310").await?;
    let (r, w) = stream.into_split();
    // let (mut tx, mut rx) = channel::<BotMessage, _, _>(r, w);
    // let tx = Arc::new(RwLock::new(tx));
    let (mut tx_sink, mut rx_stream) = channel::<BotMessage, _, _>(r, w);
    let (tx_filerequest, rx_filerequest) = tokio::sync::mpsc::channel::<FileRequest>(100);

    let tx_shared = Arc::new(RwLock::new(tx_sink));
    let tx_filerequest_shared = Arc::new(RwLock::new(tx_filerequest));
    loop {
        match rx_stream.recv().await { // Or rx_stream.next().await if using futures::StreamExt
            Ok(msg) => { // Use Some(msg) if Framed::next() returns Option<Result<T, E>>
                tracing::debug!("Received message: {:?}", msg);
                let tx_clone_for_task = Arc::clone(&tx_shared); // Clone Arc for each task
                let tx_filerequest_clone = Arc::clone(&tx_filerequest_shared);

                tokio::spawn(async move {
                    match msg {
                        socket::BotMessage::Heartbeat => {
                            // Check if send is successful, handle error if not.
                            let mut tx = tx_clone_for_task.write().await;
                            if let Err(e) = tx.send(socket::BotMessage::Pong).await {
                                tracing::error!("Failed to send Pong: {}", e);
                            }
                        }
                        socket::BotMessage::Pong => {
                            // No action needed for Pong
                        }
                        socket::BotMessage::Chat { content, .. } => {
                            let response = router::bot_message_handler(
                                content.clone(),
                                tx_filerequest_clone.clone()
                            ).await;
                            let mut tx = tx_clone_for_task.write().await;
                            let response_content = MsgContent {
                                command: None,
                                payload: content.payload.clone(),
                                message: None,
                                api_response: Some(response),
                            };
                            if let Err(e) = tx.send(socket::BotMessage::Chat {
                                from: "CiRCLE_sat_bot_server".to_string(),
                                to: "CiRCLE_sat_bot_core".to_string(),
                                content: response_content,
                            }).await {
                                tracing::error!("Failed to send Chat response: {}", e);
                            }
                        }
                    }
                });
            }
            Err(e) => {
                tracing::error!("Error receiving message from stream: {}", e);
                break;
            }
        }
    }

    // loop {
    //     match rx.recv().await {
    //         Ok(msg) => {
    //             tracing::debug!("Received message: {:?}", msg);
    //             match msg {
    //                 socket::BotMessage::Heartbeat => {
    //                     let tx_clone = Arc::clone(&tx);
    //                     tx_clone.write().await.send(socket::BotMessage::Pong).await.unwrap();
    //                 }
    //                 socket::BotMessage::Pong => {}
    //                 socket::BotMessage::Chat { content, .. } => {
    //                     let response = router::bot_message_handler(content.clone()).await;
    //                     // send response back to bot core
    //                     let tx_clone = Arc::clone(&tx);
    //                     let content = MsgContent {
    //                         command: None,
    //                         payload: content.payload.clone(),
    //                         message: None,
    //                         api_response: Some(response),
    //                     };
    //                     tx_clone.write().await.send(socket::BotMessage::Chat {
    //                         from: "CiRCLE_sat_bot_server".to_string(),
    //                         to: "CiRCLE_sat_bot_core".to_string(),
    //                         content,
    //                     }).await.unwrap();
    //                 }
    //             }
    //         }
    //         Err(e) => {
    //             tracing::error!("Error receiving message: {}", e);
    //             break;
    //         }
    //     }
    // }

    Ok(())
}