mod sat_status;
mod query;
mod task_manager;
mod logger;
mod msg_sys;
mod response;
use sat_status::amsat_parser;
use std::sync::Arc;
use tokio::sync::RwLock;
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

    sat_status::amsat_parser::run_amsat_module().await?;

    // start the scheduled task for AMSAT module updates
    task_manager::scheduled_tasks::start_scheduled_amsat_module();

    // let addr = "0.0.0.0:3301";
    // let app = Router::new()
    //     .route("/", post(msg_sys::group_chat::send_group_msg_from_request));
    // let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    // tracing::info!("Server listening on {}", addr);
    // axum::serve(listener, app.into_make_service()).await.unwrap();

    let url = "http://127.0.0.1:3300/_events";
    let mut client = ClientBuilder::for_url(url)?
        .header("Accept", "text/event-stream")?
        .build();

    tracing::info!("Connecting to SSE server at {}", url);
    let mut stream = client.stream();

    while let Some(event) = stream.try_next().await? {
        match event {
            eventsource_client::SSE::Event(evt) => {
                if evt.event_type == "message" {
                    send_group_msg_from_request(evt.data.clone()).await;
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

use crate::msg_sys::group_chat::send_group_msg_from_request;
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