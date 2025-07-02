use std::time::Duration;
use crate::amsat_parser::run_amsat_module;
use crate::config::Config;
use chrono::{self, Utc, Timelike};

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
}