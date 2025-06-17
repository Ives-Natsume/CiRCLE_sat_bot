mod sat_status;
mod query;
mod task_manager;
mod logger;
use sat_status::amsat_parser;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let _logger = logger::init_logging("logs", "CiRCLE_sat_bot");
    tracing::info!("Logging system initialized");

    sat_status::amsat_parser::run_amsat_module().await?;

    // let query_sat_name = "hadesr"; // Example satellite name to query
    // let json_file_path = "amsat_status.json";
    // let toml_file_path = "satellites.toml";

    // let query_result = query::sat_query::look_up_sat_status_from_json(
    //     json_file_path,
    //     toml_file_path,
    //     query_sat_name,
    // );

    // match query_result {
    //     Some(status) => {
    //         println!("Satellite status for '{}':", query_sat_name);
    //         for entry in status {
    //             println!("{}", entry);
    //         }
    //     }
    //     None => {
    //         println!("No status found for satellite '{}'.", query_sat_name);
    //     }
    // }

    Ok(())
}