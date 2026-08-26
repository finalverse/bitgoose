//! What the newsroom is converging on.
//!
//! A trend is not the loudest thing; it is the thing *independent outlets
//! reached separately*. That distinction is the whole design. Virality metrics
//! measure how hard something is being pushed; convergence measures how many
//! newsrooms decided, on their own, that something mattered. For a publication
//! whose pitch is corroboration, the second is both truer and cheaper to
//! compute — it falls out of data we already hold.
//!
//! Deterministic and free: no model is consulted to notice that eleven outlets
//! wrote about the same chip export ban this morning. On a tier that allows
//! 200,000 tokens a day, spending any of them on something arithmetic can do
//! would be indefensible.
//!
//! WASM-safe — pure string work, so the front end can score client-side too.

use std::collections::{HashMap, HashSet};

/// Words that begin sentences and headlines constantly, and would otherwise
/// dominate any capitalisation-based extraction.
const STOP: &[&str] = &[
    "the",
    "a",
    "an",
    "and",
    "or",
    "but",
    "for",
    "with",
    "from",
    "into",
    "onto",
    "over",
    "under",
    "after",
    "before",
    "how",
    "why",
    "what",
    "when",
    "where",
    "who",
    "this",
    "that",
    "these",
    "those",
    "new",
    "now",
    "says",
    "said",
    "will",
    "can",
    "could",
    "should",
    "may",
    "might",
    "here",
    "there",
    "more",
    "most",
    "less",
    "than",
    "then",
    "its",
    "it",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "has",
    "have",
    "had",
    "not",
    "no",
    "yes",
    "all",
    "some",
    "any",
    "one",
    "two",
    "three",
    "first",
    "last",
    "next",
    "top",
    "big",
    "biggest",
    "just",
    "still",
    "amid",
    "as",
    "at",
    "by",
    "in",
    "of",
    "on",
    "to",
    "up",
    "out",
    "off",
    "via",
    "per",
    "you",
    "your",
    "we",
    "our",
    "they",
    "their",
    "he",
    "she",
    "his",
    "her",
    "report",
    "reports",
    "update",
    // Everything below was found by scoring 1,235 real headlines and reading
    // the top of the list, not by imagining what might rank.
    // Months and days: capitalised, frequent, and never the subject.
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    // Titles and roles. "CEO" made the top fifteen; it says nothing about what
    // the story is about.
    "ceo",
    "cto",
    "cfo",
    "coo",
    "president",
    "chair",
    "chairman",
    "founder",
    "analyst",
    "analysts",
    "investors",
    "traders",
    // Terms so central to the beat that everything matches them. On a crypto
    // and AI site, "crypto" as a trending topic is noise by construction.
    "crypto",
    "cryptocurrency",
    "bitcoin\u{2019}s",
    "ai",
    "blockchain",
    "token",
    "tokens",
    "market",
    "markets",
    "price",
    "prices",
];

const CJK_STOP: &[&str] = &[
    "最新",
    "消息",
    "新闻",
    "报道",
    "现场",
    "视频",
    "官方",
    "发布",
    "表示",
    "回应",
    "关于",
    "关注",
    "进行",
    "发生",
    "宣布",
    "今日",
    "今天",
    "目前",
    "记者",
    "速報",
    "発表",
    "明らか",
    "ニュース",
    "관련",
    "발표",
    "소식",
    "오늘",
    "기자",
    "정부",
    "당국",
];

fn is_han_or_kana(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x3040..=0x30FF | 0x31F0..=0x31FF)
}

fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF)
}

/// Script-aware candidates for editions whose headlines do not signal named
/// things with capital letters or even spaces.
///
/// This is intentionally a frequency detector, not a word segmenter. Long CJK
/// runs yield overlapping 2–5-character phrases; the independent-source gate
/// and historical spike comparison decide which phrases are meaningful. A
/// single hot-list platform can therefore suggest a lead, but cannot create a
/// special topic on its own.
fn non_latin_topics(headline: &str) -> Vec<String> {
    let mut segments: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_hangul = false;
    for c in headline.chars().chain(std::iter::once(' ')) {
        let hangul = is_hangul(c);
        let usable = is_han_or_kana(c) || hangul;
        if usable && (current.is_empty() || hangul == current_hangul) {
            current.push(c);
            current_hangul = hangul;
        } else {
            if !current.is_empty() {
                segments.push((std::mem::take(&mut current), current_hangul));
            }
            if usable {
                current.push(c);
                current_hangul = hangul;
            }
        }
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (segment, hangul) in segments {
        let chars: Vec<char> = segment.chars().collect();
        if chars.len() < 2 {
            continue;
        }
        let candidates: Vec<String> = if chars.len() <= 10 {
            vec![segment]
        } else {
            let lengths: &[usize] = if hangul { &[3, 4, 5] } else { &[2, 3, 4, 5] };
            lengths
                .iter()
                .flat_map(|&n| {
                    chars
                        .windows(n)
                        .map(|w| w.iter().collect::<String>())
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        for candidate in candidates {
            if !CJK_STOP.contains(&candidate.as_str()) && seen.insert(candidate.clone()) {
                out.push(candidate);
            }
        }
    }
    out
}

/// A candidate topic pulled from a headline, ignoring corpus knowledge.
///
/// Capitalised runs, because in English headlines those are the named things —
/// companies, people, protocols, bills, chips. Crude next to entity resolution
/// and, on this corpus, sufficient: "Nvidia", "Federal Reserve", "Ethereum
/// Foundation" all survive, while "The Best Way To" does not.
///
/// The first word is the hard case and is skipped here: every headline
/// capitalises it, so "Falls" in "Bitcoin falls below 60k" would read as a
/// name. But headlines also *lead* with the most important entity, so throwing
/// position zero away loses the best signal in the corpus. [`rank`] resolves it
/// with knowledge this function does not have — see [`topics_known`].
pub fn topics(headline: &str) -> Vec<String> {
    topics_known(headline, &HashSet::new())
}

/// As [`topics`], but keeping a leading word already proven to be a name.
///
/// `known` holds words seen capitalised somewhere a capital was *not*
/// automatic — mid-headline. Seeing "Nvidia" in the middle of one headline is
/// what licenses reading it as a name at the start of another.
pub fn topics_known(headline: &str, known: &HashSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut run: Vec<&str> = Vec::new();

    let flush = |run: &mut Vec<&str>, out: &mut Vec<String>| {
        if run.is_empty() {
            return;
        }
        let joined = run.join(" ");
        run.clear();
        // A single lowercase-able stopword that happened to start the headline
        // is not a topic.
        if joined.chars().count() < 3 {
            return;
        }
        out.push(joined);
    };

    for (i, raw) in headline.split_whitespace().enumerate() {
        // Strip the possessive before anything else: "Wall Street's" and
        // "Wall Street" are the same subject and must not split the count.
        let raw = raw
            .trim_end_matches("'s")
            .trim_end_matches("\u{2019}s")
            .trim_end_matches('\u{2019}')
            .trim_end_matches('\'');
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '&');
        if word.is_empty() {
            flush(&mut run, &mut out);
            continue;
        }
        let lower = word.to_lowercase();
        let capitalised = word
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_numeric());
        // An all-caps token is a ticker or an acronym — SEC, ETF, BTC, GPU —
        // and those are among the most useful topics on this beat.
        let acronym = word.len() >= 2
            && word.len() <= 6
            && word.chars().all(|c| c.is_uppercase() || c.is_numeric());

        // The first word is capitalised by convention, so it counts only when
        // something else vouches for it: an acronym, or a word the corpus has
        // shown capitalised where it did not have to be.
        let vouched = acronym || known.contains(&lower);
        let usable =
            (capitalised || acronym) && !STOP.contains(&lower.as_str()) && !(i == 0 && !vouched);

        if usable {
            run.push(word);
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out.extend(non_latin_topics(headline));
    out
}

/// One subject, and how widely it is being covered.
#[derive(Debug, Clone, PartialEq)]
pub struct Heat {
    pub topic: String,
    /// Distinct outlets. The number that decides whether this is news.
    pub sources: usize,
    /// Distinct stories. High with few sources means one outlet is repeating
    /// itself, which is not a trend.
    pub stories: usize,
    pub score: f32,
}

/// Score topics across a set of (topic-bearing headline, source) pairs.
///
/// Weighted hard toward *independent sources* rather than story count. Ten
/// pieces from one outlet is a campaign; three outlets arriving separately is a
/// story. Squaring the source count makes that preference decisive rather than
/// a tiebreak.
pub fn rank(items: &[(String, String)], min_sources: usize) -> Vec<Heat> {
    // First pass: learn which words appear capitalised where the capital was
    // not automatic. That is the evidence that licenses trusting them at the
    // start of a headline.
    let mut known: HashSet<String> = HashSet::new();
    for (headline, _) in items {
        for (i, raw) in headline.split_whitespace().enumerate() {
            if i == 0 {
                continue;
            }
            let w = raw.trim_matches(|c: char| !c.is_alphanumeric());
            if w.chars().next().is_some_and(|c| c.is_uppercase()) {
                known.insert(w.to_lowercase());
            }
        }
    }

    let mut by_topic: HashMap<String, (HashSet<String>, usize)> = HashMap::new();

    for (headline, source) in items {
        // One story mentioning a topic twice still counts once.
        let mut seen = HashSet::new();
        for t in topics_known(headline, &known) {
            let key = t.to_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            let e = by_topic.entry(key).or_insert_with(|| (HashSet::new(), 0));
            e.0.insert(source.clone());
            e.1 += 1;
        }
    }

    // Merge a plural into its singular when both are present. "ETF" and "ETFs"
    // came back as separate rows on real data, splitting one subject's source
    // count across two entries and pushing it down the list. Only merged when
    // both forms actually occur, so this cannot mangle a word that merely ends
    // in s — "Davos" stays "Davos" unless a "Davo" shows up beside it.
    let singulars: Vec<String> = by_topic
        .keys()
        .filter(|k| k.ends_with('s'))
        .filter(|k| by_topic.contains_key(k.trim_end_matches('s')))
        .cloned()
        .collect();
    for plural in singulars {
        let Some((srcs, stories)) = by_topic.remove(&plural) else {
            continue;
        };
        let singular = plural.trim_end_matches('s').to_string();
        if let Some(e) = by_topic.get_mut(&singular) {
            e.0.extend(srcs);
            e.1 += stories;
        }
    }

    let mut out: Vec<Heat> = by_topic
        .into_iter()
        .filter(|(_, (srcs, _))| srcs.len() >= min_sources)
        .map(|(topic, (srcs, stories))| {
            let s = srcs.len() as f32;
            Heat {
                topic,
                sources: srcs.len(),
                stories,
                // Sources dominate; stories break ties among equally-sourced
                // topics.
                score: s * s + stories as f32,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_things_survive_and_headline_furniture_does_not() {
        let t = topics("Chipmakers Nvidia and AMD clash over the new export rules");
        assert!(t.iter().any(|x| x.contains("Nvidia")), "got {t:?}");
        assert!(t.iter().any(|x| x.contains("AMD")), "got {t:?}");
        assert!(
            !t.iter().any(|x| x.to_lowercase().contains("the new")),
            "headline furniture leaked: {t:?}"
        );
    }

    #[test]
    fn a_leading_name_is_recovered_once_the_corpus_vouches_for_it() {
        // Headlines lead with the most important entity, so dropping position
        // zero outright loses the best signal there is. Seeing the word
        // mid-headline elsewhere is what makes it safe to keep.
        let items: Vec<(String, String)> = vec![
            (
                "Nvidia beats earnings expectations".into(),
                "reuters".into(),
            ),
            ("Analysts raise Nvidia targets".into(), "bloomberg".into()),
            ("Nvidia ships new accelerator".into(), "ft".into()),
        ];
        let ranked = rank(&items, 2);
        assert!(
            ranked.iter().any(|h| h.topic == "nvidia" && h.sources == 3),
            "leading name never recovered: {ranked:?}"
        );
    }

    #[test]
    fn the_first_word_is_not_a_topic_merely_for_being_capitalised() {
        // Every headline starts capitalised; treating that as a signal makes
        // the commonest verbs the hottest topics on the site.
        let t = topics("Bitcoin falls below sixty thousand");
        assert!(
            !t.iter().any(|x| x == "Falls"),
            "sentence-initial capital treated as a name: {t:?}"
        );
    }

    #[test]
    fn tickers_and_acronyms_are_kept() {
        let t = topics("SEC approves spot ETF as BTC rallies");
        for want in ["SEC", "ETF", "BTC"] {
            assert!(
                t.iter().any(|x| x.contains(want)),
                "{want} missing from {t:?}"
            );
        }
    }

    #[test]
    fn chinese_headlines_converge_without_spaces_or_capitals() {
        let items = vec![
            ("霍尔木兹海峡航运风险上升".into(), "weibo".into()),
            ("油轮绕行霍尔木兹海峡成本增加".into(), "baidu".into()),
            ("霍尔木兹海峡局势影响国际油价".into(), "netease".into()),
            ("美伊对峙令霍尔木兹海峡受关注".into(), "cna".into()),
            ("霍尔木兹海峡通航仍在继续".into(), "rthk".into()),
        ];
        let ranked = rank(&items, 5);
        assert!(
            ranked.iter().any(|h| h.topic.contains("霍尔木兹")),
            "CJK convergence was invisible: {ranked:?}"
        );
    }

    #[test]
    fn one_chinese_platform_cannot_manufacture_a_topic() {
        let items = (0..12)
            .map(|i| (format!("俄乌停火谈判进展{i}"), "one-platform".into()))
            .collect::<Vec<_>>();
        assert!(rank(&items, 3).is_empty());
    }

    #[test]
    fn one_outlet_repeating_itself_is_not_a_trend() {
        // The distinction the whole metric exists to draw.
        let same_outlet: Vec<(String, String)> = (0..8)
            .map(|i| {
                (
                    format!("Acme Corp does thing {i}"),
                    "one-outlet".to_string(),
                )
            })
            .collect();
        let ranked = rank(&same_outlet, 3);
        assert!(
            ranked.is_empty(),
            "eight stories from one source scored as a trend: {ranked:?}"
        );
    }

    #[test]
    fn independent_outlets_converging_is_a_trend() {
        let converging: Vec<(String, String)> = ["reuters", "bloomberg", "ft", "cnbc"]
            .iter()
            .map(|s| {
                (
                    "Regulators open Acme Corp inquiry".to_string(),
                    s.to_string(),
                )
            })
            .collect();
        let ranked = rank(&converging, 3);
        assert!(!ranked.is_empty(), "four independent outlets should rank");
        assert_eq!(ranked[0].sources, 4);
    }

    #[test]
    fn a_plural_folds_into_its_singular_when_both_appear() {
        // Real output had "etf" and "etfs" as separate rows, splitting one
        // subject across two entries.
        let items: Vec<(String, String)> = vec![
            ("Regulators approve spot ETF".into(), "a".into()),
            ("More spot ETFs list".into(), "b".into()),
            ("Issuers file for ETFs".into(), "c".into()),
        ];
        let ranked = rank(&items, 2);
        let etf: Vec<_> = ranked
            .iter()
            .filter(|h| h.topic.starts_with("etf"))
            .collect();
        assert_eq!(etf.len(), 1, "etf/etfs did not merge: {ranked:?}");
        assert_eq!(etf[0].sources, 3);
    }

    #[test]
    fn a_word_merely_ending_in_s_is_left_alone() {
        // Merging unconditionally would turn "Davos" into "Davo".
        let items: Vec<(String, String)> = ["a", "b", "c"]
            .iter()
            .map(|s| ("Leaders gather at Davos".to_string(), s.to_string()))
            .collect();
        let ranked = rank(&items, 2);
        assert!(ranked.iter().any(|h| h.topic == "davos"), "got {ranked:?}");
    }

    #[test]
    fn more_sources_always_outranks_more_stories() {
        let mut items: Vec<(String, String)> = (0..20)
            .map(|i| {
                (
                    format!("Loud Outlet covers Widgets again {i}"),
                    "loud".into(),
                )
            })
            .collect();
        for s in ["a", "b", "c", "d"] {
            items.push(("Quiet Consensus on Gizmos".to_string(), s.to_string()));
        }
        let ranked = rank(&items, 2);
        // The quiet topic wins on sources; which of its words heads the list
        // does not matter, only that the loud single-source one is beaten.
        assert_eq!(
            ranked[0].sources, 4,
            "twenty stories from one outlet outranked four independent ones: {ranked:?}"
        );
        assert!(
            !ranked[0].topic.contains("widgets"),
            "single-outlet repetition took the top slot: {ranked:?}"
        );
    }
}

/// A subject's heat now, measured against its own normal.
///
/// Raw convergence answers "what is most covered", which on any broad news
/// site is permanently the most active leaders and institutions. That is not a
/// *special* topic — it is the daily beat.
///
/// What makes a topic special is departure from its own baseline. Bitcoin at
/// seven sources is unremarkable; a bill nobody had written about last week at
/// seven sources is the story. So a topic is scored on how far above its usual
/// rate it currently sits, and a subject with no history at all counts as a
/// full spike, because novelty is exactly what a newsroom should notice.
#[derive(Debug, Clone, PartialEq)]
pub struct Spike {
    pub topic: String,
    pub sources: usize,
    pub stories: usize,
    /// Current rate divided by baseline rate. 1.0 is business as usual.
    pub ratio: f32,
    pub score: f32,
}

/// Rank subjects by how far today departs from their own recent history.
///
/// `recent` and `baseline` are (headline, source) pairs from the current window
/// and a longer preceding one. `baseline_days` scales the older window down to
/// a comparable per-day rate.
pub fn rank_spikes(
    recent: &[(String, String)],
    baseline: &[(String, String)],
    baseline_days: f32,
    min_sources: usize,
) -> Vec<Spike> {
    let now = rank(recent, min_sources);
    let before = rank(baseline, 1);

    let mut prior: HashMap<&str, f32> = HashMap::new();
    for h in &before {
        // Stories per day over the baseline window.
        prior.insert(&h.topic, h.stories as f32 / baseline_days.max(1.0));
    }

    let mut out: Vec<Spike> = now
        .into_iter()
        .map(|h| {
            // A day of the current window, for comparison with the baseline
            // rate. The recent window is short, so treat it as roughly a day of
            // activity rather than pretending to more precision than headline
            // counts support.
            let current = h.stories as f32;
            let base = prior.get(h.topic.as_str()).copied().unwrap_or(0.0);
            // Smoothed: a topic seen once before should not read as a 30x
            // spike, and one never seen should not divide by zero.
            let ratio = current / (base + 1.0);
            Spike {
                // Sources still gate entry; the ratio only orders what got in.
                score: ratio * h.sources as f32,
                topic: h.topic,
                sources: h.sources,
                stories: h.stories,
                ratio,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

#[cfg(test)]
mod spike_tests {
    use super::*;

    fn items(headline: &str, sources: &[&str]) -> Vec<(String, String)> {
        sources
            .iter()
            .map(|s| (headline.to_string(), s.to_string()))
            .collect()
    }

    #[test]
    fn a_perennial_subject_does_not_count_as_a_special_topic() {
        // The failure this exists to prevent: the first gaggle opened was
        // "bitcoin, 7 sources", which on a crypto site is simply Tuesday.
        let mut recent = items("Bitcoin holds steady", &["a", "b", "c", "d", "e"]);
        recent.extend(items(
            "Novel Widget Act advances",
            &["a", "b", "c", "d", "e"],
        ));

        // Bitcoin has a long history; the Act has none.
        let mut baseline = Vec::new();
        for i in 0..60 {
            baseline.extend(items(
                &format!("Bitcoin does something {i}"),
                &["a", "b", "c"],
            ));
        }

        let ranked = rank_spikes(&recent, &baseline, 14.0, 3);
        let top = &ranked[0].topic;
        assert!(
            top.contains("widget"),
            "the perennial subject outranked the novel one: {ranked:?}"
        );
    }

    #[test]
    fn a_subject_with_no_history_is_treated_as_a_spike() {
        let recent = items("Unheard Of Thing happens", &["a", "b", "c"]);
        let ranked = rank_spikes(&recent, &[], 14.0, 3);
        assert!(!ranked.is_empty(), "novelty should register");
        assert!(ranked[0].ratio > 1.0, "got {:?}", ranked[0]);
    }

    #[test]
    fn the_source_threshold_still_gates_entry() {
        // The ratio orders what gets in; it must not let one excited outlet in.
        let recent = items("Solo Outlet Scoop", &["only-one"]);
        assert!(rank_spikes(&recent, &[], 14.0, 3).is_empty());
    }
}
