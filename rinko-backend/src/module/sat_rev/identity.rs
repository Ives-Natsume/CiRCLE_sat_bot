//! Stable satellite identity, decoupled from upstream naming.
//!
//! # The problem this solves
//!
//! AMSAT labels are upstream-controlled and change without notice. The previous
//! design used the label itself as the primary key, so a rename was
//! indistinguishable from a new satellite: the old entry became an orphan that was
//! polled forever, curated aliases stayed attached to it, and the replacement
//! entry started empty. One upstream rename wiped months of curation.
//!
//! A [`SatKey`] is derived from the two most durable properties of a payload — its
//! satellite designator and its *functional* mode class — and never changes once
//! assigned. Labels become mere observations recorded against the key.
//!
//! # Why the mode *class* and not the raw token
//!
//! Raw tokens drift (`SSDV` vs `SSTV`, `TLM` vs `UHF_TLM`) but the function does
//! not: the AO-123 voice repeater is a voice repeater under any spelling. Keying on
//! [`ModeClass`] therefore survives token-level renames automatically:
//! `AO-7 A` and `AO-7_[V/a]` collapse to the same key without any rename table.
//!
//! This module is pure — no I/O, no state.

use super::naming::{self, ModeClass, ParsedLabel};
use serde::{Deserialize, Serialize};

/// A stable internal identity for one satellite payload.
///
/// Rendered as `"<base>#<class>"`, e.g. `"ao123#voice"`, or just `"<base>"` when the
/// satellite has no distinguishable mode (`"rs44"`).
///
/// Treat the inner string as opaque: it is a persistence format, not a display name.
/// Use [`Self::as_str`] for storage and lookups, never for user-facing output —
/// that is what [`crate::module::sat_rev::types::SatRecord::current_label`] is for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SatKey(String);

impl SatKey {
    /// Derive the key for a parsed label.
    ///
    /// Two labels yield the same key when they name the same designator *and* the
    /// same functional class — the property that makes renames non-destructive.
    pub fn from_parsed(parsed: &ParsedLabel) -> Self {
        let base = naming::normalize(&parsed.base);

        // An unclassifiable token cannot be trusted to be stable, so fall back to
        // the raw spelling: a wrong-but-consistent key is still usable, whereas
        // lumping every unknown payload under one key would merge distinct
        // satellites. Recognised classes take the stable path.
        let suffix = match &parsed.mode {
            None => None,
            Some(tag) if tag.class == ModeClass::Unknown => Some(naming::normalize(&tag.raw)),
            Some(tag) => Some(tag.class.as_str().to_string()),
        };

        let raw = match suffix {
            Some(s) if !s.is_empty() => format!("{base}#{s}"),
            _ => base,
        };

        SatKey(raw)
    }

    /// Derive the key directly from an upstream label.
    pub fn from_label(label: &str) -> Self {
        Self::from_parsed(&naming::parse_label(label))
    }

    /// Compose a key from an already-normalised base and optional class.
    ///
    /// Used by overlay providers, which know the satellite in domain terms
    /// (“the AO-123 voice repeater”) rather than by upstream label.
    pub fn compose(base: &str, class: Option<ModeClass>) -> Self {
        let base = naming::normalize(base);
        match class {
            Some(c) => SatKey(format!("{base}#{}", c.as_str())),
            None => SatKey(base),
        }
    }

    /// Opaque string form, for persistence and map lookups.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The designator portion, without the class suffix.
    pub fn base(&self) -> &str {
        self.0.split('#').next().unwrap_or(&self.0)
    }

    /// Whether this key was built with no mode component.
    pub fn is_modeless(&self) -> bool {
        !self.0.contains('#')
    }
}

impl std::fmt::Display for SatKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How an external data source names the payload it is reporting about.
///
/// Providers describe their target in domain terms and let the registry resolve it.
/// The previous ASRTU integration hard-coded the literal `"AO-123"` and compared it
/// against real labels like `"AO-123_[FM]"`; because those never matched it silently
/// created an orphan record on every poll. Expressing intent instead of a guessed
/// string removes that whole failure mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SatTarget {
    /// An exact key, when the provider already knows it.
    Key(SatKey),
    /// A designator plus optional functional class, e.g. “AO-123, voice”.
    BaseAndClass {
        /// Satellite designator; normalised during resolution.
        base: String,
        /// Functional class, or [`None`] to match a modeless payload.
        class: Option<ModeClass>,
    },
}

impl SatTarget {
    /// Convenience constructor for the common designator-plus-class case.
    pub fn base_class(base: impl Into<String>, class: Option<ModeClass>) -> Self {
        SatTarget::BaseAndClass {
            base: base.into(),
            class,
        }
    }

    /// Resolve to the key this target denotes.
    pub fn resolve(&self) -> SatKey {
        match self {
            SatTarget::Key(k) => k.clone(),
            SatTarget::BaseAndClass { base, class } => SatKey::compose(base, *class),
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn key(label: &str) -> String {
        SatKey::from_label(label).as_str().to_string()
    }

    #[test]
    fn derives_stable_keys() {
        assert_eq!(key("AO-123_[FM]"), "ao123#voice");
        assert_eq!(key("AO-91_[FM]"), "ao91#voice");
        assert_eq!(key("RS-44"), "rs44");
        assert_eq!(key("TEVEL2-1"), "tevel21");
    }

    /// The central guarantee: a token-level rename must not change the key.
    /// `AO-7 A` was the historical spelling of what upstream now calls `AO-7_[V/a]`.
    #[test]
    fn survives_upstream_relabelling() {
        assert_eq!(key("AO-7_[V/a]"), key("AO-7 V/a"));
        assert_eq!(key("FO-118[H/u]"), key("FO-118_[H/u]"));
        assert_eq!(key("AO-123_[FM]"), key("AO-123 FM"));
        // Decoration and case are irrelevant.
        assert_eq!(key("ao-123_[fm]"), key("AO-123_[FM]"));
    }

    /// Different spellings of the same *function* converge, which is what makes the
    /// key robust: imaging is imaging whether upstream writes SSTV or SSDV.
    #[test]
    fn equivalent_modes_share_a_key() {
        assert_eq!(key("Foo_[SSTV]"), key("Foo_[SSDV]"));
        assert_eq!(key("Bar_[TLM]"), key("Bar_[UHF_TLM]"));
    }

    /// Distinct payloads on one satellite must stay distinct.
    #[test]
    fn sibling_payloads_get_distinct_keys() {
        let fm = key("IO-86_[FM]");
        let aprs = key("IO-86_[APRS]");
        let ssdv = key("IO-86_[SSDV]");
        assert_ne!(fm, aprs);
        assert_ne!(fm, ssdv);
        assert_ne!(aprs, ssdv);
        // …while still sharing a base.
        for k in [&fm, &aprs, &ssdv] {
            assert_eq!(SatKey(k.clone()).base(), "io86");
        }
    }

    /// Unknown tokens fall back to their raw spelling so distinct unknown payloads
    /// are not merged into a single bucket.
    #[test]
    fn unknown_modes_do_not_collide() {
        let a = key("NewSat_[QQQ]");
        let b = key("NewSat_[ZZZ]");
        assert_ne!(a, b, "distinct unknown payloads must not merge");
        assert_eq!(a, "newsat#qqq");
    }

    #[test]
    fn exposes_base_and_modelessness() {
        let k = SatKey::from_label("AO-123_[FM]");
        assert_eq!(k.base(), "ao123");
        assert!(!k.is_modeless());

        let k = SatKey::from_label("RS-44");
        assert_eq!(k.base(), "rs44");
        assert!(k.is_modeless());
    }

    /// A provider expressing domain intent must land on the same key as the label.
    /// This is precisely what the old hard-coded `"AO-123"` failed to do.
    #[test]
    fn provider_target_resolves_to_label_key() {
        let from_label = SatKey::from_label("AO-123_[FM]");
        let from_target = SatTarget::base_class("AO-123", Some(ModeClass::Voice)).resolve();
        assert_eq!(from_label, from_target);
    }

    #[test]
    fn modeless_target_resolves() {
        let from_label = SatKey::from_label("RS-44");
        let from_target = SatTarget::base_class("RS-44", None).resolve();
        assert_eq!(from_label, from_target);
    }

    #[test]
    fn key_target_passes_through() {
        let k = SatKey::from_label("AO-91_[FM]");
        assert_eq!(SatTarget::Key(k.clone()).resolve(), k);
    }

    #[test]
    fn empty_label_yields_empty_key() {
        assert_eq!(key(""), "");
        assert_eq!(key("   "), "");
    }
}
