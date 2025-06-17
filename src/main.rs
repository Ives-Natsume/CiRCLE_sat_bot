mod sat_status;
mod query;
use sat_status::amsat_parser;

fn main() {
    amsat_parser::run_amsat_module();

    let query_sat_name = "hadesr"; // Example satellite name to query
    let json_file_path = "amsat_status.json";
    let toml_file_path = "satellites.toml";

    let query_result = query::sat_query::look_up_sat_status_from_json(
        json_file_path,
        toml_file_path,
        query_sat_name,
    );

    match query_result {
        Some(status) => {
            println!("Satellite status for '{}':", query_sat_name);
            for entry in status {
                println!("{}", entry);
            }
        }
        None => {
            println!("No status found for satellite '{}'.", query_sat_name);
        }
    }
}