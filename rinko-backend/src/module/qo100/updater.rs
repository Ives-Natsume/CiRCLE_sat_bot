///! QO-100 DX Cluster updater
///!
///! Fetches the QO-100 DX Cluster JSON data via AJAX API,
///! parses it, renders it to a PNG, and caches the latest snapshot in memory.

use anyhow::{Context, Result};
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::parser::parse_qo100_json;
use super::types::Qo100Snapshot;
use crate::module::renderer::Qo100Renderer;
use crate::module::IMAGE_CACHE_PATH;

const QO100_CLUSTER_URL: &str = "https://qo100dx.club/cluster/";

/// Shared QO-100 updater – owns the HTTP client, renderer, and in-memory cache.
pub struct Qo100Updater {
    client:   Client,
    renderer: Qo100Renderer,
    /// Most-recently-fetched snapshot (None until first successful fetch)
    snapshot: Arc<RwLock<Option<Qo100Snapshot>>>,
}

impl Qo100Updater {
    pub fn new(image_dir: Option<PathBuf>) -> Self {
        let image_dir = image_dir.unwrap_or_else(|| PathBuf::from(IMAGE_CACHE_PATH));
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("Mozilla/5.0 Rinko-bot/1.0")
                .build()
                .expect("Failed to build reqwest client"),
            renderer: Qo100Renderer::new(image_dir),
            snapshot: Arc::new(RwLock::new(None)),
        }
    }

    /// Return a handle to the shared snapshot (for read-only access from handlers).
    pub fn snapshot_handle(&self) -> Arc<RwLock<Option<Qo100Snapshot>>> {
        self.snapshot.clone()
    }

    /// Fetch → parse → render one cycle.  Returns the path to the PNG.
    pub async fn update(&self) -> Result<PathBuf> {
        tracing::info!("Fetching QO-100 DX Cluster from {}", QO100_CLUSTER_URL);

        // The cluster page loads spots via AJAX; we must send the
        // X-Requested-With header to get JSON instead of HTML.
        let json = self
            .client
            .get(QO100_CLUSTER_URL)
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .await
            .context("Failed to GET QO-100 cluster JSON")?
            .text()
            .await
            .context("Failed to read QO-100 response body")?;

        let snapshot = parse_qo100_json(&json)
            .context("Failed to parse QO-100 JSON")?;

        tracing::info!(
            "QO-100 snapshot: {} spots, fetched at {}",
            snapshot.spots.len(),
            snapshot.fetched_at
        );

        let png_path = self
            .renderer
            .render(&snapshot)
            .await
            .context("Failed to render QO-100 PNG")?;

        // Update in-memory cache
        *self.snapshot.write().await = Some(snapshot);

        Ok(png_path)
    }
}
