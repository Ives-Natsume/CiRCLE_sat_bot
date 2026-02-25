///! ASRTU telemetry API client.
///!
///! The ASRTU API commonly returns data wrapped in `data_callback(...)`.

use anyhow::{Context, Result};
use chrono::{NaiveDateTime, SecondsFormat, TimeZone, Utc};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_SECONDS: u64 = 2;
const REQUEST_TIMEOUT_SECONDS: u64 = 60;
const CTCSS_ON_VALUE: &str = "0x5A";

#[derive(Debug, Clone)]
pub struct AsrtuRepeaterStatus {
    pub satellite_name: String,
    pub ctcss_value: String,
    pub repeater_on: bool,
    pub observed_at_rfc3339: String,
}

pub async fn fetch_repeater_status(api_url: &str) -> Result<AsrtuRepeaterStatus> {
    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .build()
        .context("Failed to build ASRTU HTTP client")?;

    for attempt in 1..=MAX_RETRIES {
        if attempt > 1 {
            let delay = Duration::from_secs(RETRY_DELAY_SECONDS * attempt as u64);
            tracing::debug!(
                "Retrying ASRTU telemetry request after {:?} (attempt {}/{})",
                delay,
                attempt,
                MAX_RETRIES
            );
            tokio::time::sleep(delay).await;
        }

        match fetch_attempt(&client, api_url).await {
            Ok(status) => return Ok(status),
            Err(err) => {
                if attempt == MAX_RETRIES {
                    return Err(err);
                }
                tracing::warn!(
                    "ASRTU telemetry request attempt {}/{} failed: {}",
                    attempt,
                    MAX_RETRIES,
                    err
                );
            }
        }
    }

    Err(anyhow::anyhow!(
        "ASRTU telemetry request failed after {} attempts",
        MAX_RETRIES
    ))
}

async fn fetch_attempt(client: &Client, api_url: &str) -> Result<AsrtuRepeaterStatus> {
    let response = client
        .get(api_url)
        .send()
        .await
        .context("Failed to send ASRTU telemetry request")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "ASRTU telemetry HTTP error: {}",
            response.status()
        ));
    }

    let text = response
        .text()
        .await
        .context("Failed to read ASRTU telemetry response body")?;

    let json = parse_callback_payload(&text)?;
    extract_ctcss_status(&json)
}

fn parse_callback_payload(raw: &str) -> Result<Value> {
    let trimmed = raw.trim().trim_end_matches(';').trim();

    let json_str = if trimmed.starts_with("data_callback") {
        let start = trimmed
            .find('(')
            .context("ASRTU callback payload missing '('")?;
        let end = trimmed
            .rfind(')')
            .context("ASRTU callback payload missing ')'")?;
        &trimmed[start + 1..end]
    } else {
        trimmed
    };

    serde_json::from_str(json_str).context("Failed to parse ASRTU telemetry JSON payload")
}

fn extract_ctcss_status(root: &Value) -> Result<AsrtuRepeaterStatus> {
    let satellite_name = root
        .get("sat")
        .and_then(|v| v.as_str())
        .unwrap_or("ASRTU-1")
        .to_string();

    let data_obj = root
        .get("data")
        .and_then(|v| v.as_object())
        .context("ASRTU payload missing data object")?;

    let sat_block = data_obj
        .get(&satellite_name)
        .or_else(|| data_obj.values().next())
        .and_then(|v| v.as_object())
        .context("ASRTU payload missing satellite telemetry block")?;

    for (key, item) in sat_block {
        let english_code_name = item
            .get("english_code_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let is_ctcss = english_code_name.eq_ignore_ascii_case("ctcss_en")
            || (key.contains("CTCSS") && key.contains("使能"));

        if !is_ctcss {
            continue;
        }

        let ctcss_value = item
            .get("f_text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                item.get("f_double")
                    .and_then(|v| v.as_i64())
                    .map(|v| format!("{}", v))
                    .unwrap_or_default()
            });

        if ctcss_value.is_empty() {
            return Err(anyhow::anyhow!("ASRTU CTCSS value is empty"));
        }

        let observed_at_rfc3339 = item
            .get("server_receive_time")
            .and_then(|v| v.as_str())
            .and_then(parse_server_receive_time)
            .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));

        let repeater_on = ctcss_value.eq_ignore_ascii_case(CTCSS_ON_VALUE);

        return Ok(AsrtuRepeaterStatus {
            satellite_name,
            ctcss_value,
            repeater_on,
            observed_at_rfc3339,
        });
    }

    Err(anyhow::anyhow!(
        "ASRTU payload does not include CTCSS enable telemetry"
    ))
}

fn parse_server_receive_time(raw: &str) -> Option<String> {
    let naive = NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S").ok()?;
    Some(
        Utc.from_utc_datetime(&naive)
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_callback_payload() {
        let raw = r#"data_callback({"sat":"ASRTU-1","data":{"ASRTU-1":{"CTCSS使能":{"english_code_name":"ctcss_en","f_text":"0x5A","server_receive_time":"2026-02-20 14:41:25"}}}})"#;
        let json = parse_callback_payload(raw).unwrap();
        assert_eq!(json["sat"], "ASRTU-1");
    }

    #[test]
    fn test_extract_ctcss_status_on() {
        let json: Value = serde_json::from_str(
            r#"{"sat":"ASRTU-1","data":{"ASRTU-1":{"CTCSS使能":{"english_code_name":"ctcss_en","f_text":"0x5A","server_receive_time":"2026-02-20 14:41:25"}}}}"#,
        )
        .unwrap();

        let status = extract_ctcss_status(&json).unwrap();
        assert_eq!(status.satellite_name, "ASRTU-1");
        assert_eq!(status.ctcss_value, "0x5A");
        assert!(status.repeater_on);
        assert!(!status.observed_at_rfc3339.is_empty());
    }

    #[test]
    fn test_extract_ctcss_status_off() {
        let json: Value = serde_json::from_str(
            r#"{"sat":"ASRTU-1","data":{"ASRTU-1":{"CTCSS使能":{"english_code_name":"ctcss_en","f_text":"0x00","server_receive_time":"2026-02-20 14:41:25"}}}}"#,
        )
        .unwrap();

        let status = extract_ctcss_status(&json).unwrap();
        assert!(!status.repeater_on);
    }

    #[test]
    fn test_parse_server_receive_time_no_timezone_shift() {
        let parsed = parse_server_receive_time("2026-02-20 14:41:25").unwrap();
        assert_eq!(parsed, "2026-02-20T14:41:25Z");
    }
}
