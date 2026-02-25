///! LoTW queue status HTML parser
///!
///! Parses the ARRL Logbook of the World queue status page HTML
///! (https://www.arrl.org/logbook-queue-status) into structured data.

use anyhow::{Context, Result};
use chrono::Utc;
use scraper::{Html, Selector};
use tracing::warn;

use super::types::{LotwQueueRow, LotwQueueSnapshot};

/// Parse the LoTW queue status latency text into total seconds.
///
/// Accepts formats like:
///   "0d 00h 00m 59s ago"
///   "(0d 00h 37m 25s ago)"
fn parse_latency_secs(text: &str) -> u64 {
    let text = text.trim_matches(|c| c == '(' || c == ')').trim();

    let mut total = 0u64;

    // days
    if let Some(pos) = text.find('d') {
        let s = text[..pos].trim().rsplit_once(' ').map(|(_, v)| v).unwrap_or(&text[..pos]);
        total += s.trim().parse::<u64>().unwrap_or(0) * 86400;
    }
    // hours
    if let Some(pos) = text.find('h') {
        let start = text[..pos].rfind(|c: char| c == ' ' || c == 'd').unwrap_or(0) + 1;
        total += text[start..pos].trim().parse::<u64>().unwrap_or(0) * 3600;
    }
    // minutes
    if let Some(pos) = text.find('m') {
        let start = text[..pos].rfind(|c: char| c == ' ' || c == 'h').unwrap_or(0) + 1;
        total += text[start..pos].trim().parse::<u64>().unwrap_or(0) * 60;
    }
    // seconds
    if let Some(pos) = text.find('s') {
        let start = text[..pos].rfind(|c: char| c == ' ' || c == 'm').unwrap_or(0) + 1;
        total += text[start..pos].trim().parse::<u64>().unwrap_or(0);
    }

    total
}

/// Strip HTML tags and normalise whitespace from a plain-text cell value.
fn clean_cell(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove thousands-separator commas so numbers can be parsed.
fn strip_commas(s: &str) -> String {
    s.replace(',', "")
}

/// Parse the ARRL LoTW queue status HTML into a [`LotwQueueSnapshot`].
pub fn parse_lotw_html(html: &str) -> Result<LotwQueueSnapshot> {
    let document = Html::parse_document(html);

    let row_sel = Selector::parse("tbody tr").map_err(|e| anyhow::anyhow!("selector error: {}", e))?;
    let td_sel  = Selector::parse("td").map_err(|e| anyhow::anyhow!("selector error: {}", e))?;

    let mut rows = Vec::new();

    for tr in document.select(&row_sel) {
        let tds: Vec<String> = tr
            .select(&td_sel)
            .map(|td| clean_cell(&td.text().collect::<String>()))
            .collect();

        // Expect exactly 5 columns: epoch, logs, qsos, bytes, currently-processing
        if tds.len() < 5 {
            warn!("Skipping malformed LoTW row ({} columns): {:?}", tds.len(), tds);
            continue;
        }

        let epoch               = tds[0].clone();
        let logs: u64           = strip_commas(&tds[1]).parse().unwrap_or(0);
        let qsos: u64           = strip_commas(&tds[2]).parse().unwrap_or(0);
        let bytes: u64          = strip_commas(&tds[3]).parse().unwrap_or(0);

        // Column 5 contains both the processing timestamp and the latency text,
        // separated by whitespace after the date/time string.
        // Example: "2026-02-18 04:58:03 (0d 00h 00m 59s ago)"
        let raw_processing = tds[4].clone();
        let (currently_processing, latency_text) = split_processing_cell(&raw_processing);
        let latency_secs = parse_latency_secs(&latency_text);

        rows.push(LotwQueueRow {
            epoch,
            logs,
            qsos,
            bytes,
            currently_processing,
            latency_text,
            latency_bad: latency_secs > LotwQueueSnapshot::BAD_LATENCY_SECS,
        });
    }

    Ok(LotwQueueSnapshot {
        fetched_at: Utc::now(),
        rows,
    })
}

/// Split "2026-02-18 04:58:03 (0d 00h 00m 59s ago)" into
/// ("2026-02-18 04:58:03", "0d 00h 00m 59s ago")
fn split_processing_cell(raw: &str) -> (String, String) {
    // The timestamp is always exactly "YYYY-MM-DD HH:MM:SS" (19 chars).
    if raw.len() > 19 {
        let ts   = raw[..19].to_string();
        let rest = raw[19..].trim().trim_matches(|c| c == '(' || c == ')').trim().to_string();
        // Strip trailing "ago" marker that was pulled in already
        (ts, rest)
    } else {
        (raw.to_string(), String::new())
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_parse_latency_secs_ok() {
//         assert_eq!(parse_latency_secs("0d 00h 00m 59s ago"), 59);
//         assert_eq!(parse_latency_secs("0d 00h 37m 25s ago"), 37 * 60 + 25);
//         assert_eq!(parse_latency_secs("1d 02h 03m 04s ago"), 86400 + 7200 + 180 + 4);
//         assert_eq!(parse_latency_secs("(0d 00h 09m 21s ago)"), 9 * 60 + 21);
//     }

//     #[test]
//     fn test_split_processing_cell() {
//         let (ts, lat) = split_processing_cell("2026-02-18 04:58:03 (0d 00h 00m 59s ago)");
//         assert_eq!(ts, "2026-02-18 04:58:03");
//         assert_eq!(lat, "0d 00h 00m 59s ago");

//         let (ts2, lat2) = split_processing_cell("2026-02-17 21:21:36 (0d 00h 37m 25s ago)");
//         assert_eq!(ts2, "2026-02-17 21:21:36");
//         assert_eq!(lat2, "0d 00h 37m 25s ago");
//     }
// }
