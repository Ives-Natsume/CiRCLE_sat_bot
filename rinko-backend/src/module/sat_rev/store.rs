//! Persistence for the satellite registry.
//!
//! # Why this exists
//!
//! Report history was previously held in memory only. Every restart therefore had to
//! re-fetch the full window for every satellite, serially and with a delay between
//! requests — a multi-minute cold start during which `/q` returned nothing at all.
//! Persisting the registry turns that outage into a disk read.
//!
//! # Format
//!
//! A single JSON file holding a schema version plus the records. JSON rather than
//! TOML because the payload is machine-owned and nested; the human-owned curated
//! alias file stays TOML. The two are deliberately separate: ingest writes only here
//! and never touches the curated file, which is what keeps manual edits safe.
//!
//! Writes go to a temporary file and are then renamed over the target, so a crash
//! mid-write cannot leave a truncated snapshot behind.

use super::registry::SatRegistry;
use super::types::SatRecord;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default location of the registry snapshot.
pub const REGISTRY_PATH: &str = "data/sat_registry.json";

/// Current on-disk schema version.
///
/// A snapshot written by a different version is ignored rather than migrated: the
/// registry can always rebuild itself from upstream, so refusing to guess is both
/// simpler and safer than a migration path that is exercised once.
const SCHEMA_VERSION: u32 = 1;

/// Root object of the snapshot file.
#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    /// Schema version; see [`SCHEMA_VERSION`].
    version: u32,
    /// When this snapshot was written, for diagnostics.
    saved_at: chrono::DateTime<chrono::Utc>,
    /// Persisted records.
    records: Vec<SatRecord>,
}

/// Reads and writes registry snapshots.
pub struct RegistryStore {
    path: PathBuf,
}

impl RegistryStore {
    /// Store using the default path.
    pub fn new() -> Self {
        Self {
            path: PathBuf::from(REGISTRY_PATH),
        }
    }

    /// Store at a custom path, for tests.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Load a registry from disk.
    ///
    /// Returns [`None`] when the file is missing, unreadable, malformed or written by
    /// a different schema version. Every one of those cases is recoverable by
    /// re-fetching from upstream, so none is treated as an error.
    pub async fn load(&self) -> Option<SatRegistry> {
        let raw = tokio::fs::read_to_string(&self.path).await.ok()?;

        let snapshot: Snapshot = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "Ignoring unreadable registry snapshot at {}: {}",
                    self.path.display(),
                    e
                );
                return None;
            }
        };

        if snapshot.version != SCHEMA_VERSION {
            tracing::warn!(
                "Ignoring registry snapshot with schema v{} (expected v{}); will rebuild from upstream",
                snapshot.version,
                SCHEMA_VERSION
            );
            return None;
        }

        tracing::info!(
            "Restored {} record(s) from snapshot saved at {}",
            snapshot.records.len(),
            snapshot.saved_at.to_rfc3339()
        );

        Some(SatRegistry::from_records(snapshot.records))
    }

    /// Write the registry to disk atomically.
    pub async fn save(&self, registry: &SatRegistry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating {}", parent.display()))?;
        }

        let snapshot = Snapshot {
            version: SCHEMA_VERSION,
            saved_at: chrono::Utc::now(),
            records: registry.records().to_vec(),
        };

        let json = serde_json::to_string(&snapshot).context("serialising registry snapshot")?;

        // Write-then-rename: a crash mid-write leaves the previous snapshot intact
        // rather than a half-written file.
        let tmp = temp_path(&self.path);
        tokio::fs::write(&tmp, &json)
            .await
            .with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &self.path)
            .await
            .with_context(|| format!("replacing {}", self.path.display()))?;

        tracing::debug!(
            "Saved {} record(s) to {}",
            snapshot.records.len(),
            self.path.display()
        );

        Ok(())
    }
}

impl Default for RegistryStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Sibling temporary path used for atomic replacement.
fn temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    target.with_file_name(name)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::sat_rev::identity::SatKey;
    use crate::module::sat_rev::types::AmsatReport;

    /// Unique scratch path per test, so cases cannot interfere.
    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rinko_registry_{tag}_{}.json", std::process::id()))
    }

    fn report(iso: &str, callsign: &str) -> AmsatReport {
        AmsatReport {
            name: "t".into(),
            reported_time: iso.into(),
            callsign: callsign.into(),
            report: "Heard".into(),
            grid_square: "AA00".into(),
        }
    }

    fn now_iso() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    #[tokio::test]
    async fn missing_file_loads_as_none() {
        let store = RegistryStore::at(scratch("missing"));
        let _ = tokio::fs::remove_file(&store.path).await;
        assert!(store.load().await.is_none());
    }

    /// The core promise: a restart must recover reports rather than re-fetch them.
    #[tokio::test]
    async fn round_trips_records_and_reports() {
        let path = scratch("roundtrip");
        let store = RegistryStore::at(&path);

        let mut reg = SatRegistry::new();
        reg.observe_upstream(&["AO-91_[FM]".to_string(), "RS-44".to_string()]);
        let key = SatKey::from_label("AO-91_[FM]");
        reg.apply_curated_aliases(&key, &["ao91".to_string()]);
        reg.merge_reports(&key, &[report(&now_iso(), "BJ1AAA")]);

        store.save(&reg).await.expect("save should succeed");
        let restored = store.load().await.expect("snapshot should load");

        assert_eq!(restored.len(), 2);
        let record = restored.get(&key).expect("record restored");
        assert_eq!(record.current_label, "AO-91_[FM]");
        assert_eq!(record.total_reports(), 1, "reports must survive a restart");
        assert_eq!(record.manual_aliases, vec!["ao91"]);

        let _ = tokio::fs::remove_file(&path).await;
    }

    /// Historical labels must persist, or a rename would make old names
    /// unsearchable after the next restart.
    #[tokio::test]
    async fn preserves_label_history() {
        let path = scratch("history");
        let store = RegistryStore::at(&path);

        let mut reg = SatRegistry::new();
        reg.observe_upstream(&["AO-7 V/a".to_string()]);
        reg.observe_upstream(&["AO-7_[V/a]".to_string()]);
        store.save(&reg).await.unwrap();

        let restored = store.load().await.unwrap();
        assert!(
            restored.key_for_label("AO-7 V/a").is_some(),
            "old label must still resolve after reload"
        );

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn retired_records_survive_reload() {
        let path = scratch("retired");
        let store = RegistryStore::at(&path);

        let mut reg = SatRegistry::new();
        reg.observe_upstream(&["RS-44".to_string(), "AO-91_[FM]".to_string()]);
        let key = SatKey::from_label("RS-44");
        reg.get_mut(&key).unwrap().retired = true;
        store.save(&reg).await.unwrap();

        let restored = store.load().await.unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.active().len(), 1);
        assert!(restored.get(&key).unwrap().retired);

        let _ = tokio::fs::remove_file(&path).await;
    }

    /// Corrupt input must degrade to a rebuild, not a panic.
    #[tokio::test]
    async fn malformed_snapshot_is_ignored() {
        let path = scratch("malformed");
        tokio::fs::write(&path, "{ not json").await.unwrap();

        let store = RegistryStore::at(&path);
        assert!(store.load().await.is_none());

        let _ = tokio::fs::remove_file(&path).await;
    }

    /// A snapshot from another schema version is discarded rather than guessed at.
    #[tokio::test]
    async fn foreign_schema_version_is_ignored() {
        let path = scratch("version");
        let payload = serde_json::json!({
            "version": SCHEMA_VERSION + 99,
            "saved_at": chrono::Utc::now(),
            "records": []
        });
        tokio::fs::write(&path, payload.to_string()).await.unwrap();

        let store = RegistryStore::at(&path);
        assert!(store.load().await.is_none());

        let _ = tokio::fs::remove_file(&path).await;
    }

    /// Saving twice must leave no stray temporary file behind.
    #[tokio::test]
    async fn save_is_idempotent_and_leaves_no_temp() {
        let path = scratch("idempotent");
        let store = RegistryStore::at(&path);

        let mut reg = SatRegistry::new();
        reg.observe_upstream(&["AO-91_[FM]".to_string()]);

        store.save(&reg).await.unwrap();
        store.save(&reg).await.unwrap();

        assert!(!temp_path(&path).exists(), "temporary file must be renamed away");
        assert_eq!(store.load().await.unwrap().len(), 1);

        let _ = tokio::fs::remove_file(&path).await;
    }
}
