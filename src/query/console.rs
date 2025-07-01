/// Used for console debugging and testing.

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