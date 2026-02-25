//! # News Module
//!
//! A global, lock-free news reporting and querying system for the backend.
//!
//! ## Design
//!
//! * A **global** `OnceLock` holds an `mpsc::UnboundedSender` and an
//!   `Arc<RwLock<Vec<NewsItem>>>` (the store).
//! * Any module can call [`report()`] / [`report_general()`] /
//!   [`report_internal()`] to submit news **without holding any lock or
//!   reference** — just `use crate::module::news;`.
//! * Any module can call [`query()`] / [`latest()`] to read back stored news
//!   through the same global interface.
//! * A background [`NewsManager`] task drains the channel, stores items,
//!   logs them, and (in the future) triggers push notifications.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! // In main.rs — initialise once:
//! let news_manager = news::NewsManager::init();
//! tokio::spawn(news_manager.run());
//!
//! // Anywhere else:
//! news::report_general("lotw", "LoTW queue updated", "Backlog is 3 days", NewsUrgency::Low);
//! let recent = news::latest(10);
//! ```

pub mod types;
pub use types::*;

use std::sync::{atomic::{AtomicU64, Ordering}, Arc, OnceLock};
use tokio::sync::{mpsc, RwLock};
use chrono::Utc;

// ─── Global state ────────────────────────────────────────────────────

/// Holds the sender half and the shared store, initialised exactly once by
/// [`NewsManager::init()`].
#[derive(Debug)]
struct NewsGlobal {
    tx: mpsc::UnboundedSender<NewsItem>,
    store: Arc<RwLock<Vec<NewsItem>>>,
}

static NEWS: OnceLock<NewsGlobal> = OnceLock::new();

/// Monotonic counter for unique IDs.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Maximum number of items kept in the in-memory store.
/// Oldest items are evicted when this limit is exceeded.
const MAX_STORE_SIZE: usize = 500;

// ─── Public report API (fire-and-forget, no lock needed) ─────────────

/// Submit a [`NewsItem`] to the news system.
///
/// This is non-blocking and will never panic.  If the module has not been
/// initialised yet the item is silently dropped (with a warning log).
pub fn report(mut item: NewsItem) {
    item.id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    item.timestamp = Utc::now();
    if let Some(g) = NEWS.get() {
        let _ = g.tx.send(item);
    } else {
        tracing::warn!("News module not initialised, dropping: {}", item.title);
    }
}

/// Convenience: report a **general** news item.
pub fn report_general(source: &str, title: &str, content: &str, urgency: NewsUrgency) {
    report(NewsItem {
        id: 0,
        news_type: NewsType::General,
        urgency,
        source: source.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        timestamp: Utc::now(),
    });
}

/// Convenience: report a **server-internal** news item.
pub fn report_internal(source: &str, title: &str, content: &str, urgency: NewsUrgency) {
    report(NewsItem {
        id: 0,
        news_type: NewsType::ServerInternal,
        urgency,
        source: source.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        timestamp: Utc::now(),
    });
}

/// Convenience: report an **other** news item.
pub fn report_other(source: &str, title: &str, content: &str, urgency: NewsUrgency) {
    report(NewsItem {
        id: 0,
        news_type: NewsType::Other,
        urgency,
        source: source.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        timestamp: Utc::now(),
    });
}

// ─── Public query API (read from shared store) ───────────────────────

/// Query stored news items by [`NewsQuery`] filter.
///
/// Returns a `Vec` of cloned items sorted newest-first.
/// If the module is not yet initialised, returns an empty Vec.
pub async fn query(q: &NewsQuery) -> Vec<NewsItem> {
    let Some(g) = NEWS.get() else {
        return Vec::new();
    };
    let store = g.store.read().await;
    let mut results: Vec<NewsItem> = store
        .iter()
        .rev() // newest first
        .filter(|item| q.matches(item))
        .cloned()
        .collect();
    if let Some(limit) = q.limit {
        results.truncate(limit);
    }
    results
}

/// Return the `n` most recent news items (all types, all urgencies).
pub async fn latest(n: usize) -> Vec<NewsItem> {
    query(&NewsQuery {
        limit: Some(n),
        ..Default::default()
    })
    .await
}

/// Return the total number of stored items (useful for health checks).
pub async fn count() -> usize {
    match NEWS.get() {
        Some(g) => g.store.read().await.len(),
        None => 0,
    }
}

// ─── NewsManager (background consumer) ───────────────────────────────

/// Background consumer that drains news from the channel, stores them,
/// and dispatches actions based on urgency.
pub struct NewsManager {
    rx: mpsc::UnboundedReceiver<NewsItem>,
    store: Arc<RwLock<Vec<NewsItem>>>,
}

impl NewsManager {
    /// Initialise the global news system and return a `NewsManager` that
    /// must be spawned as a background task.
    ///
    /// # Panics
    /// Panics if called more than once (same semantics as `OnceLock::set`).
    pub fn init() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let store = Arc::new(RwLock::new(Vec::with_capacity(MAX_STORE_SIZE)));
        NEWS.set(NewsGlobal {
            tx,
            store: Arc::clone(&store),
        })
        .expect("NewsManager::init() called more than once");
        tracing::info!("News module initialised (store capacity: {})", MAX_STORE_SIZE);
        Self { rx, store }
    }

    /// Run the consumer loop.  This future never returns unless the sender
    /// side is dropped (i.e. the process is shutting down).
    pub async fn run(mut self) {
        tracing::info!("News manager consumer loop started");
        while let Some(item) = self.rx.recv().await {
            self.handle(item).await;
        }
        tracing::info!("News manager consumer loop exited (channel closed)");
    }

    /// Process a single incoming news item.
    async fn handle(&self, item: NewsItem) {
        // 1. Log according to urgency
        match item.urgency {
            NewsUrgency::High => {
                tracing::warn!("🔴 HIGH-URGENCY NEWS  {}", item);
            }
            NewsUrgency::Medium => {
                tracing::info!("🟡 MEDIUM NEWS  {}", item);
            }
            NewsUrgency::Low => {
                tracing::debug!("🟢 LOW NEWS  {}", item);
            }
        }

        // 2. Store
        {
            let mut store = self.store.write().await;
            store.push(item);
            // Evict oldest items if we exceeded the cap.
            if store.len() > MAX_STORE_SIZE {
                let excess = store.len() - MAX_STORE_SIZE;
                store.drain(..excess);
            }
        }

        // TODO: 3. Push high-urgency items to frontend via gRPC stream
        // TODO: 4. Persist important items to disk / database
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: report + query round-trip.
    ///
    /// Because `OnceLock` is process-global, this test should be run in
    /// isolation (`cargo test -- --test-threads=1`) or via `#[tokio::test]`
    /// if no other test initialises NewsManager in the same process.
    #[tokio::test]
    async fn test_report_and_query() {
        // Init
        let manager = NewsManager::init();
        let handle = tokio::spawn(manager.run());

        // Report a few items
        report_general("test", "Hello", "world", NewsUrgency::Low);
        report_internal("test", "Oops", "disk full", NewsUrgency::High);
        report_other("test", "FYI", "something happened", NewsUrgency::Medium);

        // Give the consumer a moment to drain
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Query all
        let all = latest(100).await;
        assert_eq!(all.len(), 3);
        // newest first
        assert_eq!(all[0].title, "FYI");
        assert_eq!(all[1].title, "Oops");
        assert_eq!(all[2].title, "Hello");

        // Query by type
        let internal = query(&NewsQuery {
            news_type: Some(NewsType::ServerInternal),
            ..Default::default()
        })
        .await;
        assert_eq!(internal.len(), 1);
        assert_eq!(internal[0].title, "Oops");

        // Query by urgency
        let high = query(&NewsQuery {
            min_urgency: Some(NewsUrgency::High),
            ..Default::default()
        })
        .await;
        assert_eq!(high.len(), 1);

        // Query by source
        let from_test = query(&NewsQuery {
            source: Some("test".to_string()),
            ..Default::default()
        })
        .await;
        assert_eq!(from_test.len(), 3);

        // Cleanup
        handle.abort();
    }
}
