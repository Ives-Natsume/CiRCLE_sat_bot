//! Overlay providers: sources of satellite facts other than AMSAT.
//!
//! Each submodule implements [`super::overlay::OverlayProvider`] for one upstream
//! feed. Adding a source means adding a module here and polling it in the manager —
//! no changes to the registry or the data model.

pub mod asrtu;
