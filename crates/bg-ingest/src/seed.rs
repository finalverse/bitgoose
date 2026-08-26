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
    // ---- Independent Chinese edition ------------------------------------
    SeedSource {
        slug: "gnews-zh-ai",
        name: "Google 新闻 · 人工智能",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/search?q=%E4%BA%BA%E5%B7%A5%E6%99%BA%E8%83%BD+OR+OpenAI+OR+Anthropic+when:1d&hl=zh-CN&gl=CN&ceid=CN:zh-Hans",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 240,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "gnews-zh-crypto",
        name: "Google 新闻 · 数字资产",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/search?q=%E6%AF%94%E7%89%B9%E5%B8%81+OR+%E4%BB%A5%E5%A4%AA%E5%9D%8A+OR+%E7%A8%B3%E5%AE%9A%E5%B8%81+when:1d&hl=zh-CN&gl=CN&ceid=CN:zh-Hans",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 240,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "gnews-zh-business",
        name: "Google 新闻 · 财经",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/BUSINESS?hl=zh-CN&gl=CN&ceid=CN:zh-Hans",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 300,
        beat: Some(Beat::Markets),
    },
    SeedSource {
        slug: "gnews-zh-tech",
        name: "Google 新闻 · 科技",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/TECHNOLOGY?hl=zh-CN&gl=CN&ceid=CN:zh-Hans",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 300,
        beat: Some(Beat::Tech),
    },
    SeedSource {
        slug: "voa-zh-news",
        name: "美国之音中文网",
        kind: SourceKind::Rss,
        url: "https://www.voachinese.com/api/zm_yql-vomx-tpeybti",
        homepage: "https://www.voachinese.com",
        trust: 82,
        poll_interval_s: 300,
        beat: Some(Beat::World),
    },
    SeedSource {
        slug: "rthk-zh-world",
        name: "香港电台 · 国际",
        kind: SourceKind::Rss,
        url: "https://rthk.hk/rthk/news/rss/c_expressnews_cinternational.xml",
        homepage: "https://news.rthk.hk",
        trust: 82,
        poll_interval_s: 300,
        beat: Some(Beat::World),
    },

    // ---- Independent French edition -------------------------------------
    SeedSource {
        slug: "gnews-fr-ai",
        name: "Google Actualités · IA",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/search?q=intelligence+artificielle+OR+OpenAI+OR+Anthropic+when:1d&hl=fr&gl=FR&ceid=FR:fr",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 240,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "gnews-fr-crypto",
        name: "Google Actualités · Crypto",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/search?q=bitcoin+OR+ethereum+OR+cryptomonnaie+when:1d&hl=fr&gl=FR&ceid=FR:fr",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 240,
        beat: Some(Beat::Crypto),
    },
    SeedSource {
        slug: "france24-fr",
        name: "France 24",
        kind: SourceKind::Rss,
        url: "https://www.france24.com/fr/rss",
        homepage: "https://www.france24.com/fr/",
        trust: 84,
        poll_interval_s: 300,
        beat: Some(Beat::World),
    },
    SeedSource {
        slug: "numerama-fr",
        name: "Numerama",
        kind: SourceKind::Rss,
        url: "https://www.numerama.com/feed/",
        homepage: "https://www.numerama.com/",
        trust: 76,
        poll_interval_s: 360,
        beat: Some(Beat::Tech),
    },
    SeedSource {
        slug: "journal-du-coin-fr",
        name: "Journal du Coin",
        kind: SourceKind::Rss,
        url: "https://journalducoin.com/feed/",
        homepage: "https://journalducoin.com/",
        trust: 66,
        poll_interval_s: 300,
        beat: Some(Beat::Crypto),
    },

    // ---- Aggregators: what is actually hot right now ---------------------
    //
    // Every source above is one publisher's judgement of what matters. None of
    // them answers the question a newsroom asks first — *what is the story
    // today* — and the gap showed: Bitcoin rose 7.9% in a day while the front
    // page led on "Logit-Guided Neural Routing for Billion-Scale Vector
    // Search", because an arXiv firehose out-published the wires and nothing
    // in the pipeline knew the difference between volume and heat.
    //
    // Google News ranks by how many independent outlets are covering a story
    // and how fast that number is moving, which is the same signal the Curator
    // computes downstream and is worth having at intake as well. A topic feed
    // that suddenly repeats across ten publishers is the Flyway's input.
    //
    // These are RSS endpoints published for exactly this use. Items are
    // headline, link and timestamp — the link-out goes to the original
    // publisher, who keeps the traffic, and the ≤25-word quote and attribution
    // rules in `bg-core::policy` apply to these as to everything else.
    //
    // `beat: None` on the search feeds: a query for "bitcoin ETF" can surface a
    // markets story, a policy story or a technology story, and pinning the
    // whole feed to one desk is how Al Jazeera's `all.xml` once put a football
    // result at the top of World.
    SeedSource {
        slug: "gnews-top",
        name: "Google News",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 300,
        beat: None,
    },
    SeedSource {
        slug: "gnews-world",
        name: "Google News · World",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/WORLD?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 300,
        beat: Some(Beat::World),
    },
    SeedSource {
        slug: "gnews-business",
        name: "Google News · Business",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/BUSINESS?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 300,
        beat: Some(Beat::Markets),
    },
    SeedSource {
        slug: "gnews-tech",
        name: "Google News · Technology",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/TECHNOLOGY?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 300,
        beat: Some(Beat::Tech),
    },
    SeedSource {
        slug: "gnews-science",
        name: "Google News · Science",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/SCIENCE?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 600,
        beat: Some(Beat::Science),
    },
    SeedSource {
        slug: "gnews-health",
        name: "Google News · Health",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/HEALTH?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 600,
        beat: Some(Beat::Science),
    },
    SeedSource {
        slug: "gnews-entertainment",
        name: "Google News · Entertainment",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/headlines/section/topic/ENTERTAINMENT?hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 68,
        poll_interval_s: 600,
        beat: Some(Beat::Culture),
    },
    // The two beats the other CDAX properties consume, so they are polled at
    // wire speed and queried rather than taken from a section editor's page.
    SeedSource {
        slug: "gnews-crypto",
        name: "Google News · Crypto",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/search?q=crypto+OR+bitcoin+OR+ethereum+OR+stablecoin+when:1d&hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 180,
        beat: None,
    },
    SeedSource {
        slug: "gnews-ai",
        name: "Google News · AI",
        kind: SourceKind::Rss,
        url: "https://news.google.com/rss/search?q=%22artificial+intelligence%22+OR+OpenAI+OR+Anthropic+OR+%22machine+learning%22+when:1d&hl=en-US&gl=US&ceid=US:en",
        homepage: "https://news.google.com",
        trust: 70,
        poll_interval_s: 180,
        beat: None,
    },
    SeedSource {
        slug: "yahoo-news",
        name: "Yahoo News",
        kind: SourceKind::Rss,
        url: "https://news.yahoo.com/rss/",
        homepage: "https://news.yahoo.com",
        trust: 66,
        poll_interval_s: 300,
        beat: None,
    },
    // ---- World, Science and Culture -------------------------------------
    //
    // Three desks that existed in the navigation with nothing behind them, and
    // two that were starving: Markets had one source and Tech had none, which
    // is why both had published nothing for over a week while AI and Crypto ran
    // hourly. A desk in the nav that never updates is worse than no desk.
    //
    // Chosen for breadth of subject rather than volume, and every one checked
    // for reachability and AI posture before being added. Several publish their
    // feed on a host that carries none of their robots rules — `feeds.bbci.co.uk`
    // says nothing while `www.bbc.co.uk` blocks five AI crawlers — so the
    // posture is read from `homepage`, not from `url`.
    SeedSource {
        slug: "npr-world",
        name: "NPR",
        kind: SourceKind::Rss,
        url: "https://feeds.npr.org/1004/rss.xml",
        homepage: "https://www.npr.org",
        trust: 82,
        poll_interval_s: 900,
        beat: Some(Beat::World),
    },
    SeedSource {
        slug: "bbc-world",
        name: "BBC News",
        kind: SourceKind::Rss,
        url: "https://feeds.bbci.co.uk/news/world/rss.xml",
        homepage: "https://www.bbc.co.uk",
        trust: 85,
        poll_interval_s: 900,
        beat: Some(Beat::World),
    },
    SeedSource {
        slug: "aljazeera",
        name: "Al Jazeera",
        kind: SourceKind::Rss,
        url: "https://www.aljazeera.com/xml/rss/all.xml",
        homepage: "https://www.aljazeera.com",
        trust: 76,
        poll_interval_s: 900,
        // `all.xml` is everything they publish — world, sport, business,
        // culture — so pinning it to one desk files a football result under
        // World, which is what it did on the first render. Routed per item,
        // for the same reason MarketWatch is: a general feed that claims a
        // desk bypasses the classifier entirely.
        beat: None,
    },
    SeedSource {
        slug: "sciencedaily",
        name: "ScienceDaily",
        kind: SourceKind::Rss,
        url: "https://www.sciencedaily.com/rss/all.xml",
        homepage: "https://www.sciencedaily.com",
        trust: 70,
        // Sixty items a poll; a quarter-hour cadence would re-read the same
        // wall of research releases all day for nothing.
        poll_interval_s: 3600,
        beat: Some(Beat::Science),
    },
    SeedSource {
        slug: "physorg",
        name: "Phys.org",
        kind: SourceKind::Rss,
        url: "https://phys.org/rss-feed/",
        homepage: "https://phys.org",
        trust: 72,
        poll_interval_s: 1800,
        beat: Some(Beat::Science),
    },
    SeedSource {
        slug: "nasa",
        name: "NASA",
        kind: SourceKind::Rss,
        // A primary source: the agency announcing its own missions, rather
        // than a report of the announcement.
        url: "https://www.nasa.gov/news-release/feed/",
        homepage: "https://www.nasa.gov",
        trust: 90,
        poll_interval_s: 3600,
        beat: Some(Beat::Science),
    },
    SeedSource {
        slug: "npr-health",
        name: "NPR Health",
        kind: SourceKind::Rss,
        url: "https://feeds.npr.org/1128/rss.xml",
        homepage: "https://www.npr.org",
        trust: 80,
        poll_interval_s: 1800,
        beat: Some(Beat::Science),
    },
    SeedSource {
        slug: "bbc-culture",
        name: "BBC Arts",
        kind: SourceKind::Rss,
        url: "https://feeds.bbci.co.uk/news/entertainment_and_arts/rss.xml",
        homepage: "https://www.bbc.co.uk",
        trust: 78,
        poll_interval_s: 1800,
        beat: Some(Beat::Culture),
    },
    SeedSource {
        slug: "bbc-sport",
        name: "BBC Sport",
        kind: SourceKind::Rss,
        url: "https://feeds.bbci.co.uk/sport/rss.xml",
        homepage: "https://www.bbc.co.uk",
        trust: 80,
        poll_interval_s: 900,
        beat: Some(Beat::Culture),
    },
    SeedSource {
        slug: "npr-culture",
        name: "NPR Culture",
        kind: SourceKind::Rss,
        url: "https://feeds.npr.org/1008/rss.xml",
        homepage: "https://www.npr.org",
        trust: 78,
        poll_interval_s: 1800,
        beat: Some(Beat::Culture),
    },
    // ---- Feeding the two desks that had starved ---------------------------
    //
    // Ars Technica and MarketWatch are deliberately *not* here: both were
    // already on the roster with `beat: None`, routed per item on purpose, and
    // adding them again with a pinned desk silently overrode that — the later
    // entry wins the upsert. See the guard in the tests below.
    SeedSource {
        slug: "engadget",
        name: "Engadget",
        kind: SourceKind::Rss,
        url: "https://www.engadget.com/rss.xml",
        homepage: "https://www.engadget.com",
        trust: 70,
        poll_interval_s: 1200,
        beat: Some(Beat::Tech),
    },
    SeedSource {
        slug: "npr-business",
        name: "NPR Business",
        kind: SourceKind::Rss,
        url: "https://feeds.npr.org/1006/rss.xml",
        homepage: "https://www.npr.org",
        trust: 80,
        poll_interval_s: 1800,
        beat: Some(Beat::Markets),
    },
    // Two outlets that welcome crawlers and refuse the model, which is now a
    // distinction BitGoose can honour rather than one it has to choose between.
    //
    // theaiinsider.tech publishes `Content-Signal: search=yes,ai-train=no,
    // use=reference` and blocks GPTBot, ClaudeBot, CCBot and Google-Extended by
    // name while allowing `*`. Business Insider blocks the same crawlers. Both
    // are read as headline-and-link-out sources: polled, ranked, cited, never
    // extracted and never put in a prompt. `refresh_robots` sets the flag from
    // what they actually publish, so it follows them if they change their mind.
    SeedSource {
        slug: "aiinsider",
        name: "AI Insider",
        kind: SourceKind::Rss,
        url: "https://theaiinsider.tech/feed/",
        homepage: "https://theaiinsider.tech",
        trust: 65,
        // Their robots.txt asks for `Crawl-delay: 10`; a half-hour poll is far
        // inside that and matches how often the feed actually moves.
        poll_interval_s: 1800,
        beat: Some(Beat::Ai),
    },
    SeedSource {
        slug: "businessinsider",
        name: "Business Insider",
        kind: SourceKind::Rss,
        // The markets desk feed. `businessinsider.com/rss` answers 200 with no
        // items; this one carries them.
        url: "https://markets.businessinsider.com/rss/news",
        homepage: "https://www.businessinsider.com",
        trust: 70,
        poll_interval_s: 1200,
        beat: Some(Beat::Markets),
    },
    // The first source with no feed at all — /rss and /feed both 404, and
    // robots.txt permits the index. It is read by crawling, which is the whole
    // reason `bg-ingest::crawl` exists, and it closes an obvious gap in an AI
    // roster that already carries OpenAI and DeepMind.
    SeedSource {
        slug: "anthropic",
        name: "Anthropic",
        kind: SourceKind::Html,
        url: "https://www.anthropic.com/news",
        homepage: "https://www.anthropic.com",
        trust: 80,
        // An index page changes far less often than a wire feed, and each poll
        // is a full HTML fetch rather than a small XML one.
        poll_interval_s: 1800,
        beat: Some(Beat::Ai),
    },
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

#[cfg(test)]
mod seed_tests {
    use super::*;
    use std::collections::HashSet;

    /// Two slugs, one source. The later entry wins the upsert, so a duplicate
    /// silently overrides the earlier one's settings and nothing complains.
    ///
    /// Found the hard way: Ars Technica and MarketWatch were already on the
    /// roster routed per item, and adding them again with a pinned desk quietly
    /// reverted to `beat: None` on every seed.
    #[test]
    fn no_two_sources_share_a_slug() {
        let mut seen = HashSet::new();
        for s in SOURCES {
            assert!(
                seen.insert(s.slug),
                "duplicate slug in the roster: {}",
                s.slug
            );
        }
    }

    /// Two entries polling the same feed is the same waste with a different
    /// shape: two rows, two polls, and the items deduplicate downstream.
    #[test]
    fn no_two_sources_poll_the_same_url() {
        let mut seen = HashSet::new();
        for s in SOURCES {
            assert!(seen.insert(s.url), "two sources poll {}", s.url);
        }
    }

    /// Every desk in the navigation must have something behind it.
    ///
    /// The failure this whole change exists to fix: Markets had one source and
    /// Tech had none, and both sat in the nav publishing nothing for over a
    /// week. A desk with no pinned source can still be filled by the per-item
    /// router, so the bar is deliberately low — but it is not zero.
    #[test]
    fn every_desk_has_at_least_one_source() {
        for beat in [
            Beat::Ai,
            Beat::Crypto,
            Beat::Markets,
            Beat::Tech,
            Beat::World,
            Beat::Science,
            Beat::Culture,
        ] {
            let n = SOURCES.iter().filter(|s| s.beat == Some(beat)).count();
            assert!(n > 0, "{beat:?} has no dedicated source");
        }
    }
}
