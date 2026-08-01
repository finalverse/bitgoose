//! Text primitives shared by the clustering, policy and rendering layers.
//!
//! Everything here is pure and deterministic — no RNG, no clock, no I/O — so
//! the same input yields the same fingerprint on the server, in the browser and
//! across process restarts. That matters because [`simhash64`] values are
//! persisted to Postgres and compared across runs; `std`'s `DefaultHasher` is
//! explicitly *not* stable across Rust releases and would silently corrupt the
//! dedupe index on a toolchain bump, so we hash with FNV-1a by hand.

/// Words in a quote we will publish verbatim, and the longest run of source
/// wording a generated draft may share with its source. See [`crate::policy`].
pub const DEFAULT_MAX_QUOTE_WORDS: usize = 25;

/// Lowercases, strips punctuation, and splits into word tokens.
pub fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '$' || *c == '%')
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word count as the policy engine counts it.
pub fn word_count(s: &str) -> usize {
    s.split_whitespace()
        .filter(|w| !w.trim().is_empty())
        .count()
}

/// Truncates to at most `max` words, appending an ellipsis if it cut anything.
pub fn truncate_words(s: &str, max: usize) -> String {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() <= max {
        return s.trim().to_string();
    }
    format!("{}…", parts[..max].join(" "))
}

/// Very common words carry no topical signal; dropping them stops SimHash from
/// clustering on grammar instead of subject matter.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "if", "of", "to", "in", "on", "for", "with", "as", "at",
    "by", "from", "is", "are", "was", "were", "be", "been", "it", "its", "that", "this", "these",
    "those", "has", "have", "had", "will", "would", "can", "could", "after", "over", "into",
    "than", "then", "up", "out", "new", "says", "said",
];

fn is_stopword(w: &str) -> bool {
    STOPWORDS.contains(&w)
}

/// FNV-1a. Chosen for stability across toolchains, not for cryptographic value.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 64-bit SimHash over content words.
///
/// Near-duplicate detection without an embedding provider: two rewrites of the
/// same wire story land within a few bits of each other, so [`hamming`] under a
/// small threshold is a cheap first-pass "is this the same event?".
pub fn simhash64(s: &str) -> u64 {
    let toks: Vec<String> = words(s).into_iter().filter(|w| !is_stopword(w)).collect();
    if toks.is_empty() {
        return 0;
    }
    let mut acc = [0i32; 64];
    for t in &toks {
        let h = fnv1a64(t.as_bytes());
        for (i, slot) in acc.iter_mut().enumerate() {
            if (h >> i) & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }
    let mut out = 0u64;
    for (i, v) in acc.iter().enumerate() {
        if *v > 0 {
            out |= 1u64 << i;
        }
    }
    out
}

/// Bit distance between two SimHashes. 0 = identical fingerprint.
pub const fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Character trigrams of the normalized string.
pub fn trigrams(s: &str) -> Vec<[char; 3]> {
    let norm: Vec<char> = words(s).join(" ").chars().collect();
    if norm.len() < 3 {
        return Vec::new();
    }
    norm.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

/// Jaccard similarity over trigram sets, in `0.0..=1.0`.
///
/// Complements SimHash: SimHash is bag-of-words and ignores order, trigrams
/// catch shared phrasing. Agreement between the two is a strong dupe signal.
pub fn trigram_similarity(a: &str, b: &str) -> f32 {
    use std::collections::HashSet;
    let sa: HashSet<[char; 3]> = trigrams(a).into_iter().collect();
    let sb: HashSet<[char; 3]> = trigrams(b).into_iter().collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    inter / union
}

/// Cosine similarity of two equal-length vectors. Returns 0.0 on mismatch.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Length of the longest run of consecutive words shared by `a` and `b`.
///
/// This is the plagiarism tripwire. A model handed source text will sometimes
/// reproduce a clause or a whole sentence of it, and no amount of prompt
/// instruction reliably prevents that. Measuring the overlap directly catches
/// it regardless of why it happened.
pub fn longest_common_word_run(a: &str, b: &str) -> usize {
    let wa = words(a);
    let wb = words(b);
    if wa.is_empty() || wb.is_empty() {
        return 0;
    }
    // Two-row DP over the classic longest-common-substring recurrence.
    let mut prev = vec![0usize; wb.len() + 1];
    let mut cur = vec![0usize; wb.len() + 1];
    let mut best = 0usize;
    for i in 1..=wa.len() {
        for j in 1..=wb.len() {
            cur[j] = if wa[i - 1] == wb[j - 1] {
                prev[j - 1] + 1
            } else {
                0
            };
            if cur[j] > best {
                best = cur[j];
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.iter_mut().for_each(|v| *v = 0);
    }
    best
}

/// Reading time in seconds at 240 wpm, floored at 30s.
pub fn reading_time_s(body: &str) -> i32 {
    let w = word_count(body) as f32;
    ((w / 240.0 * 60.0).round() as i32).max(30)
}

/// Collapses whitespace and strips any HTML tags. Feed summaries arrive full of
/// markup and tracking pixels; this is the sanitizer for anything we display.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simhash_is_stable_and_near_for_rewrites() {
        let a = "Bitcoin ETF inflows hit a record $1.2 billion on Tuesday";
        let b = "Record $1.2 billion flowed into Bitcoin ETFs on Tuesday";
        let c = "Ethereum developers delay the Fusaka hard fork to March";
        assert_eq!(simhash64(a), simhash64(a), "must be deterministic");
        assert!(
            hamming(simhash64(a), simhash64(b)) < hamming(simhash64(a), simhash64(c)),
            "a rewrite must fingerprint closer than an unrelated story"
        );
    }

    #[test]
    fn simhash_survives_the_empty_string() {
        assert_eq!(simhash64(""), 0);
        assert_eq!(
            simhash64("the and of"),
            0,
            "all-stopword input has no signal"
        );
    }

    #[test]
    fn longest_common_run_finds_lifted_wording() {
        let src = "The exchange said it had frozen the attacker's funds within four minutes.";
        let clean = "Funds tied to the attacker were frozen quickly, the venue reported.";
        let lifted = "Sources say it had frozen the attacker's funds within four minutes.";
        assert!(longest_common_word_run(src, clean) < 4);
        assert!(longest_common_word_run(src, lifted) >= 8);
    }

    #[test]
    fn truncate_words_respects_the_cap() {
        let s = "one two three four five";
        assert_eq!(truncate_words(s, 3), "one two three…");
        assert_eq!(truncate_words(s, 99), s);
    }

    #[test]
    fn strip_html_removes_markup_and_entities() {
        assert_eq!(strip_html("<p>Hello   &amp; <b>bye</b></p>"), "Hello & bye");
    }

    #[test]
    fn trigram_similarity_is_bounded_and_ordered() {
        let a = "solana outage halts block production";
        let b = "solana outage halts block production again";
        let c = "sec approves spot ether etf applications";
        let ab = trigram_similarity(a, b);
        let ac = trigram_similarity(a, c);
        assert!((0.0..=1.0).contains(&ab) && (0.0..=1.0).contains(&ac));
        assert!(ab > ac);
    }
}
