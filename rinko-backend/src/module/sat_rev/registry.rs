//! Keyed satellite store with incremental updates and bounded retention.
//!
//! # What changed and why
//!
//! The previous manager rebuilt its entry list from scratch on every scheduled
//! update (`entries.clear()` every 15 minutes). Three defects followed from that
//! single line:
//!
//! - **Reports were discarded** on every cycle, so the whole 24-hour window had to
//!   be re-fetched each time rather than topped up.
//! - **Memory growth was masked.** No retention pruning existed; `clear()` happened
//!   to hide it. Switching to incremental updates without adding pruning would have
//!   turned a hidden bug into a live leak, so the two are implemented together here.
//! - **Renames orphaned data.** Merging was keyed on the upstream label and only
//!   ever appended, so a renamed satellite appeared as a new one while the old
//!   record lingered, still being polled.
//!
//! This registry is keyed on [`SatKey`], upserts in place, prunes on a retention
//! window, and retires records that vanish upstream instead of deleting or polling
//! them forever.
//!
//! No I/O happens here — persistence lives in `store.rs`, so the whole update
//! algorithm stays synchronously testable.

use super::identity::SatKey;
use super::naming;
use super::overlay::Overlay;
use super::types::{AmsatReport, SatRecord, SatelliteDataBlock};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// How long crowd-sourced reports are kept before pruning.
///
/// Must stay at or above the render window; anything older cannot be displayed and
/// only costs memory.
pub const RETENTION_HOURS: i64 = 48;

/// How long a record may be absent upstream before it is retired.
///
/// Generous on purpose: a single failed scrape must not retire the entire fleet.
pub const RETIRE_AFTER_HOURS: i64 = 72;

/// Fraction of records that must be relabelled in one cycle to be considered a
/// scheme change rather than routine churn.
const MASS_RELABEL_RATIO: f64 = 0.30;

/// Outcome of reconciling the registry against an upstream label list.
///
/// Deliberately minimal: enough to log meaningfully and raise one alert, with no
/// extra machinery. Drift reporting is a diagnostic aid, not core functionality.
#[derive(Debug, Default, Clone)]
pub struct DriftReport {
    /// Keys seen for the first time.
    pub added: Vec<SatKey>,
    /// Records whose upstream label changed: `(key, previous, current)`.
    pub relabelled: Vec<(SatKey, String, String)>,
    /// Known keys missing from this scrape.
    pub absent: Vec<SatKey>,
    /// Records retired for prolonged absence.
    pub retired: Vec<SatKey>,
}

impl DriftReport {
    /// Whether anything noteworthy happened.
    pub fn is_quiet(&self) -> bool {
        self.added.is_empty() && self.relabelled.is_empty() && self.retired.is_empty()
    }

    /// Whether so many labels changed at once that upstream likely changed scheme.
    ///
    /// AMSAT has already renamed its whole catalogue once; this is the cheap early
    /// warning for the next time.
    pub fn looks_like_scheme_change(&self, total: usize) -> bool {
        total >= 10 && (self.relabelled.len() as f64) >= (total as f64 * MASS_RELABEL_RATIO)
    }

    /// One-line summary for logs.
    pub fn summary(&self) -> String {
        format!(
            "{} added, {} relabelled, {} absent, {} retired",
            self.added.len(),
            self.relabelled.len(),
            self.absent.len(),
            self.retired.len()
        )
    }
}

/// Satellite records with stable-identity lookup.
///
/// Records live in a `Vec` kept sorted by key, with a side index for O(1) lookup.
/// A `HashMap` would be the obvious choice, but the search layer wants a contiguous
/// `&[SatRecord]`; holding the vector as the source of truth avoids either copying on
/// every query or bending the search API to accommodate storage details.
#[derive(Debug, Default)]
pub struct SatRegistry {
    /// Sorted by [`SatRecord::key`], active records first.
    records: Vec<SatRecord>,
    /// `key -> index into records`.
    index: HashMap<SatKey, usize>,
}

impl SatRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from previously persisted records.
    pub fn from_records(records: Vec<SatRecord>) -> Self {
        let mut reg = Self {
            records,
            index: HashMap::new(),
        };
        reg.reorder();
        reg
    }

    /// Restore the invariant: active records first, each group sorted by key, with
    /// the index rebuilt to match.
    fn reorder(&mut self) {
        self.records.sort_by(|a, b| {
            a.retired
                .cmp(&b.retired)
                .then_with(|| a.key.cmp(&b.key))
        });
        self.index = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.key.clone(), i))
            .collect();
    }

    /// Number of records, including retired ones.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the registry holds no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Look up a record.
    pub fn get(&self, key: &SatKey) -> Option<&SatRecord> {
        self.index.get(key).map(|&i| &self.records[i])
    }

    /// Mutably look up a record.
    pub fn get_mut(&mut self, key: &SatKey) -> Option<&mut SatRecord> {
        let idx = *self.index.get(key)?;
        self.records.get_mut(idx)
    }

    /// All records, retired ones last.
    pub fn records(&self) -> &[SatRecord] {
        &self.records
    }

    /// Active records as a contiguous slice — the search and fetch surface.
    ///
    /// Relies on the ordering invariant that active records precede retired ones.
    pub fn active(&self) -> &[SatRecord] {
        let split = self
            .records
            .iter()
            .position(|r| r.retired)
            .unwrap_or(self.records.len());
        &self.records[..split]
    }

    /// Labels that should be polled this cycle.
    pub fn fetch_targets(&self) -> Vec<String> {
        self.active()
            .iter()
            .map(|r| r.current_label.clone())
            .collect()
    }

    /// Reconcile against the labels just observed upstream.
    ///
    /// New keys are inserted; existing ones have their label refreshed (with the
    /// previous spelling appended to `known_labels`, keeping old names searchable);
    /// absent keys are reported but neither deleted nor immediately retired.
    ///
    /// Reports and overlays are preserved throughout — that is the whole point.
    pub fn observe_upstream(&mut self, labels: &[String]) -> DriftReport {
        let now = Utc::now();
        let mut report = DriftReport::default();
        let mut seen: Vec<SatKey> = Vec::with_capacity(labels.len());

        for label in labels {
            let parsed = naming::parse_label(label);
            if parsed.base.is_empty() {
                continue;
            }
            let key = SatKey::from_parsed(&parsed);
            seen.push(key.clone());

            match self.get_mut(&key) {
                Some(existing) => {
                    if existing.current_label != *label {
                        report.relabelled.push((
                            key.clone(),
                            existing.current_label.clone(),
                            label.clone(),
                        ));
                        existing.adopt_label(label);
                    }
                    // Refresh derived data in case parsing rules improved.
                    existing.refresh_derived(&parsed);
                    existing.last_seen_upstream = now;
                    existing.retired = false;
                }
                None => {
                    self.records
                        .push(SatRecord::new(key.clone(), label, &parsed, now));
                    report.added.push(key);
                }
            }
        }

        for record in &self.records {
            if !seen.contains(&record.key) && !record.retired {
                report.absent.push(record.key.clone());
            }
        }

        // Insertions and reactivations may have broken the ordering invariant.
        self.reorder();

        report.added.sort();
        report.absent.sort();
        report
    }

    /// Attach curated aliases, replacing any previously loaded set.
    ///
    /// Curated data is authoritative and never touched by ingest, so it is applied
    /// as a separate step rather than merged during reconciliation.
    pub fn apply_curated_aliases(&mut self, key: &SatKey, aliases: &[String]) {
        if let Some(record) = self.get_mut(key) {
            record.manual_aliases = aliases
                .iter()
                .map(|a| naming::normalize(a))
                .filter(|a| !a.is_empty())
                .collect();
            record.manual_aliases.sort();
            record.manual_aliases.dedup();
        }
    }

    /// Merge freshly fetched reports into a record, de-duplicating against what is
    /// already stored.
    ///
    /// Incremental merging is what allows the scheduled fetch window to shrink from
    /// 24 hours to a couple of hours.
    pub fn merge_reports(&mut self, key: &SatKey, reports: &[AmsatReport]) {
        let now = Utc::now();
        let Some(record) = self.get_mut(key) else {
            return;
        };

        for report in reports {
            let bucket_time = hour_bucket(&report.reported_time);

            match record.reports.iter_mut().find(|b| b.time == bucket_time) {
                Some(bucket) => {
                    let duplicate = bucket.reports.iter().any(|r| {
                        r.callsign == report.callsign
                            && r.reported_time == report.reported_time
                            && r.report == report.report
                    });
                    if !duplicate {
                        bucket.reports.push(report.clone());
                    }
                }
                None => record.reports.push(SatelliteDataBlock {
                    time: bucket_time,
                    reports: vec![report.clone()],
                }),
            }
        }

        // Newest bucket first — the renderer and `latest_status` rely on this.
        record.reports.sort_by(|a, b| b.time.cmp(&a.time));
        record.last_fetch_success = Some(now);
        record.update_success = true;
        record.last_updated = now;
    }

    /// Attach an overlay to a record, replacing any previous one from that source.
    ///
    /// Returns `false` when no record matches, which is the signal a provider is
    /// pointing at something that does not exist — the failure the old ASRTU code hid
    /// by silently creating an orphan record instead.
    pub fn set_overlay(&mut self, key: &SatKey, overlay: Overlay) -> bool {
        match self.get_mut(key) {
            Some(record) => {
                record.set_overlay(overlay);
                true
            }
            None => false,
        }
    }

    /// Record a failed fetch without discarding existing data.
    pub fn mark_fetch_failed(&mut self, key: &SatKey) {
        if let Some(record) = self.get_mut(key) {
            record.update_success = false;
            record.last_updated = Utc::now();
        }
    }

    /// Resolve a label to the key of an existing record.
    ///
    /// Matches the current label first, then historical ones, so a fetch keyed on an
    /// older spelling still lands correctly.
    pub fn key_for_label(&self, label: &str) -> Option<SatKey> {
        let candidate = SatKey::from_label(label);
        if self.index.contains_key(&candidate) {
            return Some(candidate);
        }
        self.records
            .iter()
            .find(|r| r.current_label == label || r.known_labels.iter().any(|l| l == label))
            .map(|r| r.key.clone())
    }

    /// Drop report buckets older than the retention window.
    ///
    /// **Required companion to incremental updates.** Without it the registry grows
    /// without bound, since nothing else removes old buckets any more.
    pub fn prune(&mut self, retention_hours: i64) -> usize {
        let cutoff = Utc::now() - Duration::hours(retention_hours);
        let mut dropped = 0usize;

        for record in self.records.iter_mut() {
            let before = record.reports.len();
            record.reports.retain(|bucket| match parse_bucket_time(&bucket.time) {
                Some(t) => t >= cutoff,
                // Unparseable timestamps cannot be aged out safely; drop them rather
                // than keep them forever.
                None => false,
            });
            dropped += before - record.reports.len();
        }

        dropped
    }

    /// Retire records absent upstream for longer than `after_hours`.
    ///
    /// Retiring rather than deleting preserves curated aliases and history while
    /// stopping the pointless polling that orphaned records used to incur.
    pub fn retire_stale(&mut self, after_hours: i64) -> Vec<SatKey> {
        let cutoff = Utc::now() - Duration::hours(after_hours);
        let mut retired = Vec::new();

        for record in self.records.iter_mut() {
            if !record.retired && record.last_seen_upstream < cutoff {
                record.retired = true;
                retired.push(record.key.clone());
            }
        }

        if !retired.is_empty() {
            // Retirement moves records to the tail, so the invariant must be restored.
            self.reorder();
        }

        retired.sort();
        retired
    }
}

/// Truncate an RFC3339 timestamp to its hour bucket key.
///
/// Unparseable input is bucketed under `"unknown"`, which [`SatRegistry::prune`]
/// then discards.
fn hour_bucket(reported_time: &str) -> String {
    match DateTime::parse_from_rfc3339(reported_time) {
        Ok(dt) => dt.format("%Y-%m-%dT%H:00:00Z").to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// Parse a bucket key back to a timestamp.
fn parse_bucket_time(bucket: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(bucket)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn report_at(iso: &str, callsign: &str) -> AmsatReport {
        AmsatReport {
            name: "test".to_string(),
            reported_time: iso.to_string(),
            callsign: callsign.to_string(),
            report: "Heard".to_string(),
            grid_square: "AA00".to_string(),
        }
    }

    /// Timestamp `hours_ago`, formatted as upstream would send it.
    fn ago(hours: i64) -> String {
        (Utc::now() - Duration::hours(hours)).to_rfc3339()
    }

    #[test]
    fn inserts_new_records() {
        let mut reg = SatRegistry::new();
        let drift = reg.observe_upstream(&labels(&["AO-91_[FM]", "RS-44"]));

        assert_eq!(reg.len(), 2);
        assert_eq!(drift.added.len(), 2);
        assert!(drift.relabelled.is_empty());
    }

    #[test]
    fn repeated_observation_is_quiet() {
        let mut reg = SatRegistry::new();
        let l = labels(&["AO-91_[FM]"]);
        reg.observe_upstream(&l);
        let drift = reg.observe_upstream(&l);

        assert!(drift.is_quiet(), "steady state should report nothing: {drift:?}");
        assert_eq!(reg.len(), 1);
    }

    /// The headline fix: an upstream rename must preserve reports and curated
    /// aliases instead of orphaning the record.
    #[test]
    fn rename_preserves_data() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-7 V/a"]));

        let key = SatKey::from_label("AO-7 V/a");
        reg.apply_curated_aliases(&key, &["ao7".to_string()]);
        reg.merge_reports(&key, &[report_at(&ago(1), "BJ1ABC")]);

        // Upstream switches to bracket notation.
        let drift = reg.observe_upstream(&labels(&["AO-7_[V/a]"]));

        assert_eq!(reg.len(), 1, "rename must not create a second record");
        assert_eq!(drift.relabelled.len(), 1);
        assert!(drift.added.is_empty());

        let record = reg.get(&key).expect("record survives rename");
        assert_eq!(record.current_label, "AO-7_[V/a]");
        assert!(
            record.known_labels.iter().any(|l| l == "AO-7 V/a"),
            "old label kept for search: {:?}",
            record.known_labels
        );
        assert_eq!(record.total_reports(), 1, "reports survive rename");
        assert_eq!(record.manual_aliases, vec!["ao7"], "curation survives rename");
    }

    /// Old spellings must remain resolvable after a rename.
    #[test]
    fn resolves_historical_labels() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-7 V/a"]));
        reg.observe_upstream(&labels(&["AO-7_[V/a]"]));

        let by_old = reg.key_for_label("AO-7 V/a").expect("old label resolves");
        let by_new = reg.key_for_label("AO-7_[V/a]").expect("new label resolves");
        assert_eq!(by_old, by_new);
    }

    /// Incremental merging must accumulate rather than replace.
    #[test]
    fn merges_reports_incrementally() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]"]));
        let key = SatKey::from_label("AO-91_[FM]");

        reg.merge_reports(&key, &[report_at(&ago(2), "BJ1AAA")]);
        reg.merge_reports(&key, &[report_at(&ago(1), "BJ1BBB")]);

        assert_eq!(reg.get(&key).unwrap().total_reports(), 2);
    }

    #[test]
    fn deduplicates_repeated_reports() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]"]));
        let key = SatKey::from_label("AO-91_[FM]");

        let r = report_at(&ago(1), "BJ1AAA");
        reg.merge_reports(&key, &[r.clone()]);
        reg.merge_reports(&key, &[r.clone(), r]);

        assert_eq!(
            reg.get(&key).unwrap().total_reports(),
            1,
            "overlapping fetch windows must not duplicate reports"
        );
    }

    /// A scheduled update must not wipe accumulated reports — the `clear()` bug.
    #[test]
    fn scheduled_update_does_not_wipe_reports() {
        let mut reg = SatRegistry::new();
        let l = labels(&["AO-91_[FM]"]);
        reg.observe_upstream(&l);
        let key = SatKey::from_label("AO-91_[FM]");
        reg.merge_reports(&key, &[report_at(&ago(3), "BJ1AAA")]);

        reg.observe_upstream(&l); // simulate the 15-minute cycle

        assert_eq!(reg.get(&key).unwrap().total_reports(), 1);
    }

    /// Pruning is the mandatory companion to incremental updates.
    #[test]
    fn prunes_beyond_retention_window() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]"]));
        let key = SatKey::from_label("AO-91_[FM]");

        reg.merge_reports(
            &key,
            &[
                report_at(&ago(1), "FRESH"),
                report_at(&ago(RETENTION_HOURS + 5), "STALE"),
            ],
        );
        assert_eq!(reg.get(&key).unwrap().total_reports(), 2);

        let dropped = reg.prune(RETENTION_HOURS);
        assert_eq!(dropped, 1, "one stale bucket expected");

        let record = reg.get(&key).unwrap();
        assert_eq!(record.total_reports(), 1);
        assert!(record.reports.iter().all(|b| b
            .reports
            .iter()
            .all(|r| r.callsign == "FRESH")));
    }

    #[test]
    fn prune_discards_unparseable_buckets() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]"]));
        let key = SatKey::from_label("AO-91_[FM]");

        reg.merge_reports(&key, &[report_at("not-a-timestamp", "JUNK")]);
        assert_eq!(reg.get(&key).unwrap().total_reports(), 1);

        reg.prune(RETENTION_HOURS);
        assert_eq!(reg.get(&key).unwrap().total_reports(), 0);
    }

    /// Absence is reported but must not retire a record immediately, so one failed
    /// scrape cannot retire the fleet.
    #[test]
    fn absence_is_reported_without_retiring() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]", "RS-44"]));

        let drift = reg.observe_upstream(&labels(&["AO-91_[FM]"]));
        assert_eq!(drift.absent.len(), 1);
        assert!(drift.retired.is_empty());

        assert_eq!(reg.active().len(), 2, "still polled until retirement");
    }

    #[test]
    fn retires_long_absent_records() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]", "RS-44"]));

        // Backdate one record past the retirement horizon.
        let stale = SatKey::from_label("RS-44");
        reg.get_mut(&stale).unwrap().last_seen_upstream =
            Utc::now() - Duration::hours(RETIRE_AFTER_HOURS + 1);

        let retired = reg.retire_stale(RETIRE_AFTER_HOURS);
        assert_eq!(retired, vec![stale.clone()]);
        assert_eq!(reg.active().len(), 1, "retired records are not polled");
        assert_eq!(reg.len(), 2, "retired records are kept, not deleted");
    }

    /// A returning satellite must be reactivated rather than duplicated.
    #[test]
    fn reappearance_reactivates_record() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["RS-44"]));
        let key = SatKey::from_label("RS-44");
        reg.get_mut(&key).unwrap().last_seen_upstream =
            Utc::now() - Duration::hours(RETIRE_AFTER_HOURS + 1);
        reg.retire_stale(RETIRE_AFTER_HOURS);
        assert!(reg.get(&key).unwrap().retired);

        reg.observe_upstream(&labels(&["RS-44"]));

        assert!(!reg.get(&key).unwrap().retired);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn fetch_targets_use_current_labels() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-7 V/a"]));
        reg.observe_upstream(&labels(&["AO-7_[V/a]"]));

        assert_eq!(reg.fetch_targets(), vec!["AO-7_[V/a]".to_string()]);
    }

    #[test]
    fn curated_aliases_are_normalised() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-123_[FM]"]));
        let key = SatKey::from_label("AO-123_[FM]");

        reg.apply_curated_aliases(&key, &["ASRTU-1".to_string(), "  asrtu  ".to_string()]);

        let aliases = &reg.get(&key).unwrap().manual_aliases;
        assert!(aliases.contains(&"asrtu1".to_string()), "got {aliases:?}");
        assert!(aliases.contains(&"asrtu".to_string()), "got {aliases:?}");
    }

    /// Cheap early warning for the next upstream naming overhaul.
    #[test]
    fn detects_mass_relabelling() {
        let mut reg = SatRegistry::new();
        let old: Vec<String> = (1..=10).map(|i| format!("SAT-{i} FM")).collect();
        reg.observe_upstream(&old);

        let new: Vec<String> = (1..=10).map(|i| format!("SAT-{i}_[FM]")).collect();
        let drift = reg.observe_upstream(&new);

        assert_eq!(drift.relabelled.len(), 10);
        assert!(drift.looks_like_scheme_change(reg.len()));
    }

    #[test]
    fn routine_churn_is_not_a_scheme_change() {
        let mut reg = SatRegistry::new();
        let mut names: Vec<String> = (1..=20).map(|i| format!("SAT-{i}_[FM]")).collect();
        reg.observe_upstream(&names);

        names[0] = "SAT-1_[SSTV]".to_string(); // one payload reclassified
        let drift = reg.observe_upstream(&names);

        assert!(!drift.looks_like_scheme_change(reg.len()));
    }

    #[test]
    fn failed_fetch_keeps_existing_reports() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]"]));
        let key = SatKey::from_label("AO-91_[FM]");
        reg.merge_reports(&key, &[report_at(&ago(1), "BJ1AAA")]);

        reg.mark_fetch_failed(&key);

        let record = reg.get(&key).unwrap();
        assert!(!record.update_success);
        assert_eq!(record.total_reports(), 1, "failure must not lose data");
    }

    /// Overlays attach by key and supersede rather than accumulate.
    #[test]
    fn overlays_attach_and_replace_per_source() {
        use super::super::overlay::{Overlay, OverlayPayload, OverlaySource};

        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-123_[FM]"]));
        let key = SatKey::from_label("AO-123_[FM]");

        let make = |on: bool| Overlay {
            source: OverlaySource::Asrtu,
            observed_at: Utc::now(),
            fetched_at: Utc::now(),
            payload: OverlayPayload::CommandedState {
                on,
                detail: "CTCSS".into(),
            },
        };

        assert!(reg.set_overlay(&key, make(true)));
        assert!(reg.set_overlay(&key, make(false)));

        let record = reg.get(&key).unwrap();
        assert_eq!(record.overlays.len(), 1, "one overlay per source");
        assert!(matches!(
            record.overlay(OverlaySource::Asrtu).map(|o| &o.payload),
            Some(OverlayPayload::CommandedState { on: false, .. })
        ));
    }

    /// A provider aimed at a non-existent satellite must be reported, not papered
    /// over by creating a record — the old ASRTU orphan bug.
    #[test]
    fn overlay_for_unknown_key_is_rejected() {
        use super::super::overlay::{Overlay, OverlayPayload, OverlaySource};

        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["AO-91_[FM]"]));

        let bogus = SatKey::from_label("NOPE-1_[FM]");
        let applied = reg.set_overlay(
            &bogus,
            Overlay {
                source: OverlaySource::Asrtu,
                observed_at: Utc::now(),
                fetched_at: Utc::now(),
                payload: OverlayPayload::Announcement { text: "x".into() },
            },
        );

        assert!(!applied, "must report the miss");
        assert_eq!(reg.len(), 1, "must not create a record");
    }

    #[test]
    fn ignores_empty_labels() {
        let mut reg = SatRegistry::new();
        let drift = reg.observe_upstream(&labels(&["", "   ", "AO-91_[FM]"]));
        assert_eq!(reg.len(), 1);
        assert_eq!(drift.added.len(), 1);
    }

    #[test]
    fn record_order_is_deterministic() {
        let mut reg = SatRegistry::new();
        reg.observe_upstream(&labels(&["RS-44", "AO-91_[FM]", "IO-86_[FM]"]));

        let first: Vec<String> = reg.records().iter().map(|r| r.key.to_string()).collect();
        let second: Vec<String> = reg.records().iter().map(|r| r.key.to_string()).collect();
        assert_eq!(first, second);
    }
}
