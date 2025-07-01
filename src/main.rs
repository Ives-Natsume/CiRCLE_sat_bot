mod sat_status;
mod query;
mod task_manager;
mod logger;
mod msg_sys;
mod response;
mod config;
mod satellites;
mod sat_pass_predict;
use sat_status::amsat_parser;
use sat_pass_predict::sat_pass_predict;
use msg_sys::group_chat::message_handler;
use std::sync::Arc;
use tokio::{
    sync::{
        Semaphore,
    },
    time::{
        timeout,
        Duration,
    }
};
use futures::{TryStreamExt};
use eventsource_client::{
    ClientBuilder,
    Client,
};

#[allow(unused_mut)]
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot");
    tracing::info!("Logging system initialized");

    let config = config::load_config("config.json");
    sat_status::amsat_parser::run_amsat_module(&config).await?;
    task_manager::scheduled_tasks::start_scheduled_amsat_module(&config);

    let url = format!("{}/_events", config.bot_config.url);
    let mut client = ClientBuilder::for_url(url.as_str())?
        .header("Accept", "text/event-stream")?
        .build();

    tracing::info!("Connecting to SSE server at {}", url);
    let mut stream = client.stream();
    let semaphore = Arc::new(Semaphore::new(config.backend_config.concurrent_limit as usize));

    while let Some(event) = stream.try_next().await? {
        match event {
            eventsource_client::SSE::Event(evt) => {
                if evt.event_type == "message" {
                    let config = config.clone();
                    let data = evt.data.clone();
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let timeout_duration = Duration::from_secs(config.backend_config.timeout);
                        match timeout(timeout_duration, message_handler(data, &config)).await {
                            Ok(_) => {

                            }
                            Err(e) => {
                                tracing::error!("Timeout or error processing message: {}", e);
                            }
                        }
                    });
                }
            }
            eventsource_client::SSE::Comment(_) => {

            }
            eventsource_client::SSE::Connected(_) => {
                tracing::info!("Connected to SSE server at {}", url);
            }
        }
    }

    Ok(())
}
