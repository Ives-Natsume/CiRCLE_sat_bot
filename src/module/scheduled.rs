use std::sync::Arc;
use std::time::Duration;
use chrono::{self, Utc, Timelike};
use crate::{
    app_status::AppStatus,
    module::{
        amsat,
        solar_image
    },
    msg::group_msg::send_group_message_to_multiple_groups, response
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
                "Next AMSAT update scheduled at: {}",
                next_trigger.to_rfc3339()
            );
            tokio::time::sleep(sleep_duration).await;

            let mut attempt = 0;
            loop {
                attempt += 1;
                tracing::info!("Attempt {} to update AMSAT data", attempt);
                
                let response = amsat::official_report::amsat_data_handler(&app_status_cp1).await;
                let success = response.success;
                send_group_message_to_multiple_groups(response, &app_status_cp1).await;
                if !success {
                    if attempt >= MAX_RETRIES {
                            tracing::error!("AMSAT update failed after {} attempts", MAX_RETRIES);
                            break;
                    }
                    tracing::warn!("Retrying in {} seconds...", RETRY_DELAY.as_secs());
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                else {
                    tracing::info!("AMSAT update completed successfully");
                    break;
                }
            }
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
                "Next solar image update scheduled at: {}",
                next_trigger.to_rfc3339()
            );
            tokio::time::sleep(sleep_duration).await;

            let mut attempt = 0;
            loop {
                attempt += 1;
                tracing::info!("Attempt {} to update solar image", attempt);
                
                match solar_image::get_image::get_solar_image(&app_status_cp2).await {
                    Ok(_) => {
                        tracing::info!("Solar image update completed successfully");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Solar image update failed: {}", e);
                        if attempt >= MAX_RETRIES {
                            tracing::error!("Solar image update failed after {} attempts", MAX_RETRIES);
                            let response = response::ApiResponse::<Vec<String>>::error(
                                format!("太阳活动图保存失败: {}", e),
                            );
                            send_group_message_to_multiple_groups(response, &app_status_cp2).await;
                            break;
                        }
                        tracing::warn!("Retrying in {} seconds...", RETRY_DELAY.as_secs());
                        tokio::time::sleep(RETRY_DELAY).await;
                    }
                }
            }
        }
    });
}