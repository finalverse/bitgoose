//! Is a mainstream-finance item about crypto?
//!
//! The nine crypto desks can be taken wholesale — everything they publish is on
//! topic. Yahoo Finance, Bloomberg, CNBC, MarketWatch and the FT cannot: their
//! feeds are overwhelmingly equities, rates and earnings, and ingesting them
//! raw would bury the crypto coverage we actually want under general business
//! news.
//!
//! So items from a [`SourceKind::Finance`](bg_core::domain::SourceKind) source
//! pass through this gate first. It is deliberately deterministic — a keyword
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
