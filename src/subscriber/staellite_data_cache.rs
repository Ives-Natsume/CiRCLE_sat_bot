use crate::sat_status::amsat_parser::SatelliteStatus;
use std::collections::HashMap;
use tokio::sync::RwLock;

pub struct SatelliteDataCache {
    /// Cache for satellite data, mapping satellite IDs to their status
    data: RwLock<HashMap<String, SatelliteStatus>>,
}

impl SatelliteDataCache {
    pub fn new() -> Self {
        SatelliteDataCache {
            data: RwLock::new(HashMap::new()),
        }
    }

    /// Insert or update satellite status in the cache
    pub async fn update(&self, satellite_id: String, status: SatelliteStatus) {
        let mut data = self.data.write().await;
        data.insert(satellite_id, status);
    }

    /// Retrieve satellite status from the cache
    pub async fn get(&self, satellite_id: &str) -> Option<SatelliteStatus> {
        let data = self.data.read().await;
        data.get(satellite_id).cloned()
    }

    /// Check if a satellite is in the cache
    pub async fn contains(&self, satellite_id: &str) -> bool {
        let data = self.data.read().await;
        data.contains_key(satellite_id)
    }
}