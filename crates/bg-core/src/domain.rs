//! The BitGoose domain model.
//!
//! Enums are serialized as lowercase strings rather than integers: they cross
//! into Postgres columns, JSON API responses, MCP tool output and LLM prompts,
//! and in every one of those a readable token beats an opaque ordinal.

use crate::ids::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Generates `as_str`, `Display`, `FromStr`, `ALL` and serde string
/// (de)serialization for a C-like enum.
macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $lit:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        $vis enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];

            pub const fn as_str(&self) -> &'static str {
                match self { $( $name::$variant => $lit ),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = crate::error::CoreError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $lit => Ok($name::$variant), )+
                    other => Err(crate::error::CoreError::parse(stringify!($name), other)),
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Sources & raw ingestion
// ---------------------------------------------------------------------------

str_enum! {
    /// How a source is polled.
    pub enum SourceKind {
        Rss => "rss",
        JsonApi => "json_api",
        /// Regulatory filings (SEC EDGAR, court dockets).
        Filing => "filing",
        /// Chain data — large transfers, contract deploys, governance votes.
        Onchain => "onchain",
        Social => "social",
        /// Mainstream financial press. Ingested only when an item is
        /// crypto-relevant — their feeds are mostly equities and rates, and
        /// taking them wholesale would bury the coverage we want.
        Finance => "finance",
        /// A preprint server. A paper is not a news item — it has authors, an
        /// abstract and no editor — so it gets its own kind and its own card.
        Research => "research",
        /// Hacker News, Reddit. Discussion rather than reporting: the signal is
        /// that practitioners are arguing about something. Never corroboration.
        Forum => "forum",
        /// A channel that syndicates video (YouTube channel feeds today).
        /// Entries carry a provider video id and are embedded, never rehosted.
        Video => "video",
        /// First-party press releases. Treated as interested parties, never as
        /// corroboration for a claim they are the subject of.
        Wire => "wire",
    }
}

str_enum! {
    /// Editorial sections. Deliberately narrower than Decrypt's twelve — a
    /// section nobody files to is dead weight in the nav.
    /// Which desk a story belongs to.
    ///
    /// BitGoose started as a crypto property and is now a frontier-technology
    /// newsroom whose primary beat is AI. Beat is kept separate from
    /// [`Category`] because they are genuinely orthogonal: "policy" means the
    /// EU AI Act on one desk and a stablecoin bill on the other, and a reader
    /// who wants one rarely wants the other. Collapsing them into a single flat
    /// list would force a choice between losing the section or duplicating it.
    pub enum Beat {
        Ai => "ai",
        Crypto => "crypto",
    }
}

str_enum! {
    pub enum Category {
        Markets => "markets",
        Policy => "policy",
        Tech => "tech",
        Defi => "defi",
        Business => "business",
        Security => "security",
        Ai => "ai",
        Nft => "nft",
        Gaming => "gaming",
        Culture => "culture",
        /// Published research — papers, benchmarks, evaluations.
        Research => "research",
        /// A model or system shipping: weights, APIs, capability jumps.
        Models => "models",
        /// The physical layer — chips, datacentres, energy, supply.
        Compute => "compute",
        /// Alignment, evaluations, misuse, incidents.
        Safety => "safety",
    }
}

impl Category {
    /// Display label for nav and chips.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Markets => "Markets",
            Self::Policy => "Policy",
            Self::Tech => "Tech",
            Self::Defi => "DeFi",
            Self::Business => "Business",
            Self::Security => "Security",
            Self::Ai => "AI",
            Self::Nft => "NFTs",
            Self::Gaming => "Gaming",
            Self::Culture => "Culture",
            Self::Research => "Research",
            Self::Models => "Models",
            Self::Compute => "Compute",
            Self::Safety => "Safety",
        }
    }
}

impl Beat {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Ai => "AI",
            Self::Crypto => "Crypto",
        }
    }

    /// Where a category sits when nothing better is known.
    ///
    /// Only used as a fallback: the ingest-time classifier decides a story's
    /// beat from its text, and a source can pin one. This exists so a category
    /// that is inherently one-sided never lands on the wrong desk by default.
    pub const fn of_category(c: Category) -> Option<Beat> {
        match c {
            Category::Research | Category::Models | Category::Compute | Category::Safety => {
                Some(Beat::Ai)
            }
            Category::Defi | Category::Nft => Some(Beat::Crypto),
            _ => None,
        }
    }
}

/// An upstream publisher we poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub slug: String,
    pub name: String,
    pub kind: SourceKind,
    /// Feed / endpoint URL actually polled.
    pub url: String,
    pub homepage: String,
    /// 0–100. Weights corroboration: three low-trust aggregators echoing each
    /// other is not the same as one tier-1 outlet with a named reporter.
    pub trust: i16,
    /// Pins the beat of everything this source publishes. `None` for
    /// general-interest sources, whose items are routed one at a time.
    pub beat: Option<Beat>,
    /// Result of the last robots.txt check. `false` means Scout skips it.
    pub robots_ok: bool,
    pub poll_interval_s: i32,
    /// HTTP conditional-GET state, so we re-fetch politely.
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// One item as it arrived from a source, before any editorial judgement.
///
/// `body_raw` is a **private working copy** used only for claim extraction and
/// verbatim-overlap checks. It is never selected into any API response or
/// rendered page — see [`crate::policy`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawItem {
    pub id: RawItemId,
    pub source_id: SourceId,
    pub external_id: Option<String>,
    pub canonical_url: String,
    /// SHA-256 of `canonical_url`. Unique — the dedupe key.
    pub url_hash: String,
    pub title: String,
    pub dek: Option<String>,
    pub authors: Vec<String>,
    pub published_at: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    /// Short summary supplied by the feed itself.
    pub summary_raw: Option<String>,
    #[serde(skip_serializing)]
    pub body_raw: Option<String>,
    pub body_hash: Option<String>,
    /// 64-bit SimHash over the normalized title+lede, for cheap near-dupe
    /// detection without an embedding provider.
    pub simhash: i64,
    pub lang: String,
    pub image_url: Option<String>,
    /// Provider video id when this came from a video source; `None` otherwise.
    pub video_id: Option<String>,
    /// Desk this item was routed to at ingest.
    pub beat: Option<Beat>,
    pub story_id: Option<StoryId>,
    pub triaged: bool,
}

impl RawItem {
    /// The public projection. Enforces that `body_raw` never leaves the server.
    pub fn public(&self) -> RawItemPublic {
        RawItemPublic {
            id: self.id,
            source_id: self.source_id,
            canonical_url: self.canonical_url.clone(),
            title: self.title.clone(),
            authors: self.authors.clone(),
            published_at: self.published_at,
            image_url: self.image_url.clone(),
            video_id: self.video_id.clone(),
        }
    }
}

/// What the outside world may see of a source item: a pointer, never the text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawItemPublic {
    pub id: RawItemId,
    pub source_id: SourceId,
    pub canonical_url: String,
    pub title: String,
    pub authors: Vec<String>,
    pub published_at: DateTime<Utc>,
    pub image_url: Option<String>,
    pub video_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Stories
// ---------------------------------------------------------------------------

str_enum! {
    /// Which surface a story is destined for.
    pub enum StoryKind {
        /// The Wire: fast aggregation. Headline, short AI summary, link out.
        Wire => "wire",
        /// The Desk: original synthesis across multiple sources.
        Desk => "desk",
        /// Golden Egg: long-form research.
        GoldenEgg => "golden_egg",
    }
}

str_enum! {
    pub enum StoryStatus {
        Triage => "triage",
        Clustering => "clustering",
        Drafting => "drafting",
        Review => "review",
        Published => "published",
        /// Real but not yet publishable — usually single-source on a big claim.
        Held => "held",
        Killed => "killed",
    }
}

/// An *event*, distinct from any single report of it.
///
/// Five outlets covering one hack produce five `RawItem`s and exactly one
/// `Story`. That is the whole point: it is what makes cross-source
/// corroboration and disagreement expressible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    pub id: StoryId,
    pub slug: String,
    pub kind: StoryKind,
    pub status: StoryStatus,
    /// Working title until Copydesk writes the real headline.
    pub title: String,
    /// Two or three sentences in our own words. What the Wire renders, and the
    /// fallback blurb anywhere a full article does not exist yet.
    pub summary: Option<String>,
    pub category: Category,
    /// 0–100. Drives the Desk/Wire split and front-page ranking.
    pub newsworthiness: i16,
    /// Independent-source velocity: how fast corroboration is arriving.
    /// A story picking up four outlets in ten minutes is a different animal
    /// from one that accreted four over two days.
    pub velocity: f32,
    pub source_count: i32,
    pub primary_asset: Option<String>,
    pub assets: Vec<String>,
    /// Which desk this belongs to.
    pub beat: Beat,
    pub image_url: Option<String>,
    /// Provider video id when this story came from a video source. An id, not
    /// a URL — the embed host is chosen at render time.
    pub video_id: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// Set when Gander holds or kills, so the decision is auditable.
    pub editor_note: Option<String>,
}

str_enum! {
    /// How a given source item relates to the story it was attached to.
    pub enum ItemRole {
        /// First item that created the cluster.
        Seed => "seed",
        Corroborating => "corroborating",
        /// Disagrees with the seed on a material fact. Kept deliberately —
        /// disagreement is signal, and burying it is how aggregators mislead.
        Contradicting => "contradicting",
        /// Related background, not evidence.
        Context => "context",
    }
}

// ---------------------------------------------------------------------------
// Claims — the unit of truth
// ---------------------------------------------------------------------------

str_enum! {
    pub enum ClaimKind {
        /// A discrete assertion about the world.
        Fact => "fact",
        /// A quantity. Carries `numeric_value` + `unit` so it can be checked.
        Figure => "figure",
        /// Attributed speech.
        Quote => "quote",
        /// A prediction. Never verifiable at publish time; labelled as such.
        Forecast => "forecast",
    }
}

str_enum! {
    /// Verification state. This is the number the reader actually cares about.
    pub enum Verification {
        Unverified => "unverified",
        /// Exactly one independent source. Publishable, but flagged in the UI.
        SingleSource => "single_source",
        /// Two or more independent sources agree.
        Corroborated => "corroborated",
        /// Sources materially disagree. Shown to the reader, both sides.
        Disputed => "disputed",
        /// Affirmatively contradicted by a higher-trust source.
        Refuted => "refuted",
        /// Checked against chain data or a primary filing — the strongest tier.
        PrimaryVerified => "primary_verified",
    }
}

impl Verification {
    /// Whether a claim in this state may appear in published prose.
    pub const fn publishable(&self) -> bool {
        !matches!(self, Self::Refuted)
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Unverified => "Unverified",
            Self::SingleSource => "Single source",
            Self::Corroborated => "Corroborated",
            Self::Disputed => "Disputed",
            Self::Refuted => "Refuted",
            Self::PrimaryVerified => "Primary-verified",
        }
    }
}

str_enum! {
    pub enum Stance {
        Supports => "supports",
        Contradicts => "contradicts",
        Mentions => "mentions",
    }
}

/// A single checkable assertion, with everything needed to defend it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: ClaimId,
    pub story_id: StoryId,
    /// One sentence, self-contained, no pronouns pointing outside itself.
    pub text: String,
    pub kind: ClaimKind,
    /// 0.0–1.0, assigned by Sentinel after cross-source checking.
    pub confidence: f32,
    pub verification: Verification,
    pub numeric_value: Option<Decimal>,
    pub unit: Option<String>,
    /// The moment the claim is true *as of* — figures go stale fast in crypto.
    pub as_of: Option<DateTime<Utc>>,
    pub created_by_run: Option<RunId>,
    pub created_at: DateTime<Utc>,
}

/// Links a claim to a source item, with the exact excerpt that backs it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimSource {
    pub claim_id: ClaimId,
    pub raw_item_id: RawItemId,
    pub stance: Stance,
    /// Hard-capped at [`crate::policy::MAX_QUOTE_WORDS`] words, both in the
    /// policy engine and by a database CHECK constraint.
    pub excerpt: Option<String>,
}

// ---------------------------------------------------------------------------
// Articles — a rendering of a claim set
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub id: ArticleId,
    pub story_id: StoryId,
    /// Monotonic. Corrections create a new version; old ones stay readable.
    pub version: i32,
    pub headline: String,
    pub dek: String,
    pub slug: String,
    /// Markdown. Citation markers are `[^c1]`-style and resolve through
    /// [`ArticleCitation`] to claims.
    pub body_md: String,
    pub seo_title: String,
    pub seo_desc: String,
    pub reading_time_s: i32,
    pub status: StoryStatus,
    pub published_at: Option<DateTime<Utc>>,
    /// SHA-256 of the rendered body. Makes any post-hoc edit detectable.
    pub content_hash: String,
    pub editor_run_id: Option<RunId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleCitation {
    pub article_id: ArticleId,
    /// The marker as it appears in `body_md`, e.g. `c1`.
    pub marker: String,
    pub claim_id: ClaimId,
}

/// Append-only. We never silently edit a published page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    pub id: CorrectionId,
    pub article_id: ArticleId,
    pub from_version: i32,
    pub to_version: i32,
    pub reason: String,
    pub diff_md: String,
    pub issued_at: DateTime<Utc>,
    pub agent_id: Option<AgentId>,
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

str_enum! {
    pub enum EntityKind {
        Person => "person",
        Company => "company",
        Protocol => "protocol",
        Token => "token",
        Chain => "chain",
        Regulator => "regulator",
        Fund => "fund",
        Exchange => "exchange",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub name: String,
    pub slug: String,
    pub ticker: Option<String>,
    pub aliases: Vec<String>,
    pub summary: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// The Flock
// ---------------------------------------------------------------------------

str_enum! {
    /// The ten operational roles. Every one is an AI agent; there are no humans
    /// in the publishing path.
    pub enum AgentRole {
        /// Polls sources, normalizes, dedupes. Deterministic — no LLM.
        Scout => "scout",
        /// Triage: is this news at all? Category, assets, spam filter.
        Gosling => "gosling",
        /// Clusters items into events, scores newsworthiness.
        Curator => "curator",
        /// Extracts claims and drafts the story.
        Scribe => "scribe",
        /// Cross-source verification. Assigns confidence, flags disputes.
        Sentinel => "sentinel",
        /// Attaches market and on-chain context to figures.
        Quant => "quant",
        /// Headline, dek, SEO, house style.
        Copydesk => "copydesk",
        /// Editor-in-chief. Publish / hold / kill, and front-page ranking.
        Gander => "gander",
        /// Distribution: Wire, newsletter, feeds, push.
        Herald => "herald",
        /// Post-publish monitoring; issues corrections.
        Ombuds => "ombuds",
    }
}

impl AgentRole {
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Scout => "Scout",
            Self::Gosling => "Gosling",
            Self::Curator => "Curator",
            Self::Scribe => "Scribe",
            Self::Sentinel => "Sentinel",
            Self::Quant => "Quant",
            Self::Copydesk => "Copydesk",
            Self::Gander => "Gander",
            Self::Herald => "Herald",
            Self::Ombuds => "Ombuds",
        }
    }

    /// One-line job description, shown on `/flock`.
    pub const fn beat(&self) -> &'static str {
        match self {
            Self::Scout => "Watches every source, around the clock",
            Self::Gosling => "First read on everything that lands",
            Self::Curator => "Decides what is one story and what is five",
            Self::Scribe => "Extracts the claims and writes the draft",
            Self::Sentinel => "Checks every claim against every source",
            Self::Quant => "Puts the numbers in context",
            Self::Copydesk => "Headlines, deks and house style",
            Self::Gander => "Editor-in-chief. Publishes, holds, or kills",
            Self::Herald => "Gets it to the Wire, the inbox and the feed",
            Self::Ombuds => "Re-reads what we published and corrects it",
        }
    }

    pub const fn tier(&self) -> ModelTier {
        match self {
            Self::Scout => ModelTier::None,
            Self::Gosling | Self::Curator | Self::Copydesk | Self::Herald => ModelTier::Fast,
            Self::Scribe | Self::Quant | Self::Ombuds => ModelTier::Mid,
            Self::Sentinel | Self::Gander => ModelTier::Top,
        }
    }
}

str_enum! {
    /// Capability tier a role needs. Concrete model IDs are resolved per
    /// provider in `bg-llm`, so swapping providers never touches agent code.
    pub enum ModelTier {
        /// Deterministic, no model call.
        None => "none",
        Fast => "fast",
        Mid => "mid",
        Top => "top",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub slug: String,
    pub name: String,
    pub role: AgentRole,
    pub tier: ModelTier,
    pub system_prompt: String,
    pub temperature: f32,
    pub enabled: bool,
}

str_enum! {
    pub enum RunStatus {
        Running => "running",
        Ok => "ok",
        Failed => "failed",
        /// Nothing to do — not a failure.
        Skipped => "skipped",
        /// Refused by the run budget.
        Budgeted => "budgeted",
    }
}

/// One agent invocation. Written for *every* stage, LLM-backed or not.
///
/// This table is the substrate for `/flock`: BitGoose publishes its own
/// operating costs and error rate. If we are going to claim an AI newsroom is
/// trustworthy, the ledger has to be public.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: RunId,
    pub agent_id: AgentId,
    pub role: AgentRole,
    pub story_id: Option<StoryId>,
    pub stage: String,
    pub status: RunStatus,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cost_usd: Decimal,
    pub latency_ms: i32,
    pub input_hash: Option<String>,
    pub output_hash: Option<String>,
    pub note: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Market data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    pub symbol: String,
    pub name: String,
    pub coingecko_id: Option<String>,
    pub rank: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceTick {
    pub symbol: String,
    pub ts: DateTime<Utc>,
    pub price_usd: Decimal,
    pub change_24h_pct: Option<f64>,
    pub volume_24h: Option<Decimal>,
    pub market_cap: Option<Decimal>,
}

// ---------------------------------------------------------------------------
// Composite read models
// ---------------------------------------------------------------------------

/// A claim together with everything backing it — what the ledger sidebar renders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimWithSources {
    #[serde(flatten)]
    pub claim: Claim,
    pub sources: Vec<ClaimSourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimSourceRef {
    pub raw_item_id: RawItemId,
    pub stance: Stance,
    pub excerpt: Option<String>,
    pub source_name: String,
    pub source_slug: String,
    pub source_trust: i16,
    pub url: String,
    pub title: String,
    pub published_at: DateTime<Utc>,
}

/// Everything needed to render `/story/:slug` in one payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFull {
    pub story: Story,
    pub article: Option<Article>,
    pub claims: Vec<ClaimWithSources>,
    pub sources: Vec<SourceRef>,
    pub corrections: Vec<Correction>,
    pub runs: Vec<AgentRunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub name: String,
    pub slug: String,
    pub url: String,
    pub title: String,
    pub trust: i16,
    pub role: ItemRole,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunSummary {
    pub role: AgentRole,
    pub status: RunStatus,
    pub model: String,
    pub cost_usd: Decimal,
    pub latency_ms: i32,
    pub started_at: DateTime<Utc>,
    pub note: Option<String>,
}

/// A Wire entry: pointer plus our own short summary. Never the source's text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEntry {
    pub story_id: StoryId,
    pub slug: String,
    pub title: String,
    /// 2–3 sentences, written by us.
    pub summary: String,
    pub category: Category,
    pub source_name: String,
    pub source_slug: String,
    pub source_url: String,
    pub source_count: i32,
    pub published_at: DateTime<Utc>,
    pub newsworthiness: i16,
    pub image_url: Option<String>,
    pub assets: Vec<String>,
}

/// Live newsroom stats for `/flock`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlockStats {
    pub role: AgentRole,
    pub name: String,
    pub runs_24h: i64,
    pub ok_24h: i64,
    pub failed_24h: i64,
    pub cost_24h_usd: Decimal,
    pub avg_latency_ms: i64,
    pub tokens_24h: i64,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_note: Option<String>,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn enum_roundtrips_through_its_wire_string() {
        for v in Verification::ALL {
            assert_eq!(Verification::from_str(v.as_str()).unwrap(), *v);
        }
        for c in Category::ALL {
            assert_eq!(Category::from_str(c.as_str()).unwrap(), *c);
        }
        for r in AgentRole::ALL {
            assert_eq!(AgentRole::from_str(r.as_str()).unwrap(), *r);
        }
    }

    #[test]
    fn unknown_enum_token_is_an_error_not_a_default() {
        assert!(Verification::from_str("probably_true").is_err());
    }

    #[test]
    fn refuted_claims_are_not_publishable() {
        assert!(!Verification::Refuted.publishable());
        assert!(Verification::Disputed.publishable());
    }

    #[test]
    fn the_flock_has_ten_roles_and_scout_needs_no_model() {
        assert_eq!(AgentRole::ALL.len(), 10);
        assert_eq!(AgentRole::Scout.tier(), ModelTier::None);
        assert_eq!(AgentRole::Gander.tier(), ModelTier::Top);
    }

    #[test]
    fn raw_item_public_projection_drops_the_body() {
        let json = serde_json::to_string(&RawItem {
            id: RawItemId::new(),
            source_id: SourceId::new(),
            external_id: None,
            canonical_url: "https://example.com/a".into(),
            url_hash: "deadbeef".into(),
            title: "T".into(),
            dek: None,
            authors: vec![],
            published_at: Utc::now(),
            fetched_at: Utc::now(),
            summary_raw: None,
            body_raw: Some("SECRET SOURCE TEXT".into()),
            body_hash: None,
            simhash: 0,
            lang: "en".into(),
            image_url: None,
            video_id: None,
            beat: None,
            story_id: None,
            triaged: false,
        })
        .unwrap();
        assert!(!json.contains("SECRET SOURCE TEXT"));
    }
}
