use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use super::identity::SatKey;
use super::naming::{self, ModeClass};
use super::overlay::{Overlay, OverlaySource};

/// Former name of [`SatRecord`].
///
/// Kept as an alias so the renderer and query layer need not be rewritten alongside
/// the identity change; new code should say [`SatRecord`].
pub type AmsatEntry = SatRecord;

/// The curated satellite file (`data/satellite_list.toml`).
///
/// This is the **human-owned** half of the data model: operators add nicknames here
/// and ingest never rewrites it. Machine state lives in the registry instead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SatelliteList {
    /// Curated entries.
    #[serde(default)]
    pub satellites: Vec<SatelliteEntry>,
}

/// One curated entry: an upstream label plus the aliases a human attached to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteEntry {
    /// Upstream label, used only to resolve which record the aliases belong to.
    pub api_name: String,
    /// Nicknames, callsigns and colloquial names that cannot be derived mechanically.
    #[serde(default)]
    pub aliases: Vec<String>,
}

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

/// One satellite payload: the unit of querying, storage and display.
///
/// Identity lives in [`key`], which is derived from durable properties and never
/// changes. The upstream label is demoted to [`current_label`] — an observation that
/// may be replaced at any time — with past spellings retained in [`known_labels`] so
/// operators can still search by the name they remember.
///
/// [`key`]: Self::key
/// [`current_label`]: Self::current_label
/// [`known_labels`]: Self::known_labels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatRecord {
    /// Stable internal identity. Assigned once, never rewritten.
    pub key: SatKey,

    /// The label upstream currently uses. Used to build API requests, and shown to
    /// users. Expect it to change.
    pub current_label: String,

    /// Every label previously observed for this payload.
    ///
    /// Kept so a rename does not make the old name unsearchable.
    #[serde(default)]
    pub known_labels: Vec<String>,

    /// Machine-derived search aliases.
    ///
    /// Regenerated on every reconciliation — never hand-edit, edits here are lost.
    /// See [`naming::derive_aliases`].
    #[serde(default)]
    pub aliases: Vec<String>,

    /// Curated aliases carrying knowledge that cannot be derived from the label:
    /// project nicknames (`"asrtu"`), operator callsigns, colloquial names.
    ///
    /// Loaded from `satellite_list.toml` and **never overwritten by ingest**. Keeping
    /// these separate from [`aliases`] is what prevents a scrape from destroying
    /// months of manual curation.
    ///
    /// [`aliases`]: Self::aliases
    #[serde(default)]
    pub manual_aliases: Vec<String>,

    /// Parsed satellite designator, e.g. `"AO-91"`.
    pub satellite_base_name: String,

    /// Raw mode token from the label, e.g. `Some("FM")`, `Some("U/v")`, `None`.
    #[serde(default)]
    pub mode: Option<String>,

    /// Functional classification of [`mode`], used for category searches such as
    /// "show me the FM birds".
    ///
    /// [`mode`]: Self::mode
    #[serde(default)]
    pub mode_class: Option<ModeClass>,

    /// Crowd-sourced AMSAT reports, newest hourly bucket first.
    #[serde(default)]
    pub reports: Vec<SatelliteDataBlock>,

    /// Authoritative facts from non-AMSAT sources, at most one per source.
    ///
    /// Kept apart from [`reports`] because they differ in kind: a commanded state is
    /// ground truth, a crowd report is an observation. Merging them would misrepresent
    /// both.
    ///
    /// [`reports`]: Self::reports
    #[serde(default)]
    pub overlays: Vec<Overlay>,

    /// When this payload was last present in an upstream scrape.
    ///
    /// Drives retirement, so a vanished satellite stops being polled without being
    /// deleted.
    pub last_seen_upstream: DateTime<Utc>,

    /// Last time the record was touched by any update.
    pub last_updated: DateTime<Utc>,

    /// Last successful report fetch.
    pub last_fetch_success: Option<DateTime<Utc>>,

    /// Whether the most recent fetch succeeded.
    #[serde(default)]
    pub update_success: bool,

    /// Absent upstream long enough to stop polling. Retained, not deleted, so
    /// curated aliases and history survive a temporary disappearance.
    #[serde(default)]
    pub retired: bool,
}

impl SatRecord {
    /// Create a record for a newly observed label.
    pub fn new(
        key: SatKey,
        label: &str,
        parsed: &naming::ParsedLabel,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            key,
            current_label: label.to_string(),
            known_labels: vec![label.to_string()],
            aliases: naming::derive_aliases(parsed),
            manual_aliases: Vec::new(),
            satellite_base_name: parsed.base.clone(),
            mode: parsed.mode.as_ref().map(|m| m.raw.clone()),
            mode_class: parsed.mode.as_ref().map(|m| m.class),
            reports: Vec::new(),
            overlays: Vec::new(),
            last_seen_upstream: now,
            last_updated: now,
            last_fetch_success: None,
            update_success: false,
            retired: false,
        }
    }

    /// Convenience constructor from a label alone, for tests and simple call sites.
    pub fn from_label(label: &str) -> Self {
        let parsed = naming::parse_label(label);
        let key = SatKey::from_parsed(&parsed);
        Self::new(key, label, &parsed, Utc::now())
    }

    /// Adopt a new upstream label, remembering the previous one.
    pub fn adopt_label(&mut self, label: &str) {
        if self.current_label != label {
            let previous = std::mem::replace(&mut self.current_label, label.to_string());
            if !self.known_labels.contains(&previous) {
                self.known_labels.push(previous);
            }
        }
        if !self.known_labels.iter().any(|l| l == label) {
            self.known_labels.push(label.to_string());
        }
    }

    /// Recompute derived fields from a fresh parse.
    ///
    /// Lets improvements to the parser propagate to existing records without a
    /// migration, while leaving curated data untouched.
    pub fn refresh_derived(&mut self, parsed: &naming::ParsedLabel) {
        self.aliases = naming::derive_aliases(parsed);
        self.satellite_base_name = parsed.base.clone();
        self.mode = parsed.mode.as_ref().map(|m| m.raw.clone());
        self.mode_class = parsed.mode.as_ref().map(|m| m.class);
    }

    /// Every string a query may match against: current and historical labels,
    /// derived aliases, curated aliases.
    pub fn all_aliases(&self) -> impl Iterator<Item = &String> {
        self.aliases
            .iter()
            .chain(self.manual_aliases.iter())
            .chain(self.known_labels.iter())
    }

    /// The upstream label, under its historical field name.
    ///
    /// Retained so existing display and search code keeps reading one accessor while
    /// the underlying value is now understood to be mutable.
    pub fn api_name(&self) -> &str {
        &self.current_label
    }

    /// Replace this source's overlay, keeping at most one entry per source.
    ///
    /// Overlays describe *current* state, so a new reading supersedes the old rather
    /// than accumulating.
    pub fn set_overlay(&mut self, overlay: Overlay) {
        self.overlays.retain(|o| o.source != overlay.source);
        self.overlays.push(overlay);
    }

    /// The overlay from a given source, if any.
    pub fn overlay(&self, source: OverlaySource) -> Option<&Overlay> {
        self.overlays.iter().find(|o| o.source == source)
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