//! The source roster.
//!
//! Trust scores weight corroboration in `bg-agents::sentinel`. They are a
//! judgement about *editorial process* — does the outlet employ named reporters,
//! does it correct itself, does it break stories or reprint them — not about
//! whether we like its coverage. They are visible on `/standards` precisely
//! because they are contestable.
//!
//! Every URL here was verified reachable before it was added.

use bg_core::domain::{Beat, SourceKind};
use bg_db::{sources, Db, Result};

pub struct SeedSource {
    pub slug: &'static str,
    pub name: &'static str,
    pub kind: SourceKind,
    pub url: &'static str,
    pub homepage: &'static str,
    pub trust: i16,
    pub poll_interval_s: i32,
    /// Pins every item from this source to one desk. `None` means the source
    /// publishes across beats and each item is routed by its own text.
    pub beat: Option<Beat>,
}

pub const SOURCES: &[SeedSource] = &[
    SeedSource {
        slug: "coindesk",
        name: "CoinDesk",
        kind: SourceKind::Rss,
        // No trailing slash: the slashed form 308-redirects.
        url: "https://www.coindesk.com/arc/outboundfeeds/rss",
        homepage: "https://www.coindesk.com",
        trust: 85,
        poll_interval_s: 180,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "theblock",
        name: "The Block",
        kind: SourceKind::Rss,
        url: "https://www.theblock.co/rss.xml",
        homepage: "https://www.theblock.co",
        trust: 84,
        poll_interval_s: 180,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "decrypt",
        name: "Decrypt",
        kind: SourceKind::Rss,
        url: "https://decrypt.co/feed",
        homepage: "https://decrypt.co",
        trust: 78,
        poll_interval_s: 180,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "dlnews",
        name: "DL News",
        kind: SourceKind::Rss,
        url: "https://www.dlnews.com/arc/outboundfeeds/rss/",
        homepage: "https://www.dlnews.com",
        trust: 76,
        poll_interval_s: 300,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "blockworks",
        name: "Blockworks",
        kind: SourceKind::Rss,
        // .com, not .co — the .co domain 308-redirects here.
        url: "https://blockworks.com/feed",
        homepage: "https://blockworks.com",
        trust: 76,
        poll_interval_s: 300,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "thedefiant",
        name: "The Defiant",
        kind: SourceKind::Rss,
        url: "https://thedefiant.io/api/feed",
        homepage: "https://thedefiant.io",
        trust: 72,
        poll_interval_s: 300,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "bitcoinmagazine",
        name: "Bitcoin Magazine",
        kind: SourceKind::Rss,
        url: "https://bitcoinmagazine.com/feed",
        homepage: "https://bitcoinmagazine.com",
        trust: 70,
        poll_interval_s: 600,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "cointelegraph",
        name: "Cointelegraph",
        kind: SourceKind::Rss,
        url: "https://cointelegraph.com/rss",
        homepage: "https://cointelegraph.com",
        trust: 64,
        poll_interval_s: 300,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "cryptoslate",
        name: "CryptoSlate",
        kind: SourceKind::Rss,
        url: "https://cryptoslate.com/feed/",
        homepage: "https://cryptoslate.com",
        trust: 58,
        poll_interval_s: 600,
        beat: Some(Beat::Crypto),
    },
    // -- mainstream finance ---------------------------------------------------
    // When Bloomberg or the FT covers crypto it is itself news: it signals the
    // story has reached the institutional audience, and their sourcing is often
    // better than the crypto desks'. But their feeds are overwhelmingly
    // equities and rates, so every item passes `relevance::is_crypto` before it
    // is stored — see that module for why the gate is a word list and not a
    // model call.
    //
    // Reuters and CNN Money were tested and dropped: Reuters' public RSS
    // endpoints 404 and CNN's money feed no longer resolves.
    //
    // Trust is high — these are large newsrooms with real corrections policies.
    SeedSource {
        slug: "yahoofinance",
        name: "Yahoo Finance",
        kind: SourceKind::Finance,
        url: "https://finance.yahoo.com/news/rssindex",
        homepage: "https://finance.yahoo.com",
        trust: 68,
        poll_interval_s: 600,
        // Routed per item: these feeds are mostly equities and rates, and
        // pinning a beat here would bypass `relevance::classify`
        // altogether — which it briefly did, letting iron ore and yen
        // intervention into the database.
        beat: None,
    },
    SeedSource {
        slug: "bloomberg",
        name: "Bloomberg",
        kind: SourceKind::Finance,
        url: "https://feeds.bloomberg.com/markets/news.rss",
        homepage: "https://www.bloomberg.com/markets",
        trust: 90,
        poll_interval_s: 600,
        // Routed per item: these feeds are mostly equities and rates, and
        // pinning a beat here would bypass `relevance::classify`
        // altogether — which it briefly did, letting iron ore and yen
        // intervention into the database.
        beat: None,
    },
    SeedSource {
        slug: "cnbc",
        name: "CNBC",
        kind: SourceKind::Finance,
        url: "https://www.cnbc.com/id/10000664/device/rss/rss.html",
        homepage: "https://www.cnbc.com/finance",
        trust: 80,
        poll_interval_s: 600,
        // Routed per item: these feeds are mostly equities and rates, and
        // pinning a beat here would bypass `relevance::classify`
        // altogether — which it briefly did, letting iron ore and yen
        // intervention into the database.
        beat: None,
    },
    SeedSource {
        slug: "marketwatch",
        name: "MarketWatch",
        kind: SourceKind::Finance,
        url: "https://feeds.content.dowjones.io/public/rss/mw_topstories",
        homepage: "https://www.marketwatch.com",
        trust: 79,
        poll_interval_s: 600,
        // Routed per item: these feeds are mostly equities and rates, and
        // pinning a beat here would bypass `relevance::classify`
        // altogether — which it briefly did, letting iron ore and yen
        // intervention into the database.
        beat: None,
    },
    SeedSource {
        slug: "ft",
        name: "Financial Times",
        kind: SourceKind::Finance,
        url: "https://www.ft.com/companies?format=rss",
        homepage: "https://www.ft.com",
        trust: 90,
        poll_interval_s: 900,
        // Routed per item: these feeds are mostly equities and rates, and
        // pinning a beat here would bypass `relevance::classify`
        // altogether — which it briefly did, letting iron ore and yen
        // intervention into the database.
        beat: None,
    },
    // -- video ---------------------------------------------------------------
    // None of the text feeds above carry any video: no video MIME types, no
    // YouTube links, no iframes — every enclosure is an image. Video therefore
    // comes from channels that syndicate it directly. These are public RSS
    // feeds, and playback goes through YouTube's own embed, so the creator
    // keeps control and their analytics. Trust scores are lower than the news
    // desks on purpose: these are commentary, useful as colour and context,
    // never as corroboration for a claim.
    //
    // Polled every 30 minutes; channels publish a few times a day at most.
    SeedSource {
        slug: "yt-coinbureau",
        name: "Coin Bureau",
        kind: SourceKind::Video,
        url: "https://www.youtube.com/feeds/videos.xml?channel_id=UCnThE8FLrlN-tYvZhZL0uaA",
        homepage: "https://www.youtube.com/@coinbureau",
        trust: 70,
        poll_interval_s: 1800,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "yt-bankless",
        name: "Bankless",
        kind: SourceKind::Video,
        url: "https://www.youtube.com/feeds/videos.xml?channel_id=UCCRxYlYOmLE2l5wxs3ckJtg",
        homepage: "https://www.youtube.com/@Bankless",
        trust: 72,
        poll_interval_s: 1800,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "yt-milkroad",
        name: "Milk Road",
        kind: SourceKind::Video,
        url: "https://www.youtube.com/feeds/videos.xml?channel_id=UCWPil6c2lnmMbh2cJRwuHzQ",
        homepage: "https://www.youtube.com/@MilkRoadDaily",
        trust: 65,
        poll_interval_s: 1800,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "yt-unchained",
        name: "Unchained",
        kind: SourceKind::Video,
        url: "https://www.youtube.com/feeds/videos.xml?channel_id=UCuKiSkbYrUOOEEiYQEVPniQ",
        homepage: "https://www.youtube.com/@unchainedcrypto",
        trust: 75,
        poll_interval_s: 1800,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "yt-cryptobanter",
        name: "Crypto Banter",
        kind: SourceKind::Video,
        url: "https://www.youtube.com/feeds/videos.xml?channel_id=UCybasP-2D2b5kTLAb_kvhWQ",
        homepage: "https://www.youtube.com/@CryptoBanterGroup",
        trust: 55,
        poll_interval_s: 1800,
        beat: Some(Beat::Crypto),
    },
    // =====================================================================
    // The AI desk
    // =====================================================================
    //
    // Every URL below was fetched and checked for fresh items before being
    // added. Four candidates were tested and rejected: Anthropic publishes no
    // RSS at all (HTML only), Hugging Face's papers feed returns 401,
    // VentureBeat's AI feed has not updated since May, and arXiv's RSS returned
    // an empty channel on a Sunday — it is included because the emptiness looks
    // like the weekday publishing cycle rather than a dead feed, and the health
    // check will flag it if that reading is wrong.
    //
    // X/Twitter is deliberately absent. Its official API starts at $200/month,
    // and the free route is third-party scrapers (nitter, xcancel) that work
    // today, break often, and operate against X's terms. A newsroom whose whole
    // posture is honouring robots.txt and crediting publishers should not fund
    // itself on a scraping proxy.

    // -- the labs, first-party ------------------------------------------------
    // Highest trust on this desk: when OpenAI announces a model, OpenAI is the
    // primary source and everyone else is reporting on this.
    SeedSource {
        slug: "openai",
        name: "OpenAI",
        kind: SourceKind::Rss,
        url: "https://openai.com/news/rss.xml",
        homepage: "https://openai.com/news",
        trust: 88,
        poll_interval_s: 600,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "deepmind",
        name: "Google DeepMind",
        kind: SourceKind::Rss,
        url: "https://deepmind.google/blog/rss.xml",
        homepage: "https://deepmind.google/discover/blog",
        trust: 88,
        poll_interval_s: 900,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "huggingface",
        name: "Hugging Face",
        kind: SourceKind::Rss,
        url: "https://huggingface.co/blog/feed.xml",
        homepage: "https://huggingface.co/blog",
        trust: 80,
        poll_interval_s: 900,
        beat: Some(Beat::Ai),
    },
    // -- the trade press ------------------------------------------------------
    // General technology outlets: they cover phones and antitrust as well as
    // models, so their items are routed per item by `relevance::classify`
    // rather than pinned to a beat.
    SeedSource {
        slug: "techcrunch-ai",
        name: "TechCrunch",
        kind: SourceKind::Finance,
        url: "https://techcrunch.com/category/artificial-intelligence/feed/",
        homepage: "https://techcrunch.com/category/artificial-intelligence/",
        trust: 74,
        poll_interval_s: 300,
        beat: None,
    },
    SeedSource {
        slug: "arstechnica",
        name: "Ars Technica",
        kind: SourceKind::Finance,
        url: "https://feeds.arstechnica.com/arstechnica/technology-lab",
        homepage: "https://arstechnica.com",
        trust: 84,
        poll_interval_s: 600,
        beat: None,
    },
    SeedSource {
        slug: "techreview",
        name: "MIT Technology Review",
        kind: SourceKind::Finance,
        url: "https://www.technologyreview.com/feed/",
        homepage: "https://www.technologyreview.com",
        trust: 86,
        poll_interval_s: 900,
        beat: None,
    },
    SeedSource {
        slug: "theverge-ai",
        name: "The Verge",
        kind: SourceKind::Finance,
        url: "https://www.theverge.com/rss/ai-artificial-intelligence/index.xml",
        homepage: "https://www.theverge.com/ai-artificial-intelligence",
        trust: 76,
        poll_interval_s: 600,
        beat: None,
    },
    // -- practitioners --------------------------------------------------------
    // Independent voices who are read by the people building this. Lower trust
    // than an institutional newsroom — one person, no corrections desk — but
    // frequently ahead of it.
    SeedSource {
        slug: "simonwillison",
        name: "Simon Willison",
        kind: SourceKind::Rss,
        url: "https://simonwillison.net/atom/everything/",
        homepage: "https://simonwillison.net",
        trust: 72,
        poll_interval_s: 900,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "importai",
        name: "Import AI",
        kind: SourceKind::Rss,
        url: "https://importai.substack.com/feed",
        homepage: "https://importai.substack.com",
        trust: 78,
        poll_interval_s: 3600,
        beat: Some(Beat::Ai),
    },
    // -- research -------------------------------------------------------------
    // A preprint is not a news item and is not treated as one: no peer review,
    // no editor, and authors who are also the interested party. It earns a
    // place because the frontier genuinely moves here first, and it is marked
    // `Research` so the renderer can say what it is.
    SeedSource {
        slug: "arxiv-ai",
        name: "arXiv cs.AI",
        kind: SourceKind::Research,
        url: "https://rss.arxiv.org/rss/cs.AI",
        homepage: "https://arxiv.org/list/cs.AI/recent",
        trust: 60,
        poll_interval_s: 3600,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "arxiv-lg",
        name: "arXiv cs.LG",
        kind: SourceKind::Research,
        url: "https://rss.arxiv.org/rss/cs.LG",
        homepage: "https://arxiv.org/list/cs.LG/recent",
        trust: 60,
        poll_interval_s: 3600,
        beat: Some(Beat::Ai),
    },
    // -- forums ---------------------------------------------------------------
    // Discussion, not reporting. The signal is that practitioners are arguing
    // about something — which is often the earliest signal there is — but a
    // thread is never corroboration for a claim, hence the low trust.
    SeedSource {
        slug: "reddit-ml",
        name: "r/MachineLearning",
        kind: SourceKind::Forum,
        url: "https://www.reddit.com/r/MachineLearning/.rss",
        homepage: "https://www.reddit.com/r/MachineLearning/",
        trust: 45,
        poll_interval_s: 1800,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "reddit-localllama",
        name: "r/LocalLLaMA",
        kind: SourceKind::Forum,
        url: "https://www.reddit.com/r/LocalLLaMA/.rss",
        homepage: "https://www.reddit.com/r/LocalLLaMA/",
        trust: 45,
        poll_interval_s: 1800,
        beat: Some(Beat::Ai),
    },
];

pub async fn seed_sources(db: &Db) -> Result<usize> {
    for s in SOURCES {
        sources::upsert(
            db,
            s.slug,
            s.name,
            s.kind,
            s.url,
            s.homepage,
            s.trust,
            s.poll_interval_s,
            s.beat,
        )
        .await?;
    }
    Ok(SOURCES.len())
}

/// Seed the tracked assets so the ticker strip has rows before the first price
/// poll completes.
pub async fn seed_assets(db: &Db) -> Result<usize> {
    for (i, (sym, name, gecko)) in crate::market::TRACKED.iter().enumerate() {
        bg_db::prices::upsert_asset(db, sym, name, Some(gecko), Some(i as i32 + 1)).await?;
    }
    Ok(crate::market::TRACKED.len())
}

/// Seed the entity graph with the names that recur in almost every story, so
/// hub pages are populated on day one instead of waiting for extraction.
pub async fn seed_entities(db: &Db) -> Result<usize> {
    use bg_core::domain::EntityKind::*;
    /// kind, display name, slug, ticker, aliases.
    type SeedEntity<'a> = (
        bg_core::domain::EntityKind,
        &'a str,
        &'a str,
        Option<&'a str>,
        &'a [&'a str],
    );
    let rows: &[SeedEntity] = &[
        (Token, "Bitcoin", "bitcoin", Some("BTC"), &["XBT"]),
        (Token, "Ethereum", "ethereum", Some("ETH"), &["Ether"]),
        (Chain, "Solana", "solana", Some("SOL"), &[]),
        (
            Regulator,
            "Securities and Exchange Commission",
            "sec",
            None,
            &["SEC", "the SEC"],
        ),
        (
            Regulator,
            "Commodity Futures Trading Commission",
            "cftc",
            None,
            &["CFTC"],
        ),
        (Exchange, "Coinbase", "coinbase", None, &["Coinbase Global"]),
        (Exchange, "Binance", "binance", None, &[]),
        (Exchange, "Kraken", "kraken", None, &["Payward"]),
        (
            Company,
            "Tether",
            "tether",
            Some("USDT"),
            &["Tether Limited"],
        ),
        (
            Company,
            "Circle",
            "circle",
            Some("USDC"),
            &["Circle Internet Financial"],
        ),
        (
            Company,
            "MicroStrategy",
            "microstrategy",
            None,
            &["Strategy"],
        ),
        (Fund, "BlackRock", "blackrock", None, &["IBIT"]),
        (Protocol, "Uniswap", "uniswap", Some("UNI"), &[]),
        (Protocol, "Lido", "lido", Some("LDO"), &[]),
        (Protocol, "Aave", "aave", Some("AAVE"), &[]),
    ];
    for (kind, name, slug, ticker, aliases) in rows {
        let aliases: Vec<String> = aliases.iter().map(|s| s.to_string()).collect();
        bg_db::entities::upsert(db, *kind, name, slug, *ticker, &aliases).await?;
    }
    Ok(rows.len())
}
