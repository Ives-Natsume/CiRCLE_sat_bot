use rinko_backend::config;
use rinko_backend::service;
use rinko_backend::module::news;
use rinko_backend::module::sat_rev::SatManager;
use rinko_backend::module::scheduled::ScheduledTaskManager;

use anyhow::Result;
use tonic::transport::Server;
use std::sync::Arc;
use tokio::sync::RwLock;
use rinko_common::proto::bot_backend_server::BotBackendServer;
use service::BotBackendService;

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    config::read_config()?;
    let config = config::CONFIG.get().unwrap();

    // Initialize logging
    let _logging_guard = rinko_backend::logging::init_logging(
        "logs",
        "rinko-backend",
        &config.log_level,
    );

    // Initialise the global news module (must happen before any module calls news::report).
    let news_manager = news::NewsManager::init();
    tokio::spawn(news_manager.run());

    tracing::info!("Rinko Backend starting...");
    tracing::info!("Server will listen on {}", config.server_address());

    // Initialize satellite manager V2
    tracing::info!("Initializing satellite manager (V2)...");
    let satellite_manager = Arc::new(RwLock::new(SatManager::init().await));

    // Configure and start scheduled tasks
    let mut task_manager = ScheduledTaskManager::new(satellite_manager.clone());
    task_manager.start_all().await?;
    tracing::info!("All scheduled tasks started successfully");

    // Create gRPC service with satellite manager and LoTW updater
    let lotw_updater = task_manager.lotw_updater();
    let qo100_updater = task_manager.qo100_updater();
    let bot_service = BotBackendService::new(satellite_manager, lotw_updater, qo100_updater);
    let server_addr = config.server_address().parse()?;

    tracing::info!("gRPC server starting on {}", server_addr);

    // B-3: use tokio::select! so Ctrl-C triggers graceful shutdown even if the
    // gRPC server is still listening.  This gives scheduled tasks a chance to
    // finish in-flight work (e.g. flush amsat_cache.json) before exit.
    tokio::select! {
        result = Server::builder()
            .add_service(BotBackendServer::new(bot_service))
            .serve(server_addr)
        => {
            if let Err(e) = result {
                tracing::error!("gRPC server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutdown signal received");
        }
    }

    // Abort all background tasks cleanly.
    task_manager.shutdown().await;
    tracing::info!("Rinko Backend stopped.");

    Ok(())
}
