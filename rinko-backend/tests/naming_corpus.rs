//! Corpus-level guarantees for label parsing, checked against the real
//! `data/satellite_list.toml` rather than hand-picked examples.
//!
//! The previous parser passed its own unit tests while mis-parsing roughly 46% of
//! production labels, because those tests only covered invented inputs. These tests
//! exist so that class of regression cannot recur: they load whatever labels are
//! actually cached and assert corpus-wide invariants.

use rinko_backend::module::sat_rev::naming::{derive_aliases, parse_label, ModeClass};

/// Read every `api_name` from the cached satellite list.
///
/// Parsed with a light scan rather than the TOML crate so the test stays useful
/// even if the file schema drifts.
fn corpus() -> Vec<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/satellite_list.toml");
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read satellite list at {path}: {e}"));

    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("api_name")?.trim_start();
            let rest = rest.strip_prefix('=')?.trim();
            let inner = rest.strip_prefix('"')?;
            let end = inner.find('"')?;
            Some(inner[..end].to_string())
        })
        .collect()
}

#[test]
fn corpus_is_present_and_plausible() {
    let labels = corpus();
    assert!(
        labels.len() >= 50,
        "corpus looks truncated: {} labels",
        labels.len()
    );
}

/// Every bracketed label must yield a mode. The old whitelist-gated parser failed
/// this for tokens such as `SSDV`, `MUSIC`, `UHF_DIGI` and `GFSK` — about 46% of
/// the corpus — silently collapsing them into the base name.
#[test]
fn every_bracketed_label_yields_a_mode() {
    let mut failures = Vec::new();

    for label in corpus() {
        if !label.contains('[') {
            continue;
        }
        let parsed = parse_label(&label);
        if parsed.mode.is_none() {
            failures.push(label);
        }
    }

    assert!(
        failures.is_empty(),
        "labels with bracket notation produced no mode: {failures:?}"
    );
}

/// Bases must be free of the decoration left behind by suffix removal.
///
/// A stale trailing `_` (as in `AO-123_`) was previously masked by the search
/// normaliser dropping punctuation, but it corrupts any grouping keyed on the base
/// name — exactly what identity resolution will rely on.
#[test]
fn bases_are_clean() {
    let mut dirty = Vec::new();

    for label in corpus() {
        let parsed = parse_label(&label);
        let base = &parsed.base;

        let bad = base.is_empty()
            || base.ends_with('_')
            || base.ends_with('-')
            || base.ends_with(' ')
            || base.contains('[')
            || base.contains(']')
            || base.contains('(')
            || base.contains(')');

        if bad {
            dirty.push(format!("{label:?} -> {base:?}"));
        }
    }

    assert!(dirty.is_empty(), "unclean bases: {dirty:?}");
}

/// Report classification coverage and require that the corpus is fully classified.
///
/// If AMSAT introduces a new mode token this test fails loudly with the offending
/// label, which is the intended signal to add a mapping — rather than letting search
/// quality decay unnoticed.
#[test]
fn corpus_modes_are_fully_classified() {
    let mut unknown = Vec::new();

    for label in corpus() {
        let parsed = parse_label(&label);
        if let Some(mode) = &parsed.mode {
            if mode.class == ModeClass::Unknown {
                unknown.push(format!("{label:?} -> raw mode {:?}", mode.raw));
            }
        }
    }

    assert!(
        unknown.is_empty(),
        "unclassified mode tokens (add them to classify_mode_token): {unknown:?}"
    );
}

/// Every label must become searchable without human curation.
///
/// 45% of the corpus shipped with an empty alias list, making those satellites
/// reachable only by typing the full decorated label.
#[test]
fn every_label_gets_usable_aliases() {
    let mut bare = Vec::new();

    for label in corpus() {
        let parsed = parse_label(&label);
        let aliases = derive_aliases(&parsed);

        // The collapsed base must always be derivable — that is the short form
        // operators actually type.
        let base_norm: String = parsed
            .base
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();

        if !aliases.contains(&base_norm) {
            bare.push(format!("{label:?} -> {aliases:?} (missing {base_norm:?})"));
        }
    }

    assert!(bare.is_empty(), "labels lacking a base alias: {bare:?}");
}

/// Aliases must be normalised and duplicate-free, since the search index will treat
/// them as lookup keys.
#[test]
fn aliases_are_normalised_and_unique() {
    for label in corpus() {
        let parsed = parse_label(&label);
        let aliases = derive_aliases(&parsed);

        let mut seen = std::collections::HashSet::new();
        for alias in &aliases {
            assert!(
                alias.chars().all(|c| c.is_alphanumeric()),
                "alias {alias:?} for {label:?} is not normalised"
            );
            assert!(
                seen.insert(alias.clone()),
                "duplicate alias {alias:?} for {label:?}"
            );
        }
    }
}

/// Multi-payload satellites must share a base, so that one satellite's entries can
/// be grouped. `IO-86_[APRS]`, `IO-86_[FM]` and `IO-86_[SSDV]` are the canonical case.
#[test]
fn sibling_payloads_share_a_base() {
    use std::collections::HashMap;

    let mut by_base: HashMap<String, Vec<String>> = HashMap::new();
    for label in corpus() {
        let parsed = parse_label(&label);
        by_base.entry(parsed.base).or_default().push(label);
    }

    let io86 = by_base
        .get("IO-86")
        .expect("IO-86 payloads should group under a single base");
    assert!(
        io86.len() >= 2,
        "expected multiple IO-86 payloads, found {io86:?}"
    );
}
