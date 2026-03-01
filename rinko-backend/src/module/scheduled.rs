///! Scheduled task manager - Centralize all periodic tasks
///!
///! Tasks managed:
///! - Satellite data update    (every 15 min)
///! - Image cache cleanup      (every 60 min, deletes files older than 60 min)
///! - DX World cache cleanup   (every 60 min, deletes files older than 60 min)
///! - DX World scraper         (every 6 hours)
///! - QO-100 cluster update    (every 10 min)
///! - LoTW queue update        (every 20 min)

use super::dx_world::dx_world::DxWorldScraper;
use super::lotw::LotwUpdater;
use super::qo100::Qo100Updater;
use super::sat_rev::SatManager;
use crate::module::{DX_WORLD_CACHE_PATH, IMAGE_CACHE_PATH};
use chrono::{NaiveDateTime, Utc};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

// ── Interval constants (seconds) ────────────────────────────────────────────

const SAT_UPDATE_INTERVAL: u64 = 15 * 60;
const IMAGE_CLEANUP_INTERVAL: u64 = 60 * 60;
const DX_WORLD_CLEANUP_INTERVAL: u64 = 60 * 60;
const QO100_UPDATE_INTERVAL: u64 = 10 * 60;
const LOTW_UPDATE_INTERVAL: u64 = 20 * 60;
const DX_WORLD_SCRAPE_INTERVAL: u64 = 6 * 3600;

/// Maximum age (in minutes) before a cached file is considered expired.
const CACHE_MAX_AGE_MINUTES: i64 = 60;

/// Scheduled task manager
pub struct ScheduledTaskManager {
    satellite_manager: Arc<RwLock<SatManager>>,
    lotw_updater: Arc<LotwUpdater>,
    qo100_updater: Arc<Qo100Updater>,
    task_handles: Vec<JoinHandle<()>>,
}

impl ScheduledTaskManager {
    /// Create a new scheduled task manager
    pub fn new(satellite_manager: Arc<RwLock<SatManager>>) -> Self {
        Self {
            satellite_manager,
            lotw_updater: Arc::new(LotwUpdater::new(None)),
            qo100_updater: Arc::new(Qo100Updater::new(None)),
            task_handles: Vec::new(),
        }
    }

    /// Expose the LoTW updater for use by message handlers
    pub fn lotw_updater(&self) -> Arc<LotwUpdater> {
        self.lotw_updater.clone()
    }

    /// Expose the QO-100 updater for use by message handlers
    pub fn qo100_updater(&self) -> Arc<Qo100Updater> {
        self.qo100_updater.clone()
    }

    /// Start all scheduled tasks
    pub async fn start_all(&mut self) -> anyhow::Result<()> {
        tracing::info!("Starting scheduled task manager...");

        self.task_handles
            .push(Self::spawn_satellite_update(self.satellite_manager.clone()));
        self.task_handles
            .push(Self::spawn_image_cache_cleanup());
        self.task_handles
            .push(Self::spawn_dx_world_cleanup());
        self.task_handles
            .push(Self::spawn_dx_world_scraper());
        self.task_handles
            .push(Self::spawn_qo100_update(self.qo100_updater.clone()));
        self.task_handles
            .push(Self::spawn_lotw_update(self.lotw_updater.clone()));

        tracing::info!("All scheduled tasks started successfully");
        Ok(())
    }

    /// Gracefully shutdown all tasks
    pub async fn shutdown(self) {
        tracing::info!("Shutting down scheduled task manager...");
        for handle in self.task_handles {
            handle.abort();
        }
        tracing::info!("All scheduled tasks stopped");
    }

    // ── Task spawners ────────────────────────────────────────────────────

    /// Satellite data update — every 15 min, with retry
    fn spawn_satellite_update(mgr: Arc<RwLock<SatManager>>) -> JoinHandle<()> {
        tracing::info!(
            "Scheduling satellite update task (interval: {}s)",
            SAT_UPDATE_INTERVAL
        );
        tokio::spawn(async move {
            // Perform initial update immediately
            run_satellite_update(&mgr).await;

            loop {
                tokio::time::sleep(Duration::from_secs(SAT_UPDATE_INTERVAL)).await;
                run_satellite_update(&mgr).await;
            }
        })
    }

    /// Image cache cleanup — every 60 min
    fn spawn_image_cache_cleanup() -> JoinHandle<()> {
        tracing::info!(
            "Scheduling image cache cleanup (interval: {}s)",
            IMAGE_CLEANUP_INTERVAL
        );
        tokio::spawn(async move {
            // Initial cleanup on startup
            cleanup_old_images().await;
            loop {
                tokio::time::sleep(Duration::from_secs(IMAGE_CLEANUP_INTERVAL)).await;
                cleanup_old_images().await;
            }
        })
    }

    /// DX World cache cleanup — every 60 min
    fn spawn_dx_world_cleanup() -> JoinHandle<()> {
        tracing::info!(
            "Scheduling DX World cache cleanup (interval: {}s)",
            DX_WORLD_CLEANUP_INTERVAL
        );
        tokio::spawn(async move {
            cleanup_dx_world_cache().await;
            loop {
                tokio::time::sleep(Duration::from_secs(DX_WORLD_CLEANUP_INTERVAL)).await;
                cleanup_dx_world_cache().await;
            }
        })
    }

    /// DX World scraper — every 6 hours
    fn spawn_dx_world_scraper() -> JoinHandle<()> {
        tracing::info!(
            "Scheduling DX World scraper (interval: {}s)",
            DX_WORLD_SCRAPE_INTERVAL
        );
        tokio::spawn(async move {
            let scraper = DxWorldScraper::new(None, None);
            loop {
                match scraper.fetch_and_save().await {
                    Ok(_) => tracing::info!("DX World scraper completed successfully"),
                    Err(e) => tracing::error!("DX World scraper failed: {}", e),
                }
                tokio::time::sleep(Duration::from_secs(DX_WORLD_SCRAPE_INTERVAL)).await;
            }
        })
    }

    /// QO-100 DX Cluster update — every 10 min
    fn spawn_qo100_update(updater: Arc<Qo100Updater>) -> JoinHandle<()> {
        tracing::info!(
            "Scheduling QO-100 update (interval: {}s)",
            QO100_UPDATE_INTERVAL
        );
        tokio::spawn(async move {
            // Initial update
            if let Err(e) = updater.update().await {
                tracing::error!("Initial QO-100 update failed: {}", e);
            }
            loop {
                tokio::time::sleep(Duration::from_secs(QO100_UPDATE_INTERVAL)).await;
                match updater.update().await {
                    Ok(path) => tracing::info!("QO-100 update OK → {:?}", path),
                    Err(e) => tracing::error!("QO-100 update failed: {}", e),
                }
            }
        })
    }

    /// LoTW queue update — every 20 min
    fn spawn_lotw_update(updater: Arc<LotwUpdater>) -> JoinHandle<()> {
        tracing::info!(
            "Scheduling LoTW update (interval: {}s)",
            LOTW_UPDATE_INTERVAL
        );
        tokio::spawn(async move {
            // Initial update
            if let Err(e) = updater.update().await {
                tracing::error!("Initial LoTW update failed: {}", e);
            }
            loop {
                tokio::time::sleep(Duration::from_secs(LOTW_UPDATE_INTERVAL)).await;
                match updater.update().await {
                    Ok(path) => tracing::info!("LoTW update OK → {:?}", path),
                    Err(e) => tracing::error!("LoTW update failed: {}", e),
                }
            }
        })
    }
}

// ── Free helper functions (no &self capture → 'static-safe) ─────────────────

/// Run a single satellite update with up to 3 retries.
async fn run_satellite_update(mgr: &Arc<RwLock<SatManager>>) {
    const MAX_RETRIES: u32 = 3;
    const TIMEOUT_SECS: u64 = 300;

    for attempt in 1..=MAX_RETRIES {
        let result = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), async {
            let mut guard = mgr.write().await;
            guard.update_satellite_data().await
        })
        .await;

        match result {
            Ok(Ok(())) => {
                tracing::info!("Satellite update completed successfully");
                return;
            }
            Ok(Err(e)) => {
                if attempt < MAX_RETRIES {
                    tracing::warn!(
                        "Satellite update failed (attempt {}/{}): {}. Retrying in 60s...",
                        attempt,
                        MAX_RETRIES,
                        e
                    );
                    tokio::time::sleep(Duration::from_secs(60)).await;
                } else {
                    tracing::error!(
                        "Satellite update failed after {} attempts: {}",
                        MAX_RETRIES,
                        e
                    );
                }
            }
            Err(_) => {
                tracing::error!(
                    "Satellite update timed out (attempt {}/{}, {}s limit)",
                    attempt,
                    MAX_RETRIES,
                    TIMEOUT_SECS
                );
                if attempt < MAX_RETRIES {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }
        }
    }
}

/// Clean up expired images in `data/image_cache/`.
///
/// File naming conventions (from renderer.rs):
///   sat_YYYYMMDD_HHMM_xxx.png
///   lotw_YYYYMMDD_HHMM.png
///   qo100_YYYYMMDD_HHMM.png
///
/// Files named `*_latest.png` are always kept.
async fn cleanup_old_images() {
    let Ok(mut entries) = tokio::fs::read_dir(IMAGE_CACHE_PATH).await else {
        tracing::warn!("Cannot read image cache dir: {}", IMAGE_CACHE_PATH);
        return;
    };

    let now = Utc::now();
    let mut deleted = 0u32;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();

        // Always keep *_latest.png convenience copies
        if name.ends_with("_latest.png") {
            continue;
        }

        if !name.ends_with(".png") {
            continue;
        }

        // Try to extract a YYYYMMDD_HHMM timestamp from the filename.
        if let Some(ts) = extract_timestamp_hhmm(&name) {
            let age = now.signed_duration_since(ts.and_utc());
            if age.num_minutes() > CACHE_MAX_AGE_MINUTES {
                let path = entry.path();
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    tracing::warn!("Failed to delete old image {}: {}", path.display(), e);
                } else {
                    deleted += 1;
                }
            }
        }
    }

    if deleted > 0 {
        tracing::info!("Image cache cleanup: deleted {} expired file(s)", deleted);
    }
}

/// Clean up expired files in `data/dx_world/`.
///
/// File naming convention (from dx_world.rs):
///   dxw_timeline_YYYYMMDD_HHMMSS.{html,json,png}
async fn cleanup_dx_world_cache() {
    let Ok(mut entries) = tokio::fs::read_dir(DX_WORLD_CACHE_PATH).await else {
        tracing::warn!("Cannot read DX World cache dir: {}", DX_WORLD_CACHE_PATH);
        return;
    };

    let now = Utc::now();
    let mut deleted = 0u32;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();

        // Expect: dxw_timeline_YYYYMMDD_HHMMSS.ext
        if let Some(ts) = extract_timestamp_hhmmss(&name) {
            let age = now.signed_duration_since(ts.and_utc());
            if age.num_minutes() > CACHE_MAX_AGE_MINUTES {
                let path = entry.path();
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    tracing::warn!("Failed to delete old DX World file {}: {}", path.display(), e);
                } else {
                    deleted += 1;
                }
            }
        }
    }

    if deleted > 0 {
        tracing::info!(
            "DX World cache cleanup: deleted {} expired file(s)",
            deleted
        );
    }
}

/// Extract a `YYYYMMDD_HHMM` timestamp from a filename like
/// `sat_20260301_1215_iss.png` or `lotw_20260301_1200.png`.
///
/// Strategy: find the first pair of underscore-delimited segments that looks
/// like an 8-digit date followed by a ≥4-digit time.
fn extract_timestamp_hhmm(filename: &str) -> Option<NaiveDateTime> {
    let parts: Vec<&str> = filename.split('_').collect();
    for window in parts.windows(2) {
        if window[0].len() == 8 && window[1].len() >= 4 {
            let date_part = window[0];
            let time_part = &window[1][..4];
            if date_part.chars().all(|c| c.is_ascii_digit())
                && time_part.chars().all(|c| c.is_ascii_digit())
            {
                let combined = format!("{}_{}", date_part, time_part);
                if let Ok(ts) = NaiveDateTime::parse_from_str(&combined, "%Y%m%d_%H%M") {
                    return Some(ts);
                }
            }
        }
    }
    None
}

/// Extract a `YYYYMMDD_HHMMSS` timestamp from a filename like
/// `dxw_timeline_20260216_231353.html`.
fn extract_timestamp_hhmmss(filename: &str) -> Option<NaiveDateTime> {
    let parts: Vec<&str> = filename.split('_').collect();
    for window in parts.windows(2) {
        if window[0].len() == 8 && window[1].len() >= 6 {
            let date_part = window[0];
            let time_part_raw = window[1];
            // Strip file extension from time part
            let time_part = time_part_raw.split('.').next().unwrap_or(time_part_raw);
            let time_part = if time_part.len() >= 6 {
                &time_part[..6]
            } else {
                continue;
            };
            if date_part.chars().all(|c| c.is_ascii_digit())
                && time_part.chars().all(|c| c.is_ascii_digit())
            {
                let combined = format!("{}_{}", date_part, time_part);
                if let Ok(ts) = NaiveDateTime::parse_from_str(&combined, "%Y%m%d_%H%M%S") {
                    return Some(ts);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_timestamp_hhmm() {
        // sat renderer: sat_20260301_1215_iss.png
        let ts = extract_timestamp_hhmm("sat_20260301_1215_iss.png").unwrap();
        assert_eq!(ts.to_string(), "2026-03-01 12:15:00");

        // lotw renderer: lotw_20260301_1200.png
        let ts = extract_timestamp_hhmm("lotw_20260301_1200.png").unwrap();
        assert_eq!(ts.to_string(), "2026-03-01 12:00:00");

        // qo100 renderer: qo100_20260301_0930.png
        let ts = extract_timestamp_hhmm("qo100_20260301_0930.png").unwrap();
        assert_eq!(ts.to_string(), "2026-03-01 09:30:00");

        // latest convenience file — should return None
        assert!(extract_timestamp_hhmm("dxw_latest.png").is_none());
        assert!(extract_timestamp_hhmm("lotw_latest.png").is_none());
    }

    #[test]
    fn test_extract_timestamp_hhmmss() {
        let ts = extract_timestamp_hhmmss("dxw_timeline_20260216_231353.html").unwrap();
        assert_eq!(ts.to_string(), "2026-02-16 23:13:53");

        let ts = extract_timestamp_hhmmss("dxw_timeline_20260216_231353.json").unwrap();
        assert_eq!(ts.to_string(), "2026-02-16 23:13:53");

        let ts = extract_timestamp_hhmmss("dxw_timeline_20260216_231353.png").unwrap();
        assert_eq!(ts.to_string(), "2026-02-16 23:13:53");
    }
}