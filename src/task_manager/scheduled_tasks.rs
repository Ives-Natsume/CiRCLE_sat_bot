use std::thread::spawn;
use std::time::Duration;

use crate::amsat_parser::run_amsat_module;
pub fn start_scheduled_amsat_module() {
    spawn(move || {
        loop {
            run_amsat_module();

            std::thread::sleep(Duration::from_secs(30*60));
        }
    });
}