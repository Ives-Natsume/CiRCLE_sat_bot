use anyhow::Result;
use reqwest::Client;
use chrono::Local;
use tokio::fs;
use std::path::{Path, PathBuf};

#[allow(unused)]
pub async fn get_solar_image() -> Result<String, String> {
    let client = Client::new();
    let img_url = "https://www.hamqsl.com/solarn0nbh.php?image=random";

    // ensure the directory exists
    let now = Local::now();
    let temp_file = format!(
        "pic/solar_image_{}_{}.jpg",
        now.format("%Y%m%d"),
        now.format("%H%M%S")
    );

    // create the directory
    if let Some(parent) = PathBuf::from(&temp_file).parent() {
        if let Err(e) = fs::create_dir_all(parent).await {
            tracing::error!("Failed to create directory {:?}: {}", parent, e);
            return Err(format!("Failed to create directory: {}", e));
        }
    }

    // download
    let response = client.get(img_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch image: {}", e);
            format!("Failed to fetch image: {}", e)
        })?;

    let img_bytes = response.bytes()
        .await
        .map_err(|e| {
            tracing::error!("Failed to read image bytes: {}", e);
            format!("Failed to read image bytes: {}", e)
        })?;

    // save file
    if let Err(e) = fs::write(&temp_file, img_bytes).await {
        tracing::error!("Failed to write image to file {:?}: {}", temp_file, e);
        return Err(format!("Failed to write image to file: {}", e));
    }

    // get absolute path after ensured the file exists
    let abs_path: PathBuf = tokio::task::spawn_blocking(move || {
        PathBuf::from(&temp_file).canonicalize()
    })
    .await
    .map_err(|e| format!("Failed to join canonicalize task: {}", e))?
    .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

    tracing::info!("Solar image saved to {:?}", abs_path);
    Ok(abs_path.to_string_lossy().to_string())
}

#[allow(unused)]
pub async fn get_latest_image() -> Result<String, String> {
    let dir_path = Path::new("pic");
    let mut entries = fs::read_dir(dir_path).await
        .map_err(|e| format!("Failed to read pic directory: {}", e))?;

    let mut latest: Option<(PathBuf, std::time::SystemTime)> = None;

    while let Some(entry) = entries.next_entry().await
        .map_err(|e| format!("Failed to read entry: {}", e))? {
        let path = entry.path();
        if path.is_file() && path.extension().map(|e| e == "jpg").unwrap_or(false) {
            if let Ok(metadata) = entry.metadata().await {
                if let Ok(modified) = metadata.modified() {
                    match &latest {
                        Some((_, latest_time)) if *latest_time >= modified => {}
                        _ => latest = Some((path.clone(), modified)),
                    }
                }
            }
        }
    }

    if let Some((path, _)) = latest {
        let abs_path = tokio::task::spawn_blocking(move || {
            path.canonicalize()
        }).await
            .map_err(|e| format!("Failed to join canonicalize task: {}", e))?
            .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

        Ok(abs_path.to_string_lossy().to_string())
    } else {
        Err("No image files found in /pic".to_string())
    }
}