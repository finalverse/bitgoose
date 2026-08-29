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
use bg_core::domain::{AgentRole, EditorialLanguage, ModelTier};
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

pub const SYSTEM: &str = "Gander opens a special topic after several independent
publishers converge. Write a concise native-language title and a two-or-three-
sentence standfirst explaining why the subject matters now. Each edition has
its own agenda; do not translate another edition. Hot lists and commentators
are discovery signals, not factual corroboration. Use only supplied headlines,
never invent facts, and write entirely in the requested OUTPUT_LANGUAGE.";
pub const TRACKED_SYSTEM: &str = "Maintain a deep, permanent BitGoose topic
dossier using only the supplied published stories, named source references and
primary-source list. Write a complete native-language brief covering: latest
developments with an explicit as-of time; verified facts; timeline; competing
claims; technical, market, legal or historical roots; outcomes and downstream
impact; original BitGoose analysis; and evidence boundaries. Every claim must
map to supplied evidence. Separate facts, market data and analysis, state
single-source uncertainty, and do not rewrite merely to appear fresh. Produce
4-10 concrete, falsifiable watchpoints. Never copy source articles and write
entirely in the requested OUTPUT_LANGUAGE.";
const TRACKED_BRIEF_HOURS: i64 = 3;

#[derive(Debug, Deserialize)]
pub struct Framing {
    pub title: String,
    pub standfirst: String,
}

#[derive(Debug, Deserialize)]
struct TrackedFraming {
    standfirst: String,
    analysis_md: String,
    watchpoints: Vec<String>,
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

fn tracked_schema() -> serde_json::Value {
    sch::object(
        vec![
            (
                "standfirst",
                sch::string_hinted("2-3 sentence current status and scope", "standfirst"),
            ),
            (
                "analysis_md",
                sch::string_hinted(
                    "original Markdown brief with facts, analysis, viewpoint and evidence boundary",
                    "analysis",
                ),
            ),
            (
                "watchpoints",
                sch::array(
                    sch::string_hinted("specific future event or measurable signal", "watchpoint"),
                    "3-8 concrete signals to monitor",
                ),
            ),
        ],
        &["standfirst", "analysis_md", "watchpoints"],
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
    let tracked = bg_db::gaggles::refresh_tracked(&ctx.db).await?;
    let mut refreshed = tracked;
    for language in editions() {
        refreshed += refresh_language(ctx, language).await?;
    }
    Ok(refreshed)
}

const fn editions() -> [EditorialLanguage; 7] {
    [
        EditorialLanguage::En,
        EditorialLanguage::Zh,
        EditorialLanguage::ZhHant,
        EditorialLanguage::Fr,
        EditorialLanguage::Es,
        EditorialLanguage::Ja,
        EditorialLanguage::Ko,
    ]
}

async fn refresh_language(ctx: &Ctx, language: EditorialLanguage) -> Result<usize> {
    let headlines =
        bg_db::gaggles::recent_headlines(&ctx.db, language, WINDOW_HOURS, 4_000).await?;
    if headlines.is_empty() {
        return Ok(0);
    }
    let baseline = bg_db::gaggles::baseline_headlines(
        &ctx.db,
        language,
        WINDOW_HOURS,
        (BASELINE_DAYS as i64) * 24,
        20_000,
    )
    .await?;

    let mut refreshed = 0usize;
    for heat in bg_core::trends::rank_spikes(&headlines, &baseline, BASELINE_DAYS, MIN_SOURCES) {
        if !bg_db::gaggles::exists(&ctx.db, &heat.topic, language).await? {
            continue;
        }
        let stories = bg_db::gaggles::stories_for_topic(&ctx.db, &heat.topic, language, 60).await?;
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
                editorial_language: language,
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
    let briefed = refresh_tracked_briefs(ctx, max_new.min(1)).await?;
    let mut total = briefed;
    for language in editions() {
        total += run_language(ctx, max_new, language).await?;
    }
    Ok(total)
}

async fn run_language(ctx: &Ctx, max_new: usize, language: EditorialLanguage) -> Result<usize> {
    let headlines =
        bg_db::gaggles::recent_headlines(&ctx.db, language, WINDOW_HOURS, 4_000).await?;
    if headlines.is_empty() {
        return Ok(0);
    }

    // Measured against the subject's own history, not raw volume. The first
    // gaggle this ever opened was "bitcoin, 7 sources" — true, and not a
    // special topic on a crypto site. What earns a page is departure from
    // normal.
    let baseline = bg_db::gaggles::baseline_headlines(
        &ctx.db,
        language,
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

    // Topics the Gander has already refused, still inside their backoff. One
    // query for the pass rather than one per candidate.
    let resting = bg_db::declines::resting(&ctx.db, bg_db::declines::GAGGLE_FRAMING)
        .await
        .unwrap_or_default();

    let mut opened = 0usize;
    // Every hot subject is considered; only the ones that need *writing* are
    // rationed.
    //
    // This was `hot.iter().take(max_new)` with max_new of 1, which rationed the
    // wrong thing: it capped how many subjects were looked at, not how many
    // cost a model call. The hottest subject is almost always one we already
    // have a gaggle for, so the single slot went on refreshing it — a database
    // write, no model — and the loop ended before reaching anything new. The
    // result was a Special Topics row that had not gained an entry in days
    // while the wires moved underneath it.
    //
    // Refreshing is free and unlimited; opening is bounded by `max_new`.
    for heat in hot.iter() {
        if opened >= max_new {
            break;
        }
        if resting.contains(&heat.topic) {
            // Refused recently and nothing has changed. Asking again would cost
            // the same tokens as asking the first time and get the same answer.
            continue;
        }
        // A gaggle that already exists is refreshed without spending a call on
        // re-writing prose that has not stopped being true.
        let known = bg_db::gaggles::exists(&ctx.db, &heat.topic, language).await?;
        let slug = bg_core::slug::slugify(&heat.topic);
        if !known
            && bg_db::gaggles::by_slug(&ctx.db, &slug, language)
                .await?
                .is_some()
        {
            // Two near-identical tokens can normalize to one URL, especially
            // across case and CJK punctuation. The existing reader-facing hub
            // wins; a random suffix would manufacture duplicate topics.
            info!(topic = %heat.topic, %slug, "skipping colliding topic slug");
            continue;
        }
        let stories = bg_db::gaggles::stories_for_topic(&ctx.db, &heat.topic, language, 60).await?;
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
                    slug: &slug,
                    // Ignored by the upsert's conflict branch; the existing
                    // framing stands.
                    title: &heat.topic,
                    standfirst: "-",
                    source_count: heat.sources as i32,
                    story_count: stories.len() as i32,
                    model: None,
                    editorial_language: language,
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
                "OUTPUT_LANGUAGE={}\n\nSubject: {}\nIndependent outlets covering it: {}\nStories: {}\n\nHeadlines:\n",
                language.as_str(), heat.topic, heat.sources, heat.stories
            );
            for id in stories.iter().take(25) {
                if let Ok(s) = bg_db::stories::by_id(&ctx.db, *id).await {
                    prompt.push_str(&format!("- {}\n", s.title));
                }
            }

            let req = Request::new("gander.gaggle", ModelTier::Fast, system, prompt)
                .with_schema(schema())
                // Local reasoning models may spend part of this allowance on
                // deliberation before emitting the small JSON object. Six
                // hundred truncated a live multilingual framing in production.
                .with_max_tokens(1_000);
            let (framing, completion) = ctx.llm.complete_json::<Framing>(&req).await?;

            let title = framing.title.trim();
            let standfirst = framing.standfirst.trim();
            if title.is_empty() || standfirst.is_empty() {
                return Err(FlockError::Other("gaggle framing was empty".into()));
            }
            // A small model asked to name a topic it cannot see enough of does
            // not return an error, it writes "No story" or "Hold: insufficient
            // coverage". Stored unchecked, seven of the twelve special topics
            // on the live front page were exactly that, each labelled "5
            // outlets". An empty check was not enough — the refusal is not
            // empty, it is prose about the absence of prose.
            if bg_core::share::reads_as_a_refusal(title)
                || bg_core::share::reads_as_a_refusal(standfirst)
            {
                let why = format!("framing read as a refusal: {title:?}");
                // Write it down. Without this the topic comes back next pass,
                // and the pass after — 279 refusals over a handful of subjects,
                // each one paid for at the same rate as a story.
                let _ = bg_db::declines::note(
                    &ctx.db,
                    bg_db::declines::GAGGLE_FRAMING,
                    &heat.topic,
                    &why,
                )
                .await;
                return Err(FlockError::Other(format!(
                    "the model declined to frame this topic ({title:?}); not opening a gaggle"
                )));
            }

            let id = bg_db::gaggles::upsert(
                &ctx.db,
                &bg_db::gaggles::NewGaggle {
                    topic: &heat.topic,
                    slug: &slug,
                    title,
                    standfirst,
                    source_count: heat.sources as i32,
                    story_count: stories.len() as i32,
                    model: Some(completion.model.clone()),
                    editorial_language: language,
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

/// Re-synthesise long-running topic briefs from BitGoose's published work.
/// Story membership is refreshed separately on the fast cadence; this slower
/// pass spends one Mid-tier call only when a brief is six hours old.
async fn refresh_tracked_briefs(ctx: &Ctx, max: usize) -> Result<usize> {
    if max == 0 {
        return Ok(0);
    }
    let due = bg_db::gaggles::tracked_due(&ctx.db, TRACKED_BRIEF_HOURS, max as i64).await?;
    let mut refreshed = 0usize;

    for topic in due {
        let ids = bg_db::gaggles::story_ids(&ctx.db, &topic.slug, topic.editorial_language).await?;
        // The verified seed brief remains authoritative until the newsroom has
        // actually published new reporting. A clock alone is not new evidence.
        if ids.is_empty() {
            continue;
        }

        let mut evidence = String::new();
        for id in ids.iter().take(40) {
            if let Ok(story) = bg_db::stories::by_id(&ctx.db, *id).await {
                evidence.push_str("- ");
                evidence.push_str(&story.title);
                if let Some(summary) = story.summary.as_deref() {
                    evidence.push_str(" — ");
                    evidence.push_str(summary);
                }
                if let Some(at) = story.published_at {
                    evidence.push_str(&format!(" [{}]", at.to_rfc3339()));
                }
                evidence.push('\n');
                if let Ok(refs) = bg_db::stories::source_refs(&ctx.db, *id).await {
                    for source in refs.iter().take(8) {
                        evidence.push_str("    source: ");
                        evidence.push_str(&source.name);
                        evidence.push_str(" | ");
                        evidence.push_str(&source.title);
                        evidence.push_str(" | ");
                        evidence.push_str(&source.url);
                        evidence.push('\n');
                    }
                }
            }
        }
        if evidence.is_empty() {
            continue;
        }

        let source_lines = topic
            .primary_source_names
            .iter()
            .zip(&topic.primary_source_urls)
            .map(|(name, url)| format!("- {name}: {url}"))
            .collect::<Vec<_>>()
            .join("\n");
        let language = topic.editorial_language.as_str();
        let system = format!(
            "{}\n\n{}",
            crate::system_prompt(ctx, AgentRole::Gander).await,
            TRACKED_SYSTEM
        );
        let prompt = format!(
            "OUTPUT_LANGUAGE={language}\nTopic: {}\n\nCurrent standfirst:\n{}\n\nCurrent brief:\n{}\n\nCurrent watchpoints:\n- {}\n\nPinned primary sources:\n{}\n\nBitGoose stories, newest first:\n{}",
            topic.title,
            topic.standfirst,
            topic.analysis_md,
            topic.watchpoints.join("\n- "),
            source_lines,
            evidence,
        );
        let topic_id = topic.id;

        let n = stage(
            ctx,
            AgentRole::Gander,
            None,
            "trade-watch",
            |run| async move {
                let req = Request::new("gander.topic_dossier", ModelTier::Mid, system, prompt)
                    .with_schema(tracked_schema())
                    .with_max_tokens(3_400);
                let (brief, completion) = ctx.llm.complete_json::<TrackedFraming>(&req).await?;
                let standfirst = brief.standfirst.trim();
                let analysis = brief.analysis_md.trim();
                let watchpoints: Vec<String> = brief
                    .watchpoints
                    .into_iter()
                    .map(|w| w.trim().to_string())
                    .filter(|w| !w.is_empty())
                    .collect();
                if standfirst.len() < 80 || analysis.len() < 400 || watchpoints.len() < 3 {
                    return Err(FlockError::Other(
                        "tracked-topic brief was too thin; keeping the verified prior brief".into(),
                    ));
                }
                if bg_core::share::reads_as_a_refusal(standfirst)
                    || bg_core::share::reads_as_a_refusal(analysis)
                {
                    return Err(FlockError::Other(
                        "tracked-topic model declined; keeping the verified prior brief".into(),
                    ));
                }
                bg_db::gaggles::update_tracked_brief(
                    &ctx.db,
                    topic_id,
                    standfirst,
                    analysis,
                    &watchpoints,
                    &completion.model,
                    run,
                )
                .await?;
                Ok(StageOutput::with(
                    1usize,
                    completion,
                    "updated permanent trade-watch brief",
                ))
            },
        )
        .await?;
        refreshed += n;
    }
    Ok(refreshed)
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
