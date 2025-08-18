use std::sync::Arc;
use std::time::Duration;
use chrono::{self, DateTime, Timelike, Utc};
use crate::{
    app_status::AppStatus,
    module::{
        amsat::{self, prelude::*, official_report},
        solar_image
    },
    msg::group_msg::send_group_message_to_multiple_groups,
    response
};

pub async fn scheduled_task_handler(
    app_status: &Arc<AppStatus>,
) {
    let app_status_cp1 = Arc::clone(app_status);
    let _amsat_task = tokio::spawn(async move {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_secs(60);

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
                tracing::info!("更新 AMSAT 数据，尝试次数 {}/{}", attempt, MAX_RETRIES);

                let response = amsat::official_report::amsat_data_handler(&app_status_cp1).await;
                let success = response.success;
                send_group_message_to_multiple_groups(response, &app_status_cp1).await;
                if !success {
                    if attempt >= MAX_RETRIES {
                            tracing::error!("AMSAT 更新失败，尝试次数: {}", MAX_RETRIES);
                            break;
                    }
                    tracing::warn!("{}s 后重试", RETRY_DELAY.as_secs());
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                else {
                    tracing::info!("AMSAT 数据更新成功");
                    break;
                }
            }

            // handle the cache
            let response = official_report::sat_status_cache_handler(&app_status_cp1).await;
            send_group_message_to_multiple_groups(response, &app_status_cp1).await;
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
            let tx_filerequest = app_status_cp3.file_tx.clone();
            if let Err(e) = amsat::official_report::write_report_data(
                tx_filerequest,
                &user_reports,
                USER_REPORT_DATA.into()
            ).await {
                tracing::error!("用户报告文件写入失败: {}", e);
            }
        }
    });
}