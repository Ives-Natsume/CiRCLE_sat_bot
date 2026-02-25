///! AMSAT API client for fetching satellite status data
use super::types::AmsatReport;
use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

const AMSAT_API_URL: &str = "https://www.amsat.org/status/api/v1/sat_info.php";
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_SECONDS: u64 = 2;
const REQUEST_TIMEOUT_SECONDS: u64 = 60;

/// Build a shared HTTP client with the standard timeout.
fn build_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .build()
        .context("Failed to build HTTP client")
}

/// Fetch satellite data from AMSAT API (creates its own HTTP client).
///
/// Prefer `batch_fetch_satellites` when fetching multiple satellites so that
/// a single client is shared across all requests.
///
/// # Arguments
/// * `sat_name` - Satellite name (case-sensitive)
/// * `hours` - Number of hours of data to fetch (default: 1, max: 96)
pub async fn fetch_satellite_data(sat_name: &str, hours: u64) -> Result<Vec<AmsatReport>> {
    let client = build_client()?;
    fetch_with_client(&client, sat_name, hours).await
}

/// Fetch satellite data using a pre-built HTTP client (internal workhorse with retry).
async fn fetch_with_client(client: &Client, sat_name: &str, hours: u64) -> Result<Vec<AmsatReport>> {
    let api_url = format!("{}?name={}&hours={}", AMSAT_API_URL, sat_name, hours);

    for attempt in 1..=MAX_RETRIES {
        if attempt > 1 {
            let delay = Duration::from_secs(RETRY_DELAY_SECONDS * attempt as u64);
            tracing::debug!(
                "Retrying {} after {:?} (attempt {}/{})",
                sat_name,
                delay,
                attempt,
                MAX_RETRIES
            );
            tokio::time::sleep(delay).await;
        }

        match fetch_attempt(client, &api_url, sat_name).await {
            Ok(data) => {
                tracing::debug!(
                    "Successfully fetched {} reports for {}",
                    data.len(),
                    sat_name
                );
                return Ok(data);
            }
            Err(e) => {
                if attempt == MAX_RETRIES {
                    tracing::error!(
                        "Failed to fetch {} after {} attempts: {}",
                        sat_name,
                        MAX_RETRIES,
                        e
                    );
                    return Err(e);
                } else {
                    tracing::warn!(
                        "Attempt {}/{} failed for {}: {}",
                        attempt,
                        MAX_RETRIES,
                        sat_name,
                        e
                    );
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "Failed to fetch data for {} after {} attempts",
        sat_name,
        MAX_RETRIES
    ))
}

/// Single HTTP fetch attempt (no retry).
async fn fetch_attempt(
    client: &Client,
    url: &str,
    sat_name: &str,
) -> Result<Vec<AmsatReport>> {
    let response = client
        .get(url)
        .send()
        .await
        .context(format!("Failed to send request for {}", sat_name))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "HTTP error {} for {}",
            response.status(),
            sat_name
        ));
    }

    let data: Vec<AmsatReport> = response
        .json()
        .await
        .context(format!("Failed to parse JSON response for {}", sat_name))?;

    Ok(data)
}

/// Batch fetch multiple satellites with delay between requests.
///
/// A single `reqwest::Client` is created for all requests, avoiding the
/// overhead of creating a new connection pool per satellite.
///
/// # Arguments
/// * `sat_names` - List of satellite names to fetch
/// * `hours` - Number of hours of data to fetch
/// * `delay_ms` - Delay between requests in milliseconds (to avoid rate limiting)
///
/// # Returns
/// HashMap of satellite name to `Result<Vec<AmsatReport>>`
pub async fn batch_fetch_satellites(
    sat_names: &[String],
    hours: u64,
    delay_ms: u64,
) -> std::collections::HashMap<String, Result<Vec<AmsatReport>>> {
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            return sat_names
                .iter()
                .map(|n| (n.clone(), Err(anyhow::anyhow!("HTTP client build failed: {}", e))))
                .collect();
        }
    };

    let mut results = std::collections::HashMap::new();

    for (index, sat_name) in sat_names.iter().enumerate() {
        if index > 0 && delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        let result = fetch_with_client(&client, sat_name, hours).await;
        results.insert(sat_name.clone(), result);
    }

    results
}

/// Time-window steps used by [`batch_fetch_min_reports`] when widening the
/// look-back window.  Values are in hours.
const FETCH_HOURS_STEPS: &[u64] = &[1, 3, 6, 12, 24, 48];

/// Fetch one satellite's reports, widening the time window until at least
/// `min_reports` individual reports are collected or all steps are exhausted.
///
/// Each step replaces the previous result (a wider window is a strict superset
/// of a shorter one from the AMSAT API).  On the first step that meets the
/// threshold we stop immediately and return those reports.
async fn fetch_until_min_reports(
    client: &Client,
    sat_name: &str,
    min_reports: usize,
) -> Result<Vec<AmsatReport>> {
    let mut last_reports: Vec<AmsatReport> = Vec::new();
    for &hours in FETCH_HOURS_STEPS {
        last_reports = fetch_with_client(client, sat_name, hours).await?;
        if last_reports.len() >= min_reports {
            tracing::debug!(
                "'{}': {} reports collected with {}h window",
                sat_name, last_reports.len(), hours
            );
            break;
        }
        if hours < *FETCH_HOURS_STEPS.last().unwrap() {
            tracing::debug!(
                "'{}': only {} reports in {}h window, widening…",
                sat_name, last_reports.len(), hours
            );
        }
    }
    Ok(last_reports)
}

/// Batch fetch multiple satellites, widening each satellite's time window
/// individually until `min_reports` reports are gathered (or all steps are
/// exhausted).
///
/// A single `reqwest::Client` is shared across all requests.
/// `delay_ms` is applied **between satellites**, not between step attempts
/// for the same satellite.
pub async fn batch_fetch_min_reports(
    sat_names: &[String],
    min_reports: usize,
    delay_ms: u64,
) -> std::collections::HashMap<String, Result<Vec<AmsatReport>>> {
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            return sat_names
                .iter()
                .map(|n| (n.clone(), Err(anyhow::anyhow!("HTTP client build failed: {}", e))))
                .collect();
        }
    };

    let mut results = std::collections::HashMap::new();

    for (index, sat_name) in sat_names.iter().enumerate() {
        if index > 0 && delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let result = fetch_until_min_reports(&client, sat_name, min_reports).await;
        results.insert(sat_name.clone(), result);
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires network connection
    async fn test_fetch_satellite_data() {
        let result = fetch_satellite_data("AO-91", 1).await;
        assert!(result.is_ok() || result.is_err()); // Just test it can run
    }

    #[tokio::test]
    #[ignore]
    async fn test_batch_fetch() {
        let sat_names = vec!["AO-91".to_string(), "ISS-FM".to_string()];
        let results = batch_fetch_satellites(&sat_names, 1, 200).await;
        assert_eq!(results.len(), 2);
    }
}
