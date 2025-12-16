use tokio_tungstenite::connect_async;
use futures_util::{StreamExt, SinkExt};
use tokio::sync::mpsc;
use tokio::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader};
use std::collections::HashMap;
use serde_json::Value;
use serde::{Deserialize, Serialize};
use crate::module::tools::render;
use crate::app_status::AppStatus;
use crate::msg::group_msg;
use crate::response::ApiResponse;

const URL_LIST: [&str; 1] = [
    "wss://ws-api.wolfx.jp/jma_eew",
    // "wss://ws-api.wolfx.jp/cenc_eew"
];

pub const EQ_PIC_PATH_PREFIX: &str = "runtime_data/pic/eq/";
pub const EQ_LIST_JSON_PATH: &str = "runtime_data/eq_event_list.json";
pub const EQ_PIC_URL_PREFIX: &str = "file:///server_runtime_data/pic/eq/";

/// Wolfx API关键字均为大驼峰
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EqEventWolfxJmaEew {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "CodeType")]
    pub code_type: String,
    #[serde(rename = "Issue")]
    pub _issue: Option<Value>,
    #[serde(rename = "EventID")]
    pub event_id: String,
    #[serde(rename = "Serial")]
    pub serial: u32,
    #[serde(rename = "AnnouncedTime")]
    pub announced_time: String,
    #[serde(rename = "OriginTime")]
    pub origin_time: String,
    #[serde(rename = "Hypocenter")]
    pub hypocenter: String,
    #[serde(rename = "Latitude")]
    pub latitude: f32,
    #[serde(rename = "Longitude")]
    pub longitude: f32,
    #[serde(rename = "Magunitude")]
    pub magnitude: f32,
    #[serde(rename = "Depth")]
    pub depth: f32,
    #[serde(rename = "MaxIntensity")]
    pub max_intensity: String,
    #[serde(rename = "Accuracy")]
    pub _accuracy: Option<Value>,
    #[serde(rename = "MaxIntChange")]
    pub _max_int_change: Option<Value>,
    #[serde(rename = "WarnArea")]
    pub warn_area: Vec<Value>,
    #[serde(rename = "isSea")]
    pub is_sea: bool,
    #[serde(rename = "isTraining")]
    pub is_training: bool,
    #[serde(rename = "isAssumption")]
    pub is_assumption: bool,
    #[serde(rename = "isWarn")]
    pub is_warn: bool,
    #[serde(rename = "isFinal")]
    pub is_final: bool,
    #[serde(rename = "isCancel")]
    pub is_cancel: bool,
    #[serde(rename = "OriginalText")]
    pub _original_text: Option<String>,
    #[serde(rename = "Pond")]
    pub pond: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EqEventWolfxCencEew {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "EventID")]
    pub event_id: String,
    #[serde(rename = "ReportTime")]
    pub report_time: String,
    #[serde(rename = "ReportNum")]
    pub report_num: u32,
    #[serde(rename = "OriginTime")]
    pub origin_time: String,
    #[serde(rename = "Hypocenter")]
    pub hypocenter: String,
    #[serde(rename = "Latitude")]
    pub latitude: f32,
    #[serde(rename = "Longitude")]
    pub longitude: f32,
    #[serde(rename = "Magnitude")]
    pub magnitude: f32,
    #[serde(rename = "Depth")]
    pub depth: f32,
    #[serde(rename = "MaxIntensity")]
    pub max_intensity: u32,
    pub pond: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum EqEventEew {
    Jma(EqEventWolfxJmaEew),
    Cenc(EqEventWolfxCencEew),
}

/// 地震事件列表，包含事件信息，是否处理过等
/// 主键为事件序号字符串
#[derive(Debug, Serialize, Deserialize)]
pub struct EqEventList {
    #[serde(flatten)]
    pub events: HashMap<String, EqEventEew>,
    pub processed: HashMap<String, bool>,
}

impl EqEventList {
    /// read from json file
    pub async fn from_json_file(path: &str) -> Result<Self, serde_json::Error> {
        // check if file exists
        if !tokio::fs::metadata(path).await.is_ok() {
            // create empty file
            let empty = EqEventList {
                events: HashMap::new(),
                processed: HashMap::new(),
            };
            let data = serde_json::to_string_pretty(&empty)?;
            match tokio::fs::write(path, data).await {
                Ok(_) => (),
                Err(e) => {
                    tracing::error!("Unable to create file {}: {}", path, e);
                }
            };
            return Ok(empty);
        }

        let data = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Unable to read file {}: {}", path, e);
                return Err(serde_json::Error::io(e));
            }
        };
        serde_json::from_str(&data)
    }

    /// write to json file
    pub async fn to_json_file(&self, path: &str) -> Result<(), serde_json::Error> {
        let data = serde_json::to_string_pretty(self)?;
        match tokio::fs::write(path, data).await {
            Ok(_) => (),
            Err(e) => {
                tracing::error!("Unable to write file {}: {}", path, e);
                return Err(serde_json::Error::io(e));
            }
        };
        Ok(())
    }
}

pub async fn eq_listener(app_status: &Arc<AppStatus>) {
    // Create a channel with a buffer of 10. 
    // If the buffer is full, new requests will be dropped (or handled as "unprocessed").
    let (tx, rx) = mpsc::channel(10);
    let earthquake_config = &app_status.config.read().await.earthquake_config;
    let python_executable_path = earthquake_config.python_executable_path.clone();

    // Spawn the map generator service
    let app_status_clone = app_status.clone();
    tokio::spawn(async move {
        map_generator_service(rx, python_executable_path, &app_status_clone).await;
    });

    for url in URL_LIST {
        let url = url.to_string();
        let tx = tx.clone();
        let app_status_clone2 = app_status.clone();
        // Spawn one task per connection
        tokio::spawn(async move {
            websocket_loop(url, tx, &app_status_clone2).await;
        });
    }

    // Keep main alive
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}

async fn eq_info_handler(json_path: String) -> Result<ApiResponse<Vec<String>>, ()> {
    // 根据python返回的文件名确定渲染完成的地震事件
    // 读取runtime_data/pic/eq/jma_eew_<timestamp>.json
    let content = match tokio::fs::read_to_string(&json_path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Unable to read earthquake json file {}: {}", json_path, e);
            return Err(());
        }
    };

    let eqevent = match serde_json::from_str::<EqEventWolfxJmaEew>(&content) {
        Ok(e) => EqEventEew::Jma(e),
        Err(e) => {
            tracing::error!("Failed to parse earthquake json file {}: {}", json_path, e);
            return Err(());
        }
    };
    let (magnitude, shindo, depth, hypocenter, occurrence_time, event_id) = match eqevent {
        EqEventEew::Jma(event) => (event.magnitude, event.max_intensity.clone(), event.depth, event.hypocenter.clone(), event.origin_time.clone(), event.event_id.clone()),
        EqEventEew::Cenc(event) => (event.magnitude, "".to_string(), event.depth, event.hypocenter.clone(), event.origin_time.clone(), event.event_id.clone()),
    };

    match render::render_earthquake_map_svg(
        magnitude,
        shindo,
        depth,
        hypocenter,
        occurrence_time,
        event_id.clone(),
        json_path.replace(".json", ".png"),
    ).await {
        Ok(_) => {
            let image_path = format!("{}earthquake_{}.png", EQ_PIC_URL_PREFIX, event_id);
            let response = ApiResponse {
                data: Some(vec![image_path]),
                message: None,
                success: true,
            };
            Ok(response)
        },
        Err(e) => {
            tracing::error!("Failed to render earthquake map SVG: {}", e);
            Err(())
        }
    }
}

async fn map_generator_service(mut rx: mpsc::Receiver<String>, python_executable_path: String, app_status: &Arc<AppStatus>) {
    let script_path = "src/module/earthquake/earthquake_map_generator.py"; // Assumes running from workspace root

    tracing::info!("Starting Python map generator service...");
    
    let mut child = Command::new(python_executable_path)
        .arg(script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn python script");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let stderr = child.stderr.take().expect("Failed to open stderr");

    let app_status_clone = app_status.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::info!("[MapGen] {}", line);
            // render if succeed
            if line.contains("Success:") {
                // extract timestamp
                if let Some(ts_start) = line.find("Success: ") {
                    // output format: Success: /home/ubuntu/ws/CiRCLE_sat_bot_server/runtime_data/pic/eq/jma_eew_20251214181001.png
                    // python render image for eq info only and with a prefix of `jma_eew_`
                    let raw_img_path = line[ts_start + 9..].trim().to_string();

                    // Debug: send the raw image to multiple groups directly
                    // format: FILE_PREFIX + "jma_eew_<timestamp>.png"
                    // let raw_img_filename = raw_img_path.split('/').last().unwrap_or("");
                    // let raw_img_url = format!("{}{}", EQ_PIC_URL_PREFIX, raw_img_filename);
                    // let image_response = ApiResponse {
                    //     data: Some(vec![raw_img_url.clone()]),
                    //     message: Some("Raw earthquake map from generator".to_string()),
                    //     success: true,
                    // };
                    // group_msg::send_picture_to_multiple_groups(image_response.clone(), &app_status_clone).await;

                    let json_path = raw_img_path.replace(".png", ".json");
                    match eq_info_handler(json_path).await {
                        Ok(response) => {
                            group_msg::send_picture_to_multiple_groups(response, &app_status_clone).await;
                        },
                        Err(_) => {
                            tracing::error!("Failed to handle earthquake info after map generation");
                        }
                    }
                }
            }
        }
    });

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::error!("[MapGen] {}", line);
        }
    });

    while let Some(request_msg) = rx.recv().await {
        // Format: <json_path> <output_path>\n
        // We use quotes to handle potential spaces in paths, though simple paths are safer.
        let command = format!("{}\n", request_msg);
        
        if let Err(e) = stdin.write_all(command.as_bytes()).await {
            tracing::error!("Failed to write to python process: {}", e);
            break; 
        }
        
        if let Err(e) = stdin.flush().await {
             tracing::error!("Failed to flush to python process: {}", e);
             break;
        }
    }
    
    let _ = child.kill().await;
}

async fn websocket_loop(url: String, tx: mpsc::Sender<String>, app_status: &Arc<AppStatus>) {
    let mut delay = 1u64; // exponential backoff

    // init EqEventList
    let mut eq_event_list = EqEventList::from_json_file(EQ_LIST_JSON_PATH).await.expect("Failed to read eq_event_list.json");

    loop {
        tracing::info!("Connecting to {} ...", url);

        match connect_async(url.clone()).await {
            Ok((ws_stream, resp)) => {
                tracing::info!(
                    "Connected to {} (HTTP status {})",
                    url,
                    resp.status()
                );
                delay = 1; // reset backoff on success

                let (mut _write, mut read) = ws_stream.split();

                // Read loop
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(m) => {
                            if m.is_text() {
                                tracing::debug!("{} >> {}", url, m.clone().into_text().unwrap());
                                // save json to file if not heartbeat
                                let text = m.into_text().unwrap();
                                if !text.contains("\"type\":\"heartbeat\"") {
                                    tracing::info!("Received earthquake event from {}", url);
                                    let filename = format!("./runtime_data/pic/eq/{}_{}.json", url.replace("wss://ws-api.wolfx.jp/", ""), chrono::Utc::now().format("%Y%m%d%H%M%S"));
                                    
                                    if let Err(e) = tokio::fs::write(&filename, &text).await {
                                        tracing::error!("Unable to write file: {}", e);
                                        continue;
                                    }

                                    // try to serialize to EqEventEew to verify
                                    let parsed: Result<EqEventEew, _> = if url.contains("jma_eew") {
                                        serde_json::from_str(&text).map(EqEventEew::Jma)
                                    } else {
                                        serde_json::from_str(&text).map(EqEventEew::Cenc)
                                    };

                                    let (latitude, longitude, magnitude, event_id, is_final, location) = match &parsed {
                                        Ok(EqEventEew::Jma(event)) => (event.latitude, event.longitude, event.magnitude, event.event_id.clone(), event.is_final, event.hypocenter.clone()),
                                        Ok(EqEventEew::Cenc(event)) => (event.latitude, event.longitude, event.magnitude, event.event_id.clone(), true, "Unknown".to_string()),
                                        Err(e) => {
                                            tracing::error!("Failed to parse earthquake event: {}", e);
                                            continue;
                                        }
                                    };

                                    let event_key = format!("{}", event_id);
                                    eq_event_list.events.insert(event_key.clone(), parsed.unwrap());
                                    
                                    let img_output_path = filename.replace(".json", ".png");
                                    
                                    if !is_final {
                                        tracing::info!("Event {} is not final, skipping map generation.", event_id);

                                        // check if the event already exists in eq_event_list
                                        if let Some(processed) = eq_event_list.processed.get(&event_key) {
                                            if *processed {
                                                eq_event_list.to_json_file(EQ_LIST_JSON_PATH).await.expect("Failed to save eq_event_list.json");
                                                tracing::info!("Event {} already processed, skipping.", event_id);
                                                continue;
                                            }
                                        }
                                        eq_event_list.processed.insert(event_key, false);
                                        eq_event_list.to_json_file(EQ_LIST_JSON_PATH).await.expect("Failed to save eq_event_list.json");

                                        // generate a broadcast message for non-final event
                                        let broadcast_data: String = format!(
                                            "Preliminary Earthquake Event Received:\nEvent ID: {}\nLatitude: {}\nLongitude: {}\nMagnitude: {}\nLocation: {}",
                                            event_id, latitude, longitude, magnitude, location
                                        );
                                        let broadcast_response: ApiResponse<Vec<String>> = ApiResponse {
                                            data: Some(vec![broadcast_data]),
                                            message: Some(format!("Received preliminary earthquake event ID: {}. Awaiting final update.", event_id)),
                                            success: true,
                                        };

                                        // send to multiple groups
                                        group_msg::send_picture_to_multiple_groups(broadcast_response, app_status).await;

                                        continue;
                                    }
                                    else if let Ok(cwd) = std::env::current_dir() {
                                        let abs_img_path = cwd.join(&img_output_path).to_string_lossy().to_string();

                                        // send lat, lon, mag directly
                                        let request_msg = format!("{} {} {} {} {}", latitude, longitude, magnitude, event_id, abs_img_path);
                                        
                                        // Try to send to map generator
                                        match tx.try_send(request_msg) {
                                            Ok(_) => {
                                                // Successfully queued
                                                // add to eq_event_list as processed = true
                                                eq_event_list.processed.insert(event_key, true);
                                            },
                                            Err(mpsc::error::TrySendError::Full(_)) => {
                                                tracing::warn!("Map generation queue full. Dropping request for {}. Marked as unprocessed.", filename);
                                                // Mark as unprocessed
                                                eq_event_list.processed.insert(event_key, false);
                                                // Here you could write to a log file for later processing
                                            },
                                            Err(e) => {
                                                eq_event_list.processed.insert(event_key, false);
                                                tracing::error!("Channel error: {}", e);
                                            }
                                        }
                                        eq_event_list.to_json_file(EQ_LIST_JSON_PATH).await.expect("Failed to save eq_event_list.json");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("{} connection error: {}", url, e);
                            break; // reconnect
                        }
                    }
                }

                tracing::info!("{} disconnected, retrying...", url);
            }

            Err(e) => {
                tracing::error!("Failed to connect to {}: {}", url, e);
            }
        }

        // Exponential backoff delay with cap
        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
        delay = (delay * 2).min(60);
    }
}

