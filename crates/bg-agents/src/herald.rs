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

    crate::gander::publish_wire(ctx, story, &summary).await
}
