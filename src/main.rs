mod sat_status;
mod query;
mod task_manager;
mod logger;
mod msg_sys;
mod response;
mod config;
use sat_status::amsat_parser;
use std::sync::Arc;
use tokio::{
    sync::{
        RwLock,
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
    let semaphore = Arc::new(Semaphore::new(10));

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

use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};

use crate::msg_sys::group_chat::message_handler;
async fn _run_console_listener(client: Arc<RwLock<task_manager::query_handler::QueryClient>>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = TokioBufReader::new(stdin).lines();

    while let Some(line_result) = reader.next_line().await? {
        let input = line_result.to_string(); // Clone the input to move into the async block
        if input.is_empty() {
            continue;
        }

        if input.eq_ignore_ascii_case("exit") {
            tracing::info!("Exiting console listener");
            break;
        }

        // Spawn a new task for each query
        let client_clone = Arc::clone(&client);
        let query_input = input.to_string();
        tokio::spawn(async move {
            tracing::debug!("Querying satellite: {}", query_input);
            let start_time = tokio::time::Instant::now();
            
            // Acquire a read lock to access the client
            // This ensures that the client is not modified while we are querying
            // and allows multiple concurrent queries.
            let guard = client_clone.read().await;
            match guard.query(query_input.clone()).await {
                response::ApiResponse { success: true, data: Some(results), message: None } => {
                    let duration = start_time.elapsed();
                    tracing::info!("Query for '{}' completed in {:?}", query_input, duration);
                    for item in results {
                        println!("[Result] {}: {}", query_input, item);
                    }
                }
                response::ApiResponse { success: false, data: None, message: Some(msg) } => {
                    tracing::warn!("Error querying satellite '{}': {}", query_input, msg);
                }
                _ => {
                    tracing::warn!("Unexpected response format for satellite: {}", query_input);
                }
            }
        });
    }

    Ok(())
}