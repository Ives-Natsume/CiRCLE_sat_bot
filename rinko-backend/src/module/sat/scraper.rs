///! Web scraper for fetching satellite list from AMSAT status page
use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

const AMSAT_STATUS_URL: &str = "https://www.amsat.org/status/";

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

    /// Scrape satellite list from AMSAT website
    pub async fn scrape_satellite_list(&self) -> Result<SatelliteList> {
        let names = self.fetch_satellite_names().await?;

        let mut list = SatelliteList { satellites: Vec::new() };
        for name in names {
            list.satellites.push(SatelliteEntry {
                official_name: name,
                aliases: Vec::new(),
                catalog_number: None,
            });
        }

        Ok(list)
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

/// Temporary types for scraper compatibility
#[derive(Debug, Clone)]
pub struct SatelliteList {
    pub satellites: Vec<SatelliteEntry>,
}

#[derive(Debug, Clone)]
pub struct SatelliteEntry {
    pub official_name: String,
    pub aliases: Vec<String>,
    pub catalog_number: Option<String>,
}

