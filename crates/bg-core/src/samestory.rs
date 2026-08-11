//! Deciding that two headlines are about the same event, without asking a model.
//!
//! ## Why this exists
//!
//! Clustering was merging almost nothing: 1,407 of 1,438 published stories
//! carried exactly one source. On a site whose entire proposition is *how many
//! independent outlets confirm this*, that is not a cosmetic defect — it makes
//! the claim ledger say nothing, keeps the Desk empty, and means the front page
//! runs eight variations of one event instead of one story with eight sources.
//!
//! The cause was the shape of the evidence. The deterministic path demanded
//! **both** a close SimHash and 55% trigram overlap, which real coverage almost
//! never produces:
//!
//! ```text
//! Bitcoin and Ethereum ETFs break $1B in their best week since April
//! US spot Bitcoin ETFs post best week since April with $1B inflows
//! ```
//!
//! Obviously one event. Trigram similarity is around a third, because the two
//! newsrooms chose different words for the same facts — which is precisely what
//! independent reporting *is*. So nearly every pair fell through to the model,
//! and everything then depended on a rate-limited call that a free tier refuses
//! for most of the day.
//!
//! ## What actually identifies an event
//!
//! Not how similar the sentences are. **The rare things they both name.**
//! Both headlines above contain `$1B` and `April`; the BIP-110 stories all
//! contain `bip-110`. Two unrelated crypto headlines also share `bitcoin` — and
//! on this corpus that word carries no information at all.
//!
//! So tokens are weighted by how rare they are *in the window being clustered*.
//! `bitcoin` appears in a third of headlines and is worth almost nothing;
//! `bip-110` appears in three and is nearly conclusive. No model, no embedding
//! provider, no configuration — the corpus supplies its own weights, and they
//! stay correct as coverage moves on.
//!
//! ## Deliberately conservative
//!
//! Over-merging is worse than under-merging, and there is a live example of it:
//! one story holds thirteen unrelated items and is published under a headline
//! about a different subject than its own URL. A single shared token can never
//! be decisive here, however rare — coincidence produces those. Agreement has
//! to come from more than one place before this claims an event.

use std::collections::{HashMap, HashSet};

/// Vocabulary that is capitalised for grammatical reasons rather than because
/// it names something.
const NOISE: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "for", "to", "of", "in", "on", "at", "by", "with",
    "from", "as", "is", "are", "was", "were", "be", "been", "has", "have", "had", "will", "would",
    "can", "could", "may", "might", "says", "said", "after", "before", "amid", "over", "into",
    "its", "his", "her", "their", "this", "that", "these", "those", "new", "up", "down", "out",
    "how", "why", "what", "when", "who", "here", "now", "more", "most", "than", "not", "no", "you",
    "your", "we", "our", "it", "he", "she", "they",
];

/// Tokens worth matching on: the things a headline *names*, plus its figures.
///
/// Three kinds survive, and the reason is the same each time — they are chosen
/// by the event rather than by the writer:
///
/// * **Capitalised words**, which in an English headline are the named things:
///   Nvidia, BlackRock, Long March.
/// * **Anything containing a digit** — `BIP-110`, `$1B`, `961,632`, `7A`. These
///   are the strongest signal in the set. Two newsrooms will not independently
///   choose the same phrasing, but they will both report the same number.
/// * **All-caps runs**: SEC, ETF, CFTC.
///
/// Ordinary vocabulary is dropped. "Break" and "post" describe the same event
/// in the pair above and share nothing; matching on them would only add noise.
pub fn key_tokens(headline: &str) -> HashSet<String> {
    key_sequence(headline)
        .into_iter()
        .filter(|t| t != BREAK)
        .collect()
}

/// Marks where ordinary words were dropped.
///
/// Without it the filtered sequence lies about adjacency: "Coldcard issues Mk3"
/// keeps `coldcard` and `mk3` with `issues` removed between them, and they then
/// look like one two-word name. That mistake suppressed the very merges this
/// was built to make — the fix for over-merging must not quietly become a
/// second cause of under-merging.
const BREAK: &str = "\u{0}";

/// As [`key_tokens`], in the order written, with gaps marked.
///
/// Order is what tells a name from a coincidence. "Wall Street" is two tokens
/// and one thing, and counting it as two independent agreements is how three
/// unrelated finance stories were folded into a cluster about a crypto bill.
/// Adjacency — real adjacency, in the source text — is the evidence that two
/// tokens are one name. See [`overlap`].
pub fn key_sequence(headline: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push_break = |out: &mut Vec<String>| {
        if out.last().is_some_and(|t| t != BREAK) {
            out.push(BREAK.to_string());
        }
    };

    for raw in headline.split_whitespace() {
        // Possessives first, or "Nvidia's" and "Nvidia" count as two things.
        let raw = raw
            .trim_end_matches("'s")
            .trim_end_matches("\u{2019}s")
            .trim_end_matches('\u{2019}')
            .trim_end_matches('\'');
        // Keep `$`, `-`, `.` and `%` inside a token: they are what makes
        // `$1B`, `BIP-110` and `7.5%` the specific things they are.
        let word = raw.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '$' && c != '-' && c != '.' && c != '%'
        });
        let lower = word.to_lowercase();
        let has_digit = word.chars().any(|c| c.is_ascii_digit());
        let capitalised = word.chars().next().is_some_and(|c| c.is_uppercase());
        let acronym = word.chars().filter(|c| c.is_alphabetic()).count() >= 2
            && word
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(|c| c.is_uppercase());

        let keep = word.chars().count() >= 2
            && !NOISE.contains(&lower.as_str())
            && (has_digit || capitalised || acronym);
        if keep {
            out.push(lower);
        } else {
            push_break(&mut out);
        }
    }
    out
}

/// Shared tokens that sit next to each other in *both* headlines, as pairs.
///
/// One name written the same way twice, not two coincidences.
fn joined_in_both(a: &[String], b: &[String], shared: &HashSet<&String>) -> usize {
    let adjacent = |seq: &[String]| -> HashSet<(String, String)> {
        seq.windows(2)
            .filter(|w| shared.contains(&&w[0]) && shared.contains(&&w[1]))
            .map(|w| (w[0].clone(), w[1].clone()))
            .collect()
    };
    adjacent(a).intersection(&adjacent(b)).count()
}

/// How much each token narrows things down, learned from the window itself.
///
/// The point is that no word is inherently informative: `bitcoin` is nearly
/// worthless on this corpus and would be decisive on a general news site, and
/// neither fact needs to be configured anywhere — it falls out of counting.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    df: HashMap<String, f32>,
    n: f32,
}

/// Weight of a token not seen in the window.
///
/// Rare by definition — it is in the two headlines being compared and nowhere
/// else — but capped, because from here a typo and a genuinely novel name look
/// identical.
const UNSEEN_WEIGHT: f32 = 2.0;

impl Corpus {
    /// Count what the window contains.
    pub fn of(headlines: &[String]) -> Self {
        let mut df: HashMap<String, f32> = HashMap::new();
        for h in headlines {
            for t in key_tokens(h) {
                *df.entry(t).or_insert(0.0) += 1.0;
            }
        }
        Self {
            df,
            n: headlines.len().max(1) as f32,
        }
    }

    /// Inverse document frequency, for scoring.
    fn weight(&self, t: &str) -> f32 {
        match self.df.get(t) {
            Some(d) => (self.n / (1.0 + d)).ln().max(0.0),
            None => UNSEEN_WEIGHT,
        }
    }

    /// Whether a token is specific enough that sharing it means something.
    ///
    /// A **document-frequency ratio**, not a weight threshold. Inverse document
    /// frequency is measured in `ln(N)`, so any fixed cutoff on it means
    /// something different for a fourteen-headline window than for a
    /// three-hundred one — the first calibration of this drifted exactly that
    /// way and admitted nothing. A share of the window is the same idea at
    /// every size.
    ///
    /// The floor of three matters as much as the ratio: in a small window the
    /// three outlets that covered one event *are* the whole of that token's
    /// frequency, and a percentage alone would rule out the very thing being
    /// looked for.
    fn is_rare(&self, t: &str) -> bool {
        match self.df.get(t) {
            Some(d) => *d <= (self.n * RARE_SHARE).max(MIN_RARE_DOCS),
            None => true,
        }
    }
}

/// A token in more of the window than this is describing the beat, not an
/// event.
const RARE_SHARE: f32 = 0.05;

/// …but never fewer than this many documents, or a small window admits nothing.
const MIN_RARE_DOCS: f32 = 3.0;

/// What two headlines share, and how much it is worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlap {
    /// Shared weight as a fraction of the lighter headline's total, 0..1.
    ///
    /// Measured against the *lighter* one so that a wire brief matching part of
    /// a long analysis still scores high; the brief has said everything it has
    /// to say.
    pub score: f32,
    /// How many shared tokens were specific enough to mean something.
    pub rare_hits: usize,
}

impl Overlap {
    /// Strong enough to attach without asking a model.
    ///
    /// Two specific tokens, not one. One shared rare token is how unrelated
    /// stories end up in the same cluster: a coincidental `$1B`, or two
    /// different events on the same day both naming one company. Requiring a
    /// second, independent coincidence is what separates this from the merge
    /// that put thirteen unrelated items under a single headline.
    pub fn confident(&self) -> bool {
        self.rare_hits >= 2 && self.score >= 0.5
    }

    /// Worth spending a model call on.
    ///
    /// The Trump Media pair sits here: two outlets on one event sharing only
    /// the company name. Real, but not something arithmetic should claim.
    pub fn worth_asking(&self) -> bool {
        self.rare_hits >= 1 && self.score >= 0.28
    }
}

/// Compare two headlines through the vocabulary of the window they sit in.
pub fn overlap(a: &str, b: &str, corpus: &Corpus) -> Overlap {
    let (sa, sb) = (key_sequence(a), key_sequence(b));
    // The gap markers stay in the sequences, where adjacency is read, and are
    // kept out of the sets, where agreement is counted. Leaving them in made
    // every pair of headlines share one "token" and score a free rare hit.
    let keys = |seq: &[String]| -> HashSet<String> {
        seq.iter().filter(|t| *t != BREAK).cloned().collect()
    };
    let (ta, tb) = (keys(&sa), keys(&sb));
    if ta.is_empty() || tb.is_empty() {
        return Overlap {
            score: 0.0,
            rare_hits: 0,
        };
    }
    let total = |set: &HashSet<String>| set.iter().map(|t| corpus.weight(t)).sum::<f32>();
    let floor = total(&ta).min(total(&tb));
    if floor <= 0.0 {
        return Overlap {
            score: 0.0,
            rare_hits: 0,
        };
    }

    let mut shared_weight = 0.0;
    let mut rare_hits = 0usize;
    let mut rare_shared: HashSet<&String> = HashSet::new();
    for t in ta.intersection(&tb) {
        shared_weight += corpus.weight(t);
        if corpus.is_rare(t) {
            rare_hits += 1;
            rare_shared.insert(t);
        }
    }
    // A multi-word name is one agreement, not one per word. Without this,
    // "Wall Street" alone clears the two-hit bar and a story about a crypto
    // bill absorbs one about GoDaddy's AI strategy — which is exactly what the
    // first live run of this did.
    rare_hits = rare_hits.saturating_sub(joined_in_both(&sa, &sb, &rare_shared));

    Overlap {
        score: (shared_weight / floor).clamp(0.0, 1.0),
        rare_hits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window shaped like the real one: crypto words everywhere, so the
    /// common vocabulary is genuinely uninformative.
    fn window() -> Vec<String> {
        [
            "Bitcoin and Ethereum ETFs break $1B in their best week since April",
            "US spot Bitcoin ETFs post best week since April with $1B inflows",
            "Bitcoin's BIP-110 supporters split onto minority chain as main network pulls ahead",
            "Bitcoin's BIP-110 enters mandatory signaling with miner support below 3%",
            "Bitcoin hits block 961,632 as the controversial BIP-110 soft fork attempt begins",
            "Bitcoin Red Team Says AI Is Finding Critical Exploits Across Core Projects",
            "Bitcoin tops $65,000 after massive surprise US jobs miss",
            "Trump Media Pulls Back From Crypto Deals: Report",
            "Donald Trump's media company to terminate Crypto.com deal",
            "A Tough Week for Crypto Has Fans Downing Drinks at a Bitcoin Bar",
            "Dow Jones Futures Rise As Cloudflare Leads Big Software Winners",
            "CFTC cautions prediction markets over using American-style moneyline odds",
            "Ethereum Foundation announces new grants programme for client diversity",
            "Solana outage resolved after validators restart the network",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn scores_for_calibration() {
        let idf = Corpus::of(&window());
        let w = window();
        for (a, b, label) in [
            (0usize, 1usize, "ETF pair (should match)"),
            (2, 3, "BIP-110 a/b"),
            (2, 4, "BIP-110 a/c"),
            (3, 4, "BIP-110 b/c"),
            (7, 8, "Trump Media pair"),
            (6, 10, "unrelated"),
        ] {
            println!("{label}: {:?}", overlap(&w[a], &w[b], &idf));
            println!("   A tokens {:?}", key_tokens(&w[a]));
            println!("   B tokens {:?}", key_tokens(&w[b]));
        }
    }

    #[test]
    fn the_pair_that_used_to_fall_through_now_matches() {
        // The motivating example. Trigram similarity is about a third, which is
        // why the old decisive bar of 0.55 never fired on real coverage.
        let idf = Corpus::of(&window());
        let o = overlap(
            "Bitcoin and Ethereum ETFs break $1B in their best week since April",
            "US spot Bitcoin ETFs post best week since April with $1B inflows",
            &idf,
        );
        assert!(o.confident(), "expected a confident match, got {o:?}");
    }

    #[test]
    fn three_outlets_on_one_soft_fork_all_match() {
        let idf = Corpus::of(&window());
        let w = window();
        for (a, b) in [(2, 3), (2, 4), (3, 4)] {
            let o = overlap(&w[a], &w[b], &idf);
            assert!(o.worth_asking(), "{a} vs {b} scored {o:?}");
        }
    }

    #[test]
    fn the_common_word_carries_no_weight() {
        // Every headline in the window says "bitcoin". Two that share only that
        // must not match — this is the whole reason for weighting by rarity.
        let idf = Corpus::of(&window());
        let o = overlap(
            "Bitcoin tops $65,000 after massive surprise US jobs miss",
            "A Tough Week for Crypto Has Fans Downing Drinks at a Bitcoin Bar",
            &idf,
        );
        assert!(!o.confident(), "unrelated bitcoin stories matched: {o:?}");
    }

    #[test]
    fn the_mega_cluster_pairs_stay_apart() {
        // Every one of these was merged into a single published story, which
        // then went out under a headline about a different subject entirely.
        let idf = Corpus::of(&window());
        let w = window();
        for (a, b) in [(6, 10), (7, 11), (9, 10), (5, 13), (0, 11)] {
            let o = overlap(&w[a], &w[b], &idf);
            assert!(!o.confident(), "would have merged {a} with {b}: {o:?}");
        }
    }

    #[test]
    fn one_shared_rare_token_is_never_enough() {
        // Same company, two genuinely different events on the same day.
        let idf = Corpus::of(&window());
        let o = overlap(
            "Cloudflare outage takes down a third of the web for two hours",
            "Dow Jones Futures Rise As Cloudflare Leads Big Software Winners",
            &idf,
        );
        assert!(!o.confident(), "single-token coincidence merged: {o:?}");
    }

    #[test]
    fn figures_survive_tokenisation() {
        let t = key_tokens("Bitcoin hits block 961,632 as BIP-110 clears 7.5% with $1B behind it");
        for want in ["961,632", "bip-110", "7.5%", "$1b"] {
            assert!(t.contains(want), "lost {want} from {t:?}");
        }
    }

    #[test]
    fn ordinary_vocabulary_is_dropped() {
        let t = key_tokens("The company said it will have been more than ready by then");
        assert!(t.is_empty(), "kept vocabulary: {t:?}");
    }

    #[test]
    fn an_empty_headline_matches_nothing() {
        let idf = Corpus::of(&window());
        assert_eq!(overlap("", "Bitcoin tops $65,000", &idf).score, 0.0);
    }
}
