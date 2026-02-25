use chrono::{DateTime, Utc};
use std::fmt;

// ─── News Type ───────────────────────────────────────────────────────

/// Classification of a news item by its nature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewsType {
    /// General news (satellite status changes, DX spots, LoTW updates, etc.)
    General,
    /// Internal server messages (scheduler errors, data-source timeouts, startup events, etc.)
    ServerInternal,
    /// Anything that doesn't fit the above categories.
    Other,
}

impl fmt::Display for NewsType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NewsType::General => write!(f, "General"),
            NewsType::ServerInternal => write!(f, "ServerInternal"),
            NewsType::Other => write!(f, "Other"),
        }
    }
}

// ─── News Urgency ────────────────────────────────────────────────────

/// How urgently a news item should be processed / pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NewsUrgency {
    /// Lowest priority — log only, no push.
    Low = 0,
    /// Medium priority — include in next batch / summary.
    Medium = 1,
    /// Highest priority — push immediately.
    High = 2,
}

impl fmt::Display for NewsUrgency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NewsUrgency::Low => write!(f, "Low"),
            NewsUrgency::Medium => write!(f, "Medium"),
            NewsUrgency::High => write!(f, "High"),
        }
    }
}

// ─── NewsItem ────────────────────────────────────────────────────────

/// A single news item produced by any module in the backend.
#[derive(Debug, Clone)]
pub struct NewsItem {
    /// Unique identifier (auto-generated on creation).
    pub id: u64,
    /// Which category this item falls into.
    pub news_type: NewsType,
    /// How urgently this should be handled.
    pub urgency: NewsUrgency,
    /// Name of the originating module (e.g. `"sat_rev"`, `"lotw"`, `"qo100"`).
    pub source: String,
    /// Short headline.
    pub title: String,
    /// Full content / body text.
    pub content: String,
    /// When the item was created.
    pub timestamp: DateTime<Utc>,
}

impl fmt::Display for NewsItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}][{}][{}] {}: {}",
            self.timestamp.format("%Y-%m-%d %H:%M:%S"),
            self.news_type,
            self.urgency,
            self.source,
            self.title,
        )
    }
}

// ─── Query helpers ───────────────────────────────────────────────────

/// Filter criteria for querying stored news.
///
/// All fields are optional — `None` means "don't filter on this field".
#[derive(Debug, Clone, Default)]
pub struct NewsQuery {
    /// Only return items of this type.
    pub news_type: Option<NewsType>,
    /// Only return items at or above this urgency level.
    pub min_urgency: Option<NewsUrgency>,
    /// Only return items from this source module.
    pub source: Option<String>,
    /// Only return items created at or after this timestamp.
    pub since: Option<DateTime<Utc>>,
    /// Maximum number of items to return (newest first). `None` = unlimited.
    pub limit: Option<usize>,
}

impl NewsQuery {
    /// Returns `true` if `item` matches all non-`None` filter fields.
    pub fn matches(&self, item: &NewsItem) -> bool {
        if let Some(nt) = &self.news_type {
            if item.news_type != *nt {
                return false;
            }
        }
        if let Some(min) = &self.min_urgency {
            if item.urgency < *min {
                return false;
            }
        }
        if let Some(src) = &self.source {
            if item.source != *src {
                return false;
            }
        }
        if let Some(since) = &self.since {
            if item.timestamp < *since {
                return false;
            }
        }
        true
    }
}
