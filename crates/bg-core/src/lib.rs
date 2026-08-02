//! # bg-core
//!
//! The BitGoose domain model, shared verbatim between the Rust server and the
//! WebAssembly client. Everything here must compile for
//! `wasm32-unknown-unknown` — no tokio, no sqlx, no reqwest.
//!
//! BitGoose inverts the usual newsroom data model. Conventional CMSes treat the
//! *article* as the atomic unit: an opaque blob of prose with a byline. Here the
//! atomic units are the **event** ([`Story`]) and the **claim** ([`Claim`]), each
//! carrying its own provenance and confidence. An [`Article`] is a *rendering* of
//! a claim set, not the source of truth.
//!
//! ```text
//!   RawItem  ──cluster──▶  Story  ──extract──▶  Claim  ──render──▶  Article
//!      │                                          │                    │
//!   provenance                              corroboration          citations
//! ```
//!
//! That inversion is what lets the site show, for any sentence on any page, how
//! many independent sources back it and what happened when they disagreed.

pub mod domain;
pub mod error;
pub mod ids;
pub mod policy;
pub mod slug;
pub mod text;

pub use domain::*;
pub use error::{CoreError, Result};
pub use ids::*;

/// Wire format version for the public API and MCP surface. Bump on breaking
/// changes to any serialized shape in [`domain`].
pub const API_VERSION: &str = "v1";

/// Editorial brand constants, used by both the renderer and the agents' prompts
/// so the voice stays consistent between what we generate and what we display.
pub mod brand {
    pub const NAME: &str = "BitGoose";
    pub const DOMAIN: &str = "bitgoose.com";
    pub const TAGLINE: &str = "The AI newsroom for crypto.";
    /// Shown on every AI-written page. Non-negotiable disclosure.
    pub const AI_DISCLOSURE: &str =
        "Written by the BitGoose Flock — autonomous AI agents. Every claim links to its sources.";

    /// The crawler's identity.
    ///
    /// Lives here, not in `bg-ingest`, because two crates need to agree on it:
    /// the ingester sends it, and the web tier serves the `/bot` page it points
    /// at. If those drift, a publisher looking up an unfamiliar agent finds a
    /// page describing a different one.
    pub const DEFAULT_UA: &str =
        "Mozilla/5.0 (compatible; BitGooseBot/0.1; +https://bitgoose.com/bot)";
}
