use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct UserReport {
    pub callsign: String,
    pub grid: String,
    pub sat_name: String,
    pub report: ReportStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SatStatus {
    pub sat_name: String,
    pub status: ReportStatus,
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

pub const USER_REPORT_DATA: &str = "/data/user_report_data.json";