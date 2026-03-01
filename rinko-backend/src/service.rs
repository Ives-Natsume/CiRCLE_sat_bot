use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tonic::{Request, Response, Status, Code};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn, debug, error};

use rinko_common::proto::{
    bot_backend_server::BotBackend,
    UnifiedMessage,
    MessageResponse,
    BotCommand,
    SubscribeRequest,
    HeartbeatRequest,
    HeartbeatResponse,
    ContentType,
};
use rinko_common::Platform;

use crate::module::handler::MessageHandler;
use crate::module::sat_rev::SatManager;
use crate::module::lotw::LotwUpdater;
use crate::module::qo100::Qo100Updater;

/// Frontend connection info
#[derive(Debug, Clone)]
struct FrontendConnection {
    frontend_id: String,
    platforms: Vec<Platform>,
    command_tx: mpsc::Sender<Result<BotCommand, Status>>,
}

/// Backend service implementation
pub struct BotBackendService {
    // Store connected frontends
    frontends: Arc<RwLock<HashMap<String, FrontendConnection>>>,
    // Message handler
    message_handler: Arc<MessageHandler>,
}

impl BotBackendService {
    pub fn new(satellite_manager: Arc<RwLock<SatManager>>, lotw_updater: Arc<LotwUpdater>, qo100_updater: Arc<Qo100Updater>) -> Self {
        let message_handler = Arc::new(MessageHandler::new(satellite_manager, lotw_updater, qo100_updater));
        
        Self {
            frontends: Arc::new(RwLock::new(HashMap::new())),
            message_handler,
        }
    }

    /// Send a command to a specific frontend
    pub async fn send_command_to_frontend(
        &self,
        frontend_id: &str,
        command: BotCommand,
    ) -> Result<(), String> {
        let frontends = self.frontends.read().await;
        
        if let Some(connection) = frontends.get(frontend_id) {
            connection
                .command_tx
                .send(Ok(command))
                .await
                .map_err(|e| format!("Failed to send command: {}", e))?;
            Ok(())
        } else {
            Err(format!("Frontend {} not connected", frontend_id))
        }
    }

    /// Broadcast a command to all connected frontends
    pub async fn broadcast_command(&self, command: BotCommand) {
        let frontends = self.frontends.read().await;
        
        for (frontend_id, connection) in frontends.iter() {
            if let Err(e) = connection.command_tx.send(Ok(command.clone())).await {
                warn!("Failed to send command to frontend {}: {}", frontend_id, e);
            }
        }
    }

    /// Get list of connected frontends
    pub async fn get_connected_frontends(&self) -> Vec<String> {
        let frontends = self.frontends.read().await;
        frontends.keys().cloned().collect()
    }
}

#[tonic::async_trait]
impl BotBackend for BotBackendService {
    /// Handle message reports from frontend
    async fn report_message(
        &self,
        request: Request<UnifiedMessage>,
    ) -> Result<Response<MessageResponse>, Status> {
        let msg: UnifiedMessage = request.into_inner();
        
        info!(
            "Received message from platform {:?}: event_id={}, content_preview={}",
            Platform::from_proto(rinko_common::proto::Platform::try_from(msg.platform).unwrap_or(rinko_common::proto::Platform::Qq)),
            msg.event_id,
            &msg.content.chars().take(50).collect::<String>()
        );

        // Process the message through handler
        let response = match self.message_handler.handle_message(&msg).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to handle message: {}", e);
                MessageResponse {
                    success: false,
                    message: format!("Internal error: {}", e),
                    message_id: uuid::Uuid::now_v7().to_string(),
                    content_type: ContentType::Text as i32,
                }
            }
        };

        Ok(Response::new(response))
    }

    /// Server streaming: Send commands to frontend
    type SubscribeCommandsStream = ReceiverStream<Result<BotCommand, Status>>;

    async fn subscribe_commands(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeCommandsStream>, Status> {
        let subscribe_req = request.into_inner();
        let frontend_id = subscribe_req.frontend_id.clone();
        
        let platforms: Vec<Platform> = subscribe_req
            .platforms
            .into_iter()
            .filter_map(|p| {
                rinko_common::proto::Platform::try_from(p)
                    .ok()
                    .and_then(Platform::from_proto)
            })
            .collect();

        info!(
            "Frontend {} subscribing for platforms: {:?}",
            frontend_id, platforms
        );

        // Create channel for sending commands to this frontend
        let (tx, rx) = mpsc::channel::<Result<BotCommand, Status>>(100);

        // Store the connection
        let connection = FrontendConnection {
            frontend_id: frontend_id.clone(),
            platforms: platforms.clone(),
            command_tx: tx.clone(),
        };

        {
            let mut frontends = self.frontends.write().await;
            frontends.insert(frontend_id.clone(), connection);
        }

        info!("Frontend {} connected and subscribed", frontend_id);

        // Spawn a task to monitor disconnection
        let frontends_clone = self.frontends.clone();
        let frontend_id_clone = frontend_id.clone();
        tokio::spawn(async move {
            // Wait for the sender to be dropped (disconnection)
            tx.closed().await;
            
            // Remove from connected frontends
            let mut frontends = frontends_clone.write().await;
            frontends.remove(&frontend_id_clone);
            warn!("Frontend {} disconnected", frontend_id_clone);
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    /// Handle heartbeat from frontend
    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let heartbeat_req = request.into_inner();
        
        debug!(
            "Heartbeat from frontend {}: {:?}",
            heartbeat_req.frontend_id, heartbeat_req.status
        );

        let response = HeartbeatResponse {
            healthy: true,
            message: "Backend is healthy".to_string(),
        };

        Ok(Response::new(response))
    }

    /// Bidirectional streaming (optional, for future use)
    type BidirectionalChatStream = ReceiverStream<Result<BotCommand, Status>>;

    async fn bidirectional_chat(
        &self,
        _request: Request<tonic::Streaming<UnifiedMessage>>,
    ) -> Result<Response<Self::BidirectionalChatStream>, Status> {
        // Placeholder for bidirectional streaming
        Err(Status::new(Code::Unimplemented, "Not implemented yet"))
    }
}
