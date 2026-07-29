use super::types::AmsatReport;
use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;
use scraper::{Html, Selector};

const AMSAT_STATUS_URL: &str = "https://www.amsat.org/status/";
const AMSAT_API_URL: &str = "https://www.amsat.org/status/api/v1/sat_info.php";
pub const SATELLITE_LIST_CACHE_PATH: &str = "data/satellite_list.toml";
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

/// Satellite scraper - fetches satellite list from AMSAT
pub struct SatelliteScraper {
    client: Client,
}

impl SatelliteScraper {
    /// Create a new scraper instance
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Fetch the current satellite labels from the AMSAT status page.
    ///
    /// Purely a read: reconciling these labels against stored state, and persisting
    /// the result, belongs to the registry. The previous entry point also wrote the
    /// cache file, which is how a scrape could silently clobber curated aliases.
    pub async fn fetch_labels(&self) -> Result<Vec<String>> {
        self.fetch_satellite_names().await
    }

    /// Fetch list of satellite names from AMSAT status page using the shared client.
    async fn fetch_satellite_names(&self) -> Result<Vec<String>> {
        tracing::debug!("Fetching satellite list from {}", AMSAT_STATUS_URL);

        let response = self
            .client
            .get(AMSAT_STATUS_URL)
            .send()
            .await
            .context("Failed to fetch AMSAT status page")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to fetch AMSAT status page: HTTP {}",
                response.status()
            ));
        }

        let html_body = response
            .text()
            .await
            .context("Failed to read AMSAT status page body")?;

        let document = Html::parse_document(&html_body);

        let selector = Selector::parse(r#"select[name="SatName"] > option"#)
            .map_err(|e| anyhow::anyhow!("Invalid CSS selector: {:?}", e))?;

        let mut satellite_names = Vec::new();
        for element in document.select(&selector) {
            if let Some(value) = element.value().attr("value") {
                let trimmed = value.trim();
                if !trimmed.is_empty() && trimmed != "Select Satellite" {
                    satellite_names.push(trimmed.to_string());
                }
            }
        }

        tracing::info!(
            "Successfully fetched {} satellite names from AMSAT",
            satellite_names.len()
        );

        if satellite_names.is_empty() {
            tracing::warn!("No satellites found in AMSAT status page");
        }

        Ok(satellite_names)
    }

}

impl Default for SatelliteScraper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hits the live AMSAT status page; verifies the scrape still finds labels.
    #[tokio::test]
    async fn test_fetch_labels() {
        let scraper = SatelliteScraper::new();
        let labels = scraper.fetch_labels().await.expect("scrape should succeed");
        assert!(!labels.is_empty(), "expected at least one satellite label");
    }
}
