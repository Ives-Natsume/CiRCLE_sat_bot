use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use super::amsat::parse_amsat_name;

/// Satellite report from AMSAT API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmsatReport {
    pub name: String,
    pub reported_time: String,  // RFC3339 format
    pub callsign: String,
    pub report: String,
    pub grid_square: String,
}

impl Default for AmsatReport {
    fn default() -> Self {
        Self {
            name: String::new(),
            reported_time: String::new(),
            callsign: String::new(),
            report: ReportStatus::Grey.to_string(),
            grid_square: String::new(),
        }
    }
}

/// Report status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportStatus {
    Blue,    // Transponder/Repeater active
    Yellow,  // Beacon/Telemetry only
    Orange,  // Conflicting reports
    Red,     // No signal
    Purple,  // ISS Crew voice active
    Grey,    // Unknown status
}

impl ReportStatus {
    /// Convert to user-friendly string
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

    /// Convert to report format string
    pub fn to_report_format(&self) -> String {
        match self {
            ReportStatus::Blue => "Heard".to_string(),
            ReportStatus::Yellow => "Telemetry Only".to_string(),
            ReportStatus::Red => "Not Heard".to_string(),
            ReportStatus::Purple => "Crew Active".to_string(),
            _ => "Unknown status".to_string(),
        }
    }

    /// Parse from string
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "heard" => ReportStatus::Blue,
            "telemetry only" => ReportStatus::Yellow,
            "conflicting reports" => ReportStatus::Orange,
            "not heard" => ReportStatus::Red,
            "crew active" => ReportStatus::Purple,
            _ => ReportStatus::Grey,
        }
    }

    /// Convert to hex color for rendering
    pub fn to_color_hex(&self) -> &'static str {
        match self {
            ReportStatus::Blue => "#4297f3ff",
            ReportStatus::Yellow => "#f3cd36ff",
            ReportStatus::Orange => "#f97316",
            ReportStatus::Red => "#ed3f3fff",
            ReportStatus::Purple => "#946af5ff",
            ReportStatus::Grey => "#6b7280",
        }
    }

    /// Get color from string status
    pub fn string_to_color_hex(status: &str) -> &'static str {
        Self::from_string(status).to_color_hex()
    }
}

/// Satellite data block (one hour block)
/// 
/// Used to group AMSAT reports by time blocks for efficient storage and querying.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteDataBlock {
    pub time: String,                   // Time block (e.g., "2026-02-16T08:00:00Z")
    pub reports: Vec<AmsatReport>,      // Reports for this time block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_status_conversion() {
        assert_eq!(ReportStatus::from_string("heard"), ReportStatus::Blue);
        assert_eq!(ReportStatus::from_string("Heard"), ReportStatus::Blue);
        assert_eq!(ReportStatus::from_string("not heard"), ReportStatus::Red);
        assert_eq!(ReportStatus::from_string("unknown"), ReportStatus::Grey);
    }

    #[test]
    fn test_report_status_color() {
        assert_eq!(ReportStatus::Blue.to_color_hex(), "#4297f3ff");
        assert_eq!(ReportStatus::Red.to_color_hex(), "#ed3f3fff");
    }
}

/// AMSAT entry - the primary unit for user queries and status display
///
/// Each entry maps 1:1 with an AMSAT API satellite name.
/// Example entries: "ISS-FM", "ISS-SSTV", "AO-91", "RS-44"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmsatEntry {
    /// AMSAT API name (primary key), e.g. "ISS-FM", "AO-91"
    pub api_name: String,

    /// Search aliases (normalized variants), e.g. ["ISS FM", "ISSFM"]
    #[serde(default)]
    pub aliases: Vec<String>,

    /// Parsed satellite base name, e.g. "ISS" from "ISS-FM", "AO-91" from "AO-91"
    pub satellite_base_name: String,

    /// Parsed mode from API name, e.g. Some("FM") from "ISS-FM", None from "AO-91"
    #[serde(default)]
    pub mode: Option<String>,

    /// Status report data blocks (hourly buckets)
    #[serde(default)]
    pub reports: Vec<SatelliteDataBlock>,

    /// Last update time
    pub last_updated: DateTime<Utc>,

    /// Last successful fetch time
    pub last_fetch_success: Option<DateTime<Utc>>,

    /// Whether AMSAT update was successful
    #[serde(default)]
    pub update_success: bool,
}

impl AmsatEntry {
    /// Create a new entry from an AMSAT API name
    pub fn from_api_name(api_name: &str) -> Self {
        let parsed = parse_amsat_name(api_name);

        Self {
            api_name: api_name.to_string(),
            aliases: Vec::new(),
            satellite_base_name: parsed.base_name,
            mode: parsed.mode_hint,
            reports: Vec::new(),
            last_updated: Utc::now(),
            last_fetch_success: None,
            update_success: false,
        }
    }

    /// Get latest status from crowd-sourced AMSAT reports.
    pub fn latest_status(&self) -> ReportStatus {
        if let Some(first_block) = self.reports.first() {
            if let Some(last_report) = first_block.reports.last() {
                return ReportStatus::from_string(&last_report.report);
            }
        }
        ReportStatus::Grey
    }

    /// Get total number of individual reports
    pub fn total_reports(&self) -> usize {
        self.reports.iter().map(|b| b.reports.len()).sum()
    }

    /// Check if entry has recent data (within given hours)
    pub fn has_recent_data(&self, hours: i64) -> bool {
        let cutoff = Utc::now() - chrono::Duration::hours(hours);
        if let Some(first_block) = self.reports.first() {
            if let Ok(parsed) = DateTime::parse_from_rfc3339(&first_block.time) {
                return parsed.with_timezone(&Utc) >= cutoff;
            }
        }
        false
    }

    /// Get recent reports within given hours
    pub fn get_recent_reports(&self, hours: i64) -> Vec<&SatelliteDataBlock> {
        let cutoff = Utc::now() - chrono::Duration::hours(hours);
        self.reports.iter()
            .filter(|block| {
                if let Ok(time) = DateTime::parse_from_rfc3339(&block.time) {
                    time.with_timezone(&Utc) >= cutoff
                } else {
                    false
                }
            })
            .collect()
    }
}