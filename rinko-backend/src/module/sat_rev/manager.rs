use crate::module::news::{NewsUrgency, report_internal};
use super::{
    amsat,
    api_client::*,
    types::*,
};
use anyhow::Result;
use chrono::Utc;

#[allow(dead_code)]
const UPDATE_INTERVAL_SECONDS: u64 = 15 * 60; // 15 minutes
const REPORT_FETCH_HOURS: u64 = 24;
const BATCH_DELAY_MS: u64 = 200;

pub struct SatManager {
    /// All AMSAT entries, keyed by api_name for dedup.
    pub entries: Vec<AmsatEntry>,
}

impl SatManager {
    /// Initialise the satellite manager:
    ///
    /// 1. Scrape the satellite name list from the AMSAT website (or load TOML cache).
    /// 2. Build `AmsatEntry` objects from the satellite list.
    /// 3. Batch-fetch the latest AMSAT crowd-sourced reports for every entry.
    pub async fn init() -> Self {
        let mut manager = Self {
            entries: Vec::new(),
        };

        // ── Step 1: Satellite name list ──────────────────────────────────
        let scraper = SatelliteScraper::new();
        if let Err(e) = scraper.scrape_satellite_list().await {
            tracing::error!("Failed to scrape satellite list: {:?}", e);
            report_internal(
                "sat_rev",
                "Failed to scrape satellite list",
                &format!("{:?}", e),
                NewsUrgency::High,
            );
        }

        let sat_list = manager.load_satellite_list_cache().await.unwrap_or_else(|| {
            tracing::warn!("No satellite list cache available");
            SatelliteList { satellites: Vec::new() }
        });

        // ── Step 2: Build AmsatEntry list ────────────────────────────────
        if !sat_list.satellites.is_empty() {
            manager.build_entries(&sat_list);
        }

        // ── Step 3: Batch-fetch AMSAT reports ────────────────────────────
        manager.fetch_all_reports().await;

        tracing::info!(
            "SatManager initialised: {} entries",
            manager.entries.len(),
        );

        manager
    }

    async fn scrape_satellite_list(&mut self) -> Result<()> {
        let scraper = SatelliteScraper::new();
        if let Err(e) = scraper.scrape_satellite_list().await {
            tracing::error!("Failed to scrape satellite list: {:?}", e);
            report_internal(
                "sat_rev",
                "Failed to scrape satellite list",
                &format!("{:?}", e),
                NewsUrgency::High,
            );
        }

        Ok(())
    }

    /// Update AMSAT data:
    /// scrape satellite list, rebuild entries, fetch latest reports.
    pub async fn update_satellite_data(&mut self) -> Result<()> {
        self.scrape_satellite_list().await?;

        let sat_list = self.load_satellite_list_cache().await.unwrap_or_else(|| {
            tracing::warn!("No satellite list cache available");
            SatelliteList { satellites: Vec::new() }
        });

        if !sat_list.satellites.is_empty() {
            self.build_entries(&sat_list);
        }

        self.fetch_all_reports().await;

        tracing::info!(
            "Satellite data updated: {} entries",
            self.entries.len(),
        );

        Ok(())
    }

    // ─── Entry building ──────────────────────────────────────────────────

    /// Create [`AmsatEntry`] objects from the supplied satellite list.
    fn build_entries(&mut self, sat_list: &SatelliteList) {
        self.entries.clear();

        for sat_entry in &sat_list.satellites {
            let mut entry = AmsatEntry::from_api_name(&sat_entry.api_name);

            // Merge human-edited aliases from the TOML cache
            for alias in &sat_entry.aliases {
                if !entry.aliases.contains(alias) {
                    entry.aliases.push(alias.clone());
                }
            }

            self.entries.push(entry);
        }
    }

    // ─── AMSAT report fetching ───────────────────────────────────────────

    /// Batch-fetch AMSAT crowd-sourced reports for **every** entry and store
    /// them as hourly `SatelliteDataBlock` buckets.
    async fn fetch_all_reports(&mut self) {
        let all_names: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.api_name.clone())
            .collect();

        if all_names.is_empty() {
            tracing::warn!("No satellites to fetch reports for");
            return;
        }

        tracing::info!("Fetching AMSAT reports for {} satellites…", all_names.len());

        let results =
            batch_fetch_satellites(&all_names, REPORT_FETCH_HOURS, BATCH_DELAY_MS).await;

        let mut success_count: u32 = 0;
        let mut fail_count: u32 = 0;

        for (name, result) in &results {
            match result {
                Ok(reports) => {
                    success_count += 1;
                    if let Some(entry) = self.find_entry_mut(name) {
                        entry.reports = Self::bucket_reports(reports);
                        entry.last_fetch_success = Some(Utc::now());
                        entry.update_success = true;
                        entry.last_updated = Utc::now();
                    }
                }
                Err(e) => {
                    fail_count += 1;
                    tracing::warn!("Failed to fetch reports for {}: {}", name, e);
                    if let Some(entry) = self.find_entry_mut(name) {
                        entry.update_success = false;
                        entry.last_updated = Utc::now();
                    }
                }
            }
        }

        tracing::info!(
            "AMSAT report fetch complete: {} succeeded, {} failed",
            success_count,
            fail_count,
        );
    }

    /// Find a mutable reference to an [`AmsatEntry`] by its `api_name`.
    fn find_entry_mut(&mut self, api_name: &str) -> Option<&mut AmsatEntry> {
        self.entries.iter_mut().find(|e| e.api_name == api_name)
    }

    /// Group a flat list of [`AmsatReport`] into hourly [`SatelliteDataBlock`]s,
    /// newest block first.
    fn bucket_reports(reports: &[AmsatReport]) -> Vec<SatelliteDataBlock> {
        use chrono::DateTime;
        use std::collections::BTreeMap;

        let mut buckets: BTreeMap<String, Vec<AmsatReport>> = BTreeMap::new();

        for report in reports {
            let hour_key =
                if let Ok(dt) = DateTime::parse_from_rfc3339(&report.reported_time) {
                    dt.format("%Y-%m-%dT%H:00:00Z").to_string()
                } else {
                    "unknown".to_string()
                };
            buckets.entry(hour_key).or_default().push(report.clone());
        }

        // Newest first
        let mut blocks: Vec<SatelliteDataBlock> = buckets
            .into_iter()
            .map(|(time, reports)| SatelliteDataBlock { time, reports })
            .collect();
        blocks.sort_by(|a, b| b.time.cmp(&a.time));
        blocks
    }

    // ─── Cache helpers ───────────────────────────────────────────────────

    /// Try loading the satellite list from the local TOML cache.
    async fn load_satellite_list_cache(&self) -> Option<SatelliteList> {
        let content = tokio::fs::read_to_string(SATELLITE_LIST_CACHE_PATH)
            .await
            .ok()?;
        toml::from_str(&content).ok()
    }

    // ─── Public query API ────────────────────────────────────────────────

    /// Search for entries matching `query`.
    ///
    /// Supports:
    /// - Multi-target queries separated by `/` (e.g. `"iss/so-50"`) — results are
    ///   unioned across all sub-queries.
    /// - Name matching with priority: api_name > base_name > aliases.
    pub fn search(&self, query: &str) -> Vec<&AmsatEntry> {
        let parts: Vec<&str> = query.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();

        if parts.len() > 1 {
            let mut seen = std::collections::HashSet::new();
            let mut combined = Vec::new();
            for part in &parts {
                for entry in self.search_single(part) {
                    if seen.insert(&*entry.api_name as *const str) {
                        combined.push(entry);
                    }
                }
            }
            return combined;
        }

        self.search_single(query)
    }

    /// Core single-query search (no `/` splitting).
    fn search_single(&self, query: &str) -> Vec<&AmsatEntry> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        let normalised = amsat::normalize_for_search(trimmed);

        // ── Priority 1: exact match on api_name ──────────────────────────
        for entry in &self.entries {
            if amsat::normalize_for_search(&entry.api_name) == normalised {
                return vec![entry];
            }
        }

        // ── Priority 2: exact match on satellite_base_name ───────────────
        let results: Vec<&AmsatEntry> = self
            .entries
            .iter()
            .filter(|e| amsat::normalize_for_search(&e.satellite_base_name) == normalised)
            .collect();
        if !results.is_empty() {
            return results;
        }

        // ── Priority 3: match on aliases ─────────────────────────────────
        let results: Vec<&AmsatEntry> = self
            .entries
            .iter()
            .filter(|e| {
                e.aliases
                    .iter()
                    .any(|a| amsat::normalize_for_search(a) == normalised)
            })
            .collect();
        results
    }

    /// Return references to every entry.
    pub fn all_entries(&self) -> &[AmsatEntry] {
        &self.entries
    }
}
