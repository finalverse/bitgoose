//! View models shared by the server and the hydrated client.
//!
//! Deliberately separate from `bg_core::domain`: these are shaped for
//! rendering, carry only what a page needs, and — critically — contain nothing
//! that could hold source body text. The domain type has a `body_raw`; nothing
//! here does, so no page can render it even by mistake.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryCard {
    pub slug: String,
    pub kind: String,
    pub title: String,
    pub dek: String,
    pub category: String,
    pub category_label: String,
    pub source_count: i32,
    pub newsworthiness: i16,
    pub published_at: String,
    pub ago: String,
    pub assets: Vec<String>,
    pub lead_source: String,
    pub lead_url: String,
    /// Lead image, hotlinked from the publisher who syndicated it; empty when
    /// the feed carried none. Never copied to our own storage — it stays the
    /// publisher's asset on the publisher's CDN, credited through `lead_source`
    /// and linked back through `lead_url`.
    pub image_url: String,
    /// Which desk this came from, shown only on blended surfaces.
    pub beat: String,
    /// Whether the Skein had enough to say something about this story.
    ///
    /// A flag rather than the analysis itself: the card shows only that a take
    /// exists, so pulling the prose into every list query would be fetching
    /// paragraphs to render a chip.
    pub has_analysis: bool,
    /// What the lead source is: `research`, `forum`, `video`, or an outlet.
    ///
    /// A preprint has no editor and no peer review; a forum thread is an
    /// argument, not a report. Rendering either with the same furniture as a
    /// news article tells the reader something untrue about what they are
    /// looking at, so the card says which it is.
    pub source_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrontPage {
    pub lead: Option<StoryCard>,
    pub desk: Vec<StoryCard>,
    pub wire: Vec<StoryCard>,
    pub prices: Vec<Tick>,
    /// Live special topics, hottest first. Usually empty — that is the point.
    pub gaggles: Vec<GaggleCard>,
    pub honk: Option<StoryCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tick {
    pub symbol: String,
    pub price: String,
    pub change: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryPage {
    pub slug: String,
    /// Lead image, hotlinked from the publisher who syndicated it. Also the
    /// `og:image` — without one, every share of this story on X, Telegram or
    /// Discord renders as a bare text card.
    pub image_url: String,
    pub image_credit: String,
    pub image_credit_url: String,
    /// What to advertise to crawlers as this story's picture — always a URL on
    /// our own domain, never the publisher's CDN.
    ///
    /// Decided on the server because the decision is "is our copy already on
    /// disk?", and pointing a crawler at a mirror we have not fetched yet just
    /// moves the fetch onto its clock. Empty means "use the generated card".
    pub share_image: String,
    /// What a share card should say about this story.
    ///
    /// Separate from `dek` because the page shows a dek only when one was
    /// written, while a card must always say something. Computed on the server
    /// so the crawler document and the full page cannot drift apart.
    pub share_description: String,
    /// Whether this render should advertise the square card.
    ///
    /// True for WeChat, which shows link previews as a small centre-cropped
    /// square; a 1200x630 card cropped that way is a band out of the middle of
    /// a headline.
    pub square_card: bool,
    /// Provider video id when the story came from a video source. Rendered as
    /// an embedded player in place of the lead image.
    pub video_id: String,
    pub headline: String,
    pub dek: String,
    pub body_html: String,
    pub category_label: String,
    pub published_at: String,
    pub ago: String,
    pub reading_time_min: i32,
    pub kind: String,
    pub claims: Vec<ClaimCard>,
    /// Quotes pulled verbatim from the sources, rendered as pull-quotes.
    /// Separate from `claims` so the article can show them without the reader
    /// having to open the ledger.
    pub quotes: Vec<QuoteCard>,
    /// Present only when the Skein had enough source text to say something.
    pub analysis: Option<AnalysisCard>,
    pub sources: Vec<SourceCard>,
    pub corrections: Vec<CorrectionCard>,
    pub runs: Vec<RunLine>,
    pub assets: Vec<String>,
    /// `schema.org/NewsArticle` JSON-LD, built server-side.
    ///
    /// Without this a news site is effectively invisible to Google News and
    /// every downstream aggregator. Built on the server because it needs
    /// absolute URLs and ISO timestamps the client does not have.
    pub json_ld: String,
    /// Absolute canonical URL.
    pub canonical: String,
    pub iso_published: String,
    pub iso_modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClaimCard {
    pub marker: String,
    pub text: String,
    pub kind: String,
    pub verification: String,
    pub verification_label: String,
    pub confidence: f32,
    pub excerpt: Option<String>,
    pub sources: Vec<SourceCard>,
    pub disputed_by: Vec<SourceCard>,
}

/// The Skein's read on a story.
///
/// A separate card rather than more fields on [`StoryPage`] for the same reason
/// the table is separate: analysis is inference, and a surface that renders it
/// has to opt in. Anything reading `StoryPage` without touching this cannot
/// accidentally present a forecast as reporting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisCard {
    pub significance: String,
    pub direction: String,
    pub horizon: String,
    pub confidence: i16,
    /// "Likely" / "Leaning" / "Uncertain" / "Speculative" — derived from
    /// `confidence` on the server so the badge and the number can never drift.
    pub stance: String,
    pub watch: Vec<String>,
    /// Which model produced it. On a site whose pitch is a glass newsroom,
    /// withholding this from the one inferred section would be the wrong
    /// omission.
    pub model: String,
}

/// A quotable line lifted verbatim from a source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuoteCard {
    pub text: String,
    pub speaker: String,
    pub source_name: String,
    pub source_url: String,
}

/// A special topic — many independent outlets converging on one subject.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GaggleCard {
    pub slug: String,
    pub title: String,
    pub standfirst: String,
    /// The evidence for calling this a topic at all. Shown, because "seven
    /// newsrooms decided this mattered" is the entire argument for the page
    /// existing and a reader is entitled to check it.
    pub sources: i32,
    pub stories: i32,
    pub model: String,
    pub language: String,
    pub pinned: bool,
    pub analysis_html: String,
    pub watchpoints: Vec<String>,
    pub primary_sources: Vec<TopicSource>,
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicSource {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GagglePage {
    pub card: GaggleCard,
    pub stories: Vec<StoryCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceCard {
    pub name: String,
    pub slug: String,
    pub url: String,
    pub title: String,
    pub trust: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectionCard {
    pub reason: String,
    pub issued_at: String,
    pub from_version: i32,
    pub to_version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunLine {
    pub role: String,
    pub role_name: String,
    pub status: String,
    pub model: String,
    pub cost: String,
    pub latency_ms: i32,
    pub at: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlockPage {
    pub agents: Vec<AgentCard>,
    pub recent: Vec<RunLine>,
    pub runs_24h: i64,
    pub failures_24h: i64,
    pub tokens_24h: i64,
    pub cost_24h: String,
    pub published_24h: i64,
    pub claims_24h: i64,
    pub blocks_24h: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCard {
    pub role: String,
    pub name: String,
    pub beat: String,
    pub tier: String,
    pub runs_24h: i64,
    pub failed_24h: i64,
    pub tokens_24h: i64,
    pub cost_24h: String,
    pub avg_latency_ms: i64,
    pub last_note: Option<String>,
    /// Why this agent is mostly failing, in plain English — set only when the
    /// failure rate says something is actually wrong. A count without a reason
    /// is a number the reader can do nothing with, and it was the reason three
    /// separate faults went unnoticed for weeks.
    pub trouble: Option<String>,
    pub enabled: bool,
    /// This agent's daily spending mandate, in CCC.
    ///
    /// Derived from `agent_runs` rather than read from the worker's memory.
    /// The worker's own view of what it has spent is exactly the thing a
    /// reader has no reason to trust; the run ledger is the record, and
    /// computing the figure from it here means the page and the audit trail
    /// cannot disagree.
    pub mandate_budget: String,
    pub mandate_spent: String,
    /// Share of the mandate used, 0-100, for the bar.
    pub mandate_pct: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricesPage {
    pub ticks: Vec<PriceRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriceRow {
    pub symbol: String,
    pub name: String,
    pub price: String,
    pub change: Option<f64>,
    pub market_cap: Option<String>,
    pub volume: Option<String>,
    pub story_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandardsPage {
    pub sources: Vec<SourceHealthRow>,
    pub enforcement: Vec<(String, i64)>,
    pub blocks_24h: i64,
    pub max_quote_words: usize,
    pub max_verbatim_run: usize,
    pub min_desk_sources: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceHealthRow {
    pub slug: String,
    pub name: String,
    pub homepage: String,
    pub trust: i16,
    pub items: i64,
    pub enabled: bool,
    pub robots_ok: bool,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlywayPage {
    pub categories: Vec<CategoryTrend>,
    pub entities: Vec<(String, String, i64)>,
    pub topics: Vec<GaggleCard>,
    pub days: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryTrend {
    pub category: String,
    pub label: String,
    pub total: i64,
    /// Per-day counts, oldest first — drives the inline bar chart.
    pub series: Vec<i64>,
}

/// Compact relative time ("4m", "3h", "2d").
///
/// A crypto reader's first question about any story is how old it is, so the
/// answer has to fit in a meta line without wrapping.
pub fn ago(then: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (chrono::Utc::now() - then).num_seconds().max(0);
    match secs {
        s if s < 60 => "now".to_string(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 604_800 => format!("{}d", s / 86_400),
        s => format!("{}w", s / 604_800),
    }
}

/// Insert comma separators into an integer string.
fn thousands(digits: &str) -> String {
    let (sign, body) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits),
    };
    let mut out = String::with_capacity(body.len() + body.len() / 3 + 1);
    for (i, c) in body.chars().enumerate() {
        if i > 0 && (body.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

/// Compact large numbers: `$1.2B`, `$340M`.
pub fn compact_usd(v: f64) -> String {
    let a = v.abs();
    if a >= 1e12 {
        format!("${:.2}T", v / 1e12)
    } else if a >= 1e9 {
        format!("${:.1}B", v / 1e9)
    } else if a >= 1e6 {
        format!("${:.0}M", v / 1e6)
    } else if a >= 1e3 {
        format!("${:.0}K", v / 1e3)
    } else {
        format!("${v:.0}")
    }
}

/// Price with a sensible number of decimals for its magnitude.
///
/// A single rule would render either BTC with pointless trailing zeros or a
/// sub-cent token as `$0.00`.
pub fn fmt_price(v: f64) -> String {
    if v >= 1000.0 {
        // Rust format strings have no thousands separator, and a price ticker
        // without one is genuinely hard to read at a glance.
        thousands(&format!("{v:.0}"))
    } else if v >= 1.0 {
        format!("{v:.2}")
    } else if v >= 0.01 {
        format!("{v:.4}")
    } else {
        format!("{v:.6}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_stays_compact() {
        let now = chrono::Utc::now();
        assert_eq!(ago(now), "now");
        assert_eq!(ago(now - chrono::Duration::minutes(4)), "4m");
        assert_eq!(ago(now - chrono::Duration::hours(3)), "3h");
        assert_eq!(ago(now - chrono::Duration::days(2)), "2d");
        // A future timestamp must not render as a negative age.
        assert_eq!(ago(now + chrono::Duration::hours(1)), "now");
    }

    #[test]
    fn prices_scale_their_precision() {
        assert_eq!(fmt_price(62994.0), "62,994");
        assert_eq!(fmt_price(3.5), "3.50");
        assert_eq!(fmt_price(0.4321), "0.4321");
        assert_eq!(fmt_price(0.00001234), "0.000012");
    }

    #[test]
    fn large_numbers_compact() {
        assert_eq!(compact_usd(1_250_000_000_000.0), "$1.25T");
        assert_eq!(compact_usd(1_200_000_000.0), "$1.2B");
        assert_eq!(compact_usd(340_000_000.0), "$340M");
    }
}
