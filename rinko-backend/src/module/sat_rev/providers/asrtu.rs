//! ASRTU-1 telemetry provider.
//!
//! The ASRTU team exposes an internal endpoint reporting the repeater state their
//! ground station actually commanded. That is stronger evidence than a crowd report,
//! so it is surfaced as an [`Overlay`] rather than merged into the AMSAT report list.
//!
//! # Payload shape
//!
//! The endpoint answers with a JSONP-style wrapper, `data_callback({...})`, holding a
//! per-satellite map of telemetry points. Each point carries both a localised label
//! and an `english_code_name`; the one that matters here is `ctcss_en`, whose value
//! is `0x5A` when the CTCSS encoder — and therefore the repeater — is enabled.
//!
//! The parsing logic is carried over from the previous implementation because it
//! encodes real protocol knowledge. What is **not** carried over is how it attached
//! its result: the old code hard-coded the literal `"AO-123"` as a lookup key while
//! the actual upstream label is `AO-123_[FM]`, so the two never matched and every
//! poll created a fresh orphan record. Here the satellite is named with a
//! [`SatTarget`], which the registry resolves.

use crate::module::sat_rev::identity::SatTarget;
use crate::module::sat_rev::naming::ModeClass;
use crate::module::sat_rev::overlay::{
    Overlay, OverlayPayload, OverlayProvider, OverlaySource, TargetedOverlay,
};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

/// Satellite designator this provider reports on.
const ASRTU_BASE: &str = "AO-123";

/// The AO-123 payload carrying the repeater is its voice transponder.
const ASRTU_CLASS: ModeClass = ModeClass::Voice;

/// CTCSS enable register value meaning "repeater on".
const CTCSS_ON_VALUE: &str = "0x5A";

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_SECONDS: u64 = 2;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// Polls the ASRTU-1 telemetry endpoint.
pub struct AsrtuProvider {
    api_url: String,
    client: Client,
}

impl AsrtuProvider {
    /// Build a provider for the given endpoint.
    ///
    /// Returns [`None`] when the URL is absent or blank, so an unconfigured
    /// deployment simply has no ASRTU overlay rather than a broken one.
    pub fn new(api_url: Option<&str>) -> Option<Self> {
        let url = api_url?.trim();
        if url.is_empty() {
            return None;
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .build()
            .ok()?;

        Some(Self {
            api_url: url.to_string(),
            client,
        })
    }

    /// Fetch and parse the current repeater state, retrying transient failures.
    async fn fetch_state(&self) -> Result<RepeaterState> {
        let mut last_error = None;

        for attempt in 1..=MAX_RETRIES {
            if attempt > 1 {
                tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECONDS * attempt as u64)).await;
            }

            match self.fetch_once().await {
                Ok(state) => return Ok(state),
                Err(e) => {
                    tracing::warn!("ASRTU telemetry attempt {}/{} failed: {}", attempt, MAX_RETRIES, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("ASRTU telemetry failed")))
    }

    /// One request/parse round trip.
    async fn fetch_once(&self) -> Result<RepeaterState> {
        let response = self
            .client
            .get(&self.api_url)
            .send()
            .await
            .context("sending ASRTU telemetry request")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "ASRTU telemetry HTTP {}",
                response.status()
            ));
        }

        let body = response
            .text()
            .await
            .context("reading ASRTU telemetry body")?;

        let json = unwrap_callback(&body)?;
        extract_repeater_state(&json)
    }
}

impl OverlayProvider for AsrtuProvider {
    fn source(&self) -> OverlaySource {
        OverlaySource::Asrtu
    }

    async fn poll(&self) -> Result<Vec<TargetedOverlay>> {
        let state = self.fetch_state().await?;

        Ok(vec![TargetedOverlay {
            target: SatTarget::base_class(ASRTU_BASE, Some(ASRTU_CLASS)),
            overlay: Overlay {
                source: OverlaySource::Asrtu,
                observed_at: state.observed_at,
                fetched_at: Utc::now(),
                payload: OverlayPayload::CommandedState {
                    on: state.repeater_on,
                    detail: format!("CTCSS={}", state.ctcss_value),
                },
            },
        }])
    }
}

/// Parsed repeater telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RepeaterState {
    /// Raw CTCSS register value, kept for the overlay detail string.
    ctcss_value: String,
    /// Whether the repeater is commanded on.
    repeater_on: bool,
    /// When the telemetry server received the frame.
    observed_at: DateTime<Utc>,
}

/// Strip the `data_callback(...)` wrapper and parse the JSON inside.
///
/// Bare JSON is accepted too, so the endpoint can drop the wrapper without breaking
/// this provider.
fn unwrap_callback(raw: &str) -> Result<Value> {
    let trimmed = raw.trim().trim_end_matches(';').trim();

    let json_str = if trimmed.starts_with("data_callback") {
        let start = trimmed.find('(').context("ASRTU payload missing '('")?;
        let end = trimmed.rfind(')').context("ASRTU payload missing ')'")?;
        if end <= start {
            return Err(anyhow::anyhow!("ASRTU payload has malformed parentheses"));
        }
        &trimmed[start + 1..end]
    } else {
        trimmed
    };

    serde_json::from_str(json_str).context("parsing ASRTU telemetry JSON")
}

/// Locate the CTCSS enable point and derive the repeater state.
fn extract_repeater_state(root: &Value) -> Result<RepeaterState> {
    let sat_name = root.get("sat").and_then(|v| v.as_str()).unwrap_or("ASRTU-1");

    let data = root
        .get("data")
        .and_then(|v| v.as_object())
        .context("ASRTU payload missing data object")?;

    // Prefer the block named by `sat`, but fall back to the sole block present:
    // the endpoint has been observed to disagree with itself about the name.
    let block = data
        .get(sat_name)
        .or_else(|| data.values().next())
        .and_then(|v| v.as_object())
        .context("ASRTU payload missing telemetry block")?;

    for (label, item) in block {
        // Match on the stable English code name, with the localised label as a
        // fallback for older payloads.
        let code = item
            .get("english_code_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let is_ctcss = code.eq_ignore_ascii_case("ctcss_en")
            || (label.contains("CTCSS") && label.contains('使'));

        if !is_ctcss {
            continue;
        }

        // `f_text` carries the hex form; fall back to the numeric field.
        let ctcss_value = item
            .get("f_text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                item.get("f_double")
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
            })
            .filter(|s| !s.is_empty())
            .context("ASRTU CTCSS value is empty")?;

        let observed_at = item
            .get("server_receive_time")
            .and_then(|v| v.as_str())
            .and_then(parse_server_time)
            .unwrap_or_else(Utc::now);

        return Ok(RepeaterState {
            repeater_on: ctcss_value.eq_ignore_ascii_case(CTCSS_ON_VALUE),
            ctcss_value,
            observed_at,
        });
    }

    Err(anyhow::anyhow!(
        "ASRTU payload contains no CTCSS enable telemetry"
    ))
}

/// Parse the endpoint's `YYYY-MM-DD HH:MM:SS` timestamp, which is UTC.
fn parse_server_time(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive))
}

/// Format a timestamp the way the endpoint does. Used by tests and diagnostics.
#[allow(dead_code)]
fn format_observed(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::sat_rev::identity::SatKey;

    /// A representative payload, trimmed from a real capture.
    fn payload(ctcss: &str) -> String {
        format!(
            r#"data_callback({{"sat":"ASRTU-1","data":{{"ASRTU-1":{{"CTCSS使能":{{"full_code_name":"TTC-C1","f_double":90,"f_text":"{ctcss}","server_receive_time":"2026-02-20 14:41:25","english_code_name":"ctcss_en"}}}}}}}})"#
        )
    }

    #[test]
    fn unwraps_jsonp_envelope() {
        let json = unwrap_callback(&payload("0x5A")).expect("should unwrap");
        assert_eq!(json["sat"], "ASRTU-1");
    }

    #[test]
    fn accepts_bare_json() {
        let json = unwrap_callback(r#"{"sat":"X","data":{}}"#).expect("bare JSON is valid");
        assert_eq!(json["sat"], "X");
    }

    #[test]
    fn rejects_malformed_envelope() {
        assert!(unwrap_callback("data_callback").is_err());
        assert!(unwrap_callback("not json at all").is_err());
    }

    #[test]
    fn reads_repeater_on() {
        let json = unwrap_callback(&payload("0x5A")).unwrap();
        let state = extract_repeater_state(&json).expect("should parse");
        assert!(state.repeater_on);
        assert_eq!(state.ctcss_value, "0x5A");
    }

    #[test]
    fn reads_repeater_off() {
        let json = unwrap_callback(&payload("0x00")).unwrap();
        let state = extract_repeater_state(&json).unwrap();
        assert!(!state.repeater_on);
    }

    #[test]
    fn parses_server_timestamp_as_utc() {
        let json = unwrap_callback(&payload("0x5A")).unwrap();
        let state = extract_repeater_state(&json).unwrap();
        assert_eq!(
            state.observed_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            "2026-02-20T14:41:25Z"
        );
    }

    /// Older payloads lacked `english_code_name`; the localised label must still work.
    #[test]
    fn falls_back_to_localised_label() {
        let raw = r#"{"sat":"ASRTU-1","data":{"ASRTU-1":{"CTCSS使能":{"f_text":"0x5A","server_receive_time":"2026-02-20 14:41:25"}}}}"#;
        let json = unwrap_callback(raw).unwrap();
        let state = extract_repeater_state(&json).unwrap();
        assert!(state.repeater_on);
    }

    /// The block name is allowed to disagree with the `sat` field.
    #[test]
    fn tolerates_mismatched_block_name() {
        let raw = r#"{"sat":"ASRTU-1","data":{"SOMETHING-ELSE":{"x":{"english_code_name":"ctcss_en","f_text":"0x5A"}}}}"#;
        let json = unwrap_callback(raw).unwrap();
        assert!(extract_repeater_state(&json).is_ok());
    }

    #[test]
    fn errors_when_ctcss_absent() {
        let raw = r#"{"sat":"ASRTU-1","data":{"ASRTU-1":{"other":{"english_code_name":"vbat","f_text":"3600"}}}}"#;
        let json = unwrap_callback(raw).unwrap();
        assert!(extract_repeater_state(&json).is_err());
    }

    /// An unconfigured deployment must yield no provider rather than a broken one.
    #[test]
    fn requires_a_configured_url() {
        assert!(AsrtuProvider::new(None).is_none());
        assert!(AsrtuProvider::new(Some("   ")).is_none());
        assert!(AsrtuProvider::new(Some("https://example.invalid/api")).is_some());
    }

    /// The regression that made the old integration useless: the provider's target
    /// must resolve to the same key the real upstream label produces.
    #[test]
    fn target_matches_the_real_satellite_key() {
        let from_provider = SatTarget::base_class(ASRTU_BASE, Some(ASRTU_CLASS)).resolve();
        assert_eq!(from_provider, SatKey::from_label("AO-123_[FM]"));
        // And explicitly *not* what the old code assumed.
        assert_ne!(from_provider, SatKey::from_label("AO-123"));
    }
}
