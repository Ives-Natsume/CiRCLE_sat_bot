//! External facts attached to satellites, alongside crowd-sourced AMSAT reports.
//!
//! # Why a separate concept
//!
//! Some information about a satellite does not come from AMSAT: the ASRTU team
//! exposes an internal endpoint reporting the repeater state their ground station
//! actually commanded, and an ARISS feed could announce scheduled on/off windows.
//! These are *authoritative* where AMSAT reports are *observational*, so mixing them
//! into the report list would misrepresent both.
//!
//! # Why a provider trait
//!
//! The previous integration bolted ASRTU directly onto the manager as a bespoke
//! `merge_asrtu_status` step, keyed on the hard-coded literal `"AO-123"`. Because the
//! real upstream label is `AO-123_[FM]`, that comparison never matched and the
//! function created a fresh orphan record on every poll — the feature was broken from
//! its first commit, and every additional source would have added another such branch.
//!
//! Here each source implements [`OverlayProvider`] and names its subject with a
//! [`SatTarget`], which the registry resolves to a [`SatKey`]. Adding the ARISS feed
//! means adding a provider; the manager does not change.

use super::identity::{SatKey, SatTarget};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where an overlay came from. Shown to users as attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OverlaySource {
    /// ASRTU-1 team telemetry endpoint.
    Asrtu,
    /// ARISS announcements.
    Ariss,
}

impl OverlaySource {
    /// Stable identifier for persistence and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            OverlaySource::Asrtu => "asrtu",
            OverlaySource::Ariss => "ariss",
        }
    }

    /// Human-readable attribution for display.
    pub fn attribution(self) -> &'static str {
        match self {
            OverlaySource::Asrtu => "ASRTU-1 Group",
            OverlaySource::Ariss => "ARISS",
        }
    }
}

/// The substance of an overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverlayPayload {
    /// Hardware-confirmed state commanded from the ground.
    ///
    /// Stronger evidence than any crowd report, hence kept distinct from them.
    CommandedState {
        /// Whether the transponder is commanded on.
        on: bool,
        /// Source-specific detail, e.g. the register that was read.
        detail: String,
    },
    /// An announced activity window.
    ScheduledWindow {
        /// Window start.
        start: DateTime<Utc>,
        /// Window end, when known.
        end: Option<DateTime<Utc>>,
        /// Free-text description.
        note: String,
    },
    /// A plain announcement with no machine-readable structure.
    Announcement {
        /// Announcement text.
        text: String,
    },
}

/// One external fact about one satellite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Overlay {
    /// Originating source.
    pub source: OverlaySource,
    /// When the fact was true upstream.
    pub observed_at: DateTime<Utc>,
    /// When it was retrieved, so staleness can be distinguished from age.
    pub fetched_at: DateTime<Utc>,
    /// The fact itself.
    pub payload: OverlayPayload,
}

impl Overlay {
    /// Whether this overlay is recent enough to trust.
    ///
    /// Callers decide what to do with a stale overlay; the type only reports age so
    /// that a dead upstream feed cannot silently present old state as current.
    pub fn is_fresh(&self, max_age_hours: i64) -> bool {
        Utc::now().signed_duration_since(self.observed_at) <= chrono::Duration::hours(max_age_hours)
    }
}

/// An overlay together with the satellite it describes, before resolution.
pub struct TargetedOverlay {
    /// Which satellite the provider means.
    pub target: SatTarget,
    /// The fact being reported.
    pub overlay: Overlay,
}

impl TargetedOverlay {
    /// Resolve the target to a key.
    pub fn key(&self) -> SatKey {
        self.target.resolve()
    }
}

/// A source of external satellite facts.
///
/// Implementations own their transport and parsing, and express their subject in
/// domain terms rather than by guessing at upstream label spellings.
#[allow(dead_code)]
pub trait OverlayProvider: Send + Sync {
    /// Which source this is.
    fn source(&self) -> OverlaySource;

    /// Poll the upstream feed.
    ///
    /// Returning an empty vector means "nothing to report" and is not an error;
    /// an `Err` means the feed itself failed and is worth logging.
    fn poll(&self)
        -> impl std::future::Future<Output = Result<Vec<TargetedOverlay>>> + Send;
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::sat_rev::naming::ModeClass;

    fn commanded(on: bool, observed: DateTime<Utc>) -> Overlay {
        Overlay {
            source: OverlaySource::Asrtu,
            observed_at: observed,
            fetched_at: Utc::now(),
            payload: OverlayPayload::CommandedState {
                on,
                detail: "CTCSS=0x5A".into(),
            },
        }
    }

    /// The regression that motivated `SatTarget`: the old code compared the literal
    /// `"AO-123"` against real labels like `"AO-123_[FM]"`, never matched, and
    /// orphaned a record on every poll.
    #[test]
    fn provider_target_resolves_to_the_real_record_key() {
        let targeted = TargetedOverlay {
            target: SatTarget::base_class("AO-123", Some(ModeClass::Voice)),
            overlay: commanded(true, Utc::now()),
        };

        assert_eq!(targeted.key(), SatKey::from_label("AO-123_[FM]"));
    }

    #[test]
    fn freshness_is_bounded() {
        let fresh = commanded(true, Utc::now() - chrono::Duration::minutes(30));
        assert!(fresh.is_fresh(6));

        let stale = commanded(true, Utc::now() - chrono::Duration::hours(48));
        assert!(!stale.is_fresh(6), "a dead feed must not look current");
    }

    #[test]
    fn sources_carry_attribution() {
        assert_eq!(OverlaySource::Asrtu.as_str(), "asrtu");
        assert_eq!(OverlaySource::Asrtu.attribution(), "ASRTU-1 Group");
        assert_eq!(OverlaySource::Ariss.as_str(), "ariss");
    }

    #[test]
    fn payloads_round_trip_through_json() {
        let overlay = commanded(false, Utc::now());
        let json = serde_json::to_string(&overlay).expect("serialise");
        let back: Overlay = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.payload, overlay.payload);
    }
}
