//! Satellite search: candidate generation, scoring and ranking.
//!
//! # Design
//!
//! Search is expressed as a **pipeline that always produces an ordered result**,
//! rather than a chain of early-returning exact-match attempts. The previous
//! implementation tried three exact comparisons in sequence and returned the first
//! non-empty set, which produced two user-visible defects:
//!
//! - A single typo yielded zero results, with no suggestion.
//! - Results came back in `satellite_list.toml` file order, so `"iss"` returned
//!   several entries with no indication of which was most relevant.
//!
//! Here every entry is scored, matches are sorted by [`MatchKind`] then score, and a
//! miss returns [`Suggestions`] instead of nothing.
//!
//! # Layers
//!
//! | Layer | Example query | Mechanism |
//! |---|---|---|
//! | Exact label / alias | `ao91`, `AO-91_[FM]` | normalised equality |
//! | Category | `fm`, `sstv`, `linear` | [`ModeClass`] keyword tables |
//! | Prefix / substring | `iss`, `tevel` | normalised `starts_with` / `contains` |
//! | Fuzzy | `ao9l`, `arcticsatt` | Jaro-Winkler over labels and aliases |
//!
//! This module is pure: it borrows a slice of entries and performs no I/O.

use super::naming::{normalize, ModeClass};
use super::types::SatRecord;
use strsim::jaro_winkler;

/// Maximum entries returned for a single query.
///
/// Category searches (`"fm"`) can legitimately match dozens of satellites; the
/// renderer cannot usefully draw that many, so results are capped after ranking.
pub const MAX_RESULTS: usize = 12;

/// Number of alternatives offered when a query finds nothing.
pub const MAX_SUGGESTIONS: usize = 3;

/// Similarity floor for fuzzy matching.
///
/// The old engine used 0.95, which tolerates roughly a single character of
/// difference and therefore almost never fired. 0.84 admits realistic typos
/// (`ao9l` → `ao91`) while [`min_similarity_for`] tightens the bar for short
/// queries, where Jaro-Winkler's prefix bonus makes unrelated strings score high.
const FUZZY_THRESHOLD: f64 = 0.84;

/// Maximum edit distance tolerated between a query and a candidate.
///
/// Jaro-Winkler alone cannot separate a genuine typo from a query that shares a
/// long prefix but degenerates afterwards — measured against the real corpus,
/// `"arcticsatt"` → `"arcticsat1"` scores 0.96 while the junk query
/// `"arcticsat9zz"` → `"arcticsat1"` still scores 0.93. Since no single threshold
/// separates them, fuzzy candidates must additionally survive a bounded edit
/// distance, which *does* penalise the extra garbage characters.
const MAX_EDIT_DISTANCE: usize = 2;

/// Similarity floor for offering a suggestion after a failed search.
///
/// Deliberately below [`FUZZY_THRESHOLD`]: a near miss is not good enough to
/// auto-select, but is still worth proposing.
const SUGGEST_THRESHOLD: f64 = 0.62;

/// How a query matched an entry. Ordering is significant — [`Ord`] is derived so
/// that stronger match kinds sort first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// Query equals the full label, e.g. `"AO-91_[FM]"`.
    ExactLabel,
    /// Query equals a curated alias — human-supplied knowledge outranks derived data.
    CuratedAlias,
    /// Query equals a machine-derived alias, e.g. `"ao91fm"`.
    DerivedAlias,
    /// Query equals the satellite base designator, e.g. `"ao91"`.
    BaseName,
    /// Query names a mode category, e.g. `"fm"`, `"sstv"`.
    Category,
    /// Query is a leading fragment, e.g. `"arctic"`.
    Prefix,
    /// Query appears somewhere in the label.
    Substring,
    /// Approximate match within [`FUZZY_THRESHOLD`].
    Fuzzy,
}

impl MatchKind {
    /// Short label for logs and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::ExactLabel => "exact-label",
            MatchKind::CuratedAlias => "curated-alias",
            MatchKind::DerivedAlias => "derived-alias",
            MatchKind::BaseName => "base-name",
            MatchKind::Category => "category",
            MatchKind::Prefix => "prefix",
            MatchKind::Substring => "substring",
            MatchKind::Fuzzy => "fuzzy",
        }
    }
}

/// A scored search hit.
#[derive(Debug, Clone)]
pub struct Hit<'a> {
    /// The matched entry.
    pub entry: &'a SatRecord,
    /// How it matched.
    pub kind: MatchKind,
    /// Confidence in `0.0..=1.0`. Exact matches score `1.0`.
    pub score: f64,
}

/// Alternatives offered when a query matches nothing.
///
/// Lets the caller answer "did you mean …?" instead of an opaque failure.
#[derive(Debug, Clone, Default)]
pub struct Suggestions {
    /// Candidate labels, most similar first.
    pub labels: Vec<String>,
}

impl Suggestions {
    /// Whether any alternative was found.
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }
}

/// Outcome of a search.
#[derive(Debug, Clone)]
pub enum Outcome<'a> {
    /// At least one entry matched, ranked best-first.
    Hits(Vec<Hit<'a>>),
    /// Nothing matched; `Suggestions` may offer near misses.
    Miss(Suggestions),
}

impl<'a> Outcome<'a> {
    /// Matched entries in rank order, discarding scores.
    ///
    /// Convenience for the render path, which only needs the entries.
    pub fn entries(&self) -> Vec<&'a SatRecord> {
        match self {
            Outcome::Hits(hits) => hits.iter().map(|h| h.entry).collect(),
            Outcome::Miss(_) => Vec::new(),
        }
    }
}

/// Tighten the fuzzy threshold for short queries.
///
/// Jaro-Winkler weights common prefixes heavily, so two- and three-character
/// queries score highly against many unrelated labels. Requiring near-exactness at
/// short lengths keeps `"ao"` from matching every AMSAT-OSCAR satellite.
fn min_similarity_for(query_len: usize) -> f64 {
    match query_len {
        0..=2 => 1.01, // unreachable: effectively disables fuzzy matching
        3 => 0.94,
        4 => 0.90,
        _ => FUZZY_THRESHOLD,
    }
}

/// Whether `candidate` is a plausible correction of `needle`.
///
/// Requires *both* a high similarity score and a small edit distance. The two
/// criteria fail in different directions — similarity is blind to appended
/// garbage, edit distance is blind to transpositions — so together they admit real
/// typos while rejecting queries that merely start the same way.
fn is_plausible_typo(candidate: &str, needle: &str) -> Option<f64> {
    let score = jaro_winkler(candidate, needle);
    if score < min_similarity_for(needle.chars().count()) {
        return None;
    }

    // Scale the allowance with query length: a 4-character query gets one edit,
    // longer ones get the full budget.
    let budget = if needle.chars().count() <= 4 {
        1
    } else {
        MAX_EDIT_DISTANCE
    };

    if strsim::levenshtein(candidate, needle) <= budget {
        Some(score)
    } else {
        None
    }
}

/// Search `entries` for `query`.
///
/// A query may name several targets separated by `/` (`"iss/so-50"`); each part is
/// searched independently and the results are unioned, preserving per-part rank.
pub fn search<'a>(entries: &'a [SatRecord], query: &str) -> Outcome<'a> {
    let parts: Vec<&str> = query
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    // A lone `/` term such as "U/v" is a band pair, not a multi-target query, so
    // only treat `/` as a separator when both sides survive as real queries.
    if parts.len() > 1 && !looks_like_band_pair(query) {
        let mut merged: Vec<Hit<'a>> = Vec::new();

        for part in &parts {
            if let Outcome::Hits(hits) = search_one(entries, part) {
                for hit in hits {
                    let already = merged
                        .iter()
                        .any(|h| std::ptr::eq(h.entry, hit.entry));
                    if !already {
                        merged.push(hit);
                    }
                }
            }
        }

        if merged.is_empty() {
            return Outcome::Miss(suggest(entries, parts[0]));
        }
        merged.truncate(MAX_RESULTS);
        return Outcome::Hits(merged);
    }

    search_one(entries, query)
}

/// Detect a band-pair query such as `"U/v"`, which must not be split on `/`.
fn looks_like_band_pair(query: &str) -> bool {
    let Some((lhs, rhs)) = query.trim().split_once('/') else {
        return false;
    };
    let short_alpha = |s: &str| {
        let s = s.trim();
        !s.is_empty() && s.len() <= 3 && s.chars().all(|c| c.is_ascii_alphabetic())
    };
    short_alpha(lhs) && short_alpha(rhs)
}

/// Score every entry against a single-target query and rank the matches.
fn search_one<'a>(entries: &'a [SatRecord], query: &str) -> Outcome<'a> {
    let needle = normalize(query);
    if needle.is_empty() {
        return Outcome::Miss(Suggestions::default());
    }

    let category = category_for(&needle);

    let mut hits: Vec<Hit<'a>> = Vec::new();

    for entry in entries {
        if let Some(hit) = score_entry(entry, &needle, category) {
            hits.push(hit);
        }
    }

    if hits.is_empty() {
        return Outcome::Miss(suggest(entries, query));
    }

    // Strongest match kind first, then by score, then by label for stable output.
    hits.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| a.entry.current_label.cmp(&b.entry.current_label))
    });

    // Suppress weak matches whenever a confident one exists. Without this, an exact
    // query like "AO-91_[FM]" also drags in fuzzy neighbours such as "AO-123_[FM]",
    // which is noise the renderer would faithfully draw.
    if let Some(best) = hits.first().map(|h| h.kind) {
        if best <= MatchKind::Category {
            hits.retain(|h| h.kind <= MatchKind::Category);
        }
    }

    hits.truncate(MAX_RESULTS);
    Outcome::Hits(hits)
}

/// Determine the best match for one entry, or [`None`] if it does not match.
///
/// Checks run strongest-first and return immediately, so each entry is classified
/// by its most significant match.
fn score_entry<'a>(
    entry: &'a SatRecord,
    needle: &str,
    category: Option<ModeClass>,
) -> Option<Hit<'a>> {
    let label_norm = normalize(&entry.current_label);
    let base_norm = normalize(&entry.satellite_base_name);

    let hit = |kind: MatchKind, score: f64| {
        Some(Hit {
            entry,
            kind,
            score,
        })
    };

    if label_norm == needle {
        return hit(MatchKind::ExactLabel, 1.0);
    }

    // Curated aliases carry human intent and outrank derived ones.
    if entry.manual_aliases.iter().any(|a| normalize(a) == needle) {
        return hit(MatchKind::CuratedAlias, 1.0);
    }

    if entry.aliases.iter().any(|a| normalize(a) == needle) {
        return hit(MatchKind::DerivedAlias, 1.0);
    }

    if base_norm == needle {
        return hit(MatchKind::BaseName, 1.0);
    }

    // Category search: "fm" should list every voice bird.
    if let Some(wanted) = category {
        if entry.mode_class == Some(wanted) {
            return hit(MatchKind::Category, 0.95);
        }
    }

    if label_norm.starts_with(needle) || base_norm.starts_with(needle) {
        // Longer shared prefixes relative to the label score higher.
        let ratio = needle.len() as f64 / label_norm.len().max(1) as f64;
        return hit(MatchKind::Prefix, 0.80 + 0.15 * ratio);
    }

    if label_norm.contains(needle) {
        return hit(MatchKind::Substring, 0.70);
    }

    // Fuzzy: best plausible correction across label, base and every alias.
    let mut best: Option<f64> = None;
    let mut consider = |candidate: &str| {
        if let Some(score) = is_plausible_typo(candidate, needle) {
            best = Some(best.map_or(score, |b: f64| b.max(score)));
        }
    };

    consider(&label_norm);
    consider(&base_norm);
    for alias in entry.all_aliases() {
        consider(&normalize(alias));
    }

    if let Some(score) = best {
        return hit(MatchKind::Fuzzy, score);
    }

    None
}

/// Resolve a query to a mode category, if it names one.
fn category_for(needle: &str) -> Option<ModeClass> {
    const CLASSES: &[ModeClass] = &[
        ModeClass::Voice,
        ModeClass::Linear,
        ModeClass::Imaging,
        ModeClass::Data,
        ModeClass::Telemetry,
        ModeClass::Datv,
        ModeClass::Novelty,
        ModeClass::Crew,
    ];

    CLASSES.iter().copied().find(|class| {
        class
            .search_keywords()
            .iter()
            .any(|kw| normalize(kw) == needle)
    })
}

/// Collect near misses to offer as "did you mean …?".
fn suggest(entries: &[SatRecord], query: &str) -> Suggestions {
    let needle = normalize(query);
    if needle.len() < 2 {
        return Suggestions::default();
    }

    let mut scored: Vec<(f64, &str)> = entries
        .iter()
        .map(|entry| {
            let mut best = jaro_winkler(&normalize(&entry.current_label), &needle)
                .max(jaro_winkler(&normalize(&entry.satellite_base_name), &needle));
            for alias in entry.all_aliases() {
                best = best.max(jaro_winkler(&normalize(alias), &needle));
            }
            (best, entry.current_label.as_str())
        })
        .filter(|(score, _)| *score >= SUGGEST_THRESHOLD)
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(b.1))
    });
    scored.truncate(MAX_SUGGESTIONS);

    Suggestions {
        labels: scored.into_iter().map(|(_, name)| name.to_string()).collect(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a corpus mirroring the shapes seen in production.
    fn corpus() -> Vec<SatRecord> {
        let mut entries: Vec<SatRecord> = [
            "AO-123_[FM]",
            "AO-123_[SSTV]",
            "AO-7_[U/v]",
            "AO-91_[FM]",
            "ArcticSat-1_[SSTV]",
            "IO-86_[APRS]",
            "IO-86_[FM]",
            "IO-86_[SSDV]",
            "ISS_[Crew]",
            "RS-44",
            "TEVEL2-1",
            "FO-126_[UHF_TLM]",
        ]
        .iter()
        .map(|l| SatRecord::from_label(l))
        .collect();

        // Mirror the curated nickname from satellite_list.toml.
        for e in entries.iter_mut() {
            if e.current_label.starts_with("AO-123") {
                e.manual_aliases.push("asrtu".to_string());
            }
        }
        entries
    }

    fn labels_of(outcome: &Outcome<'_>) -> Vec<String> {
        outcome
            .entries()
            .iter()
            .map(|e| e.current_label.clone())
            .collect()
    }

    #[test]
    fn exact_label_wins() {
        let c = corpus();
        let out = search(&c, "AO-91_[FM]");
        assert_eq!(labels_of(&out), vec!["AO-91_[FM]"]);
    }

    /// Derived aliases make previously unreachable entries searchable.
    #[test]
    fn finds_by_derived_alias() {
        let c = corpus();
        assert_eq!(labels_of(&search(&c, "ao91")), vec!["AO-91_[FM]"]);
        assert_eq!(labels_of(&search(&c, "arcticsat1")), vec!["ArcticSat-1_[SSTV]"]);
    }

    /// Curated nicknames outrank derived aliases.
    #[test]
    fn curated_alias_ranks_above_derived() {
        let c = corpus();
        let out = search(&c, "asrtu");
        let found = labels_of(&out);
        assert_eq!(found.len(), 2, "both AO-123 payloads expected: {found:?}");
        assert!(found.iter().all(|l| l.starts_with("AO-123")));
    }

    /// Numeric shorthand: operators say "123", not "AO-123_[FM]".
    #[test]
    fn finds_by_bare_number() {
        let c = corpus();
        let found = labels_of(&search(&c, "123"));
        assert!(
            found.iter().any(|l| l.starts_with("AO-123")),
            "expected AO-123 payloads, got {found:?}"
        );
    }

    /// Sibling payloads all surface for a shared base query.
    #[test]
    fn base_query_returns_all_payloads() {
        let c = corpus();
        let found = labels_of(&search(&c, "io86"));
        assert_eq!(found.len(), 3, "expected 3 IO-86 payloads, got {found:?}");
    }

    /// Category search — entirely absent from the previous implementation.
    #[test]
    fn category_search_lists_class_members() {
        let c = corpus();

        let fm = labels_of(&search(&c, "fm"));
        assert!(fm.contains(&"AO-91_[FM]".to_string()), "got {fm:?}");
        assert!(fm.contains(&"IO-86_[FM]".to_string()), "got {fm:?}");
        assert!(!fm.contains(&"RS-44".to_string()), "RS-44 has no mode: {fm:?}");

        let sstv = labels_of(&search(&c, "sstv"));
        assert!(sstv.contains(&"ArcticSat-1_[SSTV]".to_string()), "got {sstv:?}");

        let tlm = labels_of(&search(&c, "telemetry"));
        assert!(tlm.contains(&"FO-126_[UHF_TLM]".to_string()), "got {tlm:?}");
    }

    /// `"linear"` must reach band-pair transponders.
    #[test]
    fn category_search_reaches_band_pairs() {
        let c = corpus();
        let found = labels_of(&search(&c, "linear"));
        assert!(found.contains(&"AO-7_[U/v]".to_string()), "got {found:?}");
    }

    /// A band-pair query must not be split on its slash.
    #[test]
    fn band_pair_query_is_not_split() {
        let c = corpus();
        let found = labels_of(&search(&c, "U/v"));
        assert!(
            found.contains(&"AO-7_[U/v]".to_string()),
            "band pair query should reach AO-7: {found:?}"
        );
    }

    /// Prefix search: partial names are natural user input.
    #[test]
    fn prefix_search_works() {
        let c = corpus();
        let found = labels_of(&search(&c, "arctic"));
        assert!(found.contains(&"ArcticSat-1_[SSTV]".to_string()), "got {found:?}");
    }

    /// Typo tolerance — the headline gap in the old exact-only engine.
    #[test]
    fn tolerates_typos() {
        let c = corpus();
        let found = labels_of(&search(&c, "arcticsatt"));
        assert!(
            found.contains(&"ArcticSat-1_[SSTV]".to_string()),
            "expected fuzzy recovery, got {found:?}"
        );
    }

    /// Short queries must stay strict, or the prefix bonus matches everything.
    #[test]
    fn short_queries_do_not_over_match() {
        let c = corpus();
        match search(&c, "ao") {
            Outcome::Hits(hits) => {
                assert!(
                    hits.iter().all(|h| h.entry.current_label.starts_with("AO")),
                    "short query leaked unrelated entries: {:?}",
                    labels_of(&Outcome::Hits(hits.clone()))
                );
            }
            Outcome::Miss(_) => {}
        }
    }

    /// The discrimination case that motivated combining similarity with edit
    /// distance: both queries share the `arcticsat` prefix and score within 0.03 of
    /// each other under Jaro-Winkler alone, yet only one is a real typo.
    #[test]
    fn separates_typos_from_prefix_garbage() {
        let c = corpus();

        let typo = labels_of(&search(&c, "arcticsatt"));
        assert!(
            typo.contains(&"ArcticSat-1_[SSTV]".to_string()),
            "genuine typo should match: {typo:?}"
        );

        assert!(
            matches!(search(&c, "arcticsat9zz"), Outcome::Miss(_)),
            "prefix-sharing garbage must not match"
        );
    }

    /// A miss must propose alternatives rather than return nothing.
    #[test]
    fn miss_offers_suggestions() {
        let c = corpus();
        match search(&c, "arcticsat9zz") {
            Outcome::Miss(s) => assert!(
                !s.is_empty(),
                "expected a did-you-mean candidate for a near miss"
            ),
            Outcome::Hits(h) => panic!("expected miss, got {:?}", h.len()),
        }
    }

    /// Genuine nonsense yields a clean miss, not noise.
    #[test]
    fn unrelated_query_misses() {
        let c = corpus();
        match search(&c, "zzzqqqxxx") {
            Outcome::Miss(_) => {}
            Outcome::Hits(h) => {
                panic!("nonsense matched: {:?}", h.iter().map(|x| &x.entry.current_label).collect::<Vec<_>>())
            }
        }
    }

    /// Results must be ordered by match strength, not file order.
    #[test]
    fn results_are_ranked_by_match_kind() {
        let c = corpus();
        if let Outcome::Hits(hits) = search(&c, "ao91") {
            let kinds: Vec<MatchKind> = hits.iter().map(|h| h.kind).collect();
            let mut sorted = kinds.clone();
            sorted.sort();
            assert_eq!(kinds, sorted, "hits are not ordered by match kind");
        } else {
            panic!("expected hits for 'ao91'");
        }
    }

    /// Multi-target queries union their results.
    #[test]
    fn multi_target_query_unions_results() {
        let c = corpus();
        let found = labels_of(&search(&c, "ao91/rs44"));
        assert!(found.contains(&"AO-91_[FM]".to_string()), "got {found:?}");
        assert!(found.contains(&"RS-44".to_string()), "got {found:?}");
    }

    /// Union must not duplicate an entry reached by two sub-queries.
    #[test]
    fn multi_target_query_deduplicates() {
        let c = corpus();
        let found = labels_of(&search(&c, "ao91/ao91"));
        assert_eq!(found.len(), 1, "duplicate entry in union: {found:?}");
    }

    #[test]
    fn respects_result_cap() {
        let c = corpus();
        if let Outcome::Hits(hits) = search(&c, "fm") {
            assert!(hits.len() <= MAX_RESULTS);
        }
    }

    #[test]
    fn empty_query_misses_cleanly() {
        let c = corpus();
        assert!(matches!(search(&c, "   "), Outcome::Miss(_)));
        assert!(matches!(search(&c, ""), Outcome::Miss(_)));
    }
}
