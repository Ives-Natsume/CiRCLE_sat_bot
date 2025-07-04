use std::time::Duration;
use crate::amsat_parser::run_amsat_module;
use crate::config::Config;
use chrono::{self, Utc, Timelike};
use std::path::Path;
use tokio::fs;

pub fn start_scheduled_module(config: &Config) {
    let config_cp1 = config.clone();
    let _amsat_task = tokio::spawn(async move {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_secs(60);

        loop {
            // schedule the AMSAT module to run at xx:05 every hour
            let now = Utc::now();
            let next_trigger = {
                let mut next = now
                    .with_minute(5)
                    .unwrap_or_else(|| now + chrono::Duration::hours(1))
                    .with_second(0)
                    .unwrap_or_else(|| now + chrono::Duration::minutes(5));

                if next <= now {
                    next = next + chrono::Duration::hours(1);
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
        const PASS_UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24); // 24 hours

        loop {
            tracing::info!("Starting satellite pass data update");
            match crate::pass_query::sat_pass_predict::update_sat_pass_cache(&config_cp2).await {
                Ok(_) => tracing::info!("Satellite pass data updated successfully"),
                Err(e) => tracing::error!("Error updating satellite pass data: {}", e),
            }
            tokio::time::sleep(PASS_UPDATE_INTERVAL).await;
        }
    });

    let _expired_cache_clean = tokio::spawn(async {
        use crate::pass_query::sat_cache_clean::clean_expired_passes;
        loop {
            match tokio::task::spawn_blocking(clean_expired_passes).await {
                Ok(Ok(())) => {},
                Ok(Err(e)) => tracing::error!("清理缓存失败: {:?}", e),
                Err(e) => tracing::error!("spawn_blocking 执行失败: {:?}", e),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });

    let _pass_notify_task = tokio::spawn(async move {
        use tokio::time::{sleep, Duration as TokioDuration};
        use crate::msg_sys::response::send_group_msg;
        use crate::msg_sys::qq_structs::GroupMessageResponse;
    
        const GROUP_ID: i64 = 965954401;
    
        loop {
            let results = crate::pass_query::sat_pass_notify::check_upcoming_passes().await;
    
            for msg in results {
                tracing::info!("Sending scheduled task message: {}", msg);
    
                let mut response = GroupMessageResponse::default();
                response.message = Some(msg);
                send_group_msg(response, GROUP_ID).await;
            }
            sleep(TokioDuration::from_secs(60)).await;
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