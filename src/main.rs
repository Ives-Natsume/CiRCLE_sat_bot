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
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use channels::channel;
use crate::socket::BotMessage;
use crate::module::handler::router;
use crate::socket::MsgContent;
use crate::module::amsat::official_report::amsat_data_handler;
use crate::module::solar_image::get_image;
use crate::module::scheduled::scheduled_task_handler;

pub const CONFIG_FILE_PATH: &str = "../CiRCLE_sat_bot_core/config.json";
pub const DOC_FILE_PATH: &str = "locales/doc.json";
pub const COMMAND_TOML_PATH: &str = "commands.toml";

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    i18n::I18N.set_lang("zh");
    
    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot_server");
    tracing::info!("{}", i18n::text("log_initialized"));

    let app_status = socket::initialize_app_status().await;
    let listen_addr = app_status.config.read().await.bot_config.listen_addr.clone();
    let stream = TcpStream::connect(listen_addr).await?;
    let (r, w) = stream.into_split();
    let (mut tx_sink, mut rx_stream) = channel::<BotMessage, _, _>(r, w);
    let app_status = Arc::new(app_status);
    let tx_botmsg = Arc::new(RwLock::new(tx_sink));
    
    tokio::spawn({
        let app_status_clone = Arc::clone(&app_status);
        async move {
            amsat_data_handler(&app_status_clone).await;
            match get_image::get_solar_image(&app_status_clone).await {
                Ok(_) => {
                    tracing::info!("Solar image updated successfully");
                }
                Err(e) => {
                    tracing::error!("Failed to update solar image: {}", e);
                }
            }
        }
    });

    let app_status_clone = Arc::clone(&app_status);
    tokio::spawn(async move {
        scheduled_task_handler(&app_status_clone).await;
    });

    loop {
        match rx_stream.recv().await { // Or rx_stream.next().await if using futures::StreamExt
            Ok(msg) => { // Use Some(msg) if Framed::next() returns Option<Result<T, E>>
                //tracing::debug!("Received message: {:?}", msg);
                let app_status_clone = Arc::clone(&app_status);
                let tx_botmsg = Arc::clone(&tx_botmsg);

                tokio::spawn(async move {
                    match msg {
                        socket::BotMessage::Heartbeat => {
                            // Check if send is successful, handle error if not.
                            let mut tx = tx_botmsg.write().await;
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
                                app_status_clone.clone()
                            ).await;
                            let mut tx = tx_botmsg.write().await;
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

    Ok(())
}