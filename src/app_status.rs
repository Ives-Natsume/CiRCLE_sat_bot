use crate::fs::handler;
use crate::config::{Config, Doc};
use crate::msg::group_msg::send_group_msg;
use crate::response::ApiResponse;
use crate::socket::BotMessage;
use channels::serdes::bincode;
use tokio::{
    sync::{
        mpsc,
        RwLock,
    },
    net::tcp::OwnedWriteHalf,
};
use std::sync::Arc;

pub type BotMessageSender = Arc<tokio::sync::Mutex<channels::Sender<BotMessage, channels::io::Tokio<OwnedWriteHalf>, bincode::Bincode>>>;

#[derive(Clone, Debug)]
pub struct AppStatus {
    pub config: Arc<RwLock<Config>>,
    pub file_tx: mpsc::Sender<handler::FileRequest>,
    pub botmsg_tx: Arc<RwLock<Option<BotMessageSender>>>,
}

impl AppStatus {
    pub async fn send_bot_message(&self, message: BotMessage) -> Result<(), Box<dyn std::error::Error>> {
        let sender_guard = self.botmsg_tx.read().await;
        if let Some(sender) = sender_guard.as_ref() {
            sender.lock().await.send(message).await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        } else {
            tracing::warn!("No bot connection available to send message");
            Err("Bot external server offline".into())
        }
    }

    pub async fn update_bot_connection(&self, sender: BotMessageSender) {
        let mut sender_guard = self.botmsg_tx.write().await;
        *sender_guard = Some(sender);
        tracing::info!("Bot connection updated in AppStatus");
    }

    pub async fn clear_bot_connection(&self) {
        let mut sender_guard = self.botmsg_tx.write().await;
        *sender_guard = None;
        tracing::info!("Bot connection cleared from AppStatus");
    }
}
