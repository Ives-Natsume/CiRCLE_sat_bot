use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

type UserID = String;
type SatelliteID = String;

#[derive(Default)]
pub struct  SubscriptionManager {
    /// user IDs mapping to satellite IDs they are subscribed to
    user_subscriptions: RwLock<HashMap<UserID, HashSet<SatelliteID>>>,

    /// satellite IDs mapping to user IDs that are subscribed to them
    satellite_subscribers: RwLock<HashMap<SatelliteID, HashSet<UserID>>>,
}

impl SubscriptionManager {
    pub fn new() -> Self {
        SubscriptionManager {
            user_subscriptions: RwLock::new(HashMap::new()),
            satellite_subscribers: RwLock::new(HashMap::new()),
        }
    }

    // Subscribe a user to a satellite
    pub async fn subscribe(&self, user_id: &str, satellite_id: &str) {
        let mut user_subs = self.user_subscriptions.write().await;
        let mut sat_subs = self.satellite_subscribers.write().await;

        user_subs
            .entry(user_id.to_string())
            .or_default()
            .insert(satellite_id.to_string());

        sat_subs
            .entry(satellite_id.to_string())
            .or_default()
            .insert(user_id.to_string());
    }

    // Unsubscribe a user from a satellite
    pub async fn unsubscribe(&self, user_id: &str, satellite_id: &str) {
        let mut user_subs = self.user_subscriptions.write().await;
        let mut sat_subs = self.satellite_subscribers.write().await;

        if let Some(sat_set) = user_subs.get_mut(user_id) {
            sat_set.remove(satellite_id);
            if sat_set.is_empty() {
                user_subs.remove(user_id);
            }
        }

        if let Some(user_set) = sat_subs.get_mut(satellite_id) {
            user_set.remove(user_id);
            if user_set.is_empty() {
                sat_subs.remove(satellite_id);
            }
        }
    }

    // Get all satellites a user is subscribed to
    pub async fn get_user_subscriptions(&self, user_id: &str) -> HashSet<SatelliteID> {
        let user_subs = self.user_subscriptions.read().await;
        user_subs.get(user_id).cloned().unwrap_or_default()
    }

    // Get all users subscribed to a satellite
    pub async fn get_satellite_subscribers(&self, satellite_id: &str) -> HashSet<UserID> {
        let sat_subs = self.satellite_subscribers.read().await;
        sat_subs.get(satellite_id).cloned().unwrap_or_default()
    }
}