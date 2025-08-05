use serde::{Deserialize, Serialize};
use strsim::jaro_winkler;

pub const _USER_REPORT_DATA: &str = "data/user_report_data.json";
/// stores the official report data
pub const OFFICIAL_REPORT_DATA: &str = "data/official_report_data.json";
/// cache for querying, a copy of the official report data
pub const OFFICIAL_STATUS_CACHE: &str = "data/official_status_cache.json";
pub const SATELLITES_TOML: &str = "data/satellites.toml";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SatStatus {
    pub name: String,
    pub reported_time: String,
    pub callsign: String,
    pub report: String,
    pub grid_square: String,
}

impl Default for SatStatus {
    fn default() -> Self {
        SatStatus {
            name: String::new(),
            reported_time: String::new(),
            callsign: String::new(),
            report: ReportStatus::Grey.to_string(),
            grid_square: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteFileElement {
    pub time: String,               // time block, e.g., "2025-08-03T13:30:00Z"
    pub report: Vec<SatStatus>,     // list of reports for this time block
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteFileFormat {
    pub name: String,
    pub data: Vec<SatelliteFileElement>,
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Serialize, Deserialize)]
pub enum ReportStatus {
    Blue,       // Transponder/Repeater active
    Yellow,     // Beacon only
    Orange,     // Conflicting reports
    Red,        // No signal
    Purple,     // ISS Crew voice active
    Grey,       // Unknown status
}

impl ReportStatus {
    pub fn to_string(&self) -> String {
        match self {
            ReportStatus::Blue => "Transponder/Repeater active".to_string(),
            ReportStatus::Yellow => "Telemetry/Beacon only".to_string(),
            ReportStatus::Orange => "Conflicting reports".to_string(),
            ReportStatus::Red => "No signal".to_string(),
            ReportStatus::Purple => "ISS Crew (Voice) Active".to_string(),
            ReportStatus::Grey => "Unknown status".to_string(),
        }
    }

    pub fn to_chinese_string(&self) -> String {
        match self {
            ReportStatus::Blue => "转发器已开机".to_string(),
            ReportStatus::Yellow => "只有遥测/信标".to_string(),
            ReportStatus::Orange => "冲突报告".to_string(),
            ReportStatus::Red => "无信号".to_string(),
            ReportStatus::Purple => "乘组语音活动".to_string(),
            ReportStatus::Grey => "未知状态".to_string(),
        }
    }

    pub fn from_string(s: &str) -> ReportStatus {
        match s.to_lowercase().as_str() {
            "heard" => ReportStatus::Blue,
            "telemetry only" => ReportStatus::Yellow,
            "conflicting reports" => ReportStatus::Orange,
            "not heard" => ReportStatus::Red,
            "crew active" => ReportStatus::Purple,
            _ => ReportStatus::Grey,
        }
    }
}

#[derive(Debug, Deserialize)]
/// Used for store satellite names and aliases
pub struct SatelliteList {
    pub satellites: Vec<SatelliteName>,
}

#[derive(Debug, Deserialize)]
pub struct SatelliteName {
    pub official_name: String,
    pub aliases: Vec<String>,
}

/// Searches for satellite names that match the input string based on a similarity threshold.
pub fn search_satellites<'a>(
    input: &str,
    satellite_list: &'a SatelliteList,
    threshold: f64,
) -> Vec<&'a str> {
    let mut results = Vec::new();

    for sat in &satellite_list.satellites {
        let mut names = vec![&sat.official_name];
        names.extend(sat.aliases.iter());

        for name in names {
            let score = jaro_winkler(&input.to_lowercase(), &name.to_lowercase());
            if score >= threshold {
                results.push((score, sat.official_name.as_str()));
                break;
            }
        }
    }

    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().map(|(_, name)| name).collect()
}