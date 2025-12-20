use reqwest;
use scraper::{Html, Selector};

const AMSAT_STATUS_URL: &str = "https://www.amsat.org/status/";

pub async fn fetch_satellite_names() -> anyhow::Result<Vec<String>> {
    // Send a GET request to the specified URL
    let query = reqwest::get(AMSAT_STATUS_URL).await?.text().await;
    let html_body = match query {
        Ok(content) => content,
        Err(e) => {
            tracing::error!("Error fetching AMSAT status page: {}", e);
            return Err(anyhow::Error::new(e));
        }
    };

    // Parse the HTML response
    let document = Html::parse_document(&html_body);
    let selector = Selector::parse(r#"select[name="SatName"] > option"#)
        .map_err(|e| anyhow::anyhow!("Error parsing selector: {}", e))?;

    // Extract satellite names from the HTML
    let mut satellite_names = Vec::new();
    for element in document.select(&selector) {
        // get the value attribute of the <option> tag
        if let Some(value) = element.value().attr("value") {
            
            // filter out empty values
            if !value.is_empty() {
                satellite_names.push(value.trim().to_string());
            }
        }
    }

    Ok(satellite_names)
}