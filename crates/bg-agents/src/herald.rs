//! **Herald** — the Wire, and everything downstream of publication.
//!
//! Most of what crosses the feeds is real but not worth an original story. The
//! Wire is where those go: our own two-or-three-sentence summary, the source's
//! name, and a link out. It is the honest treatment of someone else's
//! reporting — we tell you it happened and send you to the people who did the
//! work.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier};
use bg_core::ids::StoryId;
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use tracing::{info, warn};

pub const SYSTEM: &str = "Herald writes Wire summaries.

Two or three sentences, in your own words, telling a reader what happened and \
why it matters. This is a pointer to someone else's reporting, not a substitute \
for it — the reader should be able to decide from your summary whether to click \
through.

Never copy a sentence from the source. Never pad. If the item does not support \
two sentences of substance, write one.

No hedging phrases ('reportedly', 'it seems'), no editorialising, no adjectives \
that are not in the underlying facts.";

#[derive(Debug, Deserialize)]
struct WireCopy {
    summary: String,
}

fn schema() -> serde_json::Value {
    sch::object(
        vec![("summary", sch::string_hinted("2-3 sentences", "summary"))],
        &["summary"],
    )
}

/// Summarize a story for the Wire and publish it.
pub async fn run(ctx: &Ctx, story: StoryId) -> Result<crate::gander::Outcome> {
    let items = bg_db::items::by_story(&ctx.db, story).await?;
    let s = bg_db::stories::by_id(&ctx.db, story).await?;
    let system = crate::system_prompt(ctx, AgentRole::Herald).await;
    // Kept out here because `s` moves into the stage closure below.
    let headline = s.title.clone();

    // Never ask a model to summarise nothing.
    //
    // This is not a tuning knob, it is a correctness gate. YouTube items were
    // arriving with no text at all, so Herald was handed a bare title and told
    // to produce two or three sentences — and a small model with nothing to
    // work from does not decline, it invents. Thirty stories were published
    // asserting silver-coin schedules, IBM hiring policy and a Bitcoin price,
    // none of which appeared in any source. On a site whose disclosure line
    // reads "every claim links to its sources", that is the worst failure it
    // can have.
    //
    // The threshold is on *source* text, not output: a summary can only be
    // grounded in what was actually read.
    const MIN_SOURCE_CHARS: usize = 120;
    let available: usize = items
        .iter()
        .filter_map(|it| it.summary_raw.as_deref().or(it.body_raw.as_deref()))
        .map(|t| t.trim().len())
        .sum();
    if available < MIN_SOURCE_CHARS {
        warn!(
            story = %story, available,
            "not enough source text to summarise; publishing the pointer without one"
        );
        return crate::gander::publish_wire(ctx, story, "").await;
    }

    // Spend the writing budget where it is read.
    //
    // Measured over a day: Herald was 30% of the newsroom's entire token spend
    // — 79 calls at about 920 tokens — while the daily allowance was running at
    // 123% of its cap and 78% of ingested items were never triaged at all. Half
    // of that was going on the routine end of the Wire, where a card is already
    // complete without it: headline, outlet, link.
    //
    // The floor sits just above the median (63 over the last week), so roughly
    // half of Wire items publish as a pointer and the better half still get
    // written prose. Nothing is *lost* below the line: the card renders, and a
    // share of it falls back to `bg_core::share::coverage_line`, which states
    // who reported it from the record rather than from a model.
    if s.newsworthiness < ctx.cfg.wire_summary_floor {
        info!(
            story = %story, newsworthiness = s.newsworthiness,
            floor = ctx.cfg.wire_summary_floor,
            "below the summary floor; publishing the pointer"
        );
        return crate::gander::publish_wire(ctx, story, "").await;
    }

    let summary = stage(
        ctx,
        AgentRole::Herald,
        Some(story),
        "wire",
        |_run| async move {
            let mut prompt = format!("Headline: {}\n\nSource material:\n", s.title);
            for it in items.iter().take(3) {
                if let Some(b) = it.summary_raw.as_deref().or(it.body_raw.as_deref()) {
                    prompt.push_str(&format!("- {}\n", bg_core::text::truncate_words(b, 120)));
                }
            }
            prompt.push_str("\nWrite the Wire summary.");

            let req = Request::new("herald.wire", ModelTier::Fast, system, prompt)
                .with_schema(schema())
                .with_max_tokens(800);
            let (c, completion) = ctx.llm.complete_json::<WireCopy>(&req).await?;
            let note = format!("{} words", bg_core::text::word_count(&c.summary));
            Ok(StageOutput::with(
                c.summary.trim().to_string(),
                completion,
                note,
            ))
        },
    )
    .await?;

    // A Wire card is headline + source + link-out; the summary is the only part
    // that has to earn its place. One that restates the headline occupies the
    // slot where the reader expects something new, so it is dropped rather than
    // printed — the card renders cleanly without it.
    //
    // This matters most with the offline stub, whose summaries are restatements
    // by construction, but the rule is deliberately not conditional on the
    // provider: a live model producing a lazy dek gets the same treatment.
    let summary = if bg_core::text::dek_adds_nothing(&headline, &summary) {
        info!(story = %story, "wire summary restated the headline; publishing without one");
        String::new()
    } else {
        summary
    };

    crate::gander::publish_wire(ctx, story, &summary).await
}
