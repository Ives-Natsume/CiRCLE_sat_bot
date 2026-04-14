use rinko_frontend::logging;
use rinko_frontend::config;
use rinko_frontend::config::QQConfig;
use rinko_frontend::backend::BackendConnectionManager;
use rinko_frontend::frontend::telegram;
use rinko_frontend::utils::Platform;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::read_config()?;
    let _logging_guard = logging::init_logging("logs", "rinko-frontend", &config::CONFIG.get().unwrap().log_level);

    tracing::info!("Rinko Frontend started.");

    let bot_config = config::CONFIG.get().unwrap();

    // Initialize backend connection manager if enabled
    let backend_manager = if let Some(backend_cfg) = &bot_config.backend {
        if backend_cfg.enable {
            tracing::info!("Backend enabled, initializing connection manager...");
            
            let manager = Arc::new(BackendConnectionManager::new(backend_cfg.clone()));
            
            // Try initial connection (non-blocking)
            manager.initialize().await;
            
            // Start auto-reconnect task
            manager.clone().start_reconnect_task();
            tracing::info!("✓ Backend auto-reconnect task started (retry interval: 10s)");
            
            // Start heartbeat task
            manager.clone().start_heartbeat_task();
            tracing::info!("✓ Backend heartbeat task started (interval: {}s)", backend_cfg.heartbeat_interval);
            
            // Start command subscription task
            let mut platforms = vec![Platform::QQ];
            if bot_config.telegram.is_some() {
                platforms.push(Platform::Telegram);
            }
            manager.clone().start_command_subscription_task(platforms);
            tracing::info!("✓ Backend command subscription task started");
            
            Some(manager)
        } else {
            tracing::info!("Backend disabled in configuration");
            None
        }
    } else {
        tracing::info!("No backend configuration found");
        None
    };

    // Initialize QQ bot
    if let Some(mut qq_cfg) = bot_config.qq.clone() {
        // C-4 fix: propagate initialization errors instead of silently logging and
        // exiting, which would kill all background tasks without any clear failure signal.
        qq_cfg.init().await
            .map_err(|e| anyhow::anyhow!("Failed to initialize QQ bot: {}", e))?;

        tracing::info!("QQ bot initialized successfully.");

        // Wrap in Arc<RwLock> for thread-safe access and start auto-renewal
        let qq_cfg_shared = Arc::new(RwLock::new(qq_cfg));
        QQConfig::start_token_renewal_task(qq_cfg_shared.clone());
        tracing::info!("QQ token auto-renewal task started.");

        // Start webhook server
        let qq_cfg_for_webhook = qq_cfg_shared.clone();
        let backend_for_webhook = backend_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = QQConfig::start_webhook_server(
                qq_cfg_for_webhook,
                backend_for_webhook,
                3110
            ).await {
                tracing::error!("Webhook server error: {}", e);
            }
        });
        tracing::info!("QQ webhook server starting on port 3110...");
    } else {
        tracing::warn!("No QQ configuration found; running with backend tasks only.");
    }

    // Initialize Telegram bot
    if let Some(tg_cfg) = bot_config.telegram.clone() {
        let backend_for_tg = backend_manager.clone();
        tokio::spawn(async move {
            tracing::info!("Starting Telegram bot (long-polling)...");
            if let Err(e) = telegram::start_telegram_bot(tg_cfg, backend_for_tg).await {
                tracing::error!("Telegram bot error: {}", e);
            }
        });
        tracing::info!("Telegram bot task spawned.");
    } else {
        tracing::info!("No Telegram configuration found.");
    }

    // C-4 fix: always wait for the shutdown signal so background tasks (reconnect,
    // heartbeat, command subscription) remain alive regardless of which sub-systems
    // were successfully initialized above.
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutdown signal received.");

    Ok(())
}