use std::collections::{HashMap, HashSet};
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SatelliteToml(HashMap<String, SatelliteEntry>);

#[derive(Debug, Deserialize)]
struct SatelliteEntry {
    aliases: Vec<String>,
}

#[derive(Debug, Clone)]
struct SatelliteName {
    pub official_name: String,
    pub aliases: Vec<String>,
}

impl SatelliteName {
    #[allow(dead_code)]
    fn matches_query(&self, query: &str) -> bool {
        let normalized_query = sat_name_normalize(query);
        if sat_name_normalize(&self.official_name) == normalized_query {
            return true;
        }
        self.aliases.iter().any(|alias| sat_name_normalize(alias) == normalized_query)
    }
}

fn sat_name_normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric()) // left all alphanumeric characters
        .map(|c| c.to_ascii_uppercase()) // convert to uppercase
        .collect()
}

/// supports official name and aliases
pub fn look_up_sat_status_from_json(
    json_file_path: &str,
    toml_file_path: &str,
    sat_name: &str,
) -> Option<Vec<String>> {
    // read file to string
    let json_data = std::fs::read_to_string(json_file_path).ok()?;
    let json_value: serde_json::Value = serde_json::from_str(&json_data).ok()?;

    let satellites = load_satellites_from_toml(toml_file_path);
    let alias_index = build_alias_index(&satellites);

    let sat_name_list: Vec<String> = search_satellites(sat_name, &alias_index);

    // find all satellites in the json file that match the names in sat_name_list
    let mut found_sats = vec![];
    for sat in json_value.as_array()? {
        if let Some(name) = sat.get("name").and_then(|n| n.as_str()) {
            if sat_name_list.iter().any(|s| sat_name_normalize(s) == sat_name_normalize(name)) {
                found_sats.push(format_satellite_status(sat).join("\n"));
            }
        }
    }
    
    if found_sats.is_empty() {
        None
    } else {
        Some(found_sats)
    }
}

fn load_satellites_from_toml(toml_file_path: &str) -> Vec<SatelliteName> {
    let toml_str = std::fs::read_to_string(toml_file_path)
        .expect("Unable to read TOML file");
    let parsed: HashMap<String, SatelliteEntry> = toml::from_str(&toml_str)
        .expect("Invalid TOML format");

    parsed
        .into_iter()
        .map(|(official_name, entry)| SatelliteName {
            official_name,
            aliases: entry.aliases,
        })
        .collect()
}

fn build_alias_index(satellites: &[SatelliteName]) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();

    for sat in satellites {
        // add official name
        let norm_official = sat_name_normalize(&sat.official_name);
        index.entry(norm_official.clone())
            .or_default()
            .push(sat.official_name.clone());

        for alias in &sat.aliases {
            let norm = sat_name_normalize(alias);
            index.entry(norm)
                .or_default()
                .push(sat.official_name.clone());
        }
    }

    index
}

fn search_satellites(query: &str, alias_index: &HashMap<String, Vec<String>>) -> Vec<String> {
    let norm_query = sat_name_normalize(query);
    alias_index
        .get(&norm_query)
        .map(|names| {
            let mut set = HashSet::new();
            names.iter().cloned().filter(|n| set.insert(n.clone())).collect()
        })
        .unwrap_or_default()
}

/// Modify the function to format status
fn format_satellite_status(
    sat: &serde_json::Value,
) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    
    if let Some(name) = sat.get("name").and_then(|n| n.as_str()) {
        result.push(format!("Satellite Name: {}", name));

        // handle status
        if let Some(status_array) = sat.get("status").and_then(|s| s.as_array()) {
            // Check if there are any valid status reports
            let mut has_valid_status = false;
            let mut timeblock_status = Vec::new();

            for (idx, subdata) in status_array.iter().enumerate() {
                // we only care about the last 18 time blocks
                // that's to say, the last one and half days
                if idx >= 18 {
                    break;
                }
                if let Some(reports) = subdata.as_array() {
                    // use the first report for description and count
                    if let Some(first_report) = reports.first() {
                        let desc = first_report.get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("Unknown Status");
                        let report_count = first_report.get("report_nums")
                            .and_then(|r| r.as_u64())
                            .unwrap_or(0);

                        // 仅当有有效报告时才显示
                        if report_count > 0 {
                            has_valid_status = true;
                            timeblock_status.push(
                                format!(" Block {}: {} ({} reports)", 
                                    idx + 1, desc, report_count)
                            );
                        }
                    }
                }
            }

            if has_valid_status {
                result.push("Status per time block:".to_string());
                result.extend(timeblock_status);
            } else {
                result.push("Status: No reports for last one and half days".to_string());
            }
        } else {
            result.push("Status: Unknown - no status data".to_string());
            tracing::warn!("No status data for satellite '{}'", name);
        }
    }

    result
}