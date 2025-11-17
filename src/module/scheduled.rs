use std::sync::Arc;
use std::time::Duration;
use chrono::{self, DateTime, Timelike, Utc};
use crate::{
    app_status::AppStatus, fs::{self, handler::FileData}, module::{
        amsat::{self, prelude::*},
        solar_image,
        tools::render::{SATSTATUS_PIC_PATH_PREFIX, render_satstatus_data, floor_to_previous_quarter},
    }, msg::group_msg::send_group_message_to_multiple_groups, response,
};

pub async fn scheduled_task_handler(
    app_status: &Arc<AppStatus>,
) {
    let app_status_cp1 = Arc::clone(app_status);
    let _amsat_task = tokio::spawn(async move {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_secs(60);
        const TIMEOUT: Duration = Duration::from_secs(60 * 5);

        loop {
            // schedule to run at xx:02, xx:17, xx:32, xx:47 every hour
            let now = Utc::now();
            let next_trigger = {
                let current_minute = now.minute();
                let minute = match current_minute {
                    0..=16 => 17,
                    17..=31 => 32,
                    32..=46 => 47,
                    _ => 2, // 47..=59 -> next hour's 02
                };
                
                let mut next = now
                    .with_minute(minute)
                    .unwrap_or_else(|| now + chrono::Duration::hours(1))
                    .with_second(0)
                    .unwrap_or_else(|| now + chrono::Duration::minutes(minute as i64));

                if minute == 2 && current_minute > 46 {
                    next = next + chrono::Duration::hours(1);
                }
                
                if next <= now {
                    next = next + chrono::Duration::minutes(15);
                }
                next
            };

            let sleep_duration = (next_trigger - now).to_std().unwrap_or(Duration::from_secs(0));
            tracing::info!(
                "下次 AMSAT 更新时间: {}",
                next_trigger.to_rfc3339()
            );
            tokio::time::sleep(sleep_duration).await;

            let mut attempt = 0;
            loop {
                attempt += 1;
                if attempt > MAX_RETRIES {
                    break;
                }
                tracing::info!("更新 AMSAT 数据，尝试次数 {}/{}", attempt, MAX_RETRIES);
                let timeout = tokio::time::timeout(TIMEOUT, async {
                    // Ensure the request does not block indefinitely
                    amsat::official_report::amsat_data_handler(&app_status_cp1).await
                });

                let response = timeout.await;
                let response = match response {
                    Ok(response) => response,
                    Err(e) => {
                        let err_msg = format!("AMSAT 数据更新超时，尝试次数 {} / {}: {}\n{}s 后重试",
                            attempt,
                            MAX_RETRIES,
                            e,
                            RETRY_DELAY.as_secs()
                        );
                        tracing::error!("{}", err_msg);
                        if attempt >= MAX_RETRIES {
                            tracing::error!("AMSAT 更新失败，尝试次数: {}", MAX_RETRIES);
                            break;
                        }
                        tokio::time::sleep(RETRY_DELAY).await;
                        continue;
                    }
                };

                // render the satellite status image after successful data update
                render_satstatus_image_task(Arc::clone(&app_status_cp1)).await;

                let success = response.success;
                send_group_message_to_multiple_groups(response, &app_status_cp1).await;
                if success {
                    tracing::info!("AMSAT 数据更新成功");
                    break;
                }
            }

            // handle the cache
            // let response = official_report::sat_status_cache_handler(&app_status_cp1).await;
            // send_group_message_to_multiple_groups(response, &app_status_cp1).await;
        }
    });

    let app_status_cp2 = Arc::clone(app_status);
    let _solar_image_task = tokio::spawn(async move {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_secs(60);

        loop {
            // schedule to run at xx:00, xx:15, xx:30, xx:45 every hour
            let now = Utc::now();
            let next_trigger = {
                let current_minute = now.minute();
                let next_minute = if current_minute < 45 {
                    ((current_minute / 15) + 1) * 15
                } else {
                    0
                };

                let (next_hour, next_minute) = if next_minute == 0 {
                    (now.hour() + 1, 0)
                } else {
                    (now.hour(), next_minute)
                };

                now.with_hour(next_hour)
                    .and_then(|dt| dt.with_minute(next_minute))
                    .and_then(|dt| dt.with_second(0))
                    .and_then(|dt| dt.with_nanosecond(0))
                    .unwrap_or_else(|| now + chrono::Duration::minutes(15)) // 默认值
            };

            let next_trigger = if next_trigger <= now {
                next_trigger + chrono::Duration::hours(1)
            } else {
                next_trigger
            };

            let sleep_duration = (next_trigger - now).to_std().unwrap_or(Duration::from_secs(0));
            tracing::info!(
                "下次太阳活动图更新时间: {}",
                next_trigger.to_rfc3339()
            );
            tokio::time::sleep(sleep_duration).await;

            let mut attempt = 0;
            loop {
                attempt += 1;
                tracing::info!("正在更新太阳活动图，尝试次数 {}/{}", attempt, MAX_RETRIES);

                match solar_image::get_image::get_solar_image(&app_status_cp2).await {
                    Ok(_) => {
                        tracing::info!("太阳活动图已保存");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("太阳活动图更新失败: {}", e);
                        if attempt >= MAX_RETRIES {
                            tracing::error!("太阳活动图更新失败，尝试次数: {}", MAX_RETRIES);
                            let response = response::ApiResponse::<Vec<String>>::error(
                                format!("太阳活动图更新失败: {}", e),
                            );
                            send_group_message_to_multiple_groups(response, &app_status_cp2).await;
                            break;
                        }
                        tracing::warn!("{}s 后重试", RETRY_DELAY.as_secs());
                        tokio::time::sleep(RETRY_DELAY).await;
                    }
                }
            }
        }
    });
    
    let app_status_cp3 = Arc::clone(app_status);
    let _user_report_task = tokio::spawn(async move {
        loop {
            // schedule to run at every 10 minutes
            let now = Utc::now();
            let next_trigger = now + chrono::Duration::minutes(10);

            let sleep_duration = (next_trigger - now).to_std().unwrap_or(Duration::from_secs(0));
            tracing::info!("下次用户报告更新时间: {}", next_trigger.to_rfc3339());
            tokio::time::sleep(sleep_duration).await;

            let mut user_reports = match amsat::user_report::read_user_report_file(&app_status_cp3).await {
                Ok(data) => data,
                Err(e) => {
                    tracing::error!("读取用户报告文件失败: {}", e);
                    continue;
                }
            };

            for satellite_file_format in &mut user_reports {
                if satellite_file_format.data.is_empty() {
                    continue;
                }

                let mut data_to_keep: Vec<SatelliteFileElement> = Vec::new();

                for file_element in satellite_file_format.data.drain(..) {
                    let time_block = match DateTime::parse_from_rfc3339(&file_element.time) {
                        Ok(dt) => dt.with_timezone(&Utc),
                        Err(e) => {
                            tracing::error!("解析时间参数失败，数据将被丢弃: {}", e);
                            // invalid data, dismissed
                            continue;
                        }
                    };

                    let now = Utc::now();
                    if now - time_block > chrono::Duration::minutes(20) {
                        if file_element.report.is_empty() {
                            tracing::warn!("没有可以处理的数据");
                        }
                        for report in &file_element.report {
                            if let Err(e) = amsat::user_report::push_user_report_from_SatStatus(report).await {
                                tracing::error!("上传用户数据失败，数据将被丢弃: {}", e);
                            }
                        }
                        // discard the processed data
                    } else {
                        // keep unprocessed data
                        data_to_keep.push(file_element);
                    }
                }

                satellite_file_format.data = data_to_keep;
            }

            // write user report data back to file
            let user_reports_value = serde_json::to_value(&user_reports).unwrap_or(serde_json::Value::Null);
            let tx_filerequest = app_status_cp3.file_tx.clone();
            match fs::handler::write_file(
                tx_filerequest,
                USER_REPORT_DATA.to_string(),
                &fs::handler::FileData::Json(user_reports_value),
            ).await {
                Ok(_) => {
                    tracing::info!("用户报告数据已更新");
                }
                Err(e) => {
                    tracing::error!("写入用户报告文件失败: {}", e);
                }
            }
        }
    });

    let _old_satstatus_img_cleanup_task = tokio::spawn(start_cleanup_task());
}

async fn start_cleanup_task() {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 10));

    loop {
        interval.tick().await;
        if let Err(e) = cleanup_old_files().await {
            tracing::error!("Cleanup task failed: {}", e);
        }
    }
}

/// Cleanup the old satellite status image files
/// Matches the RFC3339 timestamp pattern
async fn cleanup_old_files() -> anyhow::Result<()> {
    let now = Utc::now();
    let dir = std::path::Path::new(SATSTATUS_PIC_PATH_PREFIX);

    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }

    let re = regex::Regex::new(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))").unwrap();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map(|ext| ext == "png").unwrap_or(false) {
            if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                if let Some(captures) = re.captures(file_name) {
                    let time_str = &captures[1];
                    if let Ok(file_time) = DateTime::parse_from_rfc3339(time_str) {
                        let file_time_utc = file_time.with_timezone(&Utc);
                        let age = now.signed_duration_since(file_time_utc).num_seconds();

                        if age > 60 * 30 {
                            tracing::info!("Deleting expired file: {:?}", path);
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn render_satstatus_image_task(app_status_cp: Arc<AppStatus>) {
    let tx_filerequest = app_status_cp.file_tx.clone();
    let official_report_raw = match fs::handler::read_file(
        tx_filerequest,
        amsat::prelude::OFFICIAL_REPORT_DATA.to_string(),
        fs::handler::FileFormat::Json
    ).await {
        Ok(FileData::Json(data)) => data,
        Ok(_) => {
            tracing::error!("Received unexpected file format while reading official report data file");
            return;
        }
        Err(e) => {
            tracing::error!("Failed to read official report data file: {}", e);
            return;
        }
    };

    let official_report: Vec<amsat::prelude::SatelliteFileFormat> = match serde_json::from_value(official_report_raw) {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("Failed to parse official report data file: {}", e);
            return;
        }
    };

    let now_utc = Utc::now();
    let floored_time = floor_to_previous_quarter(now_utc);
    let timestamp_str = floored_time.to_rfc3339();

    // render each satellite status image
    for satellite in &official_report {
        let satellite_name_normalized = string_normalize(&satellite.name);
        match render_satstatus_data(
            &vec![satellite.clone()],
            format!("{}{}-{}.png", SATSTATUS_PIC_PATH_PREFIX, timestamp_str, satellite_name_normalized)
        ).await {
            Ok(_) => {
                tracing::info!("Rendered satellite status image for {}", satellite.name);
            }
            Err(e) => {
                tracing::error!("Failed to render satellite status image for {}: {}", satellite.name, e);
            }
        }
    }
}