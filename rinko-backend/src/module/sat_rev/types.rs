use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use super::amsat::parse_amsat_name;

/// NORAD ID type (satellite unique identifier)
pub type NoradId = u32;

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

    // #[test]
    // fn test_satellite_info_creation() {
    //     let sat = SatelliteInfo::new("AO-91");
    //     assert_eq!(sat.name, "AO-91");
    //     assert!(sat.is_active);
    //     assert_eq!(sat.total_reports(), 0);
    // }
}

/// Hardware-confirmed repeater state from the ASRTU telemetry API.
///
/// This snapshot is stored **separately** from crowd-sourced [`AmsatReport`] observations
/// so that renderers can present it as a distinct, authoritative data source rather than
/// mixing it with user-submitted reports.  AMSAT reports are still fetched and displayed
/// independently for reference.
///
/// [`AmsatReport`]: super::types::AmsatReport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrtuTelemetrySnapshot {
    /// Hardware-confirmed repeater state: `true` when the CTCSS encoder is enabled.
    pub repeater_on: bool,
    /// Raw CTCSS enable register value — internal use only, not shown to users.
    #[serde(rename = "ctcss")]
    pub ctcss_value: String,
    /// RFC3339 timestamp at which the telemetry server received this frame.
    pub observed_at: String,
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

    /// Human-curated mode keywords from TOML (e.g. ["fm"], ["data", "digi", "lin"]).
    /// Used for keyword search — distinct from the single `mode` parsed from the API name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,

    /// Human-curated tags from TOML (e.g. ["leo", "experimental"]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Frequency metadata for transponders associated with this satellite (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transponder_info: Option<Vec<TransponderInfo>>,

    /// Direct ASRTU hardware telemetry snapshot.
    ///
    /// Stored separately from crowd-sourced [`reports`](Self::reports) so that
    /// renderers can present it as a clearly labelled authoritative data source
    /// alongside (not instead of) user observations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asrtu_telemetry: Option<AsrtuTelemetrySnapshot>,
}

impl AmsatEntry {
    /// Create a new entry from an AMSAT API name
    pub fn from_api_name(api_name: &str) -> Self {
        let parsed = parse_amsat_name(api_name);
        let aliases = Vec::new();

        Self {
            api_name: api_name.to_string(),
            aliases,
            satellite_base_name: parsed.base_name,
            mode: parsed.mode_hint,
            reports: Vec::new(),
            last_updated: Utc::now(),
            last_fetch_success: None,
            update_success: false,
            modes: Vec::new(),
            tags: Vec::new(),
            transponder_info: None,
            asrtu_telemetry: None,
        }
    }

    /// Get latest status from crowd-sourced AMSAT reports.
    ///
    /// ASRTU hardware telemetry is intentionally **not** considered here; it is
    /// displayed separately by renderers via [`AsrtuTelemetrySnapshot`].  This
    /// keeps user-submitted observations and hardware data visually distinct.
    pub fn latest_status(&self) -> ReportStatus {
        // Reports are sorted newest-first (by time block)
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

/// Deserialize CSV empty fields as `None` instead of `Some("")`.
fn csv_empty_string_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    Ok(s.filter(|v| !v.is_empty()))
}

/// Transponder / frequency metadata parsed from the CSV satellite database.
///
/// Field renames map the CSV headers (`uplink`, `downlink`, `beacon`) to the
/// more explicit Rust field names (`uplink_freq`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransponderInfo {
    pub name: String,
    pub norad_id: NoradId,
    #[serde(rename = "uplink", default, deserialize_with = "csv_empty_string_as_none")]
    pub uplink_freq: Option<String>,
    #[serde(rename = "downlink", default, deserialize_with = "csv_empty_string_as_none")]
    pub downlink_freq: Option<String>,
    #[serde(rename = "beacon", default, deserialize_with = "csv_empty_string_as_none")]
    pub beacon_freq: Option<String>,
    #[serde(default, deserialize_with = "csv_empty_string_as_none")]
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "csv_empty_string_as_none")]
    pub callsign: Option<String>,
    #[serde(default, deserialize_with = "csv_empty_string_as_none")]
    pub satnogs_id: Option<String>,
}

impl TransponderInfo {
    /// Get uplink/downlink info as a formatted string (e.g. "↑145.990 MHz / ↓437.800 MHz / 9k2 GMSK FM")
    pub fn formatted_transponder_info(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref uplink) = self.uplink_freq {
            parts.push(format!("↑{}", uplink));
        } else {
            parts.push("↑N/A".to_string());
        }
        if let Some(ref downlink) = self.downlink_freq {
            parts.push(format!("↓{}", downlink));
        } else {
            parts.push("↓N/A".to_string());
        }
        if let Some(ref mode) = self.mode {
            parts.push(mode.clone());
        } else {
            parts.push("Mode N/A".to_string());
        }
        parts.join(" | ")
    }
}