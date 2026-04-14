use crate::backend::connection_manager::{BackendConnectionManager, ConnectionState};
use crate::config::TelegramConfig;
use crate::utils::*;
use rinko_common::proto::ContentType;
use uuid::Uuid;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;

/// Telegram bot commands registered with BotFather-style `/` prefix.
///
/// Add new variants here to expose more slash commands.
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Rinko bot supports these commands:")]
enum Command {
    #[command(description = "display this help text.")]
    Help,
    #[command(description = "query satellite / transponder information.")]
    Q(String),
    #[command(description = "show LOTW process queue status.")]
    Lotw,
    #[command(description = "DX World roaming calendar.")]
    DxWorld,
}

/// Boot the Telegram adapter.
///
/// Uses long-polling (`teloxide::repl`-style dispatcher) so no public endpoint
/// is required.  All received messages are forwarded to the backend via gRPC
/// just like the QQ adapter does.
pub async fn start_telegram_bot(
    tg_config: TelegramConfig,
    backend_manager: Option<Arc<BackendConnectionManager>>,
) -> anyhow::Result<()> {
    let bot = Bot::new(&tg_config.token);
    let media_base_url: Option<String> = tg_config.media_base_url.clone();

    // Register commands with Telegram so the user sees the `/` menu
    if let Err(e) = bot.set_my_commands(Command::bot_commands()).await {
        tracing::warn!("Failed to register Telegram commands: {}", e);
    }

    let backend = backend_manager.clone();

    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(dptree::endpoint(handle_plain_message));

    Dispatcher::builder(bot, handler)
        .default_handler(|upd| async move {
            tracing::debug!("Unhandled Telegram update: {:?}", upd.id);
        })
        .dependencies(dptree::deps![backend, media_base_url])
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// Handle `/` commands parsed by teloxide.
async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    backend: Option<Arc<BackendConnectionManager>>,
    media_base_url: Option<String>,
) -> ResponseResult<()> {
    // Build the raw text that the backend expects (e.g. "/q ISS")
    let raw_text = match &cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string()).await?;
            return Ok(());
        }
        Command::Q(args) => format!("/q {}", args),
        Command::Lotw => "/lotw".to_string(),
        Command::DxWorld => "/dxw".to_string(),
    };

    dispatch_to_backend(&bot, &msg, &raw_text, backend, &media_base_url).await
}

/// Handle plain (non-command) text messages.
async fn handle_plain_message(
    bot: Bot,
    msg: Message,
    backend: Option<Arc<BackendConnectionManager>>,
    media_base_url: Option<String>,
) -> ResponseResult<()> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => {
            // Ignore non-text messages (stickers, photos, etc.)
            return Ok(());
        }
    };

    dispatch_to_backend(&bot, &msg, &text, backend, &media_base_url).await
}

/// Core logic shared by command and plain-message handlers.
///
/// 1. Forward the message to the backend via gRPC.
/// 2. If the backend returns a non-empty response, send it back to the user.
/// 3. If the backend is offline, reply with a fallback notice.
async fn dispatch_to_backend(
    bot: &Bot,
    msg: &Message,
    content: &str,
    backend: Option<Arc<BackendConnectionManager>>,
    media_base_url: &Option<String>,
) -> ResponseResult<()> {
    if let Some(manager) = &backend {
        let state = manager.state().await;

        if state == ConnectionState::Connected {
            let unified_msg = UnifiedMessage {
                event_id: Uuid::now_v7(),
                content: content.to_string(),
                platform: Platform::Telegram,
            };

            let mut metadata = HashMap::new();
            metadata.insert("chat_id".to_string(), msg.chat.id.0.to_string());
            metadata.insert(
                "message_id".to_string(),
                msg.id.0.to_string(),
            );
            if let Some(user) = &msg.from {
                metadata.insert("user_id".to_string(), user.id.0.to_string());
                if let Some(ref username) = user.username {
                    metadata.insert("username".to_string(), username.clone());
                }
            }

            let client_lock = manager.client();
            if let Some(client) = &mut *client_lock.write().await {
                match client.report_message(unified_msg, metadata).await {
                    Ok(response) => {
                        tracing::debug!("Message reported to backend via Telegram adapter");

                        if !response.message.is_empty() && response.message != "OK" {
                            if let Err(e) =
                                send_backend_response(bot, msg.chat.id, &response, media_base_url).await
                            {
                                tracing::error!(
                                    "Failed to send backend response to Telegram: {}",
                                    e
                                );
                                bot.send_message(
                                    msg.chat.id,
                                    format!(
                                        "Failed to deliver response.\nYour message: {}",
                                        content
                                    ),
                                )
                                .await?;
                            }
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to report message to backend: {}. Marking disconnected.",
                            e
                        );
                        manager.mark_disconnected().await;
                    }
                }
            }
        } else {
            tracing::debug!("Backend offline (Telegram adapter), using fallback");
        }
    }

    // Fallback when backend is unavailable
    bot.send_message(
        msg.chat.id,
        format!("Rinko backend offline.\nMessage received: {}", content),
    )
    .await?;

    Ok(())
}

/// Translate a backend `MessageResponse` into a Telegram reply.
async fn send_backend_response(
    bot: &Bot,
    chat_id: ChatId,
    resp: &rinko_common::proto::MessageResponse,
    media_base_url: &Option<String>,
) -> anyhow::Result<()> {
    match resp.content_type {
        ct if ct == ContentType::Text as i32 => {
            bot.send_message(chat_id, &resp.message).await?;
        }
        ct if ct == ContentType::Image as i32 => {
            // The backend may return a file:/// path or an http(s) URL.
            let url_or_path = resp
                .message
                .strip_prefix("file:///")
                .unwrap_or(&resp.message);

            if url_or_path.starts_with("http://") || url_or_path.starts_with("https://") {
                // Already a public URL — send directly.
                bot.send_photo(chat_id, teloxide::types::InputFile::url(url_or_path.parse()?))
                    .await?;
            } else if let Some(base_url) = media_base_url {
                // Construct public URL via media server, same strategy as QQ adapter.
                let filename = std::path::Path::new(url_or_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(url_or_path);
                let image_url = format!("{}/{}", base_url.trim_end_matches('/'), filename);
                tracing::info!("Sending image to Telegram via media URL: {}", image_url);
                bot.send_photo(chat_id, teloxide::types::InputFile::url(image_url.parse()?))
                    .await?;
            } else {
                // No media_base_url configured — try local file upload as last resort.
                let path = std::path::PathBuf::from(url_or_path);
                if path.exists() {
                    bot.send_photo(chat_id, teloxide::types::InputFile::file(&path))
                        .await?;
                } else {
                    bot.send_message(
                        chat_id,
                        format!("[Image unavailable: {}]", url_or_path),
                    )
                    .await?;
                }
            }
        }
        _ => {
            tracing::warn!("Unsupported content type from backend: {}", resp.content_type);
            bot.send_message(chat_id, "[Unsupported content type]")
                .await?;
        }
    }
    Ok(())
}
