///! Scheduled task manager - Centralize all periodic tasks
///!
///! This module manages all scheduled background tasks:
///! - Satellite data updates (every 10 minutes)
///! - Image cache cleanup (daily)
///! - Future tasks can be added here

use super::sat::{SatelliteManager, cleanup_old_images};
use super::dx_world::dx_world::DxWorldScraper;
use super::lotw::LotwUpdater;
use super::qo100::Qo100Updater;
use chrono::{DateTime, Timelike, Utc};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Configuration for scheduled tasks
#[derive(Debug, Clone)]
pub struct ScheduledTaskConfig {
    /// Interval for satellite updates (in minutes)
    pub satellite_update_interval_minutes: u64,

    /// Interval for LoTW status updates (in minutes)
    pub lotw_update_interval_minutes: u64,

    /// Interval for QO-100 DX Cluster updates (in minutes)
    pub qo100_update_interval_minutes: u64,
    
    /// Interval for image cleanup (in hours)
    pub image_cleanup_interval_hours: u64,
    
    /// Number of days to keep cached images
    pub image_retention_days: i64,
    
    /// Cache directory for satellite data
    pub cache_dir: String,
    
    /// Perform initial update immediately
    pub perform_initial_update: bool,
}

impl Default for ScheduledTaskConfig {
    fn default() -> Self {
        Self {
            satellite_update_interval_minutes: 15,
            lotw_update_interval_minutes: 60,
            qo100_update_interval_minutes: 10,
            image_cleanup_interval_hours: 24,
            image_retention_days: 2,
            cache_dir: "data/satellite_cache".to_string(),
            perform_initial_update: true,
        }
    }
}

/// Scheduled task manager
pub struct ScheduledTaskManager {
    config: ScheduledTaskConfig,
    satellite_manager: Arc<SatelliteManager>,
    lotw_updater: Arc<LotwUpdater>,
    qo100_updater: Arc<Qo100Updater>,
    task_handles: Vec<JoinHandle<()>>,
}

impl ScheduledTaskManager {
    /// Create a new scheduled task manager
    pub fn new(config: ScheduledTaskConfig, satellite_manager: Arc<SatelliteManager>) -> Self {
        Self {
            config,
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
        
        // Start satellite update task
        let update_handle = self.start_satellite_update_task().await?;
        self.task_handles.push(update_handle);
        
        // Start image cleanup task
        let cleanup_handle = self.start_image_cleanup_task().await?;
        self.task_handles.push(cleanup_handle);
        
        // Start DX World scraper task
        let dx_world_handle = self.start_dx_world_scraper_loop().await?;
        self.task_handles.push(dx_world_handle);

        // Start LoTW update task
        let lotw_handle = self.start_lotw_update_loop().await?;
        self.task_handles.push(lotw_handle);

        // Start QO-100 DX Cluster update task
        let qo100_handle = self.start_qo100_update_loop().await?;
        self.task_handles.push(qo100_handle);
        
        tracing::info!(
            "Started {} scheduled tasks (satellite every {} min, LoTW every {} min, QO-100 every {} min, cleanup every {} hours)",
            self.task_handles.len(),
            self.config.satellite_update_interval_minutes,
            self.config.lotw_update_interval_minutes,
            self.config.qo100_update_interval_minutes,
            self.config.image_cleanup_interval_hours
        );
        
        Ok(())
    }

    /// DX World scraper task loop (fetch every 6 hours)
    pub async fn start_dx_world_scraper_loop(&self) -> anyhow::Result<JoinHandle<()>> {
        tracing::info!("Starting DX World scraper loop (every 6 hours)...");
        let handle = tokio::spawn(async move {
            let scraper = DxWorldScraper::new(None, None);
            loop {
                match scraper.fetch_and_save().await {
                    Ok(_) => tracing::info!("DX World scraper completed successfully"),
                    Err(e) => tracing::error!("DX World scraper failed: {}", e),
                }
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await; // Sleep for 6 hours
            }
        });
        Ok(handle)
    }

    // LOTW update loop (fetch every N minutes)
    pub async fn start_lotw_update_loop(&self) -> anyhow::Result<JoinHandle<()>> {
        let lotw = self.lotw_updater.clone();
        let interval_minutes = self.config.lotw_update_interval_minutes;
        tracing::info!("Starting LoTW update loop (every {} minutes)...", interval_minutes);

        let handle = tokio::spawn(async move {
            // Initial update immediately
            match lotw.update().await {
                Ok(path) => tracing::info!("Initial LoTW update OK → {:?}", path),
                Err(e)   => tracing::error!("Initial LoTW update failed: {}", e),
            }

            loop {
                tokio::time::sleep(Duration::from_secs(interval_minutes * 60)).await;
                match lotw.update().await {
                    Ok(path) => tracing::info!("LoTW update OK → {:?}", path),
                    Err(e)   => tracing::error!("LoTW update failed: {}", e),
                }
            }
        });

        Ok(handle)
    }

    // QO-100 DX Cluster update loop (fetch every N minutes)
    pub async fn start_qo100_update_loop(&self) -> anyhow::Result<JoinHandle<()>> {
        let qo100 = self.qo100_updater.clone();
        let interval_minutes = self.config.qo100_update_interval_minutes;
        tracing::info!("Starting QO-100 update loop (every {} minutes)...", interval_minutes);

        let handle = tokio::spawn(async move {
            // Initial update immediately
            match qo100.update().await {
                Ok(path) => tracing::info!("Initial QO-100 update OK → {:?}", path),
                Err(e)   => tracing::error!("Initial QO-100 update failed: {}", e),
            }

            loop {
                tokio::time::sleep(Duration::from_secs(interval_minutes * 60)).await;
                match qo100.update().await {
                    Ok(path) => tracing::info!("QO-100 update OK → {:?}", path),
                    Err(e)   => tracing::error!("QO-100 update failed: {}", e),
                }
            }
        });

        Ok(handle)
    }

    /// Start satellite data update task
    async fn start_satellite_update_task(&self) -> anyhow::Result<JoinHandle<()>> {
        let manager = self.satellite_manager.clone();
        let interval_minutes = self.config.satellite_update_interval_minutes;
        let perform_initial = self.config.perform_initial_update;
        
        tracing::info!(
            "Scheduling satellite update task (interval: {} minutes, initial: {})",
            interval_minutes,
            perform_initial
        );
        
        let handle = tokio::spawn(async move {
            // Perform initial update if configured
            if perform_initial {
                tracing::info!("Performing initial satellite update...");
                if let Err(e) = Self::run_satellite_update(&manager).await {
                    tracing::error!("Initial satellite update failed: {}", e);
                }
            }
            
            // Run scheduled updates
            Self::satellite_update_loop(manager, interval_minutes).await;
        });
        
        Ok(handle)
    }

    /// Satellite update loop
    async fn satellite_update_loop(manager: Arc<SatelliteManager>, interval_minutes: u64) {
        loop {
            let now = Utc::now();
            let next_trigger = Self::calculate_next_update_time(now, interval_minutes);
            let sleep_duration = (next_trigger - now)
                .to_std()
                .unwrap_or(Duration::from_secs(60));

            tracing::info!(
                "Next satellite update at: {} (in {:.1} min)",
                next_trigger.format("%Y-%m-%d %H:%M:%S UTC"),
                sleep_duration.as_secs_f64() / 60.0
            );

            tokio::time::sleep(sleep_duration).await;

            // Run update with retries
            const MAX_RETRIES: u32 = 3;
            for attempt in 1..=MAX_RETRIES {
                match Self::run_satellite_update(&manager).await {
                    Ok(_) => {
                        tracing::info!("Satellite update completed successfully");
                        break;
                    }
                    Err(e) => {
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
                }
            }
        }
    }

    /// Calculate the next trigger time for satellite updates.
    ///
    /// B-6 fix: trigger minutes are now derived from `interval_minutes` instead of
    /// being hard-coded to [2, 17, 32, 47] (which implicitly assumed 15-min intervals
    /// regardless of the configured value).
    ///
    /// The sequence starts at offset=2 and advances by `interval_minutes`:
    ///   interval=10  →  [2, 12, 22, 32, 42, 52]
    ///   interval=15  →  [2, 17, 32, 47]
    ///   interval=30  →  [2, 32]
    ///   interval=60  →  [2]
    ///
    /// The +2-minute offset pulls the trigger slightly after the standard TLE
    /// refresh cycle (xx:00, xx:15, …) to avoid hitting the AMSAT API during
    /// peak update traffic.
    fn calculate_next_update_time(now: DateTime<Utc>, interval_minutes: u64) -> DateTime<Utc> {
        let interval = interval_minutes.max(1) as u32;
        let offset: u32 = 2;

        // Build the sorted list of trigger minutes within [0, 60)
        let mut targets: Vec<u32> = (0..)
            .map(|i| offset + i * interval)
            .take_while(|&m| m < 60)
            .collect();
        targets.sort_unstable();

        let current_minute = now.minute();
        let current_hour = now.hour();

        // Find the next trigger minute strictly after the current minute
        if let Some(&target) = targets.iter().find(|&&t| t > current_minute) {
            return now
                .with_minute(target)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap();
        }

        // Wrap to the first trigger minute of the next hour
        let next_hour = if current_hour == 23 {
            now + chrono::Duration::hours(1)
        } else {
            now.with_hour(current_hour + 1).unwrap()
        };

        next_hour
            .with_minute(targets[0])
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    }

    /// Run a single satellite update
    async fn run_satellite_update(manager: &Arc<SatelliteManager>) -> anyhow::Result<()> {
        let timeout_duration = Duration::from_secs(300); // 5 minutes
        
        match tokio::time::timeout(timeout_duration, manager.update_all_satellites()).await {
            Ok(result) => result.map(|report| {
                tracing::info!(
                    "Satellite update: {} total, {} successful, {} failed, {} new",
                    report.total_entries,
                    report.successful_updates,
                    report.failed_updates,
                    report.new_entries.len(),
                );
            }),
            Err(_) => {
                anyhow::bail!("Satellite update timed out after {} seconds", timeout_duration.as_secs());
            }
        }
    }

    /// Start image cleanup task
    async fn start_image_cleanup_task(&self) -> anyhow::Result<JoinHandle<()>> {
        let cache_dir = self.config.cache_dir.clone();
        let interval_hours = self.config.image_cleanup_interval_hours;
        let retention_days = self.config.image_retention_days;
        
        tracing::info!(
            "Scheduling image cleanup task (interval: {} hours, retention: {} days)",
            interval_hours,
            retention_days
        );
        
        let handle = tokio::spawn(async move {
            Self::image_cleanup_loop(cache_dir, interval_hours, retention_days).await;
        });
        
        Ok(handle)
    }

    /// Image cleanup loop
    ///
    /// B-1 fix: sleep until the next scheduled time *first*, then run the cleanup.
    /// The previous implementation ran cleanup at the top of every iteration
    /// (labelled "Initial" but firing on every cycle) AND again after the sleep,
    /// resulting in two cleanup passes per 24-hour period.
    async fn image_cleanup_loop(cache_dir: String, interval_hours: u64, retention_days: i64) {
        loop {
            let now = Utc::now();
            let next_trigger = Self::calculate_next_cleanup_time(now, interval_hours);
            let sleep_duration = (next_trigger - now)
                .to_std()
                .unwrap_or(Duration::from_secs(3600));

            tracing::info!(
                "Next image cleanup at: {} (in {:.1} hours)",
                next_trigger.format("%Y-%m-%d %H:%M:%S UTC"),
                sleep_duration.as_secs_f64() / 3600.0
            );

            tokio::time::sleep(sleep_duration).await;

            // Run image cleanup
            match Self::run_image_cleanup(&cache_dir, retention_days).await {
                Ok(deleted_count) => {
                    if deleted_count > 0 {
                        tracing::info!("Image cleanup completed: deleted {} old images", deleted_count);
                    } else {
                        tracing::debug!("Image cleanup completed: no old images to delete");
                    }
                }
                Err(e) => {
                    tracing::error!("Image cleanup failed: {}", e);
                }
            }

            // Run DX World file cleanup
            match super::dx_world::dx_world::cleanup_old_dx_world_files(
                &std::path::PathBuf::from("data/dx_world"),
                retention_days,
            ).await {
                Ok(deleted_count) => {
                    if deleted_count > 0 {
                        tracing::info!("DX World file cleanup completed: deleted {} old files", deleted_count);
                    } else {
                        tracing::debug!("DX World file cleanup completed: no old files to delete");
                    }
                }
                Err(e) => {
                    tracing::error!("DX World file cleanup failed: {}", e);
                }
            }
        }
    }

    /// Calculate next cleanup time (daily at 03:00 UTC)
    fn calculate_next_cleanup_time(now: DateTime<Utc>, _interval_hours: u64) -> DateTime<Utc> {
        let target_hour = 3; // 3 AM UTC = 11 AM BJT
        let current_hour = now.hour();

        if current_hour < target_hour {
            // Today at 03:00
            now.with_hour(target_hour)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap()
        } else {
            // Tomorrow at 03:00
            (now + chrono::Duration::days(1))
                .with_hour(target_hour)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap()
        }
    }

    /// Run image cleanup
    async fn run_image_cleanup(cache_dir: &str, retention_days: i64) -> anyhow::Result<usize> {
        use std::path::Path;
        
        let cache_path = Path::new(cache_dir);
        let deleted_count = cleanup_old_images(cache_path, retention_days).await?;
        
        Ok(deleted_count)
    }

    /// Gracefully shutdown all tasks
    pub async fn shutdown(self) {
        tracing::info!("Shutting down scheduled task manager...");
        
        for handle in self.task_handles {
            handle.abort();
        }
        
        tracing::info!("All scheduled tasks stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn test_calculate_next_update_time() {
        // interval=15 → triggers at :02, :17, :32, :47

        // At 10:00, next trigger is 10:02 (first minute > 0 in the sequence)
        let now = Utc::now()
            .with_hour(10)
            .unwrap()
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap();
        let next = ScheduledTaskManager::calculate_next_update_time(now, 15);
        assert_eq!(next.minute(), 2);
        assert_eq!(next.hour(), 10);

        // At 10:05, next trigger is 10:17
        let now = now.with_minute(5).unwrap();
        let next = ScheduledTaskManager::calculate_next_update_time(now, 15);
        assert_eq!(next.minute(), 17);
        assert_eq!(next.hour(), 10);

        // At 10:50, all triggers in hour 10 are past → wraps to 11:02
        let now = now.with_minute(50).unwrap();
        let next = ScheduledTaskManager::calculate_next_update_time(now, 15);
        assert_eq!(next.minute(), 2);
        assert_eq!(next.hour(), 11);

        // interval=10 → triggers at :02, :12, :22, :32, :42, :52
        let now = now.with_minute(25).unwrap();
        let next = ScheduledTaskManager::calculate_next_update_time(now, 10);
        assert_eq!(next.minute(), 32);
        assert_eq!(next.hour(), 10);
    }

    #[test]
    #[ignore] // Time-dependent test
    fn test_calculate_next_cleanup_time() {
        // Test at 01:00 - should return today 03:00
        let now = Utc::now()
            .with_hour(1)
            .unwrap()
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap();
        let next = ScheduledTaskManager::calculate_next_cleanup_time(now, 24);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.day(), now.day());

        // Test at 05:00 - should return tomorrow 03:00
        let now = now.with_hour(5).unwrap();
        let next = ScheduledTaskManager::calculate_next_cleanup_time(now, 24);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.day(), now.day() + 1);
    }
}
