//! Opening a special topic when coverage converges.
//!
//! A skein is geese in flight; a gaggle is geese on the ground. The Skein reads
//! where one story is heading — this is what happens when many stories turn out
//! to be about the same thing.
//!
//! **Detection costs nothing.** Which subjects independent outlets are
//! converging on falls out of headline arithmetic ([`bg_core::trends`]), and on
//! a tier that allows 200,000 tokens a day, spending any of them to notice that
//! seven newsrooms wrote about the same bill would be indefensible.
//!
//! A model is consulted for one thing only: naming and framing the topic once
//! it has already been established as hot. That is a writing job, and it is a
//! single small call per topic — not per story, and not per pass, since a
//! gaggle that stays hot is refreshed in place rather than re-written.
//!
//! The threshold is deliberately high. A special topic that appears whenever
//! three outlets mention a company is not special, and a site covered in
//! auto-generated topic hubs is the thing this must not become.

use crate::{stage, Ctx, FlockError, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier};
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use tracing::info;

/// Independent outlets required before a subject earns a page of its own.
///
/// Five, against a roster of twenty-five polled sources — a fifth of the
/// newsroom arriving at the same subject independently. Measured over 1,235
/// live headlines this admitted four topics in 48 hours (the Clarity Act at
/// seven sources, Trump, Bitcoin, OpenAI) and rejected everything else, which
/// is about the rate a front page can carry.
pub const MIN_SOURCES: usize = 5;

/// How far back to look for convergence.
pub const WINDOW_HOURS: i64 = 48;

/// The history a subject is measured against.
///
/// Two weeks. Long enough that a beat's regulars — Bitcoin, OpenAI, the Fed —
/// establish a baseline, short enough that a subject genuinely new a month ago
/// still reads as new.
pub const BASELINE_DAYS: f32 = 14.0;

pub const SYSTEM: &str = "Gander is opening a special topic — a page collecting \
everything the newsroom has on one subject that many independent outlets are \
covering at once.

You are given the subject, how many separate outlets have covered it, and the \
headlines. Write two things.

TITLE. What this topic is, as a reader would name it. Four to eight words. Not \
a headline about one event — the subject the events share. 'The Clarity Act \
fight', not 'Senate advances Clarity Act'. No colons, no clickbait.

STANDFIRST. Two or three sentences on why this is a topic right now: what is \
actually at stake and why so many newsrooms turned to it at once. Concrete. If \
the honest answer is that a lot of outlets covered a routine announcement, say \
that plainly rather than inflating it — a topic page that oversells itself is \
worse than no topic page.

Write from the headlines you are given and nothing else. Do not assert facts \
they do not contain.";

#[derive(Debug, Deserialize)]
pub struct Framing {
    pub title: String,
    pub standfirst: String,
}

fn schema() -> serde_json::Value {
    sch::object(
        vec![
            (
                "title",
                sch::string_hinted("4-8 words naming the subject", "topic"),
            ),
            (
                "standfirst",
                sch::string_hinted("2-3 sentences: why this, why now", "standfirst"),
            ),
        ],
        &["title", "standfirst"],
    )
}

/// Re-score heat and refresh existing gaggles, without consulting a model.
///
/// The half of the job that is free. Called on the fast cadence so a live topic
/// page reflects the last few minutes rather than the last full pipeline pass,
/// which on a constrained tier can be an hour apart.
///
/// Opens nothing new — naming a topic costs a call, and that decision belongs
/// in the budgeted pass.
pub async fn refresh(ctx: &Ctx) -> Result<usize> {
    let headlines = bg_db::gaggles::recent_headlines(&ctx.db, WINDOW_HOURS, 4_000).await?;
    if headlines.is_empty() {
        return Ok(0);
    }
    let baseline = bg_db::gaggles::baseline_headlines(
        &ctx.db,
        WINDOW_HOURS,
        (BASELINE_DAYS as i64) * 24,
        20_000,
    )
    .await?;

    let mut refreshed = 0usize;
    for heat in bg_core::trends::rank_spikes(&headlines, &baseline, BASELINE_DAYS, MIN_SOURCES) {
        if !bg_db::gaggles::exists(&ctx.db, &heat.topic).await? {
            continue;
        }
        let stories = bg_db::gaggles::stories_for_topic(&ctx.db, &heat.topic, 60).await?;
        let id = bg_db::gaggles::upsert(
            &ctx.db,
            &bg_db::gaggles::NewGaggle {
                topic: &heat.topic,
                slug: &bg_core::slug::slugify(&heat.topic),
                // The conflict branch keeps the existing framing.
                title: &heat.topic,
                standfirst: "-",
                source_count: heat.sources as i32,
                story_count: stories.len() as i32,
                model: None,
            },
            None,
        )
        .await?;
        bg_db::gaggles::set_stories(&ctx.db, id, &stories).await?;
        refreshed += 1;
    }
    Ok(refreshed)
}

/// Detect convergence and open a gaggle for anything that clears the bar.
///
/// Returns how many were opened or refreshed.
pub async fn run(ctx: &Ctx, max_new: usize) -> Result<usize> {
    let headlines = bg_db::gaggles::recent_headlines(&ctx.db, WINDOW_HOURS, 4_000).await?;
    if headlines.is_empty() {
        return Ok(0);
    }

    // Measured against the subject's own history, not raw volume. The first
    // gaggle this ever opened was "bitcoin, 7 sources" — true, and not a
    // special topic on a crypto site. What earns a page is departure from
    // normal.
    let baseline = bg_db::gaggles::baseline_headlines(
        &ctx.db,
        WINDOW_HOURS,
        (BASELINE_DAYS as i64) * 24,
        20_000,
    )
    .await?;
    let hot = bg_core::trends::rank_spikes(&headlines, &baseline, BASELINE_DAYS, MIN_SOURCES);
    if hot.is_empty() {
        info!(
            headlines = headlines.len(),
            "nothing converged past the threshold"
        );
        return Ok(0);
    }

    let mut opened = 0usize;
    for heat in hot.iter().take(max_new) {
        // A gaggle that already exists is refreshed without spending a call on
        // re-writing prose that has not stopped being true.
        let known = bg_db::gaggles::exists(&ctx.db, &heat.topic).await?;
        let stories = bg_db::gaggles::stories_for_topic(&ctx.db, &heat.topic, 60).await?;
        if stories.is_empty() {
            // Hot across the wires but nothing published yet. It will still be
            // hot next pass, by which point the pipeline may have caught up.
            continue;
        }

        if known {
            let id = bg_db::gaggles::upsert(
                &ctx.db,
                &bg_db::gaggles::NewGaggle {
                    topic: &heat.topic,
                    slug: &bg_core::slug::slugify(&heat.topic),
                    // Ignored by the upsert's conflict branch; the existing
                    // framing stands.
                    title: &heat.topic,
                    standfirst: "-",
                    source_count: heat.sources as i32,
                    story_count: stories.len() as i32,
                    model: None,
                },
                None,
            )
            .await?;
            bg_db::gaggles::set_stories(&ctx.db, id, &stories).await?;
            continue;
        }

        let heat = heat.clone();
        let stories = stories.clone();
        let system = crate::system_prompt(ctx, AgentRole::Gander).await;

        let n = stage(ctx, AgentRole::Gander, None, "gaggle", |run| async move {
            let mut prompt = format!(
                "Subject: {}\nIndependent outlets covering it: {}\nStories: {}\n\nHeadlines:\n",
                heat.topic, heat.sources, heat.stories
            );
            for id in stories.iter().take(25) {
                if let Ok(s) = bg_db::stories::by_id(&ctx.db, *id).await {
                    prompt.push_str(&format!("- {}\n", s.title));
                }
            }

            let req = Request::new("gander.gaggle", ModelTier::Fast, system, prompt)
                .with_schema(schema())
                .with_max_tokens(600);
            let (framing, completion) = ctx.llm.complete_json::<Framing>(&req).await?;

            let title = framing.title.trim();
            let standfirst = framing.standfirst.trim();
            if title.is_empty() || standfirst.is_empty() {
                return Err(FlockError::Other("gaggle framing was empty".into()));
            }

            let id = bg_db::gaggles::upsert(
                &ctx.db,
                &bg_db::gaggles::NewGaggle {
                    topic: &heat.topic,
                    slug: &bg_core::slug::slugify(&heat.topic),
                    title,
                    standfirst,
                    source_count: heat.sources as i32,
                    story_count: stories.len() as i32,
                    model: Some(completion.model.clone()),
                },
                Some(run),
            )
            .await?;
            bg_db::gaggles::set_stories(&ctx.db, id, &stories).await?;

            let note = format!("{} ({} sources)", title, heat.sources);
            info!(topic = %heat.topic, sources = heat.sources, "gaggle opened");
            Ok(StageOutput::with(1usize, completion, note))
        })
        .await?;
        opened += n;
    }
    Ok(opened)
}

/// Twenty-five sources are polled. A threshold low enough for three outlets
/// would open a topic page most days for most companies, and a site of
/// auto-generated hubs is the failure this guards against. Checked at compile
/// time so lowering it is a deliberate act, not a tuning accident.
const _: () = assert!(
    MIN_SOURCES >= 5,
    "threshold low enough to make every company a special topic"
);

/// A story breaking on Friday should still be able to gather a gaggle by
/// Sunday, when fewer outlets publish.
const _: () = assert!(WINDOW_HOURS >= 48);
