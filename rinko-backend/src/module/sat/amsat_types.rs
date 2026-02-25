///! AMSAT data types - Primary data model for satellite status
///!
///! AmsatEntry is the first-class citizen for user queries.
///! Each entry corresponds to one AMSAT API name (e.g., "ISS-FM", "AO-91").
///! Metadata from GitHub CSV is attached lazily at render time.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use super::types::{SatelliteDataBlock, ReportStatus};

// ============ ASRTU Telemetry ============

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
    pub(crate) ctcss_value: String,
    /// RFC3339 timestamp at which the telemetry server received this frame.
    pub observed_at: String,
}

impl AsrtuTelemetrySnapshot {
    /// Human-readable power state: `"Repeater On"` or `"Repeater Off"`.
    pub fn status_label(&self) -> &'static str {
        if self.repeater_on { "Repeater On" } else { "Repeater Off" }
    }

    /// Observation time as a display string, e.g.
    /// `"Observed at 2026-02-21 14:41 UTC (2 hours ago)"`.
    pub fn observed_description(&self) -> String {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&self.observed_at) {
            let dt_utc = dt.with_timezone(&Utc);
            let diff = Utc::now().signed_duration_since(dt_utc);
            let time_str = dt_utc.format("%Y-%m-%d %H:%M UTC");
            let hours = diff.num_hours();
            let minutes = diff.num_minutes();
            if hours >= 1 {
                format!("Observed at {} ({} hour{} ago)", time_str, hours, if hours == 1 { "" } else { "s" })
            } else if minutes >= 1 {
                format!("Observed at {} ({} minute{} ago)", time_str, minutes, if minutes == 1 { "" } else { "s" })
            } else {
                format!("Observed at {} (just now)", time_str)
            }
        } else {
            format!("Observed at {}", self.observed_at)
        }
    }

    /// Attribution label to show in the UI.
    pub fn data_source() -> &'static str {
        "ASRTU-1 Group"
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

    /// Parsed mode hint from API name, e.g. Some("FM") from "ISS-FM", None from "AO-91"
    #[serde(default)]
    pub mode_hint: Option<String>,

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
        let aliases = generate_aliases(api_name);

        Self {
            api_name: api_name.to_string(),
            aliases,
            satellite_base_name: parsed.base_name,
            mode_hint: parsed.mode_hint,
            reports: Vec::new(),
            last_updated: Utc::now(),
            last_fetch_success: None,
            update_success: false,
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

// ============ AMSAT Name Parsing ============

/// Known mode keywords that can appear as a suffix in AMSAT API names
const MODE_KEYWORDS: &[&str] = &[
    "FM", "SSTV", "DATA", "DATV", "LINEAR", "LIN", "IMAGE", "IMG",
    "CW", "SSB", "DIGI", "APRS", "PACKET", "V/U", "U/V", "H/U", "V/U FM",
    "L", "S", "X", "A", "B"
];

/// Result of parsing an AMSAT API name
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAmsatName {
    /// The satellite base name (e.g., "ISS", "AO-91")
    pub base_name: String,
    /// The mode hint extracted from the name (e.g., Some("FM"), None)
    pub mode_hint: Option<String>,
}

/// Parse an AMSAT API name into (base_name, mode_hint)
///
/// Rules:
/// 1. Split by space or hyphen from the right
/// 2. If the last token is a known mode keyword, extract it as mode_hint
/// 3. Remaining tokens form the base_name
/// 4. Special handling: names like "AO-91" where the number after hyphen
///    is NOT a mode keyword → the whole name is the base_name
///
/// Examples:
/// - "ISS-FM"     → ("ISS",   Some("FM"))
/// - "ISS FM"     → ("ISS",   Some("FM"))
/// - "ISS-SSTV"   → ("ISS",   Some("SSTV"))
/// - "AO-91"      → ("AO-91", None)
/// - "AO-7"       → ("AO-7",  None)
/// - "RS-44"      → ("RS-44", None)
/// - "TEVEL2 FM"  → ("TEVEL2", Some("FM"))
/// - "IO-117"     → ("IO-117", None)
pub fn parse_amsat_name(api_name: &str) -> ParsedAmsatName {
    let trimmed = api_name.trim();

    if trimmed.is_empty() {
        return ParsedAmsatName {
            base_name: String::new(),
            mode_hint: None,
        };
    }

    // First try splitting by space (most unambiguous separator)
    if let Some(space_idx) = trimmed.rfind(' ') {
        let candidate = trimmed[space_idx + 1..].trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..space_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    // Then try splitting by hyphen from the right
    // But be careful: "AO-91" is a satellite designation, not "AO" + mode "91"
    if let Some(hyphen_idx) = trimmed.rfind('-') {
        let candidate = trimmed[hyphen_idx + 1..].trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..hyphen_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    // Try splitting by `[]` and `()` as well
    // e.g. QMR-KWT-2_(RS95s) → base: "QMR-KWT-2_(RS95s)", mode: None
    // e.g. FO-118[H/u] → base: "FO-118", mode: "H/U"
    if let Some(bracket_idx) = trimmed.rfind('[') {
        let candidate = trimmed[bracket_idx + 1..].trim_end_matches(']').trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..bracket_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    if let Some(paren_idx) = trimmed.rfind('(') {
        let candidate = trimmed[paren_idx + 1..].trim_end_matches(')').trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..paren_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    // No mode keyword found - the whole name is the base name
    ParsedAmsatName {
        base_name: trimmed.to_string(),
        mode_hint: None,
    }
}

/// Check if a string is a known mode keyword (case-insensitive)
fn is_mode_keyword(s: &str) -> bool {
    let upper = s.to_uppercase();
    MODE_KEYWORDS.iter().any(|kw| *kw == upper)
}

/// Generate search aliases for an AMSAT API name
///
/// Produces normalized variants to help with search matching.
/// e.g., "ISS-FM" → ["ISS FM", "ISSFM", "ISS-FM"]
fn generate_aliases(api_name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    let trimmed = api_name.trim();

    // Original with spaces replaced by nothing
    let no_sep: String = trimmed.chars()
        .filter(|c| !c.is_ascii_punctuation() && !c.is_whitespace())
        .collect();
    if !no_sep.is_empty() && no_sep != trimmed {
        aliases.push(no_sep);
    }

    // With hyphens replaced by spaces
    let spaces = trimmed.replace('-', " ");
    if spaces != trimmed && !aliases.contains(&spaces) {
        aliases.push(spaces);
    }

    // With spaces replaced by hyphens
    let hyphens = trimmed.replace(' ', "-");
    if hyphens != trimmed && !aliases.contains(&hyphens) {
        aliases.push(hyphens);
    }

    aliases
}

/// Normalize a string for search matching (lowercase, no punctuation/whitespace)
pub fn normalize_for_search(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_punctuation() && !c.is_whitespace())
        .collect()
}

// ============ Metadata matching helpers ============

/// Attempt to find matching metadata from a list of transponders.
///
/// Given a mode_hint (e.g., "FM") and a list of transponder labels/modes,
/// return the index of the best-matching transponder.
///
/// Match strategy:
/// 1. Exact label match (case-insensitive)
/// 2. Label contains mode_hint
/// 3. Mode string contains mode_hint
/// 4. If no mode_hint, return the primary transponder (or first)
pub fn find_matching_transponder_index(
    mode_hint: Option<&str>,
    labels: &[(String, String)], // (label, mode) pairs
) -> Option<usize> {
    match mode_hint {
        Some(hint) => {
            let hint_upper = hint.to_uppercase();

            // 1. Exact label match
            if let Some(idx) = labels.iter().position(|(label, _)| {
                label.to_uppercase() == hint_upper
            }) {
                return Some(idx);
            }

            // 2. Label contains hint
            if let Some(idx) = labels.iter().position(|(label, _)| {
                label.to_uppercase().contains(&hint_upper)
            }) {
                return Some(idx);
            }

            // 3. Mode contains hint
            if let Some(idx) = labels.iter().position(|(_, mode)| {
                mode.to_uppercase().contains(&hint_upper)
            }) {
                return Some(idx);
            }

            None
        }
        None => {
            // No mode hint - return first (primary) if available
            if !labels.is_empty() {
                Some(0)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iss_fm() {
        let result = parse_amsat_name("ISS-FM");
        assert_eq!(result, ParsedAmsatName {
            base_name: "ISS".to_string(),
            mode_hint: Some("FM".to_string()),
        });
    }

    #[test]
    fn test_parse_iss_sstv() {
        let result = parse_amsat_name("ISS-SSTV");
        assert_eq!(result, ParsedAmsatName {
            base_name: "ISS".to_string(),
            mode_hint: Some("SSTV".to_string()),
        });
    }

    #[test]
    fn test_parse_iss_space_fm() {
        let result = parse_amsat_name("ISS FM");
        assert_eq!(result, ParsedAmsatName {
            base_name: "ISS".to_string(),
            mode_hint: Some("FM".to_string()),
        });
    }

    #[test]
    fn test_parse_ao91() {
        let result = parse_amsat_name("AO-91");
        assert_eq!(result, ParsedAmsatName {
            base_name: "AO-91".to_string(),
            mode_hint: None,
        });
    }

    #[test]
    fn test_parse_ao7() {
        let result = parse_amsat_name("AO-7");
        assert_eq!(result, ParsedAmsatName {
            base_name: "AO-7".to_string(),
            mode_hint: None,
        });
    }

    #[test]
    fn test_parse_rs44() {
        let result = parse_amsat_name("RS-44");
        assert_eq!(result, ParsedAmsatName {
            base_name: "RS-44".to_string(),
            mode_hint: None,
        });
    }

    #[test]
    fn test_parse_iss_data() {
        let result = parse_amsat_name("ISS-DATA");
        assert_eq!(result, ParsedAmsatName {
            base_name: "ISS".to_string(),
            mode_hint: Some("DATA".to_string()),
        });
    }

    #[test]
    fn test_parse_iss_datv() {
        let result = parse_amsat_name("ISS-DATV");
        assert_eq!(result, ParsedAmsatName {
            base_name: "ISS".to_string(),
            mode_hint: Some("DATV".to_string()),
        });
    }

    #[test]
    fn test_parse_plain_name() {
        let result = parse_amsat_name("TEVEL-1");
        assert_eq!(result, ParsedAmsatName {
            base_name: "TEVEL-1".to_string(),
            mode_hint: None,
        });
    }

    #[test]
    fn test_parse_io117() {
        let result = parse_amsat_name("IO-117");
        assert_eq!(result, ParsedAmsatName {
            base_name: "IO-117".to_string(),
            mode_hint: None,
        });
    }

    #[test]
    fn test_normalize_for_search() {
        assert_eq!(normalize_for_search("ISS-FM"), "issfm");
        assert_eq!(normalize_for_search("AO-91"), "ao91");
        assert_eq!(normalize_for_search("  ISS SSTV  "), "isssstv");
    }

    #[test]
    fn test_generate_aliases() {
        let aliases = generate_aliases("ISS-FM");
        assert!(aliases.contains(&"ISSFM".to_string()));
        assert!(aliases.contains(&"ISS FM".to_string()));
    }

    #[test]
    fn test_find_matching_transponder_fm() {
        let labels = vec![
            ("SSTV".to_string(), "SSTV".to_string()),
            ("Digipeater".to_string(), "1200bps AFSK Digipeater".to_string()),
            ("FM".to_string(), "FM tone 67.0Hz".to_string()),
        ];
        let idx = find_matching_transponder_index(Some("FM"), &labels);
        assert_eq!(idx, Some(2));
    }

    #[test]
    fn test_find_matching_transponder_sstv() {
        let labels = vec![
            ("SSTV".to_string(), "SSTV".to_string()),
            ("Digipeater".to_string(), "1200bps AFSK Digipeater".to_string()),
            ("FM".to_string(), "FM tone 67.0Hz".to_string()),
        ];
        let idx = find_matching_transponder_index(Some("SSTV"), &labels);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn test_find_matching_transponder_no_hint() {
        let labels = vec![
            ("SSTV".to_string(), "SSTV".to_string()),
            ("FM".to_string(), "FM tone 67.0Hz".to_string()),
        ];
        let idx = find_matching_transponder_index(None, &labels);
        assert_eq!(idx, Some(0)); // Returns first
    }

    #[test]
    fn test_find_matching_transponder_data_in_mode() {
        let labels = vec![
            ("FM".to_string(), "FM tone 67.0Hz".to_string()),
            ("Digipeater".to_string(), "1200bps AFSK DATA".to_string()),
        ];
        // "DATA" matches mode string of Digipeater
        let idx = find_matching_transponder_index(Some("DATA"), &labels);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn test_amsat_entry_creation() {
        let entry = AmsatEntry::from_api_name("ISS-FM");
        assert_eq!(entry.api_name, "ISS-FM");
        assert_eq!(entry.satellite_base_name, "ISS");
        assert_eq!(entry.mode_hint, Some("FM".to_string()));
        assert!(entry.aliases.contains(&"ISSFM".to_string()));
        assert_eq!(entry.total_reports(), 0);
    }

    #[test]
    fn test_amsat_entry_no_mode() {
        let entry = AmsatEntry::from_api_name("AO-91");
        assert_eq!(entry.api_name, "AO-91");
        assert_eq!(entry.satellite_base_name, "AO-91");
        assert_eq!(entry.mode_hint, None);
    }
}
