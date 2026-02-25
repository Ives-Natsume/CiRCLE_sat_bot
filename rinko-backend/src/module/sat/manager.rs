use crate::module::sat::search::DEFAULT_THRESHOLD;

///! Satellite Manager - Dual-store architecture
///!
///! Primary store: AMSAT entries (keyed by API name)
///! Secondary store: FrequencyDatabase (GitHub CSV metadata, read-only reference)
///!
///! The manager fetches status data from AMSAT API and stores it directly
///! as AmsatEntry objects. Metadata from the CSV database is looked up
///! lazily at render/query time.

use super::{
    api_client, asrtu_client, scraper,
    amsat_types::{AmsatEntry, AsrtuTelemetrySnapshot, normalize_for_search, find_matching_transponder_index},
    types::{AmsatReport, SatelliteDataBlock},
    frequency_db::FrequencyDatabase,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Timelike, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use strsim::jaro_winkler;

const DATA_RETENTION_HOURS: i64 = 48;
const API_REQUEST_DELAY_MS: u64 = 200;
const ASRTU_PRIMARY_API_NAME: &str = "AO-123";
const ASRTU_ALIAS_NAME: &str = "ASRTU-1";
const ASRTU_CALLSIGN: &str = "BJ2CR";

/// Update report
#[derive(Debug, Clone)]
pub struct UpdateReport {
    pub total_entries: usize,
    pub successful_updates: usize,
    pub failed_updates: usize,
    pub new_entries: Vec<String>,
    pub duration_seconds: f64,
}

impl UpdateReport {
    pub fn new() -> Self {
        Self {
            total_entries: 0,
            successful_updates: 0,
            failed_updates: 0,
            new_entries: Vec::new(),
            duration_seconds: 0.0,
        }
    }
}

impl Default for UpdateReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Satellite Manager - dual store architecture
pub struct SatelliteManager {
    /// AMSAT entries indexed by API name (primary data store)
    amsat_entries: Arc<RwLock<HashMap<String, AmsatEntry>>>,

    /// Frequency database from GitHub CSV (metadata reference, read-only after init)
    frequency_db: Arc<FrequencyDatabase>,

    /// Cache directory
    cache_dir: PathBuf,

    /// Update interval (minutes) - stored for reference
    update_interval_minutes: i64,

    /// Optional ASRTU telemetry API endpoint.
    asrtu_api_url: Option<String>,
}

impl SatelliteManager {
    /// Create a new manager
    ///
    /// Downloads CSV from GitHub if not present, loads frequency database.
    pub async fn new(
        cache_dir: impl AsRef<Path>,
        update_interval_minutes: i64,
        asrtu_api_url: Option<String>,
    ) -> Result<Arc<Self>> {
        let cache_dir_path = cache_dir.as_ref();
        let sat_cache_dir = cache_dir_path.join("satellite_cache");
        let csv_path = sat_cache_dir.join("amsat-active-frequencies.csv");

        // Ensure satellite cache directory exists
        tokio::fs::create_dir_all(&sat_cache_dir).await?;

        // Download CSV if it doesn't exist
        if !csv_path.exists() {
            tracing::info!("Downloading CSV database...");
            super::frequency_db::download_csv_from_github(&csv_path).await?;
        }

        Self::new_with_csv(cache_dir, csv_path, update_interval_minutes, asrtu_api_url).await
    }

    /// Create a new manager with custom CSV path
    pub async fn new_with_csv(
        cache_dir: impl AsRef<Path>,
        csv_path: impl AsRef<Path>,
        update_interval_minutes: i64,
        asrtu_api_url: Option<String>,
    ) -> Result<Arc<Self>> {
        let cache_dir = cache_dir.as_ref().to_path_buf();

        // Load frequency database (read-only metadata reference)
        let frequency_db = FrequencyDatabase::load_from_csv(csv_path).await?;
        let db_stats = frequency_db.stats();
        tracing::info!("Loaded frequency database: {}", db_stats);

        Ok(Arc::new(Self {
            amsat_entries: Arc::new(RwLock::new(HashMap::new())),
            frequency_db: Arc::new(frequency_db),
            cache_dir,
            update_interval_minutes,
            asrtu_api_url: asrtu_api_url.and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }),
        }))
    }

    /// Initialize manager - load AMSAT cache, save metadata snapshot
    pub async fn initialize(&self) -> Result<()> {
        tracing::info!("Initializing satellite manager...");

        let sat_cache_dir = self.cache_dir.join("satellite_cache");
        let amsat_cache_path = sat_cache_dir.join("amsat_cache.json");

        // Load AMSAT entries from cache
        if amsat_cache_path.exists() {
            tracing::info!("Loading AMSAT cache from: {}", amsat_cache_path.display());
            let content = tokio::fs::read_to_string(&amsat_cache_path).await?;
            match serde_json::from_str::<HashMap<String, AmsatEntry>>(&content) {
                Ok(cached) => {
                    tracing::info!("Loaded {} AMSAT entries from cache", cached.len());
                    *self.amsat_entries.write().await = cached;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse AMSAT cache, starting fresh: {}", e);
                }
            }
        } else {
            tracing::info!("No AMSAT cache found, will populate on first update");
        }

        // Save metadata snapshot for human inspection
        self.save_metadata_snapshot().await?;

        Ok(())
    }
    /// Update all satellites - fetch from AMSAT API and store as AmsatEntry
    pub async fn update_all_satellites(&self) -> Result<UpdateReport> {
        let start_time = std::time::Instant::now();
        let mut report = UpdateReport::new();

        tracing::info!("Starting satellite data update...");

        // Step 1: Scrape AMSAT website for current satellite list
        let sat_scraper = scraper::SatelliteScraper::new();
        let amsat_list = sat_scraper.scrape_satellite_list().await?;
        let amsat_api_names: Vec<String> = amsat_list
            .satellites
            .iter()
            .map(|s| s.official_name.clone())
            .collect();

        tracing::info!("Found {} AMSAT API names from scraper", amsat_api_names.len());

        // Step 2: Fetch status reports from AMSAT API.
        // Widen the time window per satellite until ≥ 5 individual reports are
        // collected, so the status history is never empty on low-activity birds.
        let fetch_results = api_client::batch_fetch_min_reports(
            &amsat_api_names,
            5, // target: at least 5 individual reports
            API_REQUEST_DELAY_MS,
        )
        .await;

        // Step 3: Merge into AMSAT entries
        {
            let mut entries = self.amsat_entries.write().await;
            report.total_entries = entries.len();

            for (api_name, fetch_result) in fetch_results {
                // Get or create entry
                let entry = entries
                    .entry(api_name.clone())
                    .or_insert_with(|| {
                        report.new_entries.push(api_name.clone());
                        AmsatEntry::from_api_name(&api_name)
                    });

                match fetch_result {
                    Ok(reports) if !reports.is_empty() => {
                        Self::merge_reports(&mut entry.reports, &reports)?;
                        entry.last_fetch_success = Some(Utc::now());
                        entry.update_success = true;
                        report.successful_updates += 1;
                    }
                    Ok(_) => {
                        // Empty response - not an error, just no data
                        entry.update_success = true;
                        report.successful_updates += 1;
                    }
                    Err(e) => {
                        tracing::warn!("Fetch failed for '{}': {}", api_name, e);
                        entry.update_success = false;
                        report.failed_updates += 1;
                    }
                }

                entry.last_updated = Utc::now();
            }

            // Also ensure entries exist for all scraped names (even if fetch wasn't attempted)
            for name in &amsat_api_names {
                entries.entry(name.clone()).or_insert_with(|| {
                    report.new_entries.push(name.clone());
                    AmsatEntry::from_api_name(name)
                });
            }
        } // Write lock dropped

        // Step 4: Merge ASRTU (AO-123) telemetry if configured
        if let Some(asrtu_api_url) = self.asrtu_api_url.as_deref() {
            self.merge_asrtu_status(asrtu_api_url, &mut report).await?;
        }

        // Step 5: Save caches
        self.save_amsat_cache().await?;

        report.total_entries = self.amsat_entries.read().await.len();
        report.duration_seconds = start_time.elapsed().as_secs_f64();

        tracing::info!(
            "Update complete: {} entries, {} successful, {} failed, {} new in {:.2}s",
            report.total_entries,
            report.successful_updates,
            report.failed_updates,
            report.new_entries.len(),
            report.duration_seconds
        );

        Ok(report)
    }

    async fn merge_asrtu_status(&self, asrtu_api_url: &str, report: &mut UpdateReport) -> Result<()> {
        let now = Utc::now();

        match asrtu_client::fetch_repeater_status(asrtu_api_url).await {
            Ok(status) => {
                let mut entries = self.amsat_entries.write().await;
                let mut is_new_entry = false;

                let entry = entries
                    .entry(ASRTU_PRIMARY_API_NAME.to_string())
                    .or_insert_with(|| {
                        is_new_entry = true;
                        AmsatEntry::from_api_name(ASRTU_PRIMARY_API_NAME)
                    });

                Self::ensure_asrtu_aliases(entry);

                if is_new_entry {
                    report.new_entries.push(ASRTU_PRIMARY_API_NAME.to_string());
                }

                // Store hardware-confirmed repeater state.
                // This is kept strictly separate from the crowd-sourced AmsatReport
                // list so that renderers can display it as a distinct authoritative
                // indicator rather than mix it with user observations.
                entry.asrtu_telemetry = Some(AsrtuTelemetrySnapshot {
                    repeater_on: status.repeater_on,
                    ctcss_value: status.ctcss_value.clone(),
                    observed_at: status.observed_at_rfc3339,
                });

                entry.last_updated = now;
                entry.last_fetch_success = Some(now);
                entry.update_success = true;

                report.successful_updates += 1;

                tracing::info!(
                    "Stored ASRTU telemetry for {} as {} (CTCSS={}, repeater_on={})",
                    status.satellite_name,
                    ASRTU_PRIMARY_API_NAME,
                    status.ctcss_value,
                    status.repeater_on,
                );
            }
            Err(err) => {
                tracing::warn!("Failed to fetch ASRTU telemetry: {}", err);

                let mut entries = self.amsat_entries.write().await;
                let mut is_new_entry = false;
                let entry = entries
                    .entry(ASRTU_PRIMARY_API_NAME.to_string())
                    .or_insert_with(|| {
                        is_new_entry = true;
                        AmsatEntry::from_api_name(ASRTU_PRIMARY_API_NAME)
                    });
                Self::ensure_asrtu_aliases(entry);
                entry.last_updated = now;
                entry.update_success = false;

                if is_new_entry {
                    report.new_entries.push(ASRTU_PRIMARY_API_NAME.to_string());
                }

                report.failed_updates += 1;
            }
        }

        Ok(())
    }

    fn ensure_asrtu_aliases(entry: &mut AmsatEntry) {
        let required_aliases = [ASRTU_ALIAS_NAME, "ASRTU1"];

        for alias in required_aliases {
            if !entry
                .aliases
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(alias))
            {
                entry.aliases.push(alias.to_string());
            }
        }
    }

    /// Merge new reports into existing data blocks (static helper)
    fn merge_reports(
        data_blocks: &mut Vec<SatelliteDataBlock>,
        new_reports: &[AmsatReport],
    ) -> Result<()> {
        // Group reports by time block (hourly)
        let mut blocks_map: HashMap<String, Vec<AmsatReport>> = HashMap::new();

        for report in new_reports {
            let time_block = Self::get_time_block(&report.reported_time)?;
            blocks_map
                .entry(time_block)
                .or_default()
                .push(report.clone());
        }

        // Merge with existing blocks
        for (time, reports) in blocks_map {
            if let Some(existing_block) = data_blocks.iter_mut().find(|b| b.time == time) {
                for report in reports {
                    if !existing_block.reports.iter().any(|r| {
                        r.callsign == report.callsign && r.reported_time == report.reported_time
                    }) {
                        existing_block.reports.push(report);
                    }
                }
            } else {
                data_blocks.push(SatelliteDataBlock {
                    time: time.clone(),
                    reports,
                });
            }
        }

        // Sort blocks by time (newest first)
        data_blocks.sort_by(|a, b| b.time.cmp(&a.time));

        // Clean old data
        Self::clean_old_reports(data_blocks);

        Ok(())
    }

    /// Get time block (hourly) from timestamp
    fn get_time_block(timestamp: &str) -> Result<String> {
        let dt = DateTime::parse_from_rfc3339(timestamp)?;
        let dt_utc = dt.with_timezone(&Utc);
        let time_block = dt_utc
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        Ok(time_block.to_rfc3339())
    }

    /// Clean old reports (keep last 48 hours)
    fn clean_old_reports(data_blocks: &mut Vec<SatelliteDataBlock>) {
        let cutoff = Utc::now() - Duration::hours(DATA_RETENTION_HOURS);

        data_blocks.retain(|block| {
            if let Ok(block_time) = DateTime::parse_from_rfc3339(&block.time) {
                block_time.with_timezone(&Utc) >= cutoff
            } else {
                false
            }
        });
    }

    // ─── Cache persistence ───────────────────────────────────────

    /// Save AMSAT cache to disk
    async fn save_amsat_cache(&self) -> Result<()> {
        let sat_cache_dir = self.cache_dir.join("satellite_cache");
        tokio::fs::create_dir_all(&sat_cache_dir).await?;

        let cache_path = sat_cache_dir.join("amsat_cache.json");
        let entries = self.amsat_entries.read().await;

        let json = serde_json::to_string_pretty(&*entries)?;
        tokio::fs::write(&cache_path, json).await?;

        tracing::debug!("Saved {} AMSAT entries to cache", entries.len());
        Ok(())
    }

    /// Save metadata snapshot (frequency database info) for human inspection
    async fn save_metadata_snapshot(&self) -> Result<()> {
        let sat_cache_dir = self.cache_dir.join("satellite_cache");
        tokio::fs::create_dir_all(&sat_cache_dir).await?;

        let snapshot_path = sat_cache_dir.join("metadata_snapshot.json");

        // Build a readable summary
        let all_sats = self.frequency_db.get_all_satellites();
        let summary: Vec<serde_json::Value> = all_sats
            .iter()
            .map(|sat| {
                serde_json::json!({
                    "norad_id": sat.norad_id,
                    "common_name": sat.common_name,
                    "aliases": sat.aliases,
                    "transponder_count": sat.transponders.len(),
                    "transponders": sat.transponders.iter().map(|t| {
                        serde_json::json!({
                            "label": t.label,
                            "amsat_api_name": t.amsat_api_name,
                            "mode": t.mode,
                                    "uplink": t.uplink.to_display(),
                            "downlink": t.downlink.to_display(),
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&summary)?;
        tokio::fs::write(&snapshot_path, json).await?;

        tracing::debug!(
            "Saved metadata snapshot with {} satellites",
            all_sats.len()
        );
        Ok(())
    }

    // ─── Search ──────────────────────────────────────────────────

    /// Search AMSAT entries by query string
    ///
    /// Returns matching entries sorted by relevance.
    /// Search matches against: api_name, aliases, satellite_base_name.
    pub async fn search_amsat_entries(&self, query: &str) -> Vec<AmsatSearchResult> {
        let entries = self.amsat_entries.read().await;
        let entries_vec: Vec<&AmsatEntry> = entries.values().collect();
        search_amsat_entries(query, &entries_vec)
    }

    /// Get all AMSAT entries
    pub async fn get_all_amsat_entries(&self) -> Vec<AmsatEntry> {
        self.amsat_entries.read().await.values().cloned().collect()
    }

    /// Get a single AMSAT entry by API name
    pub async fn get_amsat_entry(&self, api_name: &str) -> Option<AmsatEntry> {
        self.amsat_entries.read().await.get(api_name).cloned()
    }

    /// Lookup metadata from frequency database for a given AMSAT entry
    ///
    /// Tries to find the best-matching Transponder from the CSV database.
    /// Uses `find_matching_transponder_index` for intelligent matching.
    pub fn lookup_metadata(&self, entry: &AmsatEntry) -> Option<TransponderMetadata> {
        // Search frequency database by base name
        let csv_sats = self.frequency_db.find_satellites_by_name(&entry.satellite_base_name);

        for csv_sat in csv_sats {
            // Build (label, mode) pairs for matching
            let labels: Vec<(String, String)> = csv_sat
                .transponders
                .iter()
                .map(|t| (t.label.clone(), t.mode.clone()))
                .collect();

            if let Some(idx) = find_matching_transponder_index(
                entry.mode_hint.as_deref(),
                &labels,
            ) {
                let tp = &csv_sat.transponders[idx];
                return Some(TransponderMetadata {
                    norad_id: csv_sat.norad_id,
                    satellite_name: csv_sat.common_name.clone(),
                    transponder_label: tp.label.clone(),
                    mode: tp.mode.clone(),
                    uplink: tp.uplink.to_display(),
                    downlink: tp.downlink.to_display(),
                });
            }

            // Fallback: if satellite has only one transponder and no mode hint
            if csv_sat.transponders.len() == 1 && entry.mode_hint.is_none() {
                let tp = &csv_sat.transponders[0];
                return Some(TransponderMetadata {
                    norad_id: csv_sat.norad_id,
                    satellite_name: csv_sat.common_name.clone(),
                    transponder_label: tp.label.clone(),
                    mode: tp.mode.clone(),
                    uplink: tp.uplink.to_display(),
                    downlink: tp.downlink.to_display(),
                });
            }
        }

        None
    }

    /// Get cache directory
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Get frequency database reference (for renderer etc.)
    pub fn frequency_db(&self) -> &FrequencyDatabase {
        &self.frequency_db
    }
}

// ─── Search types and functions ─────────────────────────────────────

/// Metadata from frequency database (CSV) for a transponder
#[derive(Debug, Clone)]
pub struct TransponderMetadata {
    pub norad_id: u32,
    pub satellite_name: String,
    pub transponder_label: String,
    pub mode: String,
    pub uplink: String,
    pub downlink: String,
}

/// Search result wrapping an AMSAT entry with match info
#[derive(Debug, Clone)]
pub struct AmsatSearchResult {
    pub entry: AmsatEntry,
    pub match_type: AmsatMatchType,
    pub score: f64,
}

/// How the search matched
#[derive(Debug, Clone, PartialEq)]
pub enum AmsatMatchType {
    /// Exact match on API name
    ExactApiName,
    /// Match on base satellite name (e.g., "ISS" matches ISS-FM, ISS-SSTV)
    BaseName,
    /// Substring/contains match
    Contains,
    /// Fuzzy match (Jaro-Winkler)
    Fuzzy,
}

/// Search AMSAT entries by query
fn search_amsat_entries(query: &str, entries: &[&AmsatEntry]) -> Vec<AmsatSearchResult> {
    let query_normalized = normalize_for_search(query);

    if query_normalized.is_empty() {
        return Vec::new();
    }

    let mut results: Vec<AmsatSearchResult> = Vec::new();

    // Phase 1: Exact match on API name
    for entry in entries {
        if let Some(result) = exact_match_amsat(entry, &query_normalized) {
            results.push(result);
        }
    }
    if !results.is_empty() {
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        return results;
    }

    // Phase 2: Base name match (e.g., "iss" → ISS-FM, ISS-SSTV, ISS-DATA)
    for entry in entries {
        if let Some(result) = base_name_match(entry, &query_normalized) {
            results.push(result);
        }
    }
    if !results.is_empty() {
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        return results;
    }

    // Phase 3: Contains match
    for entry in entries {
        if let Some(result) = contains_match(entry, &query_normalized) {
            results.push(result);
        }
    }
    if !results.is_empty() {
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        return results;
    }

    // Phase 4: Fuzzy match (Jaro-Winkler)
    let fuzzy_threshold = DEFAULT_THRESHOLD;
    for entry in entries {
        if let Some(result) = fuzzy_match_amsat(entry, &query_normalized, fuzzy_threshold) {
            results.push(result);
        }
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    results
}

/// Check exact match on API name or aliases
fn exact_match_amsat(entry: &AmsatEntry, query_normalized: &str) -> Option<AmsatSearchResult> {
    let api_normalized = normalize_for_search(&entry.api_name);

    if api_normalized == query_normalized {
        return Some(AmsatSearchResult {
            entry: (*entry).clone(),
            match_type: AmsatMatchType::ExactApiName,
            score: 1.0,
        });
    }

    // Check aliases
    for alias in &entry.aliases {
        let alias_normalized = normalize_for_search(alias);
        if alias_normalized == query_normalized {
            return Some(AmsatSearchResult {
                entry: (*entry).clone(),
                match_type: AmsatMatchType::ExactApiName,
                score: 0.99,
            });
        }
    }

    None
}

/// Check base name match
fn base_name_match(entry: &AmsatEntry, query_normalized: &str) -> Option<AmsatSearchResult> {
    let base_normalized = normalize_for_search(&entry.satellite_base_name);

    if base_normalized == query_normalized {
        return Some(AmsatSearchResult {
            entry: (*entry).clone(),
            match_type: AmsatMatchType::BaseName,
            score: 0.95,
        });
    }

    None
}

/// Check contains match
fn contains_match(entry: &AmsatEntry, query_normalized: &str) -> Option<AmsatSearchResult> {
    let api_normalized = normalize_for_search(&entry.api_name);

    if api_normalized.contains(query_normalized) || query_normalized.contains(&api_normalized) {
        let score = query_normalized.len() as f64 / api_normalized.len().max(1) as f64;
        if api_normalized.contains("fm") && query_normalized.contains("fm") {
            return Some(AmsatSearchResult {
                entry: (*entry).clone(),
                match_type: AmsatMatchType::Contains,
                score: 0.98,
            });
        }
        return Some(AmsatSearchResult {
            entry: (*entry).clone(),
            match_type: AmsatMatchType::Contains,
            score: score.min(0.90),
        });
    }

    // Check aliases
    for alias in &entry.aliases {
        let alias_normalized = normalize_for_search(alias);
        if alias_normalized.contains(query_normalized) || query_normalized.contains(&alias_normalized)
        {
            let score = query_normalized.len() as f64 / alias_normalized.len().max(1) as f64;
            return Some(AmsatSearchResult {
                entry: (*entry).clone(),
                match_type: AmsatMatchType::Contains,
                score: score.min(0.89),
            });
        }
    }

    None
}

/// Fuzzy match using Jaro-Winkler similarity
fn fuzzy_match_amsat(
    entry: &AmsatEntry,
    query_normalized: &str,
    threshold: f64,
) -> Option<AmsatSearchResult> {
    let api_normalized = normalize_for_search(&entry.api_name);
    let score = jaro_winkler(&api_normalized, query_normalized);

    if score >= threshold {
        return Some(AmsatSearchResult {
            entry: (*entry).clone(),
            match_type: AmsatMatchType::Fuzzy,
            score,
        });
    }

    // Check aliases
    for alias in &entry.aliases {
        let alias_normalized = normalize_for_search(alias);
        let alias_score = jaro_winkler(&alias_normalized, query_normalized);
        if alias_score >= threshold {
            return Some(AmsatSearchResult {
                entry: (*entry).clone(),
                match_type: AmsatMatchType::Fuzzy,
                score: alias_score,
            });
        }
    }

    // Check base name
    let base_normalized = normalize_for_search(&entry.satellite_base_name);
    let base_score = jaro_winkler(&base_normalized, query_normalized);
    if base_score >= threshold {
        return Some(AmsatSearchResult {
            entry: (*entry).clone(),
            match_type: AmsatMatchType::Fuzzy,
            score: base_score,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_entries() -> Vec<AmsatEntry> {
        vec![
            AmsatEntry::from_api_name("ISS-FM"),
            AmsatEntry::from_api_name("ISS-SSTV"),
            AmsatEntry::from_api_name("ISS-DATA"),
            AmsatEntry::from_api_name("ISS-DATV"),
            AmsatEntry::from_api_name("AO-91"),
            AmsatEntry::from_api_name("AO-92"),
            AmsatEntry::from_api_name("RS-44"),
            AmsatEntry::from_api_name("FO-118[H/u]"),
            {
                let mut asrtu = AmsatEntry::from_api_name("AO-123");
                asrtu.aliases.push("ASRTU-1".to_string());
                asrtu
            },
        ]
    }

    #[test]
    fn test_search_exact_api_name() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("ISS-FM", &refs);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.api_name, "ISS-FM");
        assert_eq!(results[0].match_type, AmsatMatchType::ExactApiName);
    }

    #[test]
    fn test_search_exact_case_insensitive() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("iss-fm", &refs);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.api_name, "ISS-FM");
    }

    #[test]
    fn test_search_exact_no_separator() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        // "issfm" should match "ISS-FM" via alias
        let results = search_amsat_entries("issfm", &refs);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.api_name, "ISS-FM");
    }

    #[test]
    fn test_search_base_name_returns_all() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        // "iss" should match all ISS-* entries
        let results = search_amsat_entries("iss", &refs);
        assert_eq!(results.len(), 4);
        let names: Vec<&str> = results.iter().map(|r| r.entry.api_name.as_str()).collect();
        assert!(names.contains(&"ISS-FM"));
        assert!(names.contains(&"ISS-SSTV"));
        assert!(names.contains(&"ISS-DATA"));
        assert!(names.contains(&"ISS-DATV"));
    }

    #[test]
    fn test_search_ao91() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("ao91", &refs);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.api_name, "AO-91");
    }

    #[test]
    fn test_search_no_match() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("ZZZZZ", &refs);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_bracket_name() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("FO-118", &refs);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.api_name, "FO-118[H/u]");
    }

    #[test]
    fn test_search_empty_query() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("", &refs);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_asrtu_alias() {
        let entries = make_test_entries();
        let refs: Vec<&AmsatEntry> = entries.iter().collect();

        let results = search_amsat_entries("Asrtu-1", &refs);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.api_name, "AO-123");
    }

    #[test]
    fn test_update_report_default() {
        let report = UpdateReport::default();
        assert_eq!(report.total_entries, 0);
        assert_eq!(report.new_entries.len(), 0);
    }
}
