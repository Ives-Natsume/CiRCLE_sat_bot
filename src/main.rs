mod sat_status;
mod query;
mod task_manager;
mod logger;
mod subscriber;
mod msg_sys;
mod response;
use sat_status::amsat_parser;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot");
    tracing::info!("Logging system initialized");

    sat_status::amsat_parser::run_amsat_module().await?;

    // start the scheduled task for AMSAT module updates
    task_manager::scheduled_tasks::start_scheduled_amsat_module();

    let json_file_path = "amsat_status.json";
    let toml_file_path = "satellites.toml";
    let (query_client, query_handler) = task_manager::query_handler::init_query_system(
        json_file_path.to_string(),
        toml_file_path.to_string(),
    );

    let _query_handler_task = tokio::spawn(query_handler.run());
    let client = Arc::new(RwLock::new(query_client));

    run_console_listener(Arc::clone(&client)).await?;

    Ok(())
}

use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};
async fn run_console_listener(client: Arc<RwLock<task_manager::query_handler::QueryClient>>) -> anyhow::Result<()> {
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
        let query_input = input.to_string(); // Clone the input to move into the async block
        tokio::spawn(async move {
            tracing::debug!("Querying satellite: {}", query_input);
            let start_time = tokio::time::Instant::now();
            
            // Acquire a read lock to access the client
            // This ensures that the client is not modified while we are querying
            // and allows multiple concurrent queries.
            let guard = client_clone.read().await;
            // match guard.query(query_input.clone()).await {
            //     Some(results) => {
            //         let duration = start_time.elapsed();
            //         tracing::info!("Query for '{}' completed in {:?}", input, duration);
            //         for item in results {
            //             println!("[Result] {}: {}", input, item);
            //         }
            //     }
            //     None => {
            //         tracing::warn!("No results found for satellite: {}", input);
            //         println!("No results found for satellite: {}", input);
            //     }
            // }
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