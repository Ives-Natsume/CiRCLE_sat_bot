mod sat_status;
mod query;
mod task_manager;
mod logger;
use sat_status::amsat_parser;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot");
    tracing::info!("Logging system initialized");

    sat_status::amsat_parser::run_amsat_module().await?;

    let json_file_path = "amsat_status.json";
    let toml_file_path = "satellites.toml";
    let (query_client, query_handler) = task_manager::query_handler::init_query_system(
        json_file_path.to_string(),
        toml_file_path.to_string(),
    );

    let _query_handler_task = tokio::spawn(query_handler.run());
    let client = Arc::new(RwLock::new(query_client));

    run_console_listener(Arc::clone(&client)).await?;

    // {
    //     // initializing the query handler
    //     let json_file_path = "amsat_status.json";
    //     let toml_file_path = "satellites.toml";
    //     let (query_client, query_handler) = task_manager::query_handler::init_query_system(
    //         json_file_path.to_string(),
    //         toml_file_path.to_string(),
    //     );

    //     // Start the query handler in a separate task
    //     let _query_handler_task = tokio::spawn(query_handler.run());

    //     // Shared query client for sending requests
    //     let client = Arc::new(RwLock::new(query_client));

    //     // Send a sample query request
    //     {
    //         let client_guard = client
    //             .read()
    //             .await;
    //         let result = client_guard.query(
    //             "ao123".to_string(),
    //         ).await;

    //         // Print the result if possible
    //         match result {
    //             Some(list) => {
    //                 for item in list {
    //                     println!("{}", item);
    //                 }
                    
    //             }
    //             None => println!("No results found for the query."),
    //         }
    //     }
    // }

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
            match guard.query(query_input.clone()).await {
                Some(results) => {
                    let duration = start_time.elapsed();
                    tracing::info!("Query for '{}' completed in {:?}", input, duration);
                    for item in results {
                        println!("[Result] {}: {}", input, item);
                    }
                }
                None => {
                    tracing::warn!("No results found for satellite: {}", input);
                    println!("No results found for satellite: {}", input);
                }
            }
        });
    }

    Ok(())
}