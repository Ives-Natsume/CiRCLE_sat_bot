use std::collections::HashMap;
use std::sync::Arc;
use crate::{
    app_status::AppStatus,
    module::amsat::prelude::*,
    msg::prelude::MessageEvent
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
pub fn is_valid_maidenhead_grid(grid: &str) -> bool {
    let chars: Vec<char> = grid.chars().collect();
    let len = chars.len();

    if len < 4 || len % 2 != 0 {
        return false;
    }

    // check for first pair: uppercase A to R
    // accept lowercase here
    if !valid_uppercase(chars[0].to_ascii_uppercase()) || !valid_uppercase(chars[1].to_ascii_uppercase()) {
        return false;
    }

    // second pair: numbers 0 to 9
    if !chars[2].is_ascii_digit() || !chars[3].is_ascii_digit() {
        return false;
    }

    // third pair: lowercase a to x
    if len >= 6 {
        if !valid_lowercase(chars[4]) || !valid_lowercase(chars[5]) {
            return false;
        }
    }

    // fourth pair: 0 to 9
    if len == 8 {
        if !chars[6].is_ascii_digit() || !chars[7].is_ascii_digit() {
            return false;
        }
    }

    true
}

// Check if char is in A-R
fn valid_uppercase(c: char) -> bool {
    ('A'..='R').contains(&c)
}

// Check if char is in a-x
fn valid_lowercase(c: char) -> bool {
    ('a'..='x').contains(&c)
}

pub fn callsign_auth(
    callsign: &String,
    payload: &MessageEvent,
    admin_list: &Vec<u64>
) -> bool {
    let nickname = payload.sender.card.clone();
    let user_id = payload.sender.user_id.clone();
    if !nickname.to_uppercase().contains(callsign) && !admin_list.contains(&user_id) {
        return false;
    }

    true
}
