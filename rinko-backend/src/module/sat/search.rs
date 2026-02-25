use crate::module::sat::Transponder;

///! Search engine - NORAD ID based search
///!
///! Supports searching by NORAD ID, satellite name, and aliases

use super::types::{NoradId, Satellite};
use strsim::jaro_winkler;

/// Default similarity threshold for fuzzy matching
pub const DEFAULT_THRESHOLD: f64 = 0.95;

/// Search result
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub transponder: Transponder,
    pub score: f64,
    pub match_type: MatchType,
}

/// Match type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchType {
    NoradId,
    ExactName,
    Alias,
    FuzzyName,
}

/// Search for transponders
/// 
/// # Search Priority
/// 0. Special keywords (e.g. "FM", "Linear", "SSTV", "Image")
/// 1. AMSAT API name exact match && aliases exact match
/// 2. Exact match on NORAD ID
/// 3. Exact match on common name
/// 4. Fuzzy match (Jaro-Winkler >= threshold)
pub fn search_transponders(
    query: &str,
    satellites: &[Satellite],
    threshold: f64,
) -> Vec<SearchResult> {
    // Check for multiple queries separated by '/'
    if query.contains('/') {
        return search_multiple(query, satellites, threshold);
    }

    // Check for special keywords
    if let Some(special_results) = check_special_keywords(query, satellites) && !special_results.is_empty() {
        return special_results;
    }

    // Try AMSAT API name exact match && aliases exact match first
    let exact_matches = exact_match(query, satellites);
    if !exact_matches.is_empty() {
        return exact_matches;
    }
    
    // Try NORAD ID
    if let Ok(norad_id) = query.parse::<NoradId>() {
        return search_by_norad_id(norad_id, satellites);
    }
    
    // Fuzzy match
    fuzzy_match(query, satellites, threshold)
}

/// Search by NORAD ID
fn search_by_norad_id(norad_id: NoradId, satellites: &[Satellite]) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for sat in satellites {
        if sat.norad_id == norad_id {
            for trans in &sat.transponders {
                results.push(SearchResult {
                    transponder: trans.clone(),
                    score: 1.0,
                    match_type: MatchType::NoradId,
                });
            }
        }
    }
    results
}

/// Exact match on name or aliases
fn exact_match(query: &str, satellites: &[Satellite]) -> Vec<SearchResult> {
    let normalized_query = normalize_string(query);
    let mut results = Vec::new();
    
    for sat in satellites {
        // Check if satellite name/aliases match, then return all transponders
        let sat_matches = normalize_string(&sat.common_name) == normalized_query
            || sat.aliases.iter().any(|alias| normalize_string(alias) == normalized_query);
        
        // Check transponders
        for trans in &sat.transponders {
            // If satellite matched, include this transponder
            if sat_matches {
                results.push(SearchResult {
                    transponder: trans.clone(),
                    score: 1.0,
                    match_type: MatchType::ExactName,
                });
                continue;
            }
            
            // Otherwise check transponder-specific fields
            if normalize_string(&trans.label) == normalized_query 
                || normalize_string(&trans.mode) == normalized_query
                || normalize_string(&trans.amsat_api_name) == normalized_query
                || trans.aliases.iter().any(|alias| normalize_string(alias) == normalized_query)
            {
                results.push(SearchResult {
                    transponder: trans.clone(),
                    score: 1.0,
                    match_type: MatchType::ExactName,
                });
            }
        }
    }
    
    results
}

/// Fuzzy match using Jaro-Winkler
fn fuzzy_match(
    query: &str,
    satellites: &[Satellite],
    threshold: f64,
) -> Vec<SearchResult> {
    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();
    
    for sat in satellites {
        // Calculate satellite-level match score
        let sat_common_score = jaro_winkler(&sat.common_name.to_lowercase(), &query_lower);
        let sat_alias_score = sat.aliases.iter()
            .map(|alias| jaro_winkler(&alias.to_lowercase(), &query_lower))
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);
        let sat_score = sat_common_score.max(sat_alias_score);
        
        for trans in &sat.transponders {
            let label_score = jaro_winkler(&trans.label.to_lowercase(), &query_lower);
            let mode_score = jaro_winkler(&trans.mode.to_lowercase(), &query_lower);
            let api_name_score = jaro_winkler(&trans.amsat_api_name.to_lowercase(), &query_lower);
            let trans_alias_score = trans.aliases.iter()
                .map(|alias| jaro_winkler(&alias.to_lowercase(), &query_lower))
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(0.0);
            
            // Take the best score from all possible matches
            let score = sat_score.max(label_score).max(mode_score).max(api_name_score).max(trans_alias_score);
            
            if score > threshold {
                matches.push(SearchResult {
                    transponder: trans.clone(),
                    score,
                    match_type: MatchType::FuzzyName,
                });
            }
        }
    }
    
    // Sort by score descending
    matches.sort_by(|a, b| {
        b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
    });
    
    matches
}

/// Normalize string for matching.
///
/// Delegates to [`amsat_types::normalize_for_search`] so that the two modules
/// share a single canonical implementation.
pub fn normalize_string(s: &str) -> String {
    super::amsat_types::normalize_for_search(s)
}

/// Special search keywords.
///
/// Returns `Some(results)` when `query` is one of the recognised mode keywords
/// ("fm", "linear"/"lin", "sstv", "image"/"img"), or `None` otherwise.
///
/// Previously this function was silently broken: each match arm declared its
/// own inner `results` that shadowed the outer one, collected transponders, and
/// then dropped them without saving → always returned `None`. This is fixed by
/// having each arm produce a `Vec<Transponder>` that is bound to a single
/// variable outside the match.
pub fn check_special_keywords(query: &str, satellites: &[Satellite]) -> Option<Vec<SearchResult>> {
    let normalized = normalize_string(query);

    let transponders: Vec<Transponder> = match normalized.as_str() {
        "fm" => satellites
            .iter()
            .flat_map(|sat| sat.transponders.iter())
            .filter(|t| {
                t.mode.to_lowercase().contains("fm")
                    || t.label.to_lowercase().contains("fm")
                    || t.amsat_api_name.to_lowercase().contains("fm")
                    || t.aliases.iter().any(|a| a.to_lowercase().contains("fm"))
            })
            .cloned()
            .collect(),

        "linear" | "lin" => satellites
            .iter()
            .flat_map(|sat| sat.transponders.iter())
            .filter(|t| {
                t.mode.to_lowercase().contains("linear")
                    || t.mode.to_lowercase().contains("lin")
                    || t.mode.to_lowercase().contains("digi")
                    || t.label.to_lowercase().contains("linear")
                    || t.label.to_lowercase().contains("lin")
            })
            .cloned()
            .collect(),

        "sstv" => satellites
            .iter()
            .flat_map(|sat| sat.transponders.iter())
            .filter(|t| {
                t.mode.to_lowercase().contains("sstv")
                    || t.label.to_lowercase().contains("sstv")
                    || t.amsat_api_name.to_lowercase().contains("sstv")
                    || t.aliases.iter().any(|a| a.to_lowercase().contains("sstv"))
            })
            .cloned()
            .collect(),

        "image" | "img" => satellites
            .iter()
            .flat_map(|sat| sat.transponders.iter())
            .filter(|t| {
                t.mode.to_lowercase().contains("image")
                    || t.label.to_lowercase() == "image"
                    || t.amsat_api_name.to_lowercase().contains("image")
                    || t.aliases.iter().any(|a| a.to_lowercase().contains("image"))
            })
            .cloned()
            .collect(),

        _ => return None,
    };

    if transponders.is_empty() {
        None
    } else {
        Some(
            transponders
                .into_iter()
                .map(|t| SearchResult {
                    transponder: t,
                    score: 1.0,
                    match_type: MatchType::ExactName,
                })
                .collect(),
        )
    }
}

/// Search with special keywords support
pub fn search_with_keywords(
    query: &str,
    satellites: &[Satellite],
    threshold: f64,
) -> Vec<SearchResult> {
    // Check for special keywords
    if let Some(special_results) = check_special_keywords(query, satellites) {
        return special_results;
    }
    
    // Normal search
    search_transponders(query, satellites, threshold)
}

/// Search multiple queries (separated by '/')
pub fn search_multiple(
    input: &str,
    satellites: &[Satellite],
    threshold: f64,
) -> Vec<SearchResult> {
    let queries: Vec<&str> = input.split('/').map(|s| s.trim()).collect();
    let mut results = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    
    for query in queries {
        if query.is_empty() {
            continue;
        }
        
        let matches = search_transponders(query, satellites, threshold);
        for result in matches {
            if seen_ids.insert(result.transponder.amsat_api_name.clone()) {
                results.push(result);
            }
        }
    }
    
    results
}

/// Filter satellites by transponder characteristics
pub fn filter_by_transponder(
    satellites: &[Satellite],
    has_uplink: bool,
    has_downlink: bool,
    has_beacon: bool,
) -> Vec<Satellite> {
    satellites.iter()
        .filter(|sat| {
            sat.transponders.iter().any(|t| {
                let uplink_match = !has_uplink || t.uplink.is_some();
                let downlink_match = !has_downlink || t.downlink.is_some();
                let beacon_match = !has_beacon || t.beacon.is_some();
                uplink_match && downlink_match && beacon_match
            })
        })
        .cloned()
        .collect()
}

/// Get satellites with recent reports
pub fn get_active_satellites(satellites: &[Satellite]) -> Vec<Satellite> {
    satellites.iter()
        .filter(|sat| sat.is_active && sat.total_reports() > 0)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{Satellite, Transponder};

    fn create_test_satellites() -> Vec<Satellite> {
        let mut sats = Vec::new();
        
        // ISS
        let mut iss = Satellite::new(25544, "ISS");
        iss.aliases = vec!["International Space Station".to_string()];
        let mut fm_trans = Transponder::new("FM");
        fm_trans.amsat_api_name = "ISS FM".to_string();
        fm_trans.downlink = super::super::types::Frequency::Single(437.800);
        iss.transponders.push(fm_trans);
        sats.push(iss);
        
        // AO-91
        let mut ao91 = Satellite::new(43017, "AO-91");
        ao91.aliases = vec!["Fox-1B".to_string()];
        let mut fm_trans = Transponder::new("FM");
        fm_trans.amsat_api_name = "AO-91".to_string();
        fm_trans.downlink = super::super::types::Frequency::Single(145.960);
        ao91.transponders.push(fm_trans);
        sats.push(ao91);
        
        // AO-7
        let mut ao7 = Satellite::new(7530, "AO-7");
        let mut mode_a = Transponder::new("Mode A");
        mode_a.amsat_api_name = "AO-7".to_string();
        mode_a.uplink = super::super::types::Frequency::Range { start: 145.850, end: 145.950 };
        mode_a.downlink = super::super::types::Frequency::Range { start: 29.400, end: 29.500 };
        ao7.transponders.push(mode_a);
        sats.push(ao7);
        
        sats
    }

    #[test]
    fn test_search_by_norad_id() {
        let sats = create_test_satellites();
        let results = search_transponders("25544", &sats, 0.85);
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_type, MatchType::NoradId);
    }

    #[test]
    fn test_exact_name_match() {
        let sats = create_test_satellites();
        let results = search_transponders("AO-91", &sats, 0.85);
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_type, MatchType::ExactName);
    }

    #[test]
    fn test_alias_match() {
        let sats = create_test_satellites();
        let results = search_transponders("Fox-1B", &sats, 0.85);
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_type, MatchType::ExactName);
    }

    #[test]
    fn test_fuzzy_match() {
        let sats = create_test_satellites();
        let results = search_transponders("ao91", &sats, 0.80);
        
        assert!(!results.is_empty());
    }

    #[test]
    fn test_multiple_queries() {
        let sats = create_test_satellites();
        let results = search_multiple("ISS/AO-91", &sats, 0.85);
        
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_filter_by_transponder() {
        let sats = create_test_satellites();
        let filtered = filter_by_transponder(&sats, true, true, false);
        
        // AO-7 has both uplink and downlink
        assert!(filtered.iter().any(|s| s.common_name == "AO-7"));
    }
}
