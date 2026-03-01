///! Handles requests related to the model.
use rinko_common::proto::{UnifiedMessage, MessageResponse, ContentType};
use regex::Regex;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::CONFIG;
use super::CACHE_PATH;
use super::sat_rev::SatManager;
use super::lotw::LotwUpdater;
use super::qo100::Qo100Updater;
use super::renderer::SatelliteRenderer;

/// Message handler with satellite manager
pub struct MessageHandler {
    satellite_manager: Arc<RwLock<SatManager>>,
    lotw_updater: Arc<LotwUpdater>,
    qo100_updater: Arc<Qo100Updater>,
}

impl MessageHandler {
    /// Create a new message handler
    pub fn new(satellite_manager: Arc<RwLock<SatManager>>, lotw_updater: Arc<LotwUpdater>, qo100_updater: Arc<Qo100Updater>) -> Self {
        Self { satellite_manager, lotw_updater, qo100_updater }
    }
    
    /// Handle incoming message
    pub async fn handle_message(
        &self,
        msg: &UnifiedMessage,
    ) -> Result<MessageResponse> {
        let content = &msg.content;
        let message_id = uuid::Uuid::now_v7().to_string();

        match parse_command(&content) {
            Some((command, args)) => {
                if command.is_empty() {
                    // No command found
                    Ok(MessageResponse {
                        success: false,
                        message: "No command found, please provide a valid command prefix.".to_string(),
                        message_id,
                        content_type: rinko_common::proto::ContentType::Text as i32,
                    })
                } else {
                    // Route to appropriate handler
                    self.router(msg, &command, &args).await
                }
            },
            None => {
                // Invalid command format, return error response
                Ok(MessageResponse {
                    success: false,
                    message: "Command Parse failed, please check your input.".to_string(),
                    message_id,
                    content_type: rinko_common::proto::ContentType::Text as i32,
                })
            }
        }
    }
    
    /// Route commands to appropriate handlers
    async fn router(
        &self,
        _msg: &UnifiedMessage,
        command: &str,
        args: &str,
    ) -> Result<MessageResponse> {
        match command {
            "q" | "query" => self.amsat_query(args).await,
            "dxw" => {
                let pic_path = format!("{}/image_cache/dxw_latest.png", CACHE_PATH);
                if std::path::Path::new(&pic_path).exists() && is_media_server_running().await {
                    Ok(MessageResponse {
                        success: true,
                        message: format!("file:///{}", pic_path.replace("\\", "/")),
                        message_id: uuid::Uuid::now_v7().to_string(),
                        content_type: rinko_common::proto::ContentType::Image as i32,
                    })
                } else {
                    Ok(MessageResponse {
                        success: false,
                        message: "DXW image not found or media server down.".to_string(),
                        message_id: uuid::Uuid::now_v7().to_string(),
                        content_type: rinko_common::proto::ContentType::Text as i32,
                    })
                }
            }
            "lotw" => self.lotw_query().await,
            "qo100" | "qo-100" => self.qo100_query().await,
            _ => {
                Ok(MessageResponse {
                    success: false,
                    message: format!("Unknown command: /{}", command),
                    message_id: uuid::Uuid::now_v7().to_string(),
                    content_type: rinko_common::proto::ContentType::Text as i32,
                })
            }
        }
    }
    
    /// Query satellite information
    async fn amsat_query(&self, query: &str) -> Result<MessageResponse> {
        if !is_media_server_running().await {
            return Ok(MessageResponse {
                success: false,
                message: "Media server down, please contact rinko@rinkosoft.me".to_string(),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Text as i32,
            });
        }

        let query = query.trim();
        
        if query.is_empty() {
            return Ok(MessageResponse {
                success: false,
                message: "Please provide a satellite name to query. Example: /q ISS".to_string(),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Text as i32,
            });
        }
        
        // Search for AMSAT entries matching the query
        let mgr = self.satellite_manager.read().await;
        let search_results = mgr.search(query);
        
        if search_results.is_empty() {
            return Ok(MessageResponse {
                success: false,
                message: format!("^ ^)/"),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Text as i32,
            });
        }
        
        // Try to render as image using renderer
        let cache_dir = CACHE_PATH;
        let images_dir = format!("{}/image_cache", cache_dir);

        // Make sure image cache directory exists
        tokio::fs::create_dir_all(&images_dir).await
            .expect("Failed to create image cache directory");

        let renderer = SatelliteRenderer::new(&images_dir);
        
        match renderer.render_amsat_results(search_results).await {
            Ok(image_path) => {
                // Return image path
                let path_str = image_path.to_string_lossy().to_string();
                Ok(MessageResponse {
                    success: true,
                    message: format!("file:///{}", path_str.replace("\\", "/")),
                    message_id: uuid::Uuid::now_v7().to_string(),
                    content_type: ContentType::Image as i32,
                })
            }
            Err(e) => {
                // Fallback to text format if rendering fails
                tracing::warn!("Image rendering failed, falling back to text: {}", e);
                
                Ok(MessageResponse {
                    success: true,
                    message: "Rinko internal error >_".to_string(),
                    message_id: uuid::Uuid::now_v7().to_string(),
                    content_type: ContentType::Text as i32,
                })
            }
        }
    }
    /// Query LoTW queue status image
    async fn lotw_query(&self) -> Result<MessageResponse> {
        let lotw_img_path = "data/image_cache/lotw_latest.png";
        if std::path::Path::new(lotw_img_path).exists() && is_media_server_running().await {
            Ok(MessageResponse {
                success: true,
                message: format!("file:///{}", lotw_img_path.replace("\\", "/")),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Image as i32,
            })
        } else {
            Ok(MessageResponse {
                success: false,
                message: "LoTW image not found or media server down.".to_string(),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Text as i32,
            })
        }
    }

    /// Query QO-100 DX Cluster image
    async fn qo100_query(&self) -> Result<MessageResponse> {
        let qo100_img_path = "data/image_cache/qo100_latest.png";
        if std::path::Path::new(qo100_img_path).exists() && is_media_server_running().await {
            Ok(MessageResponse {
                success: true,
                message: format!("file:///{}", qo100_img_path.replace("\\", "/")),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Image as i32,
            })
        } else {
            Ok(MessageResponse {
                success: false,
                message: "QO-100 image not found or media server down.".to_string(),
                message_id: uuid::Uuid::now_v7().to_string(),
                content_type: ContentType::Text as i32,
            })
        }
    }
}

/// Parse command from message content
fn parse_command(content: &str) -> Option<(String, String)> {
    let re = Regex::new(r"^\s*/(\S+)\s*(.*)$").unwrap();
    if let Some(caps) = re.captures(content) {
        let command = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let args = caps.get(2).map_or("", |m| m.as_str()).to_string();
        Some((command, args))
    } else {
        None
    }
}

/// Check if the media server is running
/// by accessing config.
async fn is_media_server_running() -> bool {
    let config = CONFIG.get();
    if let Some(cfg) = config {
        if let Some(media_server_url) = &cfg.media_server_url {
            let url = format!("https://{}/health", media_server_url);
            tracing::info!("Checking media server health at {}", url);
            // Try to send a request to the media server's health endpoint
            match reqwest::get(&url).await {
                Ok(resp) => resp.status().is_success(),
                Err(_) => false,
            }
        }
        else {
            // assume media server is not running
            false
        }
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_command() {
        assert_eq!(
            parse_command("/q AO-91"),
            Some(("q".to_string(), "AO-91".to_string()))
        );
        
        assert_eq!(
            parse_command("/query ISS"),
            Some(("query".to_string(), "ISS".to_string()))
        );
        
        assert_eq!(
            parse_command("  /sat   FO-29  "),
            Some(("sat".to_string(), "FO-29  ".to_string()))
        );
        
        assert_eq!(parse_command("no command here"), None);
    }
}
