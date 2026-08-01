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
            ("same_event", sch::boolean("true only if the same underlying occurrence")),
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
    let mut attached = 0usize;

    for item in pending {
        // Items Gosling judged not-news get a story so they leave the queue,
        // but with a score that keeps them off every surface.
        let candidates = bg_db::items::clustering_candidates(&ctx.db, WINDOW_HOURS, 300).await?;
        let best = best_match(&item, &candidates);

        let target: Option<StoryId> = match best {
            Some((cand, score)) if score.decisive => cand.story_id,
            Some((cand, score)) => {
                // Ambiguous: ask.
                let cand_title = cand.title.clone();
                let item_title = item.title.clone();
                let system = system.clone();
                let same = stage(ctx, AgentRole::Curator, cand.story_id, "adjudicate", |_run| async move {
                    let prompt = format!(
                        "Item A: {item_title}\nItem B: {cand_title}\n\n\
                         Do A and B report the same underlying event?"
                    );
                    let req = Request::new("curator.same_event", ModelTier::Fast, system, prompt)
                        .with_schema(schema())
                        .with_max_tokens(500);
                    let (parsed, completion) = ctx.llm.complete_json::<SameEvent>(&req).await?;
                    let note = format!(
                        "same_event={} (simhash {}, trigram {:.2})",
                        parsed.same_event, score.hamming, score.trigram
                    );
                    Ok(StageOutput::with(parsed.same_event, completion, note))
                })
                .await
                // A failed adjudication must not merge by default — a split is
                // the safe failure.
                .unwrap_or(false);
                if same {
                    cand.story_id
                } else {
                    None
                }
            }
            None => None,
        };

        let story_id = match target {
            Some(id) => {
                bg_db::items::attach_to_story(&ctx.db, item.id, id, ItemRole::Corroborating).await?;
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
                )
                .await?;
                bg_db::items::attach_to_story(&ctx.db, item.id, story.id, ItemRole::Seed).await?;
                story.id
            }
        };

        rescore(ctx, story_id).await?;
        attached += 1;
    }

    info!(attached, "curator pass complete");
    Ok(attached)
}

struct MatchScore {
    hamming: u32,
    trigram: f32,
    /// True when the lexical signals settle it with no model call.
    decisive: bool,
}

/// Best lexical match among candidates, if any is worth considering.
fn best_match<'a>(item: &RawItem, candidates: &'a [RawItem]) -> Option<(&'a RawItem, MatchScore)> {
    let mut best: Option<(&RawItem, MatchScore)> = None;

    for c in candidates {
        if c.id == item.id || c.story_id.is_none() {
            continue;
        }
        // Two reports of one event come from different outlets. Same source
        // twice is far more likely two genuinely different stories.
        if c.source_id == item.source_id {
            continue;
        }

        let h = hamming(item.simhash as u64, c.simhash as u64);
        let t = trigram_similarity(&item.title, &c.title);
        if h > SIMHASH_FAR && t < TRIGRAM_FLOOR {
            continue;
        }

        // Both signals agreeing is what makes it decisive; either alone is the
        // ambiguous band the model adjudicates.
        let decisive = h <= SIMHASH_SAME && t >= TRIGRAM_SAME;
        let score = MatchScore { hamming: h, trigram: t, decisive };

        let better = match &best {
            None => true,
            Some((_, b)) => score.trigram > b.trigram,
        };
        if better {
            best = Some((c, score));
        }
    }
    best
}

async fn item_category(ctx: &Ctx, item: &RawItem) -> Category {
    let raw: Option<String> = sqlx::query_scalar("SELECT category FROM raw_items WHERE id = $1")
        .bind(item.id.into_uuid())
        .fetch_optional(&ctx.db.pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|c| Category::from_str(&c).ok()).unwrap_or(Category::Markets)
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
    let first = items.iter().map(|i| i.published_at).min().unwrap_or_else(Utc::now);
    let hours = ((Utc::now() - first).num_minutes() as f32 / 60.0).max(0.25);
    let velocity = sources / hours;
    let velocity_bonus = (velocity * 4.0).min(12.0);

    let score = (peak_triage + corroboration + trust_adj + velocity_bonus).clamp(0.0, 100.0) as i16;
    bg_db::stories::set_scores(&ctx.db, story, score, velocity).await?;
    Ok(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bg_core::ids::{RawItemId, SourceId};
    use bg_core::text::simhash64;

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
        let (_, score) = best_match(&a, std::slice::from_ref(&b)).expect("should match");
        assert!(score.decisive, "identical headlines must not need a model call");
    }

    #[test]
    fn unrelated_stories_do_not_match_at_all() {
        let a = item(SourceId::new(), "Solana outage halts block production");
        let b = item(SourceId::new(), "SEC approves three spot ether ETF applications");
        assert!(best_match(&a, std::slice::from_ref(&b)).is_none());
    }

    #[test]
    fn a_paraphrase_lands_in_the_ambiguous_band_for_adjudication() {
        let a = item(SourceId::new(), "Exchange freezes attacker funds after $70M exploit");
        let b = item(SourceId::new(), "Venue halts withdrawals following seventy million dollar breach");
        match best_match(&a, std::slice::from_ref(&b)) {
            Some((_, s)) => assert!(!s.decisive, "a loose paraphrase should be adjudicated, not auto-merged"),
            None => { /* also acceptable — errs toward splitting */ }
        }
    }

    #[test]
    fn the_same_source_is_never_treated_as_corroboration() {
        let src = SourceId::new();
        let a = item(src, "Solana outage halts block production for four hours");
        let b = item(src, "Solana outage halts block production for four hours");
        assert!(
            best_match(&a, std::slice::from_ref(&b)).is_none(),
            "one outlet publishing twice is not two sources"
        );
    }
}
