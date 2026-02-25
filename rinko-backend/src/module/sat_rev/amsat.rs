/// Known mode keywords that can appear as a suffix in AMSAT API names
const MODE_KEYWORDS: &[&str] = &[
    "FM", "SSTV", "DATA", "DATV", "LINEAR", "LIN", "IMAGE", "IMG",
    "CW", "SSB", "DIGI", "APRS", "PACKET", "V/U", "U/V", "H/U", "V/U FM",
    "L", "S", "X", "A", "B"
];

/// Result of parsing an AMSAT API name
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAmsatName {
    /// The satellite base name (e.g., "ISS", "AO-91")
    pub base_name: String,
    /// The mode hint extracted from the name (e.g., Some("FM"), None)
    pub mode_hint: Option<String>,
}

/// Parse an AMSAT API name into (base_name, mode_hint)
///
/// Rules:
/// 1. Split by space or hyphen from the right
/// 2. If the last token is a known mode keyword, extract it as mode_hint
/// 3. Remaining tokens form the base_name
/// 4. Special handling: names like "AO-91" where the number after hyphen
///    is NOT a mode keyword → the whole name is the base_name
///
/// Examples:
/// - "ISS-FM"     → ("ISS",   Some("FM"))
/// - "ISS FM"     → ("ISS",   Some("FM"))
/// - "ISS-SSTV"   → ("ISS",   Some("SSTV"))
/// - "AO-91"      → ("AO-91", None)
/// - "AO-7 A"     → ("AO-7",  Some("A"))
/// - "RS-44"      → ("RS-44", None)
/// - "TEVEL2-1"  → ("TEVEL2-1", None)
/// - "IO-117"     → ("IO-117", None)
pub fn parse_amsat_name(api_name: &str) -> ParsedAmsatName {
    let trimmed = api_name.trim();

    if trimmed.is_empty() {
        return ParsedAmsatName {
            base_name: String::new(),
            mode_hint: None,
        };
    }

    // First try splitting by space (most unambiguous separator)
    if let Some(space_idx) = trimmed.rfind(' ') {
        let candidate = trimmed[space_idx + 1..].trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..space_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    // Then try splitting by hyphen from the right
    // But be careful: "AO-91" is a satellite designation, not "AO" + mode "91"
    if let Some(hyphen_idx) = trimmed.rfind('-') {
        let candidate = trimmed[hyphen_idx + 1..].trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..hyphen_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    // Try splitting by `[]` and `()` as well
    // e.g. QMR-KWT-2_(RS95s) → base: "QMR-KWT-2_(RS95s)", mode: None
    // e.g. FO-118[H/u] → base: "FO-118", mode: "H/U"
    if let Some(bracket_idx) = trimmed.rfind('[') {
        let candidate = trimmed[bracket_idx + 1..].trim_end_matches(']').trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..bracket_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    if let Some(paren_idx) = trimmed.rfind('(') {
        let candidate = trimmed[paren_idx + 1..].trim_end_matches(')').trim();
        if is_mode_keyword(candidate) {
            return ParsedAmsatName {
                base_name: trimmed[..paren_idx].trim().to_string(),
                mode_hint: Some(candidate.to_uppercase()),
            };
        }
    }

    // No mode keyword found - the whole name is the base name
    ParsedAmsatName {
        base_name: trimmed.to_string(),
        mode_hint: None,
    }
}

/// Check if a string is a known mode keyword (case-insensitive)
fn is_mode_keyword(s: &str) -> bool {
    let upper = s.to_uppercase();
    MODE_KEYWORDS.iter().any(|kw| *kw == upper)
}

/// Generate search aliases for an AMSAT API name
///
/// Produces normalized variants to help with search matching.
/// e.g., "ISS-FM" → ["ISS FM", "ISSFM", "ISS-FM"]
fn generate_aliases(api_name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    let trimmed = api_name.trim();

    // Original with spaces replaced by nothing
    let no_sep: String = trimmed.chars()
        .filter(|c| !c.is_ascii_punctuation() && !c.is_whitespace())
        .collect();
    if !no_sep.is_empty() && no_sep != trimmed {
        aliases.push(no_sep);
    }

    // With hyphens replaced by spaces
    let spaces = trimmed.replace('-', " ");
    if spaces != trimmed && !aliases.contains(&spaces) {
        aliases.push(spaces);
    }

    // With spaces replaced by hyphens
    let hyphens = trimmed.replace(' ', "-");
    if hyphens != trimmed && !aliases.contains(&hyphens) {
        aliases.push(hyphens);
    }

    aliases
}

/// Normalize a string for search matching (lowercase, no punctuation/whitespace)
pub fn normalize_for_search(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_punctuation() && !c.is_whitespace())
        .collect()
}