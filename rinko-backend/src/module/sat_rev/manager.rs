use crate::module::{IMAGE_CACHE_PATH, news::{NewsUrgency, report_internal}};
use super::{
    amsat,
    api_client::*,
    types::*,
};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

#[allow(dead_code)]
const UPDATE_INTERVAL_SECONDS: u64 = 15 * 60; // 15 minutes
const REPORT_FETCH_HOURS: u64 = 24;
const BATCH_DELAY_MS: u64 = 200;

pub struct SatManager {
    /// Primary index: NORAD ID → list of AmsatEntry.
    /// e.g. 25544 → [ISS-FM, ISS-SSTV, ISS-DATA, ISS-DATV]
    pub satellite_map: HashMap<NoradId, Vec<AmsatEntry>>,

    /// Fallback list for AMSAT entries that couldn't be matched to any NORAD ID.
    pub amsat_list: Vec<AmsatEntry>,

    /// Full frequency / transponder database parsed from the CSV.
    pub frequency_db: Vec<TransponderInfo>,
}

impl SatManager {
    /// Initialise the satellite manager:
    ///
    /// 1. Scrape the satellite name list from the AMSAT website (or load TOML cache).
    /// 2. Download CSV transponder metadata (or load local cache).
    /// 3. Build `AmsatEntry` objects, attach matching `TransponderInfo`, and index
    ///    by NORAD ID where a mapping can be resolved.
    /// 4. Batch-fetch the latest AMSAT crowd-sourced reports for every entry.
    pub async fn init() -> Self {
        let mut manager = Self {
            satellite_map: HashMap::new(),
            amsat_list: Vec::new(),
            frequency_db: Vec::new(),
        };

        // ── Step 1: Satellite name list ──────────────────────────────────
        // Scraping updates the TOML cache (merging new names with human edits).
        // We always read back from the cache so that aliases / modes / tags
        // edited by hand are preserved.
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

        // ── Step 2: CSV transponder metadata ─────────────────────────────
        let csv_content = match fetch_satellite_metadata().await {
            Ok(_) => {
                tracing::debug!("Successfully fetched satellite metadata");
                match tokio::fs::read_to_string(METADATA_CACHE_PATH).await {
                    Ok(content) => content,
                    Err(e) => {
                        tracing::error!("Failed to read satellite metadata cache: {:?}", e);
                        report_internal(
                            "sat_rev",
                            "Failed to read satellite metadata cache",
                            &format!("{:?}", e),
                            NewsUrgency::High,
                        );
                        String::new()
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to fetch satellite metadata: {:?}", e);
                report_internal(
                    "sat_rev",
                    "Failed to fetch satellite metadata",
                    &format!("{:?}", e),
                    NewsUrgency::High,
                );
                // Try local cache as fallback
                tokio::fs::read_to_string(METADATA_CACHE_PATH)
                    .await
                    .unwrap_or_default()
            }
        };

        if let Err(e) = manager.parse_csv_data(&csv_content) {
            tracing::error!("Failed to parse satellite metadata CSV: {:?}", e);
        }

        // ── Step 3: Build AmsatEntry list & map to NORAD IDs ─────────────
        if !sat_list.satellites.is_empty() {
            manager.build_entries(&sat_list);
        }

        // ── Step 4: Batch-fetch AMSAT reports ────────────────────────────
        manager.fetch_all_reports().await;

        tracing::info!(
            "SatManager initialised: {} NORAD-mapped entries, {} unmapped, {} transponder records",
            manager.satellite_map.values().map(|v| v.len()).sum::<usize>(),
            manager.amsat_list.len(),
            manager.frequency_db.len(),
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


    /// Update `frequency_db` with the latest CSV data
    pub async fn update_frequency_db(&mut self) -> Result<()> {
        let csv_content = match fetch_satellite_metadata().await {
            Ok(_) => {
                tracing::debug!("Successfully fetched satellite metadata");
                match tokio::fs::read_to_string(METADATA_CACHE_PATH).await {
                    Ok(content) => content,
                    Err(e) => {
                        tracing::error!("Failed to read satellite metadata cache: {:?}", e);
                        String::new()
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to fetch satellite metadata: {:?}", e);
                // Try local cache as fallback
                tokio::fs::read_to_string(METADATA_CACHE_PATH)
                    .await
                    .unwrap_or_default()
            }
        };

        if let Err(e) = self.parse_csv_data(&csv_content) {
            tracing::error!("Failed to parse satellite metadata CSV: {:?}", e);
        }

        Ok(())
    }

    /// Update AMSAT data
    /// includes scraping satellite list, frequency db update and fetching latest reports
    /// executed in high frequency
    pub async fn update_satellite_data(&mut self) -> Result<()> {
        // Scrape latest satellite list
        self.scrape_satellite_list().await?;

        // Update frequency DB with latest CSV data
        self.update_frequency_db().await?;

        // Build entries
        let sat_list = self.load_satellite_list_cache().await.unwrap_or_else(|| {
            tracing::warn!("No satellite list cache available");
            SatelliteList { satellites: Vec::new() }
        });

        if !sat_list.satellites.is_empty() {
            self.build_entries(&sat_list);
        }

        self.fetch_all_reports().await;

        tracing::info!(
            "Satellite data updated: {} NORAD-mapped entries, {} unmapped, {} transponder records",
            self.satellite_map.values().map(|v| v.len()).sum::<usize>(),
            self.amsat_list.len(),
            self.frequency_db.len(),
        );

        Ok(())
    }

    // ─── CSV parsing ─────────────────────────────────────────────────────

    /// Parse the raw CSV string into [`TransponderInfo`] records stored in
    /// `self.frequency_db`.
    fn parse_csv_data(&mut self, csv_data: &str) -> Result<()> {
        if csv_data.is_empty() {
            tracing::warn!("Empty CSV data, skipping parse");
            return Ok(());
        }

        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(csv_data.as_bytes());

        let mut row_count: u32 = 0;
        let mut error_count: u32 = 0;

        for result in reader.deserialize::<TransponderInfo>() {
            row_count += 1;
            match result {
                Ok(info) => self.frequency_db.push(info),
                Err(e) => {
                    error_count += 1;
                    tracing::warn!("Error parsing CSV row {}: {}", row_count, e);
                }
            }
        }

        tracing::info!(
            "Parsed CSV: {} rows, {} successful, {} errors",
            row_count,
            row_count.saturating_sub(error_count),
            error_count,
        );
        Ok(())
    }

    // ─── Entry building & NORAD mapping ──────────────────────────────────

    /// Create [`AmsatEntry`] objects from the supplied satellite list, attach
    /// matching transponder info from `self.frequency_db`, and sort every entry
    /// into either `satellite_map` (keyed by NORAD ID) or `amsat_list` (unmapped).
    fn build_entries(&mut self, sat_list: &SatelliteList) {
        // Pre-build lookup: lowercase CSV satellite name → Vec<&TransponderInfo>
        let mut csv_by_name: HashMap<String, Vec<&TransponderInfo>> = HashMap::new();
        for info in &self.frequency_db {
            csv_by_name
                .entry(info.name.to_lowercase())
                .or_default()
                .push(info);
        }

        // Pre-build lookup: lowercase CSV satellite name → first NORAD ID found
        let mut norad_by_name: HashMap<String, NoradId> = HashMap::new();
        for info in &self.frequency_db {
            norad_by_name
                .entry(info.name.to_lowercase())
                .or_insert(info.norad_id);
        }

        for sat_entry in &sat_list.satellites {
            let mut entry = AmsatEntry::from_api_name(&sat_entry.api_name);

            // Merge human-edited aliases from the TOML cache
            for alias in &sat_entry.aliases {
                if !entry.aliases.contains(alias) {
                    entry.aliases.push(alias.clone());
                }
            }

            // Merge human-edited modes and tags from the TOML cache
            if let Some(ref modes) = sat_entry.mode {
                entry.modes = modes.iter().map(|m| m.to_lowercase()).collect();
            }
            if let Some(ref tags) = sat_entry.tag {
                entry.tags = tags.iter().map(|t| t.to_lowercase()).collect();
            }

            let base_lower = entry.satellite_base_name.to_lowercase();

            // Attach transponder info by matching satellite base name → CSV name
            // However the transponder info should also match the tag/mode keywords
            // Or the SatNOGS-ID must matches with the id manually added in tag keywords (if exists)
            // let base_lower = entry.satellite_base_name.to_lowercase();
            // if let Some(transponders) = csv_by_name.get(&base_lower) {
            //     entry.transponder_info =
            //         Some(transponders.iter().map(|t| (*t).clone()).collect());
            // }
            let mut matched_transponders = Vec::new();
            for info in &self.frequency_db {
                let info_satnogs_id = if let Some(id) = info.satnogs_id.clone() {
                    id.to_string().to_lowercase()
                } else {
                    continue;
                };
                let tag_match = entry.tags.iter().any(|t| t.to_lowercase() == info_satnogs_id);
                if tag_match {
                    matched_transponders.push(info.clone());
                }
            }

            if !matched_transponders.is_empty() {
                entry.transponder_info = Some(matched_transponders);
            } else {
                // If still no match, try matching on mode/tag keywords
                let mut keyword_matched_transponders = Vec::new();
                for info in &self.frequency_db {
                    let info_name_lower = info.name.to_lowercase();
                    let mode_match = entry.modes.iter().any(|m| info_name_lower.contains(m.to_lowercase().as_str()));
                    let tag_match = entry.tags.iter().any(|t| info_name_lower.contains(t.to_lowercase().as_str()));
                    if mode_match || tag_match {
                        keyword_matched_transponders.push(info.clone());
                    }
                }

                if !keyword_matched_transponders.is_empty() {
                    entry.transponder_info = Some(keyword_matched_transponders);
                }
            }

            // Finally check for base name match if no mode/tag keyword match found
            if entry.transponder_info.is_none() {
                if let Some(transponders) = csv_by_name.get(&base_lower) {
                    entry.transponder_info =
                        Some(transponders.iter().map(|t| (*t).clone()).collect());
                }
            }

            // Resolve NORAD ID (priority order):
            //   1. catalog_number from TOML (human-edited, highest priority)
            //   2. CSV metadata match by base_name
            let norad_id = sat_entry
                .catalog_number
                .as_ref()
                .and_then(|cn| cn.parse::<NoradId>().ok())
                .or_else(|| norad_by_name.get(&base_lower).copied());

            match norad_id {
                Some(id) => {
                    self.satellite_map.entry(id).or_default().push(entry);
                }
                None => {
                    self.amsat_list.push(entry);
                }
            }
        }
    }

    // ─── AMSAT report fetching ───────────────────────────────────────────

    /// Batch-fetch AMSAT crowd-sourced reports for **every** entry and store
    /// them as hourly `SatelliteDataBlock` buckets.
    async fn fetch_all_reports(&mut self) {
        let all_names: Vec<String> = self
            .satellite_map
            .values()
            .flatten()
            .chain(self.amsat_list.iter())
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

    /// Find a mutable reference to an [`AmsatEntry`] by its `api_name`
    /// (searches both `satellite_map` and `amsat_list`).
    fn find_entry_mut(&mut self, api_name: &str) -> Option<&mut AmsatEntry> {
        for entries in self.satellite_map.values_mut() {
            if let Some(entry) = entries.iter_mut().find(|e| e.api_name == api_name) {
                return Some(entry);
            }
        }
        self.amsat_list.iter_mut().find(|e| e.api_name == api_name)
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

    /// Look up all entries that share a given NORAD ID.
    pub fn get_by_norad_id(&self, norad_id: NoradId) -> Option<&Vec<AmsatEntry>> {
        self.satellite_map.get(&norad_id)
    }

    /// Search for entries matching `query`.
    ///
    /// Supports:
    /// - Multi-target queries separated by `/` (e.g. `"iss/so-50"`) — results are
    ///   unioned across all sub-queries.
    /// - Pure numeric queries are treated as NORAD IDs for a fast HashMap lookup.
    /// - Name matching with priority: api_name > base_name > aliases.
    /// - Mode / tag keyword matching (e.g. `"sstv"` matches entries whose
    ///   `modes` or `tags` contain that keyword).
    pub fn search(&self, query: &str) -> Vec<&AmsatEntry> {
        let parts: Vec<&str> = query.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();

        if parts.len() > 1 {
            // Multi-target: union results, deduplicate by api_name
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

        // ── Fast path: pure numeric → NORAD ID lookup ────────────────────
        if let Ok(norad_id) = trimmed.parse::<NoradId>() {
            if let Some(entries) = self.satellite_map.get(&norad_id) {
                return entries.iter().collect();
            }
            // Fall through — the number might also be part of a sat name
        }

        let normalised = amsat::normalize_for_search(trimmed);

        // ── Priority 1: exact match on api_name ──────────────────────────
        for entry in self.all_entries() {
            if amsat::normalize_for_search(&entry.api_name) == normalised {
                return vec![entry];
            }
        }

        // ── Priority 2: exact match on satellite_base_name ───────────────
        let mut results: Vec<&AmsatEntry> = self
            .all_entries()
            .into_iter()
            .filter(|e| amsat::normalize_for_search(&e.satellite_base_name) == normalised)
            .collect();
        if !results.is_empty() {
            return results;
        }

        // ── Priority 3: match on aliases ─────────────────────────────────
        results = self
            .all_entries()
            .into_iter()
            .filter(|e| {
                e.aliases
                    .iter()
                    .any(|a| amsat::normalize_for_search(a) == normalised)
            })
            .collect();
        if !results.is_empty() {
            return results;
        }

        // ── Priority 4: mode / tag keyword match ─────────────────────────
        let lower = trimmed.to_lowercase();
        results = self
            .all_entries()
            .into_iter()
            .filter(|e| {
                e.modes.iter().any(|m| m == &lower)
                    || e.tags.iter().any(|t| t == &lower)
            })
            .collect();
        results
    }

    /// Return references to every entry (mapped + unmapped).
    pub fn all_entries(&self) -> Vec<&AmsatEntry> {
        self.satellite_map
            .values()
            .flatten()
            .chain(self.amsat_list.iter())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Test the initialisation of the SatManager, including loading from cache and parsing CSV data.
    /// And test the search functionality with various query formats.
    fn test_sat_manager_init_and_search() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = rt.block_on(SatManager::init());
        let mut result: Vec<&AmsatEntry> = Vec::new();

        // Basic sanity checks
        assert!(!manager.frequency_db.is_empty(), "Frequency DB should not be empty");
        assert!(
            !manager.satellite_map.is_empty() || !manager.amsat_list.is_empty(),
            "There should be at least some satellite entries"
        );

        // ── 1. Search by api_name ────────────────────────────────────────
        let iss_entries = manager.search("ISS-FM");
        result.extend(iss_entries.iter());
        assert!(!iss_entries.is_empty(), "Should find entries for ISS-FM");
        assert!(
            iss_entries.iter().any(|e| e.api_name == "ISS-FM"),
            "Should find exact match for ISS-FM"
        );

        // ── 2. Search by base name ───────────────────────────────────────
        let iss_base_entries = manager.search("ISS");
        result.extend(iss_base_entries.iter());
        assert!(!iss_base_entries.is_empty(), "Should find entries for ISS base name");
        assert!(
            iss_base_entries.iter().any(|e| e.satellite_base_name == "ISS"),
            "Should find entry with base name ISS"
        );

        // ── 3. Search by alias ───────────────────────────────────────────
        let iss_alias_entries = manager.search("iss");
        result.extend(iss_alias_entries.iter());
        assert!(!iss_alias_entries.is_empty(), "Should find entries for ISS alias");
        assert!(
            iss_alias_entries.iter().any(|e| {
                e.aliases.iter().any(|a| a.to_lowercase() == "iss")
            }),
            "Should find entry with alias 'iss'"
        );

        // ── 4. Search by mode keyword ────────────────────────────────────
        let sstv_entries = manager.search("sstv");
        result.extend(sstv_entries.iter());
        assert!(
            !sstv_entries.is_empty(),
            "Should find entries with mode 'sstv'"
        );
        assert!(
            sstv_entries.iter().any(|e| e.modes.contains(&"sstv".to_string())),
            "Should find entry whose modes contain 'sstv'"
        );

        // ── 5. Multi-target search with `/` ──────────────────────────────
        let multi_entries = manager.search("ISS-FM/SO-50");
        result.extend(multi_entries.iter());
        assert!(
            multi_entries.iter().any(|e| e.api_name == "ISS-FM"),
            "Multi-search should include ISS-FM"
        );
        // SO-50 might not exist in every test environment, so only assert ISS-FM

        // ── 6. Search by NORAD ID (if any ISS entries are mapped) ────────
        if let Some(entries) = manager.satellite_map.get(&25544) {
            assert!(!entries.is_empty(), "NORAD 25544 should have entries");
            let norad_entries = manager.search("25544");
            result.extend(norad_entries.iter());
            assert!(
                !norad_entries.is_empty(),
                "Should find entries for NORAD ID 25544"
            );
        }

        // Save results to file for manual inspection
        let json_output = serde_json::to_string_pretty(&result).unwrap();
        std::fs::write("data/test_output.json", json_output).expect("Failed to write test output");
    }
}

