//! Orchestration for the AMSAT pipeline.
//!
//! This layer owns no algorithms: parsing lives in [`naming`], identity in
//! [`super::identity`], state transitions in [`super::registry`], search in
//! [`query`], and network I/O in [`super::api_client`]. What remains is sequencing,
//! which keeps the update cycle short enough to read at a glance.
//!
//! The previous version duplicated the entire cycle between `init` and
//! `update_satellite_data` — and ran the scraper twice on startup. Both paths now
//! funnel through [`SatManager::refresh`], differing only in the fetch window.

use crate::module::news::{report_internal, NewsUrgency};

use super::{
    api_client::{batch_fetch_satellites, SatelliteScraper, SATELLITE_LIST_CACHE_PATH},
    identity::SatKey,
    naming,
    overlay::OverlayProvider,
    providers::asrtu::AsrtuProvider,
    query,
    registry::{DriftReport, SatRegistry, RETENTION_HOURS, RETIRE_AFTER_HOURS},
    store::RegistryStore,
    types::{SatRecord, SatelliteList},
};
use anyhow::Result;

/// Report window for a cold start, when no history is held yet.
const COLD_START_HOURS: u64 = 24;

/// Report window for scheduled refreshes.
///
/// Incremental merging means only recent reports are needed. The old code re-fetched
/// a full day every cycle purely because it discarded everything each time.
const INCREMENTAL_HOURS: u64 = 2;

/// Report window used on startup when a snapshot was restored.
///
/// Wide enough to cover a realistic downtime, far cheaper than a full cold start.
const GAP_FILL_HOURS: u64 = 6;

/// Delay between upstream requests, to stay polite to the AMSAT API.
const BATCH_DELAY_MS: u64 = 200;

/// Owns satellite state and coordinates updates.
pub struct SatManager {
    registry: SatRegistry,
    store: RegistryStore,
    /// ASRTU telemetry source, present only when configured.
    asrtu: Option<AsrtuProvider>,
}

impl SatManager {
    /// Build a manager, restoring persisted state before contacting upstream.
    ///
    /// Restoring first is what removes the cold-start outage: queries can be served
    /// from the snapshot while the refresh runs, instead of returning nothing for the
    /// minutes it takes to poll every satellite.
    pub async fn init() -> Self {
        let store = RegistryStore::new();
        let restored = store.load().await;
        let had_history = restored.is_some();

        let asrtu = AsrtuProvider::new(
            crate::config::CONFIG
                .get()
                .and_then(|c| c.asrtu_api_url.as_deref()),
        );
        if asrtu.is_some() {
            tracing::info!("ASRTU telemetry provider enabled");
        }

        let mut manager = Self {
            registry: restored.unwrap_or_default(),
            store,
            asrtu,
        };

        // With history on hand only the gap needs filling; without it, take the long
        // window so the first render is not sparse.
        let window = if had_history {
            COLD_START_HOURS.min(GAP_FILL_HOURS)
        } else {
            COLD_START_HOURS
        };

        if let Err(e) = manager.refresh(window).await {
            tracing::error!("Initial satellite refresh failed: {:?}", e);
        }

        tracing::info!(
            "SatManager initialised: {} active record(s) (restored: {})",
            manager.registry.active().len(),
            had_history
        );

        manager
    }

    /// Scheduled update entry point.
    pub async fn update_satellite_data(&mut self) -> Result<()> {
        self.refresh(INCREMENTAL_HOURS).await
    }

    /// One full cycle: scrape, reconcile, fetch, then enforce retention.
    ///
    /// `fetch_hours` is the sole difference between a cold start and a scheduled
    /// refresh, so both share this path.
    async fn refresh(&mut self, fetch_hours: u64) -> Result<()> {
        // ── 1. Observe upstream ──────────────────────────────────────────
        let labels = self.scrape_labels().await;
        if labels.is_empty() {
            // A failed scrape must never empty the registry.
            tracing::warn!("No satellite labels available; keeping existing records");
        } else {
            let drift = self.registry.observe_upstream(&labels);
            self.report_drift(&drift);
        }

        // ── 2. Re-apply curated aliases ──────────────────────────────────
        // After reconciliation, so curated data always survives ingest.
        self.apply_curated_aliases().await;

        // ── 3. Fetch reports ─────────────────────────────────────────────
        self.fetch_reports(fetch_hours).await;
        // ── 4. Poll overlay sources ───────────────────────────────────
        self.poll_overlays().await;
        // ── 5. Bounded retention ──────────────────────────────────────
        // Mandatory companion to incremental updates: nothing else bounds growth
        // now that the registry is no longer cleared each cycle.
        let dropped = self.registry.prune(RETENTION_HOURS);
        let retired = self.registry.retire_stale(RETIRE_AFTER_HOURS);
        if dropped > 0 || !retired.is_empty() {
            tracing::info!(
                "Retention: pruned {} stale bucket(s), retired {} record(s)",
                dropped,
                retired.len()
            );
        }

        // ── 6. Persist ───────────────────────────────────────────────────
        // A failed save must not fail the cycle: the in-memory state is still good
        // and the next cycle will try again.
        if let Err(e) = self.store.save(&self.registry).await {
            tracing::warn!("Failed to persist satellite registry: {:?}", e);
        }

        tracing::info!(
            "Refresh complete: {} active, {} total",
            self.registry.active().len(),
            self.registry.len()
        );

        Ok(())
    }

    /// Fetch the upstream label list, falling back to the curated file.
    async fn scrape_labels(&self) -> Vec<String> {
        let scraper = SatelliteScraper::new();

        match scraper.fetch_labels().await {
            Ok(labels) if !labels.is_empty() => labels,
            Ok(_) => {
                tracing::warn!("Upstream returned an empty satellite list");
                Vec::new()
            }
            Err(e) => {
                tracing::error!("Failed to scrape satellite list: {:?}", e);
                report_internal(
                    "sat_rev",
                    "Failed to scrape satellite list",
                    &format!("{:?}", e),
                    NewsUrgency::High,
                );
                // Fall back to the curated file so a cold start still works offline.
                self.load_curated_list()
                    .await
                    .map(|list| list.satellites.into_iter().map(|s| s.api_name).collect())
                    .unwrap_or_default()
            }
        }
    }

    /// Apply curated aliases from the TOML file to matching records.
    async fn apply_curated_aliases(&mut self) {
        let Some(list) = self.load_curated_list().await else {
            return;
        };

        let mut applied = 0usize;
        let mut unmatched: Vec<String> = Vec::new();

        for entry in &list.satellites {
            if entry.aliases.is_empty() {
                continue;
            }
            let key = SatKey::from_label(&entry.api_name);
            if self.registry.get(&key).is_some() {
                self.registry.apply_curated_aliases(&key, &entry.aliases);
                applied += 1;
            } else {
                unmatched.push(entry.api_name.clone());
            }
        }

        if applied > 0 {
            tracing::debug!("Applied curated aliases to {} record(s)", applied);
        }
        if !unmatched.is_empty() {
            // Usually satellites that have left the upstream list — informational.
            tracing::debug!(
                "{} curated entry/entries matched no record: {}",
                unmatched.len(),
                unmatched.join(", ")
            );
        }
    }

    /// Fetch reports for every active record and merge them incrementally.
    async fn fetch_reports(&mut self, hours: u64) {
        let targets = self.registry.fetch_targets();
        if targets.is_empty() {
            tracing::warn!("No satellites to fetch reports for");
            return;
        }

        tracing::info!(
            "Fetching AMSAT reports for {} satellite(s) over {}h",
            targets.len(),
            hours
        );

        let results = batch_fetch_satellites(&targets, hours, BATCH_DELAY_MS).await;

        let mut ok = 0u32;
        let mut failed = 0u32;

        for (label, result) in &results {
            // Resolve via the registry so a label renamed mid-cycle still lands on
            // the correct record.
            let Some(key) = self.registry.key_for_label(label) else {
                tracing::debug!("Fetched label {label:?} matches no record; ignoring");
                continue;
            };

            match result {
                Ok(reports) => {
                    ok += 1;
                    self.registry.merge_reports(&key, reports);
                }
                Err(e) => {
                    failed += 1;
                    tracing::warn!("Failed to fetch reports for {}: {}", label, e);
                    self.registry.mark_fetch_failed(&key);
                }
            }
        }

        tracing::info!("Report fetch complete: {} succeeded, {} failed", ok, failed);
    }

    /// Poll every configured overlay source and attach the results.
    ///
    /// Failures are logged and skipped: an overlay is supplementary, so a dead
    /// external feed must never hold up the AMSAT pipeline.
    async fn poll_overlays(&mut self) {
        let Some(provider) = self.asrtu.as_ref() else {
            return;
        };

        let source = provider.source();
        match provider.poll().await {
            Ok(items) => {
                for item in items {
                    let key = item.key();
                    if self.registry.set_overlay(&key, item.overlay) {
                        tracing::debug!("Attached {} overlay to {}", source.as_str(), key);
                    } else {
                        // The provider is pointing at a satellite we do not have.
                        // Worth surfacing: this is the failure the old integration
                        // hid by silently creating an orphan record.
                        tracing::warn!(
                            "{} overlay targets unknown satellite {}; ignoring",
                            source.as_str(),
                            key
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("{} overlay poll failed: {}", source.as_str(), e);
            }
        }
    }

    /// Log drift, alerting only when it looks like an upstream scheme change.
    fn report_drift(&self, drift: &DriftReport) {
        if drift.is_quiet() {
            return;
        }

        tracing::info!("Upstream drift: {}", drift.summary());

        for (key, from, to) in &drift.relabelled {
            tracing::info!("Relabelled {key}: {from:?} -> {to:?}");
        }

        if drift.looks_like_scheme_change(self.registry.len()) {
            report_internal(
                "sat_rev",
                "AMSAT naming scheme may have changed",
                &format!(
                    "{} of {} records were relabelled in a single cycle.",
                    drift.relabelled.len(),
                    self.registry.len()
                ),
                NewsUrgency::High,
            );
        }

        // Unrecognised mode tokens are the other early signal of upstream change.
        let unknown: Vec<String> = self
            .registry
            .active()
            .iter()
            .filter(|r| r.mode_class == Some(naming::ModeClass::Unknown))
            .map(|r| {
                format!(
                    "{} (mode {:?})",
                    r.current_label,
                    r.mode.as_deref().unwrap_or("?")
                )
            })
            .collect();

        if !unknown.is_empty() {
            tracing::warn!(
                "{} label(s) carry unrecognised mode tokens: {}",
                unknown.len(),
                unknown.join(", ")
            );
        }
    }

    /// Read the curated satellite list, if present.
    async fn load_curated_list(&self) -> Option<SatelliteList> {
        let content = tokio::fs::read_to_string(SATELLITE_LIST_CACHE_PATH)
            .await
            .ok()?;
        toml::from_str(&content).ok()
    }

    // ─── Public query API ────────────────────────────────────────────────

    /// Search for records matching `query`, ranked best-first.
    ///
    /// Retired records are excluded, so a satellite that left the upstream list stops
    /// appearing in results without its history being destroyed.
    pub fn lookup(&self, query: &str) -> query::Outcome<'_> {
        query::search(self.registry.active(), query)
    }

    /// Search and return only the matched records, ranked best-first.
    pub fn search(&self, query: &str) -> Vec<&SatRecord> {
        self.lookup(query).entries()
    }

    /// All records, retired ones last.
    pub fn all_entries(&self) -> &[SatRecord] {
        self.registry.records()
    }
}
