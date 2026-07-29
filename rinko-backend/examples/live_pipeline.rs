//! End-to-end smoke run against the live AMSAT service.
//!
//! Exercises the whole path — scrape, parse, reconcile, fetch reports, persist,
//! search, render — and writes real PNGs so the output can be inspected by eye.
//! Unit tests cannot catch layout regressions, unreadable colour choices or a
//! template placeholder that was never substituted; this can.
//!
//! Run with:
//! ```text
//! cargo run -p rinko-backend --no-default-features --example live_pipeline
//! ```
//!
//! Requires network access and must run from the `rinko-backend` directory (or via
//! `-p`, which sets the working directory correctly), since template and data paths
//! are relative.

use rinko_backend::module::renderer::SatelliteRenderer;
use rinko_backend::module::sat_rev::identity::SatKey;
use rinko_backend::module::sat_rev::naming::ModeClass;
use rinko_backend::module::sat_rev::overlay::{Overlay, OverlayPayload, OverlaySource};
use rinko_backend::module::sat_rev::query::{self, Outcome};
use rinko_backend::module::sat_rev::registry::{SatRegistry, RETENTION_HOURS};
use rinko_backend::module::sat_rev::store::RegistryStore;
use rinko_backend::module::sat_rev::api_client::{batch_fetch_satellites, SatelliteScraper};

/// Output directory for the smoke-run images.
const OUT_DIR: &str = "data/image_cache";

/// Satellites polled for reports. Kept small so a smoke run stays quick and does not
/// hammer the upstream API.
const SAMPLE_SIZE: usize = 8;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    step("1. Scraping upstream label list");
    let scraper = SatelliteScraper::new();
    let labels = scraper.fetch_labels().await?;
    println!("   fetched {} labels", labels.len());
    for label in labels.iter().take(5) {
        println!("     {label}");
    }

    step("2. Reconciling into the registry");
    let mut registry = SatRegistry::new();
    let drift = registry.observe_upstream(&labels);
    println!("   {}", drift.summary());
    println!("   {} active record(s)", registry.active().len());

    step("3. Parse and classification coverage");
    let mut unknown = Vec::new();
    let mut by_class: std::collections::BTreeMap<String, usize> = Default::default();
    for record in registry.active() {
        match record.mode_class {
            Some(ModeClass::Unknown) => unknown.push(record.current_label.clone()),
            Some(c) => *by_class.entry(c.as_str().to_string()).or_default() += 1,
            None => *by_class.entry("(none)".to_string()).or_default() += 1,
        }
    }
    for (class, count) in &by_class {
        println!("   {class:<12} {count}");
    }
    if unknown.is_empty() {
        println!("   all mode tokens classified");
    } else {
        println!("   !! {} unclassified: {:?}", unknown.len(), unknown);
    }

    step("4. Sanity-checking derived aliases");
    for record in registry.active().iter().take(5) {
        println!(
            "   {:<24} -> {:?}",
            record.current_label,
            record.aliases
        );
    }

    step("5. Fetching reports for a sample");
    let targets: Vec<String> = registry
        .fetch_targets()
        .into_iter()
        .take(SAMPLE_SIZE)
        .collect();
    println!("   polling {} satellite(s)", targets.len());

    let results = batch_fetch_satellites(&targets, 24, 200).await;
    let mut with_data = Vec::new();
    for (label, result) in &results {
        let Some(key) = registry.key_for_label(label) else {
            continue;
        };
        match result {
            Ok(reports) => {
                registry.merge_reports(&key, reports);
                if !reports.is_empty() {
                    with_data.push(label.clone());
                }
                println!("   {:<24} {} report(s)", label, reports.len());
            }
            Err(e) => {
                registry.mark_fetch_failed(&key);
                println!("   {:<24} FAILED: {}", label, e);
            }
        }
    }

    step("6. Retention");
    let dropped = registry.prune(RETENTION_HOURS);
    println!("   pruned {dropped} stale bucket(s)");

    step("7. Persistence round-trip");
    let store = RegistryStore::at("data/sat_registry_smoke.json");
    store.save(&registry).await?;
    let reloaded = store
        .load()
        .await
        .ok_or_else(|| anyhow::anyhow!("snapshot failed to reload"))?;
    println!(
        "   saved {} record(s), reloaded {} record(s)",
        registry.len(),
        reloaded.len()
    );
    let before: usize = registry.active().iter().map(|r| r.total_reports()).sum();
    let after: usize = reloaded.active().iter().map(|r| r.total_reports()).sum();
    println!("   reports before {before}, after {after}");
    if before != after {
        println!("   !! report count changed across persistence");
    }

    step("8. Search behaviour");
    let entries = registry.active();
    for q in [
        "ao91", "123", "fm", "sstv", "linear", "iss", "arctic", "ao9l", "zzzqqq",
    ] {
        match query::search(entries, q) {
            Outcome::Hits(hits) => {
                let summary: Vec<String> = hits
                    .iter()
                    .take(3)
                    .map(|h| format!("{} [{}]", h.entry.current_label, h.kind.as_str()))
                    .collect();
                println!("   {:<8} -> {} hit(s): {}", q, hits.len(), summary.join(", "));
            }
            Outcome::Miss(s) => {
                println!("   {:<8} -> miss, suggestions: {:?}", q, s.labels);
            }
        }
    }

    step("9. Rendering");

    // Prefer satellites that actually returned reports, so the image is not empty.
    let render_targets: Vec<String> = if with_data.is_empty() {
        targets.iter().take(2).cloned().collect()
    } else {
        with_data.iter().take(3).cloned().collect()
    };

    let picked: Vec<_> = registry
        .active()
        .iter()
        .filter(|r| render_targets.contains(&r.current_label))
        .collect();
    println!("   rendering {} record(s)", picked.len());

    let renderer = SatelliteRenderer::new(OUT_DIR);
    let path = renderer.render_amsat_results(picked.clone()).await?;
    println!("   wrote {}", path.display());

    step("10. Rendering with an overlay and a broadcast");

    // Attach a synthetic overlay so the strip is exercised even when the live ASRTU
    // endpoint is unavailable.
    let mut overlay_reg = SatRegistry::from_records(registry.active().to_vec());
    if let Some(first) = render_targets.first() {
        if let Some(key) = overlay_reg.key_for_label(first) {
            let applied = overlay_reg.set_overlay(
                &key,
                Overlay {
                    source: OverlaySource::Asrtu,
                    observed_at: chrono::Utc::now() - chrono::Duration::minutes(12),
                    fetched_at: chrono::Utc::now(),
                    payload: OverlayPayload::CommandedState {
                        on: true,
                        detail: "CTCSS enable register = 0x5A".into(),
                    },
                },
            );
            println!("   overlay applied to {first}: {applied}");
        }
    }

    // A deliberately stale overlay, to confirm it is labelled rather than hidden.
    if let Some(second) = render_targets.get(1) {
        if let Some(key) = overlay_reg.key_for_label(second) {
            overlay_reg.set_overlay(
                &key,
                Overlay {
                    source: OverlaySource::Ariss,
                    observed_at: chrono::Utc::now() - chrono::Duration::hours(30),
                    fetched_at: chrono::Utc::now(),
                    payload: OverlayPayload::Announcement {
                        text: "School contact scheduled, repeater off during pass".into(),
                    },
                },
            );
            println!("   stale overlay applied to {second}");
        }
    }

    let picked2: Vec<_> = overlay_reg
        .active()
        .iter()
        .filter(|r| render_targets.contains(&r.current_label))
        .collect();

    let renderer = SatelliteRenderer::new(OUT_DIR)
        .with_broadcast("Rinko maintenance window 2026-08-02 16:00-18:00 UTC");
    let path2 = renderer.render_amsat_results(picked2).await?;
    println!("   wrote {}", path2.display());

    step("11. Empty-result rendering");
    let empty = SatelliteRenderer::new(OUT_DIR).render_amsat_results(Vec::new()).await;
    match empty {
        Ok(p) => println!("   wrote {}", p.display()),
        Err(e) => println!("   !! empty render failed: {e}"),
    }

    // Demonstrate that a provider target lands on a real record, the bug that broke
    // the previous ASRTU integration.
    step("12. Overlay target resolution");
    let target_key = SatKey::compose("AO-123", Some(ModeClass::Voice));
    println!(
        "   SatTarget(AO-123, voice) -> {} ; present upstream: {}",
        target_key,
        registry.get(&target_key).is_some()
    );

    println!("\nDone. Inspect the PNGs in {OUT_DIR}/");
    Ok(())
}

fn step(title: &str) {
    println!("\n=== {title} ===");
}
