//! AMSAT label parsing, mode classification and alias generation.
//!
//! # Why this module exists
//!
//! AMSAT's satellite naming is **upstream-controlled and historically unstable**.
//! The current `AO-123_[FM]` bracket convention is already the *second* scheme we
//! have observed; earlier data used free-form separators such as `ISS FM`,
//! `SONATE-2_SSTV` and parenthesised designators like `QMR-KWT-2_(RS95s)`.
//! There is every reason to expect a third scheme in the future.
//!
//! Consequently this module follows two hard rules:
//!
//! 1. **Structure beats vocabulary.** A mode is recognised by *where it sits* in
//!    the label (inside brackets, after a separator), not by whether it appears in
//!    a hand-maintained keyword list. The previous implementation inverted this and
//!    silently failed on ~46% of real entries because its whitelist was incomplete.
//! 2. **Unknown is a value, not a failure.** Unrecognised mode tokens become
//!    [`ModeTag::Other`] and are reported upstream, so a naming change surfaces as
//!    an alert rather than as a slow degradation of search quality.
//!
//! Nothing here performs I/O; every function is pure and snapshot-testable against
//! the real label corpus.

use serde::{Deserialize, Serialize};

// ─── Mode classification ────────────────────────────────────────────────────

/// Broad functional category of a transponder payload.
///
/// This is the axis users actually search along ("show me the FM birds"), and is
/// deliberately coarse — it groups many raw mode tokens into a handful of buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModeClass {
    /// Voice-capable repeater/transponder (FM, Codec2).
    Voice,
    /// Linear/analogue transponder, typically SSB/CW (`U/v`, `V/u`, `H/u` …).
    Linear,
    /// Image transmission (SSTV, SSDV).
    Imaging,
    /// Digital data links (GFSK, GMSK, FSK, APRS, DIGI …).
    Data,
    /// Telemetry / beacon only.
    Telemetry,
    /// Digital ATV.
    Datv,
    /// Novelty / special payloads (e.g. music transmitters).
    Novelty,
    /// Human crew voice activity (ISS astronaut contacts).
    Crew,
    /// Recognised as a mode token but not classifiable into the above.
    Unknown,
}

impl ModeClass {
    /// Stable lowercase identifier, used for search tokens and persistence.
    pub fn as_str(self) -> &'static str {
        match self {
            ModeClass::Voice => "voice",
            ModeClass::Linear => "linear",
            ModeClass::Imaging => "imaging",
            ModeClass::Data => "data",
            ModeClass::Telemetry => "telemetry",
            ModeClass::Datv => "datv",
            ModeClass::Novelty => "novelty",
            ModeClass::Crew => "crew",
            ModeClass::Unknown => "unknown",
        }
    }

    /// Search keywords that should surface every satellite of this class.
    ///
    /// Used by the query layer to answer category searches such as `"fm"` or
    /// `"sstv"` — a capability the previous implementation lost entirely.
    pub fn search_keywords(self) -> &'static [&'static str] {
        match self {
            ModeClass::Voice => &["voice", "fm", "phone"],
            ModeClass::Linear => &["linear", "lin", "ssb", "cw", "analog", "analogue"],
            ModeClass::Imaging => &["imaging", "image", "img", "sstv", "ssdv", "picture"],
            ModeClass::Data => &["data", "digi", "digital", "packet", "aprs"],
            ModeClass::Telemetry => &["telemetry", "tlm", "beacon"],
            ModeClass::Datv => &["datv", "atv", "video"],
            ModeClass::Novelty => &["novelty", "music"],
            ModeClass::Crew => &["crew", "astronaut", "voice"],
            ModeClass::Unknown => &[],
        }
    }
}

/// A parsed mode token: the raw upstream spelling plus its classification.
///
/// The raw form is retained verbatim because it is what gets rendered to users
/// and what we need when reporting an unrecognised token upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeTag {
    /// Raw token exactly as it appeared upstream, e.g. `"U/v"`, `"SSDV"`.
    pub raw: String,
    /// Coarse functional class.
    pub class: ModeClass,
}

impl ModeTag {
    /// Classify a raw mode token.
    ///
    /// Unknown tokens are preserved as [`ModeClass::Unknown`] rather than
    /// discarded — see the module-level rationale.
    pub fn classify(raw: &str) -> Self {
        let key = raw.trim().to_ascii_uppercase();
        let class = classify_mode_token(&key);
        ModeTag {
            raw: raw.trim().to_string(),
            class,
        }
    }

    /// Whether this token was recognised. Callers use this to decide whether to
    /// raise an upstream-change alert.
    pub fn is_recognised(&self) -> bool {
        self.class != ModeClass::Unknown
    }
}

/// Map a normalised (uppercase, trimmed) mode token to a [`ModeClass`].
///
/// Note the deliberate absence of bare single letters (`A`, `B`, `L`, `S`, `X`).
/// The previous whitelist contained them, yet they never occur standalone in the
/// real corpus; they only ever appear as part of a band pair such as `V/a`. Keeping
/// them was a pure false-positive source.
fn classify_mode_token(key: &str) -> ModeClass {
    // Band-pair notation (e.g. "U/V", "V/U", "H/U", "C/X", "S/X", "V/A") denotes a
    // linear transponder, identified structurally by the `/` separator rather than
    // by enumerating every possible band combination.
    if is_band_pair(key) {
        return ModeClass::Linear;
    }

    // Single-word tokens: the common case.
    if let Some(class) = classify_atom(key) {
        return class;
    }

    // ── Compound tokens ──────────────────────────────────────────────────
    // Upstream freely combines a qualifier with a band or band-pair prefix, using
    // either a space or an underscore: "V/U DIGI", "V/u_Digi", "UHF_TLM",
    // "5.8G_TLM". Rather than enumerating every combination (the mistake that made
    // the previous whitelist unmaintainable), split on either separator and let the
    // most specific recognised component decide.
    let parts: Vec<&str> = key
        .split(|c: char| c == ' ' || c == '_')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() > 1 {
        // Scan right-to-left: the payload descriptor trails the band prefix.
        for part in parts.iter().rev() {
            if let Some(class) = classify_atom(part) {
                return class;
            }
        }
        // No component was a known payload, but a band pair still implies linear.
        if parts.iter().any(|p| is_band_pair(p)) {
            return ModeClass::Linear;
        }
    }

    ModeClass::Unknown
}

/// Classify a single, separator-free mode token.
///
/// Returns [`None`] when unrecognised, letting [`classify_mode_token`] fall through
/// to compound handling.
fn classify_atom(key: &str) -> Option<ModeClass> {
    Some(match key {
        "FM" | "CODEC2" | "PHONE" | "VOICE" => ModeClass::Voice,
        "LINEAR" | "LIN" | "SSB" | "CW" | "NB" | "WB" => ModeClass::Linear,
        "SSTV" | "SSDV" | "IMAGE" | "IMG" | "CAM" => ModeClass::Imaging,
        "DATA" | "DIGI" | "APRS" | "PACKET" | "GFSK" | "GMSK" | "FSK" | "BPSK" | "QPSK"
        | "AFSK" | "LORA" => ModeClass::Data,
        "TLM" | "TELEMETRY" | "BEACON" => ModeClass::Telemetry,
        "DATV" | "ATV" => ModeClass::Datv,
        "MUSIC" => ModeClass::Novelty,
        "CREW" => ModeClass::Crew,
        _ => return None,
    })
}

/// Recognise band-pair notation such as `U/V`, `V/u`, `C/X`.
///
/// Both sides must be a short alphabetic band designator, which keeps this from
/// matching arbitrary slash-containing text.
fn is_band_pair(key: &str) -> bool {
    let Some((lhs, rhs)) = key.split_once('/') else {
        return false;
    };
    let ok = |s: &str| {
        let s = s.trim();
        !s.is_empty() && s.len() <= 3 && s.chars().all(|c| c.is_ascii_alphabetic())
    };
    ok(lhs) && ok(rhs)
}

// ─── Label parsing ──────────────────────────────────────────────────────────

/// A structurally decomposed AMSAT label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLabel {
    /// The upstream label, verbatim.
    pub raw: String,
    /// Satellite designator with mode and decoration stripped, e.g. `"AO-123"`.
    pub base: String,
    /// Mode token, when the label carries one.
    pub mode: Option<ModeTag>,
}

impl ParsedLabel {
    /// True when the label contained a mode token we could not classify.
    ///
    /// The ingest layer uses this to raise a low-urgency upstream-change alert.
    pub fn has_unrecognised_mode(&self) -> bool {
        self.mode.as_ref().is_some_and(|m| !m.is_recognised())
    }
}

/// Parse an AMSAT label into base designator plus optional mode.
///
/// Recognition proceeds from most to least reliable structure:
///
/// 1. **Bracketed suffix** `NAME_[MODE]` — authoritative. Whatever sits inside the
///    brackets *is* the mode, even if we do not recognise the token.
/// 2. **Separator suffix** `NAME_MODE` / `NAME MODE` — legacy scheme; accepted only
///    when the trailing token classifies as a known mode, since a bare suffix is
///    otherwise indistinguishable from part of the designator (`TEVEL2-1`).
/// 3. **Otherwise** the whole label is the base and there is no mode.
///
/// Parenthesised suffixes are treated as *alternate designators*, not modes: the
/// only historical example, `QMR-KWT-2_(RS95s)`, carries a secondary catalogue name
/// rather than a payload mode. They are stripped from the base but never promoted.
///
/// # Examples
///
/// ```ignore
/// parse_label("AO-123_[FM]")   // base "AO-123",     mode Voice("FM")
/// parse_label("AO-7_[V/a]")    // base "AO-7",       mode Linear("V/a")
/// parse_label("SONATE-2_SSTV") // base "SONATE-2",   mode Imaging("SSTV")
/// parse_label("TEVEL2-1")      // base "TEVEL2-1",   mode None
/// parse_label("Xyz_[QQQ]")     // base "Xyz",        mode Unknown("QQQ")
/// ```
pub fn parse_label(label: &str) -> ParsedLabel {
    let raw = label.trim();

    if raw.is_empty() {
        return ParsedLabel {
            raw: String::new(),
            base: String::new(),
            mode: None,
        };
    }

    // ── Rule 1: bracketed suffix is authoritative ────────────────────────
    if let Some(open) = raw.rfind('[') {
        let after = &raw[open + 1..];
        if let Some(close_rel) = after.rfind(']') {
            let inner = after[..close_rel].trim();
            let trailing = after[close_rel + 1..].trim();
            // Only treat as a suffix when the bracket really terminates the label.
            if !inner.is_empty() && trailing.is_empty() {
                return ParsedLabel {
                    raw: raw.to_string(),
                    base: clean_base(&raw[..open]),
                    mode: Some(ModeTag::classify(inner)),
                };
            }
        }
    }

    // ── Parenthesised suffix: alternate designator, never a mode ─────────
    if let Some(open) = raw.rfind('(') {
        let after = &raw[open + 1..];
        if let Some(close_rel) = after.rfind(')') {
            let trailing = after[close_rel + 1..].trim();
            if trailing.is_empty() {
                let base = clean_base(&raw[..open]);
                if !base.is_empty() {
                    return ParsedLabel {
                        raw: raw.to_string(),
                        base,
                        mode: None,
                    };
                }
            }
        }
    }

    // ── Rule 2: legacy separator suffix, gated on classification ─────────
    // A hyphen is NOT a candidate separator: designators like "AO-91" and
    // "TEVEL2-1" would be shredded. Only `_` and whitespace are considered.
    for sep in ['_', ' '] {
        if let Some(idx) = raw.rfind(sep) {
            let candidate = raw[idx + 1..].trim();
            if candidate.is_empty() {
                continue;
            }
            let tag = ModeTag::classify(candidate);
            if tag.is_recognised() {
                let base = clean_base(&raw[..idx]);
                if !base.is_empty() {
                    return ParsedLabel {
                        raw: raw.to_string(),
                        base,
                        mode: Some(tag),
                    };
                }
            }
        }
    }

    // ── Rule 3: no mode present ──────────────────────────────────────────
    ParsedLabel {
        raw: raw.to_string(),
        base: clean_base(raw),
        mode: None,
    }
}

/// Strip decorative separators left behind after removing a mode suffix.
///
/// Without this, `"AO-123_[FM]"` yields the base `"AO-123_"`. The stale trailing
/// underscore was invisible in the old implementation only because the search
/// normaliser happened to drop punctuation — any feature grouping by base name
/// (which is precisely what identity resolution needs) would have broken on it.
fn clean_base(s: &str) -> String {
    s.trim()
        .trim_end_matches(|c: char| c == '_' || c == '-' || c == '.' || c.is_whitespace())
        .trim()
        .to_string()
}

// ─── Search normalisation ───────────────────────────────────────────────────

/// Collapse a string to its comparison form: lowercase, alphanumeric only.
///
/// This is what makes `"AO-91"`, `"ao 91"` and `"ao_91"` interchangeable at query
/// time. Applied consistently to both stored aliases and user input.
pub fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

// ─── Alias generation ───────────────────────────────────────────────────────

/// Derive the set of search aliases for a label mechanically.
///
/// The hand-maintained alias lists in `satellite_list.toml` revealed that humans
/// were doing string manipulation the program should own. For `AO-123_[FM]` a
/// maintainer had written `["ao123", "123", "asrtu", "asrtu fm", "ao123 fm",
/// "123 fm"]` — every entry except the `asrtu` project nickname is derivable.
///
/// This function generates the derivable ones. Genuine semantic knowledge (project
/// nicknames, callsigns) stays in the curated alias list and is never overwritten.
///
/// Returned values are normalised (see [`normalize`]) and de-duplicated. The
/// numeric-only alias (`"123"`) is intentionally included: it is ambiguous across
/// satellites, and resolving that ambiguity is the ranking layer's job, not this
/// one's.
pub fn derive_aliases(parsed: &ParsedLabel) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |candidate: String| {
        if candidate.len() >= 2 && !out.contains(&candidate) {
            out.push(candidate);
        }
    };

    // Whole label, collapsed: "ao123fm"
    push(normalize(&parsed.raw));

    // Base alone: "ao123"
    let base_norm = normalize(&parsed.base);
    push(base_norm.clone());

    // Base with its alphabetic prefix removed: "123".
    // Operators routinely refer to satellites by number alone.
    let digits: String = base_norm
        .trim_start_matches(|c: char| c.is_alphabetic())
        .to_string();
    if digits != base_norm {
        push(digits.clone());
    }

    if let Some(mode) = &parsed.mode {
        let mode_norm = normalize(&mode.raw);
        if !mode_norm.is_empty() {
            // "ao123fm" is already covered by the raw label above; add the
            // base+mode and number+mode combinations.
            push(format!("{}{}", base_norm, mode_norm));
            if !digits.is_empty() && digits != base_norm {
                push(format!("{}{}", digits, mode_norm));
            }
        }
    }

    out
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: assert base and mode class in one line.
    fn check(label: &str, base: &str, class: Option<ModeClass>) {
        let p = parse_label(label);
        assert_eq!(p.base, base, "base mismatch for {label:?}");
        assert_eq!(
            p.mode.as_ref().map(|m| m.class),
            class,
            "mode class mismatch for {label:?}"
        );
    }

    #[test]
    fn parses_bracketed_modes() {
        check("AO-123_[FM]", "AO-123", Some(ModeClass::Voice));
        check("AO-123_[SSTV]", "AO-123", Some(ModeClass::Imaging));
        check("IO-86_[SSDV]", "IO-86", Some(ModeClass::Imaging));
        check("IO-86_[APRS]", "IO-86", Some(ModeClass::Data));
        check("Flamingo-1_[Music]", "Flamingo-1", Some(ModeClass::Novelty));
        check("CroCube_[GFSK]", "CroCube", Some(ModeClass::Data));
        check("Foresail-1p_[GMSK]", "Foresail-1p", Some(ModeClass::Data));
    }

    /// The trailing underscore in `AO-123_[FM]` must not leak into the base.
    #[test]
    fn strips_decoration_from_base() {
        assert_eq!(parse_label("AO-123_[FM]").base, "AO-123");
        assert_eq!(parse_label("SONATE-2_SSTV").base, "SONATE-2");
        assert_eq!(parse_label("CAS-3H_[FM]").base, "CAS-3H");
    }

    /// Band pairs are linear transponders, recognised structurally.
    #[test]
    fn parses_band_pairs_as_linear() {
        check("AO-7_[U/v]", "AO-7", Some(ModeClass::Linear));
        check("AO-7_[V/a]", "AO-7", Some(ModeClass::Linear));
        check("FO-29_[V/u]", "FO-29", Some(ModeClass::Linear));
        check("CatSat_[C/x]", "CatSat", Some(ModeClass::Linear));
        check("FO-118[H/u]", "FO-118", Some(ModeClass::Linear));
    }

    /// A qualifier after a band pair describes the payload and wins.
    #[test]
    fn band_pair_qualifier_wins() {
        check("XW-3_[V/U DIGI]", "XW-3", Some(ModeClass::Data));
        check("Foo_[V/U FM]", "Foo", Some(ModeClass::Voice));
    }

    /// Regression: tokens observed live on 2026-07-29 that the initial classifier
    /// missed. Upstream combines band prefixes with payload descriptors using either
    /// a space or an underscore, so classification must be compositional.
    #[test]
    fn classifies_compound_tokens_observed_upstream() {
        check("FO-126_[UHF_TLM]", "FO-126", Some(ModeClass::Telemetry));
        check("FO-126_[5.8G_TLM]", "FO-126", Some(ModeClass::Telemetry));
        check("INCA-2_[V/u_Digi]", "INCA-2", Some(ModeClass::Data));
        check("ISS_[Crew]", "ISS", Some(ModeClass::Crew));
    }

    /// A band prefix must not mask the payload descriptor that follows it.
    #[test]
    fn payload_descriptor_beats_band_prefix() {
        assert_eq!(classify_mode_token("UHF_TLM"), ModeClass::Telemetry);
        assert_eq!(classify_mode_token("V/U_DIGI"), ModeClass::Data);
        assert_eq!(classify_mode_token("V/U DIGI"), ModeClass::Data);
        // A band pair with no recognised payload still implies a linear transponder.
        assert_eq!(classify_mode_token("V/U_ZZZ"), ModeClass::Linear);
    }

    /// Legacy underscore/space separated labels still parse.
    #[test]
    fn parses_legacy_separator_format() {
        check("SONATE-2_SSTV", "SONATE-2", Some(ModeClass::Imaging));
        check("ISS FM", "ISS", Some(ModeClass::Voice));
        check("ISS SSTV", "ISS", Some(ModeClass::Imaging));
    }

    /// Designators must never be shredded by hyphen splitting.
    #[test]
    fn does_not_split_designators() {
        check("AO-91", "AO-91", None);
        check("RS-44", "RS-44", None);
        check("TEVEL2-1", "TEVEL2-1", None);
        check("IO-117", "IO-117", None);
    }

    /// Parenthesised suffixes are alternate designators, not modes.
    #[test]
    fn parens_are_not_modes() {
        let p = parse_label("QMR-KWT-2_(RS95s)");
        assert_eq!(p.base, "QMR-KWT-2");
        assert_eq!(p.mode, None);
    }

    /// The critical forward-compatibility property: an unknown bracketed token is
    /// still extracted as a mode, so `base` stays clean and search keeps working.
    #[test]
    fn unknown_bracketed_mode_is_preserved_not_dropped() {
        let p = parse_label("NewSat-1_[QZX]");
        assert_eq!(p.base, "NewSat-1");
        let mode = p.mode.as_ref().expect("bracketed token must be captured");
        assert_eq!(mode.raw, "QZX");
        assert_eq!(mode.class, ModeClass::Unknown);
        assert!(p.has_unrecognised_mode(), "must be flagged for alerting");
    }

    #[test]
    fn handles_empty_and_whitespace() {
        let p = parse_label("   ");
        assert_eq!(p.base, "");
        assert_eq!(p.mode, None);
    }

    #[test]
    fn normalizes_consistently() {
        assert_eq!(normalize("AO-91"), "ao91");
        assert_eq!(normalize("ao 91"), "ao91");
        assert_eq!(normalize("AO_91"), "ao91");
        assert_eq!(normalize("  Ao-91  "), "ao91");
    }

    /// Reproduces the hand-written alias list for AO-123, minus the semantic
    /// nickname `asrtu` which is curated knowledge rather than derivable.
    #[test]
    fn derives_the_aliases_humans_were_writing_by_hand() {
        let p = parse_label("AO-123_[FM]");
        let aliases = derive_aliases(&p);

        for expected in ["ao123fm", "ao123", "123", "123fm"] {
            assert!(
                aliases.contains(&expected.to_string()),
                "expected alias {expected:?} in {aliases:?}"
            );
        }
        assert!(
            !aliases.contains(&"asrtu".to_string()),
            "semantic nicknames must stay curated, not derived"
        );
    }

    /// Entries with no curated aliases become searchable by short name — this is
    /// the 45%-of-corpus gap the old code left open.
    #[test]
    fn derives_aliases_for_previously_unsearchable_entries() {
        let p = parse_label("ArcticSat-1_[SSTV]");
        let aliases = derive_aliases(&p);
        assert!(aliases.contains(&"arcticsat1".to_string()));
        assert!(aliases.contains(&"arcticsat1sstv".to_string()));
    }

    #[test]
    fn aliases_are_deduplicated_and_nonempty() {
        let p = parse_label("AO-91_[FM]");
        let aliases = derive_aliases(&p);
        let mut sorted = aliases.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), aliases.len(), "duplicates in {aliases:?}");
        assert!(aliases.iter().all(|a| a.len() >= 2));
    }

    /// Single-letter tokens were in the old whitelist but never occur standalone;
    /// treating them as modes only produced false positives.
    #[test]
    fn bare_single_letters_are_not_modes() {
        assert_eq!(classify_mode_token("A"), ModeClass::Unknown);
        assert_eq!(classify_mode_token("B"), ModeClass::Unknown);
        assert_eq!(classify_mode_token("X"), ModeClass::Unknown);
    }

    #[test]
    fn band_pair_detection_is_narrow() {
        assert!(is_band_pair("U/V"));
        assert!(is_band_pair("V/a"));
        assert!(is_band_pair("C/X"));
        assert!(!is_band_pair("AO/123456"));
        assert!(!is_band_pair("/"));
        assert!(!is_band_pair("no-slash"));
    }
}
