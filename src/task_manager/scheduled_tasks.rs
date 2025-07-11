use std::time::Duration;
use crate::{
    amsat_parser::run_amsat_module,
    response::ApiResponse,
    pass_query,
    config::Config,
    msg_sys,
};
use chrono::{self, Utc, Timelike};
use std::path::Path;
use tokio::fs;

pub fn start_scheduled_module(config: &Config) {
    let config_cp1 = config.clone();
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
                
                match run_amsat_module(&config_cp1).await {
                    Ok(_) => {
                        tracing::info!("AMSAT updated successfully");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Error updating AMSAT data: {}", e);
                        if attempt >= MAX_RETRIES {
                            tracing::error!("AMSAT update failed after {} attempts", MAX_RETRIES);
                            break;
                        }
                        tracing::warn!("Retrying in {} seconds...", RETRY_DELAY.as_secs());
                        tokio::time::sleep(RETRY_DELAY).await;
                    }
                }
            }
        }
    });

    let config_cp2 = config.clone();
    let _sat_pass_data_update = tokio::spawn(async move {
        const PASS_UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 60); // adjust the time if choose n2yo api

        loop {
            tracing::info!("Starting satellite pass data update");
            match pass_query::sat_pass_predict::update_sat_pass_cache(&config_cp2).await {
                Ok(_) => tracing::info!("Satellite pass data updated successfully"),
                Err(e) => tracing::error!("Error updating satellite pass data: {}", e),
            }
            tokio::time::sleep(PASS_UPDATE_INTERVAL).await;
        }
    });

    let _expired_cache_clean = tokio::spawn(async {
        loop {
            tracing::info!("Starting expired cache clean");
            match pass_query::sat_cache_clean::clean_expired_cache().await {
                Ok(_) => tracing::info!("Expired cache cleaned successfully"),
                Err(e) => tracing::error!("Error cleaning expired cache: {}", e),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await; // 1h is enough maybe
        }
    });

    let config_cp3 = config.clone();
    let _pass_notify_task = tokio::spawn(async move {
        let special_group_id = config_cp3.backend_config.special_group_id.clone();
    
        loop {
            let results = pass_query::sat_pass_notify::check_upcoming_passes().await;
    
            for msg in results {
                let mut response = ApiResponse {
                    success: true,
                    data: Some(vec![msg.clone()]),
                    message: None,
                };
                response.message = Some(msg);
                
                if let Some(group_ids) = &special_group_id {
                    for group_id in group_ids {
                        msg_sys::group_chat::send_group_msg(response.clone(), *group_id).await;
                    }
                } else {
                    tracing::warn!("No special group ID configured for pass notifications.");
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });

    // let _get_solar_image = tokio::spawn(async move {
    //     const SOLAR_IMAGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 60 * 1); // 1 hours

    //     loop {
    //         tracing::info!("Starting solar image update");
    //         // match crate::solar_image::get_image::get_solar_image().await {
    //         //     Ok(image_path) => tracing::info!("Solar image updated successfully: {}", image_path),
    //         //     Err(e) => tracing::error!("Error updating solar image: {}", e),
    //         // }

    //         let mut attempt = 0;
    //         const MAX_RETRIES: u32 = 3;
    //         const RETRY_DELAY: Duration = Duration::from_secs(60);

    //         loop {
    //             attempt += 1;
    //             match crate::solar_image::get_image::get_solar_image().await {
    //                 Ok(image_path) => {
    //                     tracing::info!("Solar image updated successfully: {}", image_path);
    //                     break;
    //                 }
    //                 Err(e) => {
    //                     tracing::error!("Error updating solar image: {}", e);
    //                     if attempt >= MAX_RETRIES {
    //                         tracing::error!("Solar image update failed after {} attempts", MAX_RETRIES);
    //                         break;
    //                     }
    //                     tracing::warn!("Retrying in {} seconds...", RETRY_DELAY.as_secs());
    //                     tokio::time::sleep(RETRY_DELAY).await;
    //                 }
    //             }
    //         }

    //         // remove old solar images
    //         clean_old_images(3).await.unwrap_or_else(|e| {
    //             tracing::error!("Error cleaning old solar images: {}", e);
    //         });

    //         tokio::time::sleep(SOLAR_IMAGE_UPDATE_INTERVAL).await;
    //     }
    // });
}

#[allow(unused)]
async fn clean_old_images(days: i64) -> Result<(), String> {
    let dir_path = Path::new("pic");
    let mut entries = fs::read_dir(dir_path).await.map_err(|e| e.to_string())?;
    let cutoff = Utc::now() - chrono::Duration::days(days);

    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.is_file() && path.extension().map(|e| e == "jpg").unwrap_or(false) {
            if let Ok(metadata) = entry.metadata().await {
                if let Ok(modified) = metadata.modified() {
                    let modified: chrono::DateTime<Utc> = modified.into();
                    if modified < cutoff {
                        if let Err(e) = fs::remove_file(&path).await {
                            tracing::error!("Failed to delete {:?}: {}", path, e);
                        } else {
                            tracing::info!("Deleted old image: {:?}", path);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}