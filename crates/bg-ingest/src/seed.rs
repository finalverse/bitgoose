//! The source roster.
//!
//! Trust scores weight corroboration in `bg-agents::sentinel`. They are a
//! judgement about *editorial process* — does the outlet employ named reporters,
//! does it correct itself, does it break stories or reprint them — not about
//! whether we like its coverage. They are visible on `/standards` precisely
//! because they are contestable.
//!
//! Every URL here was verified reachable before it was added.

use bg_core::domain::SourceKind;
use bg_db::{sources, Db, Result};

pub struct SeedSource {
    pub slug: &'static str,
    pub name: &'static str,
    pub kind: SourceKind,
    pub url: &'static str,
    pub homepage: &'static str,
    pub trust: i16,
    pub poll_interval_s: i32,
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
    },
    SeedSource {
        slug: "theblock",
        name: "The Block",
        kind: SourceKind::Rss,
        url: "https://www.theblock.co/rss.xml",
        homepage: "https://www.theblock.co",
        trust: 84,
        poll_interval_s: 180,
    },
    SeedSource {
        slug: "decrypt",
        name: "Decrypt",
        kind: SourceKind::Rss,
        url: "https://decrypt.co/feed",
        homepage: "https://decrypt.co",
        trust: 78,
        poll_interval_s: 180,
    },
    SeedSource {
        slug: "dlnews",
        name: "DL News",
        kind: SourceKind::Rss,
        url: "https://www.dlnews.com/arc/outboundfeeds/rss/",
        homepage: "https://www.dlnews.com",
        trust: 76,
        poll_interval_s: 300,
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
    },
    SeedSource {
        slug: "thedefiant",
        name: "The Defiant",
        kind: SourceKind::Rss,
        url: "https://thedefiant.io/api/feed",
        homepage: "https://thedefiant.io",
        trust: 72,
        poll_interval_s: 300,
    },
    SeedSource {
        slug: "bitcoinmagazine",
        name: "Bitcoin Magazine",
        kind: SourceKind::Rss,
        url: "https://bitcoinmagazine.com/feed",
        homepage: "https://bitcoinmagazine.com",
        trust: 70,
        poll_interval_s: 600,
    },
    SeedSource {
        slug: "cointelegraph",
        name: "Cointelegraph",
        kind: SourceKind::Rss,
        url: "https://cointelegraph.com/rss",
        homepage: "https://cointelegraph.com",
        trust: 64,
        poll_interval_s: 300,
    },
    SeedSource {
        slug: "cryptoslate",
        name: "CryptoSlate",
        kind: SourceKind::Rss,
        url: "https://cryptoslate.com/feed/",
        homepage: "https://cryptoslate.com",
        trust: 58,
        poll_interval_s: 600,
    },
];

pub async fn seed_sources(db: &Db) -> Result<usize> {
    for s in SOURCES {
        sources::upsert(db, s.slug, s.name, s.kind, s.url, s.homepage, s.trust, s.poll_interval_s)
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
    let rows: &[(bg_core::domain::EntityKind, &str, &str, Option<&str>, &[&str])] = &[
        (Token, "Bitcoin", "bitcoin", Some("BTC"), &["XBT"]),
        (Token, "Ethereum", "ethereum", Some("ETH"), &["Ether"]),
        (Chain, "Solana", "solana", Some("SOL"), &[]),
        (Regulator, "Securities and Exchange Commission", "sec", None, &["SEC", "the SEC"]),
        (Regulator, "Commodity Futures Trading Commission", "cftc", None, &["CFTC"]),
        (Exchange, "Coinbase", "coinbase", None, &["Coinbase Global"]),
        (Exchange, "Binance", "binance", None, &[]),
        (Exchange, "Kraken", "kraken", None, &["Payward"]),
        (Company, "Tether", "tether", Some("USDT"), &["Tether Limited"]),
        (Company, "Circle", "circle", Some("USDC"), &["Circle Internet Financial"]),
        (Company, "MicroStrategy", "microstrategy", None, &["Strategy"]),
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
