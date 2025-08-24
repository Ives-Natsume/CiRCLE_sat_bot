use reqwest::Client;
use std::path::PathBuf;
use crate::{
    app_status,
    fs::handler::*,
};
use std::path::Path;
use url::Url;

const IMAGE_PATH_PREFIX: &str = "data/pic/";

pub async fn get_solar_image(
    app_status: &app_status::AppStatus,
) -> anyhow::Result<()> {
    let client = Client::new();
    let img_url = "https://www.hamqsl.com/solarn0nbh.php?image=random";

    // ensure the directory exists
    //let now = Local::now();
    let temp_file = format!(
        "{}solar_image_latest.png",
        IMAGE_PATH_PREFIX,
    );

    // download
    let response = client.get(img_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to download image: {}", e))?;

    let img_bytes = response.bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read image bytes: {}", e))?;

    // save file
    let tx_filerequest = app_status.file_tx.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let exist_request = FileRequest::Write {
        path: temp_file.clone().into(),
        format: FileFormat::Png,
        data: FileData::Png(img_bytes.to_vec()),
        responder: tx,
    };
    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(exist_request).await {
        return Err(anyhow::anyhow!("Failed to send file write request: {}", e));
    }

    match rx.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(anyhow::anyhow!("Failed to write solar image: {}", e));
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to receive file write response: {}", e));
        }
    }

    // get absolute path after ensured the file exists
    let abs_path: PathBuf = tokio::task::spawn_blocking(move || {
        PathBuf::from(&temp_file).canonicalize()
    })
    .await
    .map_err(|e| anyhow::anyhow!("Failed to join canonicalize task: {}", e))?
    .map_err(|e| anyhow::anyhow!("Failed to canonicalize path: {}", e))?;

    tracing::info!("Solar image saved to {:?}", abs_path);
    Ok(())
}

pub fn _file_uri(path: impl AsRef<Path>) -> Result<String, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    
    // 获取绝对路径
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    
    // 在 Windows 上处理 UNC 路径和驱动器盘符
    #[cfg(windows)]
    let uri_path = {
        use std::os::windows::ffi::OsStrExt;
        
        let path_str = absolute_path
            .as_os_str()
            .encode_wide()
            .collect::<Vec<u16>>();
        
        // 检查是否是 UNC 路径 (\\server\share)
        if path_str.starts_with(&[92, 92]) { // 双反斜杠
            // 转换为标准格式
            let normalized = absolute_path.canonicalize()?;
            Url::from_file_path(&normalized).map_err(|_| "Failed to convert UNC path to URL")?
        } else {
            // 处理普通路径 (C:\path\to\file)
            let mut path_str = absolute_path
                .to_str()
                .ok_or("Invalid UTF-8 path")?
                .replace('\\', "/");
            
            // 确保驱动器盘符格式正确
            if path_str.len() > 1 && &path_str[1..2] == ":" {
                path_str = format!("/{}", path_str);
            }
            
            Url::parse(&format!("file://{}", path_str))?
        }
    };
    
    // 在 Unix-like 系统上处理
    #[cfg(not(windows))]
    let uri_path = {
        Url::from_file_path(&absolute_path).map_err(|_| "Failed to convert path to URL")?
    };
    
    Ok(uri_path.to_string())
}