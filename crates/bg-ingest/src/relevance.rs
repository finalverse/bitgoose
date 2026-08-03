//! Which desk does an item belong to — and does it belong here at all?
//!
//! The nine crypto desks can be taken wholesale — everything they publish is on
//! topic. Yahoo Finance, Bloomberg, CNBC, MarketWatch and the FT cannot: their
//! feeds are overwhelmingly equities, rates and earnings, and ingesting them
//! raw would bury the crypto coverage we actually want under general business
//! news.
//!
//! The same is true, more so, of the AI desk: TechCrunch, Ars Technica and The
//! Verge cover phones and antitrust alongside model releases, and Hacker News
//! is mostly not about AI at all.
//!
//! So items from a general-interest source pass through this gate first. It is deliberately deterministic — a keyword
//! match, not a model call. Mainstream outlets publish hundreds of items a day
//! and a model call per item would be the single largest cost in the pipeline,
//! to answer a question that a word list answers well.
//!
//! Tuned to be **specific rather than sensitive**: missing a story costs us one
//! item that the crypto desks almost certainly also covered, while a false
//! positive puts an article about semiconductor margins on a crypto front page.
//! That asymmetry is why the ambiguous words are absent — "token", "mining",
//! "wallet", "ledger" and bare "ETF" all appear constantly in ordinary
//! financial copy.

/// Terms that make an item crypto news on their own.
///
/// See [`AI_TERMS`] for the sibling list; both follow the same discipline.
///
/// Matched case-insensitively on whole words, so `eth` does not fire inside
/// "ethics" and `xrp` does not fire inside a longer string.
const TERMS: &[&str] = &[
    // assets and the space itself
    "crypto",
    "cryptocurrency",
    "cryptocurrencies",
    "bitcoin",
    "btc",
    "ethereum",
    "ether",
    "eth",
    "solana",
    "sol",
    "xrp",
    "ripple",
    "dogecoin",
    "doge",
    "cardano",
    "ada",
    "litecoin",
    "altcoin",
    "altcoins",
    "memecoin",
    "memecoins",
    "stablecoin",
    "stablecoins",
    "tether",
    "usdt",
    "usdc",
    "blockchain",
    "defi",
    "web3",
    "nft",
    "nfts",
    "onchain",
    "on-chain",
    "digital asset",
    "digital assets",
    "spot bitcoin etf",
    "spot ether etf",
    "crypto etf",
    // the companies whose news is essentially always crypto news
    "coinbase",
    "binance",
    "kraken",
    "circle internet",
    "microstrategy",
    "bitfinex",
    "bitmex",
    "grayscale",
    "chainalysis",
    "consensys",
    "bitgo",
    "gemini trust",
    "ftx",
    "celsius network",
    "riot platforms",
    "marathon digital",
    "hut 8",
    "core scientific",
];

/// Whether a mainstream-finance item is about crypto.
///
/// `haystack` should be the title plus whatever summary the feed carried;
/// checking the title alone misses stories that name the asset only in the
/// standfirst.
pub fn is_crypto(haystack: &str) -> bool {
    let hay = normalize(haystack);
    TERMS.iter().any(|t| contains_word(&hay, t))
}

/// Lowercase, and collapse anything that is not a letter or digit to a single
/// space. This is what makes whole-word matching work regardless of the
/// punctuation around a term — "(BTC)", "bitcoin's", "ETH/USD" all normalize to
/// something with the term standing alone.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push(' ');
    let mut space = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    if !space {
        out.push(' ');
    }
    out
}

/// Whole-word (or whole-phrase) containment within an already-normalized
/// haystack that is padded with spaces at both ends.
fn contains_word(hay: &str, term: &str) -> bool {
    let mut padded = String::with_capacity(term.len() + 2);
    padded.push(' ');
    padded.push_str(term);
    padded.push(' ');
    hay.contains(&padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_mainstream_crypto_coverage() {
        for s in [
            "Bitcoin tops $90,000 as ETF inflows accelerate",
            "Coinbase shares slip after Q2 revenue miss",
            "SEC approves spot Ether ETF applications",
            "Stablecoin issuer Circle Internet Group files for IPO",
            "MicroStrategy adds to its BTC holdings",
            "What the crypto selloff means for equities",
            "Ripple's XRP jumps on court ruling",
            "Tether's USDT reserves under scrutiny",
            // asset named only in the summary, not the headline
            "Treasury weighs new rules — the plan covers digital assets held by banks",
        ] {
            assert!(is_crypto(s), "should be crypto: {s}");
        }
    }

    #[test]
    fn ignores_ordinary_business_news() {
        for s in [
            "Nvidia beats on earnings as data-centre demand holds",
            "Fed holds rates steady, signals one cut this year",
            "Oil slips as OPEC weighs output increase",
            "Apple unveils new iPhone lineup",
            "Retail sales rose 0.4% in July",
            "Boeing wins order for 40 jets",
        ] {
            assert!(!is_crypto(s), "should NOT be crypto: {s}");
        }
    }

    #[test]
    fn short_tickers_do_not_fire_inside_other_words() {
        // These are exactly the false positives a naive substring match causes,
        // and each of them would put a non-crypto story on a crypto front page.
        for s in [
            "Ethics probe widens at the agency",    // eth
            "Adaptive pricing lifts margins",       // ada
            "Solar stocks rally on subsidy news",   // sol
            "The company's DOGE-adjacent branding", // this one SHOULD match
            "Btcx Holdings renames itself",         // btc inside a word
            "A sole trader files suit",             // sol
        ] {
            let expected = s.contains("DOGE");
            assert_eq!(is_crypto(s), expected, "misjudged: {s}");
        }
    }

    #[test]
    fn possessives_and_punctuation_still_match() {
        assert!(is_crypto("Bitcoin's rally stalls"));
        assert!(is_crypto("(BTC) leads gains"));
        assert!(is_crypto("ETH/USD breaks resistance"));
        assert!(is_crypto("Crypto-native firms expand"));
    }

    #[test]
    fn empty_input_is_not_crypto() {
        assert!(!is_crypto(""));
        assert!(!is_crypto("   "));
    }
}

/// Terms that make an item AI news on their own.
///
/// Same discipline as [`TERMS`]: whole-word, and specific rather than
/// sensitive. The omissions are deliberate and each one has a reason.
///
/// "ai" itself is absent — as a bare token it is a substring of nothing useful
/// but a *word* in "Ai Weiwei" and a hundred product names, and it appears in
/// headlines about AI-adjacent nothing ("this startup uses AI to sell socks").
/// The two-letter form earns its place only in compounds like "ai model", which
/// are listed. "model", "training", "agent", "inference", "transformer" and
/// "neural" are all absent for the same reason they would be in the crypto
/// list: they are ordinary English in a business context.
///
/// Lab and model names carry most of the weight, because in practice that is
/// what AI news is *about*.
const AI_TERMS: &[&str] = &[
    // the field
    "artificial intelligence",
    "machine learning",
    "deep learning",
    "generative ai",
    "ai model",
    "ai models",
    "ai safety",
    "ai agent",
    "ai agents",
    "ai research",
    "ai chip",
    "ai chips",
    "ai startup",
    "ai lab",
    "frontier model",
    "frontier models",
    "foundation model",
    "foundation models",
    "large language model",
    "large language models",
    "llm",
    "llms",
    "chatbot",
    "chatbots",
    "diffusion model",
    "neural network",
    "neural networks",
    "reinforcement learning",
    "fine-tuning",
    "open weights",
    "open-weight",
    "agi",
    "superintelligence",
    "alignment research",
    "rlhf",
    "benchmark suite",
    // policy — a major beat, and none of the terms above cover it. Added
    // because a test asserting "The EU AI Act's first compliance deadline"
    // should reach the AI desk found that it did not.
    "ai act",
    "ai bill",
    "ai regulation",
    "ai rules",
    "ai policy",
    "ai executive order",
    "ai safety institute",
    "ai moratorium",
    "compute threshold",
    "model evaluations",
    // labs and the people who run them
    "openai",
    "anthropic",
    "deepmind",
    "mistral",
    "cohere",
    "hugging face",
    "huggingface",
    "stability ai",
    "scale ai",
    "perplexity ai",
    "inflection ai",
    "xai",
    "safe superintelligence",
    // models
    "chatgpt",
    "gpt-4",
    "gpt-5",
    "claude",
    "gemini",
    "llama",
    "mistral large",
    "deepseek",
    "qwen",
    "grok",
    "sora",
    "midjourney",
    "stable diffusion",
    "whisper",
    // the compute layer, when named specifically
    "nvidia",
    "cuda",
    "tpu",
    "tpus",
    "h100",
    "h200",
    "gb200",
    "blackwell",
    "tensor core",
];

/// Whether an item is AI news.
pub fn is_ai(haystack: &str) -> bool {
    let hay = normalize(haystack);
    AI_TERMS.iter().any(|t| contains_word(&hay, t))
}

/// Terms that mark an item as high technology without being AI or crypto.
///
/// The chip and platform companies, space, biotech, energy transition — the
/// frontier that is neither a model nor a token. Nvidia is deliberately absent:
/// it sits in [`AI_TERMS`] because in practice its news *is* AI news, and
/// listing it twice would make the ordering below decide arbitrarily.
const TECH_TERMS: &[&str] = &[
    "semiconductor",
    "semiconductors",
    "chipmaker",
    "chipmakers",
    "foundry",
    "tsmc",
    "asml",
    "arm holdings",
    "qualcomm",
    "broadcom",
    "intel",
    "amd",
    "micron",
    "samsung electronics",
    "sk hynix",
    "quantum computing",
    "quantum computer",
    "spacex",
    "starship",
    "rocket lab",
    "blue origin",
    "satellite internet",
    "starlink",
    "robotaxi",
    "autonomous vehicle",
    "autonomous vehicles",
    "waymo",
    "cruise",
    "biotech",
    "crispr",
    "gene therapy",
    "mrna",
    "fusion energy",
    "small modular reactor",
    "grid storage",
    "solid-state battery",
    "data centre",
    "data center",
    "hyperscaler",
    "hyperscalers",
    "cybersecurity",
    "zero-day",
    "ransomware",
    // Mega-cap *company* names are deliberately absent. "Apple", "Amazon" and
    // the rest saturate market copy — "Amazon stock", "Apple earnings" — and
    // because tech is checked before markets, listing them sent every megacap
    // earnings story to the wrong desk. Same trap as bare "ai": too common to
    // carry a signal. Their products and platforms are specific, so those are
    // listed instead.
    "iphone",
    "macbook",
    "app store",
    "android",
    "windows 11",
    "azure",
    "google cloud",
    "vision pro",
    "antitrust",
    "chip export",
    "export controls",
];

/// Terms that mark an item as capital-markets news.
///
/// The vocabulary of the market itself rather than of any one company, so that
/// "Nvidia earnings" lands on the markets desk only when nothing more specific
/// claims it.
const MARKETS_TERMS: &[&str] = &[
    "s&p 500",
    "nasdaq",
    "dow jones",
    "russell 2000",
    "ftse",
    "nikkei",
    "stoxx",
    "hang seng",
    // Central banks. Bare "fed" is deliberately absent — it is an ordinary
    // English verb ("fed up", "fed the dog") and normalize() has already thrown
    // away the capitalisation that would disambiguate it. Compounds instead,
    // added after a test asserting "Fed holds rates steady" should reach the
    // markets desk found that the most common phrasing of central-bank news
    // matched nothing at all.
    "federal reserve",
    "the fed",
    "fomc",
    "fed holds",
    "fed cuts",
    "fed raises",
    "fed signals",
    "fed chair",
    "fed officials",
    "fed minutes",
    "fed decision",
    "rate cut",
    "rate cuts",
    "rate hike",
    "rate decision",
    "rates steady",
    "interest rates",
    "monetary policy",
    "quantitative tightening",
    "european central bank",
    "bank of england",
    "bank of japan",
    "treasury yield",
    "treasury yields",
    "bond market",
    "yield curve",
    "inflation",
    "cpi",
    "ppi",
    "payrolls",
    "unemployment rate",
    "gdp growth",
    "recession",
    // The plain forms, missing until a test asserted "Amazon earnings beat as
    // AWS growth holds" should reach the markets desk and it matched nothing.
    // These are market vocabulary in ordinary use — unlike "fed", none of them
    // is a common English word in another sense.
    "earnings",
    "stocks",
    "share price",
    "shares slide",
    "shares rise",
    "market cap",
    "investors",
    "wall street",
    "earnings season",
    "quarterly earnings",
    "profit warning",
    "guidance cut",
    "ipo",
    "listing",
    "spac",
    "buyback",
    "dividend",
    "hedge fund",
    "private equity",
    "venture capital",
    "mutual fund",
    "exchange-traded fund",
    "index fund",
    "oil prices",
    "crude oil",
    "opec",
    "gold prices",
    "commodity prices",
    "currency intervention",
    "dollar index",
    "emerging markets",
];

fn matches(haystack_normalized: &str, terms: &[&str]) -> bool {
    terms.iter().any(|t| contains_word(haystack_normalized, t))
}

/// Whether an item is high-tech news.
pub fn is_tech(haystack: &str) -> bool {
    matches(&normalize(haystack), TECH_TERMS)
}

/// Whether an item is capital-markets news.
pub fn is_markets(haystack: &str) -> bool {
    matches(&normalize(haystack), MARKETS_TERMS)
}

/// Which desk an item belongs to, if any.
///
/// `None` means "not for us" and the caller drops it.
///
/// The order is a priority, not an accident. Many items trip more than one
/// list — an Nvidia earnings story is AI *and* tech *and* markets — and a story
/// can only live on one desk. Most specific wins:
///
/// 1. **AI** and **crypto**, the two subject-matter desks this newsroom was
///    built for. A reader who wants the Nvidia story wants it under AI.
/// 2. **Markets**, when the item speaks the vocabulary of the market —
///    earnings, rates, indices, IPOs. Ordered ahead of tech because that
///    vocabulary signals what a story *is about* more reliably than a product
///    mentioned in passing: an FT piece headlined "Earnings season reaches a
///    peak" was landing on the tech desk purely because its standfirst named a
///    gadget.
/// 3. **Tech**, last of the four: a named technology or platform with no
///    market framing around it. "Apple unveils a cheaper MacBook" is tech;
///    "Apple shares slide after guidance cut" is markets.
pub fn classify(haystack: &str) -> Option<bg_core::domain::Beat> {
    use bg_core::domain::Beat;
    let hay = normalize(haystack);
    if matches(&hay, AI_TERMS) {
        Some(Beat::Ai)
    } else if matches(&hay, TERMS) {
        Some(Beat::Crypto)
    } else if matches(&hay, MARKETS_TERMS) {
        Some(Beat::Markets)
    } else if matches(&hay, TECH_TERMS) {
        Some(Beat::Tech)
    } else {
        None
    }
}

#[cfg(test)]
mod beat_tests {
    use super::*;
    use bg_core::domain::Beat;

    #[test]
    fn routes_ai_coverage_to_the_ai_desk() {
        for s in [
            "OpenAI releases GPT-5 with a longer context window",
            "Anthropic publishes new alignment research",
            "Nvidia's Blackwell chips sell out through 2027",
            "Meta open-weights Llama 4",
            "DeepSeek claims frontier performance at a fraction of the cost",
            "The EU AI Act's first compliance deadline arrives",
        ] {
            assert_eq!(classify(s), Some(Beat::Ai), "should be AI: {s}");
        }
    }

    #[test]
    fn still_routes_crypto_to_the_crypto_desk() {
        for s in [
            "Bitcoin tops $90,000 as ETF inflows accelerate",
            "Coinbase shares slip after Q2 revenue miss",
        ] {
            assert_eq!(classify(s), Some(Beat::Crypto), "should be crypto: {s}");
        }
    }

    #[test]
    fn routes_high_tech_and_capital_markets() {
        for (s, want) in [
            ("TSMC raises capex on foundry demand", Beat::Tech),
            ("SpaceX Starship completes orbital test", Beat::Tech),
            ("Apple unveils new iPhone lineup", Beat::Tech),
            ("Waymo expands robotaxi service", Beat::Tech),
            ("Fed holds rates steady, signals one cut", Beat::Markets),
            ("Oil slips as OPEC weighs output increase", Beat::Markets),
            (
                "S&P 500 closes at a record on earnings season",
                Beat::Markets,
            ),
            ("Treasury yields spike after hot CPI print", Beat::Markets),
        ] {
            assert_eq!(classify(s), Some(want), "misrouted: {s}");
        }
    }

    /// The bug this caught: listing mega-cap company names under tech sent
    /// every "Amazon stock" and "Apple earnings" story to the tech desk,
    /// because tech is checked before markets. Company names are market copy;
    /// their products are the technology.
    #[test]
    fn megacap_earnings_are_markets_not_tech() {
        for s in [
            "Earnings season reaches a peak",
            "Morgan Stanley's IPO after-party: a wealth management bonanza",
            "Amazon earnings beat as AWS growth holds",
            "Apple shares slide after guidance cut",
        ] {
            assert_eq!(classify(s), Some(Beat::Markets), "should be markets: {s}");
        }
        // Naming a megacap no longer drags a market story onto the tech desk.
        assert_eq!(
            classify("Amazon Just Left Investors Speechless"),
            Some(Beat::Markets)
        );
        // But the products still route to tech.
        for s in [
            "Apple unveils a cheaper MacBook",
            "Regulators open an App Store antitrust case",
            "TSMC raises capex on foundry demand",
        ] {
            assert_eq!(classify(s), Some(Beat::Tech), "should be tech: {s}");
        }
    }

    #[test]
    fn still_drops_what_belongs_on_no_desk() {
        for s in [
            "Boeing wins order for 40 jets",
            "Local council approves new bypass",
            "Wimbledon final draws record audience",
        ] {
            assert_eq!(classify(s), None, "should be dropped: {s}");
        }
    }

    #[test]
    fn most_specific_desk_wins_when_lists_overlap() {
        // Each of these trips two or three lists. The subject-matter desks beat
        // the broad ones, or the widest list would swallow everything.
        assert_eq!(
            classify("Nvidia GPUs power both AI training and crypto mining"),
            Some(Beat::Ai)
        );
        // "AI stocks" does not make this an AI story: bare "ai" deliberately
        // does not fire (see AI_TERMS), and the subject here really is the Fed.
        assert_eq!(
            classify("Fed holds rates steady as AI stocks rally"),
            Some(Beat::Markets)
        );
        // But name a lab and it is an AI story, whatever else it mentions.
        assert_eq!(
            classify("OpenAI raises at a $300B valuation as the Nasdaq rallies"),
            Some(Beat::Ai),
            "a named lab outranks the market vocabulary around it"
        );
        assert_eq!(
            classify("TSMC earnings beat as data centre demand holds"),
            Some(Beat::Markets),
            "market framing wins even when the subject is a chipmaker"
        );
        assert_eq!(
            classify("TSMC raises capex on foundry demand"),
            Some(Beat::Tech),
            "the same company with no market framing is a tech story"
        );
        assert_eq!(
            classify("Bitcoin ETF inflows lift the S&P"),
            Some(Beat::Crypto)
        );
    }

    #[test]
    fn bare_ai_does_not_fire_on_everything() {
        // "AI" as a bare token is in far too many headlines to be a signal, and
        // a false positive here puts a sock startup on a frontier-tech desk.
        assert_eq!(classify("This startup uses AI to sell socks"), None);
        assert_eq!(classify("Ai Weiwei opens a new exhibition"), None);
        // But the compounds do fire.
        assert_eq!(
            classify("A new AI model tops the leaderboard"),
            Some(Beat::Ai)
        );
    }
}
