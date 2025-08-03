use std::collections::HashMap;
use crate::{
    module::amsat::prelude::*,
};

impl ReportStatus {
    pub fn status_mapper(status: &str) -> Self {
        let map: HashMap<ReportStatus, Vec<&str>> = HashMap::from([
            (ReportStatus::Blue, vec!["blue", "b", "蓝"]),
            (ReportStatus::Yellow, vec!["yellow", "y", "黄"]),
            (ReportStatus::Orange, vec!["orange", "o", "橙"]),
            (ReportStatus::Red, vec!["red", "r", "红"]),
            (ReportStatus::Purple, vec!["purple", "p", "紫"]),
            (ReportStatus::Grey, vec!["grey", "unknown", "g", "灰"]),
        ]);

        for (status_enum, keywords) in map {
            if keywords.contains(&status.to_lowercase().as_str()) {
                return status_enum;
            }
        }
        ReportStatus::Grey // Default to Grey if unknown
    }
}

pub fn is_valid_callsign(
    callsign: &String,
) -> bool {
    
    if callsign.is_empty() {
        return false;
    }

    // no CJK and no special characters except `/`
    if !callsign.is_ascii() || callsign.chars().any(|c| !c.is_ascii_alphanumeric() && c != '/') {
        return false;
    }

    true
}

/// Check if the grid is a valid maidenhead grid square, accepts 4 or 6 characters or more
///  - Unfinished
pub fn is_valid_grid(
    grid: &String
) -> bool {
    let len = grid.len();
    if len % 2 != 0 || len < 4 {
        return false;
    }

    // TODO: accept 6 characters or more
    // keep only the first 4 characters for validation
    let grid = &grid[..4];

    // Check if the grid is a valid maidenhead grid square
    let (first, second) = grid.split_at(2);
    if !first.chars().all(|c| c.is_ascii_uppercase()) || !second.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    true
}
