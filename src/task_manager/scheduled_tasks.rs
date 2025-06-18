use std::time::Duration;
use crate::amsat_parser::{run_amsat_module, SatelliteStatus};
use chrono::{self, Utc, Timelike};

pub fn start_scheduled_amsat_module(mut status_table: Vec<(String, SatelliteStatus)>) {
    tracing::info!("Starting scheduled AMSAT module");
    let _amsat_task = tokio::spawn(async move {
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
                "Next AMSAT module run scheduled at: {}",
                next_trigger.to_rfc3339()
            );
            tokio::time::sleep(sleep_duration).await;

            match run_amsat_module(&mut status_table).await {
                Ok(_) => tracing::info!("AMSAT module updated successfully"),
                Err(e) => tracing::error!("Failed to update AMSAT module: {}", e),
            }
        }
    });
}