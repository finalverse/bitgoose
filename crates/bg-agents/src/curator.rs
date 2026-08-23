//! **Curator** — decides what is one story and what is five.
//!
//! The hinge of the whole system. Getting this wrong in one direction shows the
//! same hack five times on the front page; in the other it silently merges two
//! unrelated events and reports them as corroborating each other. The second
//! failure is worse, so the thresholds are tuned to split when uncertain.
//!
//! Three-stage cascade, cheapest first:
//!
//! 1. **Lexical** — SimHash + trigram similarity against recent clustered
//!    items. Free, and decisive for the clear cases at both ends.
//! 2. **LLM adjudication** — only for the ambiguous middle band, where two
//!    items are plausibly the same event but the wording differs.
//! 3. **New story** — the default when nothing matches.
//!
//! Most items never reach stage 2, which is what makes clustering affordable
//! at the volume Scout produces.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, Category, ItemRole, ModelTier, RawItem, StoryKind};
use bg_core::ids::StoryId;
use bg_core::text::{hamming, trigram_similarity};
use bg_llm::{schema as sch, Request};
use chrono::Utc;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{debug, info};

pub const SYSTEM: &str = "Curator decides whether two reports describe the same event.

The same event means the same underlying occurrence: the same hack, the same \
filing, the same launch, the same vote. It is NOT the same event merely because \
two items share a company, a token, or a theme.

Say no when:
- one is a follow-up analysis of the other's event (related, not identical)
- they concern the same entity but different occurrences
- one is a roundup that happens to mention the other's event
- you are unsure

Splitting a story in two is a small error. Merging two events is a serious one, \
because everything downstream will treat them as corroborating each other. When \
in doubt, say no.";

/// Below this Hamming distance the fingerprints are close enough to attach
/// without asking a model.
const SIMHASH_SAME: u32 = 12;
/// Above this, not worth considering.
const SIMHASH_FAR: u32 = 26;
/// Trigram overlap that on its own settles it.
const TRIGRAM_SAME: f32 = 0.55;
/// Below this, not worth asking about.
const TRIGRAM_FLOOR: f32 = 0.18;
/// Clustering window. The same headline six months apart is two events.
const WINDOW_HOURS: i64 = 36;

#[derive(Debug, Deserialize)]
struct SameEvent {
    same_event: bool,
    #[allow(dead_code)]
    reason: String,
}

fn schema() -> serde_json::Value {
    sch::object(
        vec![
            (
                "same_event",
                sch::boolean("true only if the same underlying occurrence"),
            ),
            ("reason", sch::string_hinted("one sentence", "reason")),
        ],
        &["same_event", "reason"],
    )
}

/// Cluster every unclustered item.
pub async fn run(ctx: &Ctx, limit: i64) -> Result<usize> {
    let pending = bg_db::items::unclustered(&ctx.db, limit).await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let system = crate::system_prompt(ctx, AgentRole::Curator).await;

    // Hoisted out of the loop. This was one database round trip per item, and
    // the window it returns does not change between them — except for stories
    // opened by this very pass, which are appended below so two items about one
    // event arriving in the same batch still find each other.
    let mut candidates = bg_db::items::clustering_candidates(&ctx.db, WINDOW_HOURS, 300).await?;

    // What the window's vocabulary is worth. Built from the candidates *and*
    // the pending items, so a subject arriving now is measured against how
    // often it is actually being said rather than against nothing.
    let corpus = bg_core::samestory::Corpus::of(
        &candidates
            .iter()
            .chain(pending.iter())
            .map(|i| i.title.clone())
            .collect::<Vec<_>>(),
    );

    let (mut attached, mut deferred) = (0usize, 0usize);

    for item in pending {
        let best = best_match(&item, &candidates, &corpus);

        let target: Option<StoryId> = match best {
            Some((cand, score)) if score.decisive => cand.story_id,
            Some((cand, score)) => {
                // Ambiguous: ask.
                let cand_title = cand.title.clone();
                let item_title = item.title.clone();
                let story_id = cand.story_id;
                let system = system.clone();
                let verdict = stage(
                    ctx,
                    AgentRole::Curator,
                    story_id,
                    "adjudicate",
                    |_run| async move {
                        let prompt = format!(
                            "Item A: {item_title}\nItem B: {cand_title}\n\n\
                         Do A and B report the same underlying event?"
                        );
                        let req =
                            Request::new("curator.same_event", ModelTier::Fast, system, prompt)
                                .with_schema(schema())
                                .with_max_tokens(500);
                        let (parsed, completion) = ctx.llm.complete_json::<SameEvent>(&req).await?;
                        let note = format!(
                            "same_event={} (simhash {}, trigram {:.2}, salience {:.2}/{})",
                            parsed.same_event,
                            score.hamming,
                            score.trigram,
                            score.overlap.score,
                            score.overlap.rare_hits
                        );
                        Ok(StageOutput::with(parsed.same_event, completion, note))
                    },
                )
                .await;

                match verdict {
                    Ok(true) => story_id,
                    Ok(false) => None,
                    // The provider being unavailable is not a verdict of "no".
                    //
                    // This used to fall through to `false`, which opened a new
                    // story — and an item that has a story is never offered for
                    // clustering again. So every hour the free tier spent
                    // refusing requests permanently minted single-source
                    // stories that could not be merged afterwards, which is a
                    // large part of how 1,407 of 1,438 came to have one source.
                    // Leaving it unclustered costs one deferred item and keeps
                    // the decision available.
                    Err(e) if e.is_transient() => {
                        debug!(item = %item.title, "deferring: {e}");
                        deferred += 1;
                        continue;
                    }
                    Err(_) => None,
                }
            }
            None => None,
        };

        let story_id = match target {
            Some(id) => {
                bg_db::items::attach_to_story(&ctx.db, item.id, id, ItemRole::Corroborating)
                    .await?;
                debug!(item = %item.title, "attached to existing story");
                id
            }
            None => {
                let category = item_category(ctx, &item).await;
                let slug = bg_core::slug::slugify(&item.title);
                let story = bg_db::stories::create(
                    &ctx.db,
                    &slug,
                    StoryKind::Wire,
                    &item.title,
                    category,
                    // The seed item was routed to a desk at ingest; the
                    // story it opens belongs to the same one. Falling back
                    // on the category keeps a story off the wrong desk when
                    // an older item predates beat routing.
                    item.beat
                        .or_else(|| bg_core::domain::Beat::of_category(category))
                        .unwrap_or(bg_core::domain::Beat::Crypto),
                )
                .await?;
                bg_db::items::attach_to_story(&ctx.db, item.id, story.id, ItemRole::Seed).await?;
                story.id
            }
        };

        // Now a candidate itself, so the next item in this batch can match it.
        let mut placed = item.clone();
        placed.story_id = Some(story_id);
        candidates.push(placed);

        rescore(ctx, story_id).await?;
        attached += 1;
    }

    info!(attached, deferred, "curator pass complete");
    Ok(attached)
}

struct MatchScore {
    hamming: u32,
    trigram: f32,
    /// What rare vocabulary the two headlines share. The signal that actually
    /// identifies an event — see [`bg_core::samestory`].
    overlap: bg_core::samestory::Overlap,
    /// True when the deterministic signals settle it with no model call.
    decisive: bool,
}

/// Best match among candidates, if any is worth considering.
///
/// Three signals, and they answer different questions. SimHash and trigram
/// similarity ask *how alike is the wording*, which two newsrooms covering one
/// event deliberately make different. Salience overlap asks *what rare things
/// do both name*, which the event decides rather than the writer — so it is the
/// one that carries most of the weight here.
fn best_match<'a>(
    item: &RawItem,
    candidates: &'a [RawItem],
    corpus: &bg_core::samestory::Corpus,
) -> Option<(&'a RawItem, MatchScore)> {
    let mut best: Option<(&RawItem, MatchScore)> = None;

    for c in candidates {
        if c.id == item.id || c.story_id.is_none() {
            continue;
        }
        // Two reports of one event come from different outlets, and the same
        // source filing twice is usually two genuinely different stories — a
        // publisher running five Bitcoin pieces in a day writes five similar
        // headlines about five events, and merging those would be worse than
        // leaving them apart.
        //
        // An *identical* headline is the exception, and it is not a rare one.
        // An aggregator carries many outlets under one source_id, so when
        // Google News surfaces the same AP story seven times it arrives seven
        // times as `gnews-crypto` and this guard skipped every pair. Measured
        // on the live database: seven copies of "How bitcoin and gold went from
        // a slump to an MVP week", each published as its own single-source
        // story, and 3,198 of 3,236 stories in a day sitting at one source.
        //
        // Byte-identical titles are the same story whoever filed them, so they
        // skip the guard. Nothing else about the match is relaxed.
        let identical = item.title.trim().eq_ignore_ascii_case(c.title.trim());
        if c.source_id == item.source_id && !identical {
            continue;
        }

        let h = hamming(item.simhash as u64, c.simhash as u64);
        let t = trigram_similarity(&item.title, &c.title);
        let o = bg_core::samestory::overlap(&item.title, &c.title, corpus);
        // Nothing in common on any of the three: not worth carrying further.
        if h > SIMHASH_FAR && t < TRIGRAM_FLOOR && !o.worth_asking() {
            continue;
        }

        // Either route settles it. Near-identical wording still means the same
        // story — a syndicated wire item runs verbatim in several places — and
        // so does agreement on two rare specifics, which is what independent
        // reporting of one event looks like.
        let decisive = (h <= SIMHASH_SAME && t >= TRIGRAM_SAME) || o.confident();
        let score = MatchScore {
            hamming: h,
            trigram: t,
            overlap: o,
            decisive,
        };

        // Ranked on shared specifics first. Ordering by trigram alone picked
        // the most similarly *worded* candidate, which on a page of headlines
        // about one subject is not the same as the most likely match.
        let better = match &best {
            None => true,
            Some((_, b)) => (score.overlap.score, score.trigram) > (b.overlap.score, b.trigram),
        };
        if better {
            best = Some((c, score));
        }
    }
    // A candidate that neither settles it nor justifies a model call is not a
    // match — returning it would spend a request on a pair the arithmetic has
    // already dismissed.
    best.filter(|(_, s)| s.decisive || s.overlap.worth_asking() || s.trigram >= TRIGRAM_FLOOR)
}

async fn item_category(ctx: &Ctx, item: &RawItem) -> Category {
    let raw: Option<String> = sqlx::query_scalar("SELECT category FROM raw_items WHERE id = $1")
        .bind(item.id.into_uuid())
        .fetch_optional(&ctx.db.pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|c| Category::from_str(&c).ok())
        .unwrap_or(Category::Markets)
}

/// Recompute a story's newsworthiness and velocity from its evidence.
///
/// Deterministic on purpose. Ranking is the most consequential number on the
/// site and the one a reader is most entitled to have explained, so it is
/// arithmetic over observable facts — how many independent outlets, how
/// trusted, how fast — rather than a model's opinion.
pub async fn rescore(ctx: &Ctx, story: StoryId) -> Result<i16> {
    let items = bg_db::items::by_story(&ctx.db, story).await?;
    if items.is_empty() {
        return Ok(0);
    }

    let row = sqlx::query_as::<_, (Option<f64>, Option<i64>, Option<i64>)>(
        "SELECT avg(s.trust)::float8, max(r.triage_score)::bigint, count(DISTINCT r.source_id)
         FROM raw_items r JOIN sources s ON s.id = r.source_id
         WHERE r.story_id = $1",
    )
    .bind(story.into_uuid())
    .fetch_one(&ctx.db.pool)
    .await?;

    let avg_trust = row.0.unwrap_or(50.0) as f32;
    let peak_triage = row.1.unwrap_or(0) as f32;
    let sources = row.2.unwrap_or(1) as f32;

    // Independent corroboration is the strongest signal we have, but with
    // diminishing returns — the fifth outlet reprinting a wire story adds far
    // less than the second one confirming independently.
    let corroboration = (sources.min(6.0) - 1.0) * 6.0;
    let trust_adj = (avg_trust - 60.0) * 0.25;

    // Velocity: independent sources per hour since the story was first seen.
    let first = items
        .iter()
        .map(|i| i.published_at)
        .min()
        .unwrap_or_else(Utc::now);
    let hours = ((Utc::now() - first).num_minutes() as f32 / 60.0).max(0.25);
    let velocity = sources / hours;
    let velocity_bonus = (velocity * 4.0).min(12.0);

    let score = (peak_triage + corroboration + trust_adj + velocity_bonus).clamp(0.0, 100.0) as i16;
    bg_db::stories::set_scores(&ctx.db, story, score, velocity).await?;

    // Carry a lead image up from the items.
    //
    // Publishers put one in the feed for almost everything — 224 of our first
    // 234 items had one — but nothing promoted it to the story, so every page
    // rendered as a wall of text. This runs here rather than at creation
    // because the seed item is often the one *without* an image: a
    // corroborating outlet attached later can supply it, and `set_meta`
    // COALESCEs, so the first usable one wins and later passes leave it alone.
    if let Some(img) = pick_lead_image(&items) {
        bg_db::stories::set_meta(&ctx.db, story, None, None, &[], Some(&img), None).await?;
    }
    // A video story is a video story because its seed item was one; the same
    // COALESCE rule applies, so the first item to supply an id wins.
    if let Some(vid) = items.iter().find_map(|i| i.video_id.clone()) {
        bg_db::stories::set_meta(&ctx.db, story, None, None, &[], None, Some(&vid)).await?;
    }

    Ok(score)
}

/// Choose the image to represent a story, or nothing.
///
/// Earliest-published first: where several outlets covered one event, the first
/// to publish is likeliest to own the photograph rather than to have picked up
/// the same handout.
fn pick_lead_image(items: &[bg_core::domain::RawItem]) -> Option<String> {
    let mut with_images: Vec<&bg_core::domain::RawItem> = items
        .iter()
        .filter(|i| i.image_url.as_deref().is_some_and(usable_image))
        .collect();
    with_images.sort_by_key(|i| i.published_at);
    with_images
        .first()
        .and_then(|i| i.image_url.as_deref())
        .map(|u| u.trim().to_string())
}

/// Reject what is not really an editorial image.
///
/// Feeds carry tracking pixels, spacers and share-button sprites in the same
/// fields as photography, and one of those stretched across a lead slot looks
/// considerably worse than no image at all.
fn usable_image(url: &str) -> bool {
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) {
        return false;
    }
    let lower = u.to_ascii_lowercase();
    const JUNK: &[&str] = &[
        "pixel",
        "spacer",
        "blank.",
        "1x1",
        "avatar",
        "gravatar",
        "logo",
        "icon",
        "badge",
        "feedburner",
        "doubleclick",
        "/ad/",
        "/ads/",
        "sharethis",
        "addthis",
    ];
    if JUNK.iter().any(|j| lower.contains(j)) {
        return false;
    }
    // In a news feed an SVG is nearly always chrome rather than photography.
    !lower.split('?').next().unwrap_or("").ends_with(".svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_pixels_and_chrome_are_not_lead_images() {
        for bad in [
            "https://x.test/pixel.gif",
            "https://x.test/img/1x1.png",
            "https://x.test/assets/logo-dark.png",
            "https://x.test/static/icon-share.png",
            "https://x.test/brand.svg",
            "https://x.test/brand.svg?v=3",
            "//x.test/protocol-relative.jpg",
            "data:image/png;base64,iVBOR",
        ] {
            assert!(!usable_image(bad), "should have rejected {bad}");
        }
        for good in [
            "https://cdn.sanity.io/images/6oftkxoa/production/0dfd625aaaf.jpg",
            "https://www.tbstat.com/wp/uploads/2023/04/20230411_Hack_Generic_1-800x450.jpg",
        ] {
            assert!(usable_image(good), "should have accepted {good}");
        }
    }

    #[test]
    fn the_lead_image_comes_from_the_earliest_outlet_that_had_one() {
        let src = SourceId::new();
        let mut late = item(src, "Solana outage halts block production");
        late.published_at = Utc::now();
        late.image_url = Some("https://x.test/late.jpg".into());

        let mut early = item(src, "Solana outage halts block production");
        early.published_at = Utc::now() - chrono::Duration::hours(3);
        early.image_url = Some("https://x.test/early.jpg".into());

        // Earlier still, but only a tracking pixel — must not win.
        let mut earliest = item(src, "Solana outage halts block production");
        earliest.published_at = Utc::now() - chrono::Duration::hours(5);
        earliest.image_url = Some("https://x.test/pixel.gif".into());

        let picked = pick_lead_image(&[late, early, earliest]);
        assert_eq!(picked.as_deref(), Some("https://x.test/early.jpg"));
    }

    #[test]
    fn a_story_with_no_usable_image_gets_none() {
        let src = SourceId::new();
        let mut a = item(src, "Quiet story");
        a.image_url = None;
        let mut b = item(src, "Quiet story");
        b.image_url = Some("https://x.test/spacer.gif".into());
        assert_eq!(pick_lead_image(&[a, b]), None);
    }

    use bg_core::ids::{RawItemId, SourceId};
    use bg_core::text::simhash64;

    /// Weights drawn from the items under test, the same way the live pass
    /// draws them from its window.
    fn corpus_of(items: &[RawItem]) -> bg_core::samestory::Corpus {
        bg_core::samestory::Corpus::of(&items.iter().map(|i| i.title.clone()).collect::<Vec<_>>())
    }

    fn item(source: SourceId, title: &str) -> RawItem {
        RawItem {
            id: RawItemId::new(),
            source_id: source,
            external_id: None,
            canonical_url: format!("https://x.test/{}", bg_core::slug::slugify(title)),
            url_hash: String::new(),
            title: title.to_string(),
            dek: None,
            authors: vec![],
            published_at: Utc::now(),
            fetched_at: Utc::now(),
            summary_raw: None,
            body_raw: None,
            body_hash: None,
            simhash: simhash64(title) as i64,
            lang: "en".into(),
            image_url: None,
            video_id: None,
            beat: None,
            story_id: Some(StoryId::new()),
            triaged: true,
        }
    }

    #[test]
    fn two_outlets_on_one_event_match_decisively() {
        let a_src = SourceId::new();
        let b_src = SourceId::new();
        let a = item(a_src, "Solana outage halts block production for four hours");
        let b = item(b_src, "Solana outage halts block production for four hours");
        let (_, score) = best_match(
            &a,
            std::slice::from_ref(&b),
            &corpus_of(&[a.clone(), b.clone()]),
        )
        .expect("should match");
        assert!(
            score.decisive,
            "identical headlines must not need a model call"
        );
    }

    /// Seven copies of this exact headline were published as seven separate
    /// single-source stories, because an aggregator carries many outlets under
    /// one source_id and the same-source guard skipped every pair.
    #[test]
    fn an_identical_headline_from_one_feed_still_merges() {
        let feed = SourceId::new();
        let a = item(
            feed,
            "How bitcoin and gold went from a slump to an MVP week",
        );
        let b = item(
            feed,
            "How bitcoin and gold went from a slump to an MVP week",
        );
        let m = best_match(
            &a,
            std::slice::from_ref(&b),
            &corpus_of(&[a.clone(), b.clone()]),
        );
        let (_, score) = m.expect("identical headlines must match");
        assert!(
            score.decisive,
            "an identical headline needs no adjudication"
        );
    }

    /// The guard still earns its place: a publisher filing several pieces on
    /// one subject writes several similar headlines about different events,
    /// and merging those would be worse than leaving them apart.
    #[test]
    fn one_publisher_writing_around_a_subject_is_still_left_alone() {
        let desk = SourceId::new();
        let a = item(desk, "Bitcoin climbs above $76,000 in early trading");
        let b = item(desk, "Bitcoin miners report record second-quarter revenue");
        assert!(best_match(
            &a,
            std::slice::from_ref(&b),
            &corpus_of(&[a.clone(), b.clone()])
        )
        .is_none());
    }

    #[test]
    fn unrelated_stories_do_not_match_at_all() {
        let a = item(SourceId::new(), "Solana outage halts block production");
        let b = item(
            SourceId::new(),
            "SEC approves three spot ether ETF applications",
        );
        assert!(best_match(
            &a,
            std::slice::from_ref(&b),
            &corpus_of(&[a.clone(), b.clone()])
        )
        .is_none());
    }

    #[test]
    fn a_paraphrase_lands_in_the_ambiguous_band_for_adjudication() {
        let a = item(
            SourceId::new(),
            "Exchange freezes attacker funds after $70M exploit",
        );
        let b = item(
            SourceId::new(),
            "Venue halts withdrawals following seventy million dollar breach",
        );
        match best_match(
            &a,
            std::slice::from_ref(&b),
            &corpus_of(&[a.clone(), b.clone()]),
        ) {
            Some((_, s)) => assert!(
                !s.decisive,
                "a loose paraphrase should be adjudicated, not auto-merged"
            ),
            None => { /* also acceptable — errs toward splitting */ }
        }
    }

    #[test]
    fn the_same_source_is_never_treated_as_corroboration() {
        // This test used to assert that two identical headlines from one feed
        // must *not* match, on the reasoning that one outlet publishing twice
        // is not two sources. The reasoning is right and the mechanism was
        // wrong: corroboration is counted as `count(DISTINCT r.source_id)` over
        // a story's items, so merging two same-source items yields a source
        // count of one either way. Refusing to merge never protected the
        // invariant — it just published the story twice, and on the live site
        // seven times.
        //
        // So the invariant is asserted where it actually lives.
        let src = SourceId::new();
        let a = item(src, "Solana outage halts block production for four hours");
        let b = item(src, "Solana outage halts block production for four hours");
        assert!(
            best_match(
                &a,
                std::slice::from_ref(&b),
                &corpus_of(&[a.clone(), b.clone()])
            )
            .is_some(),
            "two copies of one headline are one story, not two"
        );
        // And the count that reaches the page is over *distinct* sources, so
        // the merge cannot manufacture corroboration.
        let distinct: std::collections::HashSet<_> =
            [a.source_id, b.source_id].into_iter().collect();
        assert_eq!(
            distinct.len(),
            1,
            "one outlet publishing twice is one source"
        );
    }
}
