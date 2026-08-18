//! The agent that looks after the newsroom itself.
//!
//! Every other agent works on the news. This one works on the machine that
//! makes it: it reads the health signals, decides which are actionable, fixes
//! what it can safely fix, and writes down the rest.
//!
//! ## Why it exists
//!
//! Every fault found in this codebase over the last week was found by a person
//! running `bg doctor`, reading a number, and thinking about it. Markets and
//! Tech published nothing for eight and thirteen days. 1,407 of 1,438 stories
//! carried a single source. 359 items held text their publishers had asked not
//! be used that way. None of those were subtle — each was a number sitting in a
//! health check waiting to be noticed, and nothing was doing the noticing.
//!
//! A newsroom that runs itself has to include the running.
//!
//! ## Deliberately without a model
//!
//! Not one call. Three reasons, in order of weight:
//!
//! 1. **It has to work when nothing else does.** The condition most worth
//!    catching is the newsroom being broken, and "the inference provider is
//!    refusing us" is one of the ways it breaks. A health check that needs the
//!    provider is offline exactly when it matters.
//! 2. **There is no budget for it.** Measured, the newsroom runs at 123% of its
//!    daily token cap. Spending any of it on watching itself would come out of
//!    reporting.
//! 3. **These questions are arithmetic.** "Has this desk published today", "is
//!    the queue growing", "are we holding text we said we would not" — a model
//!    would add latency, cost and a chance of being wrong, and subtract
//!    nothing.
//!
//! ## What it may and may not do
//!
//! It acts only where the action is **reversible and its own**: reconciling a
//! denormalised counter, erasing text a publisher declined, disabling a source
//! that has failed repeatedly. It never publishes, never retracts a story,
//! never edits copy, and never touches code.
//!
//! Everything else it writes down. A finding it cannot act on is reported, not
//! silently swallowed — an agent that quietly decides a problem is unfixable is
//! worse than no agent, because it also stops anyone else looking.

use crate::{Ctx, Result};
use tracing::{info, warn};

/// How long a source must be both failing and silent before it is rested.
///
/// Long, because the most common cause of a failed poll here is not the
/// publisher: the host's uplink drops a large share of its packets, and eleven
/// of fifteen polls failing at once has meant the network, not eleven dead
/// feeds. Disabling healthy sources because the wire was bad would be the agent
/// causing the outage it exists to catch.
///
/// There is no failure counter in the schema, and this is better than one
/// anyway: it asks whether the source is *producing*, not whether the last
/// request happened to fail.
const BARREN_HOURS: i64 = 72;

/// A desk silent for longer than this has something wrong with it.
///
/// Markets went eight days and Tech thirteen before anyone noticed. Two days is
/// long enough to survive a quiet weekend on a slow desk and short enough that
/// nobody has to spot it by eye.
const DESK_SILENT_HOURS: i64 = 48;

/// Above this share of single-source stories, clustering has stopped working.
const SINGLE_SOURCE_ALARM: f64 = 92.0;

// The thresholds encode judgements that were expensive to learn, so drifting
// past them should stop the build rather than quietly change behaviour. Written
// as compile-time assertions because that is what they are — a runtime test of
// a constant tests nothing, which clippy says more briefly.
const _: () = {
    // Eleven of fifteen polls failed at once on a bad uplink. A source must be
    // silent for days, not minutes, before the Steward rests it.
    assert!(BARREN_HOURS >= 48);
    // Markets went eight days unnoticed; two is the most that should pass.
    assert!(DESK_SILENT_HOURS <= 48);
};

/// What the Steward found, and what it did about it.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Short stable key, for grouping in logs.
    pub kind: &'static str,
    /// What is wrong, in a sentence a person can act on.
    pub detail: String,
    /// What was done, or `None` when it needs a decision or a code change.
    pub action: Option<String>,
}

impl Finding {
    fn noted(kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            action: None,
        }
    }

    fn fixed(kind: &'static str, detail: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            action: Some(action.into()),
        }
    }

    /// Whether this needs a person: a decision, or a change to the code.
    pub fn needs_a_human(&self) -> bool {
        self.action.is_none()
    }
}

/// Run a full round: check everything, fix what is safe, report the rest.
///
/// `apply` false makes it read-only, which is how it should be run the first
/// time against any database it has not seen.
pub async fn run(ctx: &Ctx, apply: bool) -> Result<Vec<Finding>> {
    let mut out = Vec::new();
    out.extend(check_declined_text(ctx, apply).await);
    out.extend(check_source_counts(ctx, apply).await);
    out.extend(check_failing_sources(ctx, apply).await);
    out.extend(check_silent_desks(ctx).await);
    out.extend(check_corroboration(ctx).await);
    out.extend(check_queue(ctx).await);
    out.extend(check_junk_topics(ctx, apply).await);
    out.extend(check_call_failures(ctx).await);
    out.extend(backfill_images(ctx, apply).await);
    out.extend(check_delivery(ctx).await);

    let (fixed, noted) = out.iter().partition::<Vec<_>, _>(|f| f.action.is_some());
    info!(
        fixed = fixed.len(),
        needs_a_human = noted.len(),
        applied = apply,
        "steward round complete"
    );
    for f in &out {
        match &f.action {
            Some(a) => info!(kind = f.kind, detail = %f.detail, action = %a, "steward acted"),
            None => {
                warn!(kind = f.kind, detail = %f.detail, "steward found something it cannot fix")
            }
        }
    }
    Ok(out)
}

/// Text held from publishers who decline model input.
///
/// Safe to act on without asking: erasing it is what the publisher asked for,
/// it removes only a private working copy, and the story keeps its headline,
/// link and citation.
async fn check_declined_text(ctx: &Ctx, apply: bool) -> Vec<Finding> {
    let held = match bg_db::items::declined_text_held(&ctx.db).await {
        Ok(0) => return Vec::new(),
        Ok(n) => n,
        Err(e) => {
            return vec![Finding::noted(
                "declined-text",
                format!("cannot check: {e}"),
            )]
        }
    };
    let detail =
        format!("holding extracted text from {held} items whose publisher declines model input");
    if !apply {
        return vec![Finding::noted("declined-text", detail)];
    }
    match bg_db::items::purge_declined_text(&ctx.db).await {
        Ok(n) => vec![Finding::fixed(
            "declined-text",
            detail,
            format!("erased {n}"),
        )],
        Err(e) => vec![Finding::noted(
            "declined-text",
            format!("{detail}; purge failed: {e}"),
        )],
    }
}

/// `source_count` drifting from the evidence.
///
/// A page listing three outlets while claiming one source is worse than not
/// folding at all, and the column is denormalised so it can drift.
async fn check_source_counts(ctx: &Ctx, apply: bool) -> Vec<Finding> {
    if !apply {
        return Vec::new();
    }
    match bg_db::stories::reconcile_source_counts(&ctx.db).await {
        Ok(0) => Vec::new(),
        Ok(n) => vec![Finding::fixed(
            "source-count",
            format!("{n} stories disagreed with their own evidence"),
            format!("recomputed {n}"),
        )],
        Err(e) => vec![Finding::noted(
            "source-count",
            format!("cannot reconcile: {e}"),
        )],
    }
}

/// Sources failing every poll.
///
/// Rested rather than deleted, and only after many consecutive failures — on
/// this host a failed poll usually means the uplink, not the publisher.
async fn check_failing_sources(ctx: &Ctx, apply: bool) -> Vec<Finding> {
    let sick = match bg_db::sources::failing_and_barren(&ctx.db, BARREN_HOURS).await {
        Ok(v) => v,
        Err(e) => {
            return vec![Finding::noted(
                "source-health",
                format!("cannot check: {e}"),
            )]
        }
    };
    let mut out = Vec::new();
    for (_id, slug, quiet, err) in sick {
        let detail = format!("{slug} is failing and has produced nothing for {quiet}h: {err}");
        if !apply {
            out.push(Finding::noted("source-health", detail));
            continue;
        }
        match bg_db::sources::set_enabled(&ctx.db, &slug, false).await {
            Ok(()) => out.push(Finding::fixed(
                "source-health",
                detail,
                // `bg seed` will not undo this: the upsert leaves `enabled`
                // alone on purpose, so an operator's decision survives a
                // redeploy. Pointing at a command that does nothing would
                // strand the source and confuse whoever tried.
                format!("rested {slug}; `bg source {slug} --wake` puts it back"),
            )),
            Err(e) => out.push(Finding::noted("source-health", format!("{detail}; {e}"))),
        }
    }
    out
}

/// A desk in the navigation that has stopped publishing.
///
/// Never acted on automatically. The cause is always upstream — no sources, a
/// dead feed, a classifier sending everything elsewhere — and each has a
/// different answer. Guessing between them is how an agent makes things worse.
async fn check_silent_desks(ctx: &Ctx) -> Vec<Finding> {
    let quiet = match bg_db::stories::silent_desks(&ctx.db, DESK_SILENT_HOURS).await {
        Ok(v) => v,
        Err(e) => return vec![Finding::noted("silent-desk", format!("cannot check: {e}"))],
    };
    quiet
        .into_iter()
        .map(|(beat, hours)| {
            Finding::noted(
                "silent-desk",
                match hours {
                    Some(h) => format!("the {beat} desk has not published for {h} hours"),
                    None => format!("the {beat} desk has never published"),
                },
            )
        })
        .collect()
}

/// Corroboration collapsing.
async fn check_corroboration(ctx: &Ctx) -> Vec<Finding> {
    let (alone, total) = match bg_db::stories::corroboration_health(&ctx.db, 14).await {
        Ok(v) => v,
        Err(e) => {
            return vec![Finding::noted(
                "corroboration",
                format!("cannot check: {e}"),
            )]
        }
    };
    if total == 0 {
        return Vec::new();
    }
    let pct = (alone as f64 / total as f64) * 100.0;
    if pct < SINGLE_SOURCE_ALARM {
        return Vec::new();
    }
    // Not acted on: `bg recluster --apply` folds published stories together and
    // retires URLs. Reversible, but it is an editorial act, and an agent should
    // propose it rather than perform it.
    vec![Finding::noted(
        "corroboration",
        format!(
            "{alone} of {total} stories in 14d have a single source ({pct:.0}%); \
             consider `bg recluster --hours 336`"
        ),
    )]
}

/// Intake outrunning what the budget can process.
async fn check_queue(ctx: &Ctx) -> Vec<Finding> {
    let (waiting, lapsed) = match bg_db::items::queue_health(&ctx.db).await {
        Ok(v) => v,
        Err(e) => return vec![Finding::noted("queue", format!("cannot check: {e}"))],
    };
    // A queue is only a problem when it is growing faster than it drains. The
    // signal that it is: items ageing out of the news horizon unread, which is
    // intake being discarded rather than deferred.
    if lapsed < waiting.max(500) {
        return Vec::new();
    }
    vec![Finding::noted(
        "queue",
        format!(
            "{waiting} items waiting and {lapsed} already aged out unread — \
             intake exceeds what the token budget can triage"
        ),
    )]
}

/// Special topics whose framing is a model refusal rather than a topic.
///
/// Safe to remove without asking: a gaggle is BitGoose's own furniture, not
/// reporting, and one titled "No story" over "5 outlets" is worse than an empty
/// strip. The cause is fixed at the point of creation; this clears what already
/// shipped, and keeps clearing if a new shape of refusal gets past the guard.
async fn check_junk_topics(ctx: &Ctx, apply: bool) -> Vec<Finding> {
    let all = match bg_db::gaggles::all_titles(&ctx.db).await {
        Ok(v) => v,
        Err(e) => return vec![Finding::noted("junk-topic", format!("cannot check: {e}"))],
    };
    let junk: Vec<_> = all
        .into_iter()
        .filter(|(_, title, _)| bg_core::share::reads_as_a_refusal(title))
        .collect();
    if junk.is_empty() {
        return Vec::new();
    }
    let names: Vec<&str> = junk.iter().map(|(_, t, _)| t.as_str()).collect();
    let detail = format!(
        "{} special topics are model refusals, not topics: {}",
        junk.len(),
        names.join(", ")
    );
    if !apply {
        return vec![Finding::noted("junk-topic", detail)];
    }
    let mut gone = 0usize;
    for (id, _, _) in &junk {
        if bg_db::gaggles::delete(&ctx.db, *id).await.is_ok() {
            gone += 1;
        }
    }
    vec![Finding::fixed(
        "junk-topic",
        detail,
        format!("removed {gone}"),
    )]
}

/// Copies of publisher images for stories published before we took them.
///
/// Mirroring happens at publish now, which does nothing for the archive — and
/// a shared link to any older story therefore shows the card we drew rather
/// than the photograph, permanently, because preview clients cache per URL.
///
/// Bounded hard per round. This is the only Steward action that touches the
/// network, on a host whose uplink is the reason half the other findings exist;
/// a few each pass drains the backlog over days without competing with the
/// newsroom's own polling for the wire.
async fn backfill_images(ctx: &Ctx, apply: bool) -> Vec<Finding> {
    // Twenty-five was chosen when this was the only network check and the
    // round was not bounded. On a 7 KB/s link, twenty-five fetches plus the
    // delivery probe outlasted the pass interval and stalled the newsroom for
    // seventeen minutes. Eight drains the backlog over a few days and leaves
    // room inside the round's timeout for everything else.
    const PER_ROUND: i64 = 8;

    let candidates = match bg_db::stories::awaiting_image_mirror(&ctx.db, 400).await {
        Ok(v) => v,
        Err(e) => return vec![Finding::noted("image-mirror", format!("cannot check: {e}"))],
    };
    let missing: Vec<_> = candidates
        .into_iter()
        // `held`, not `mirrored`: the latter is size-gated, so a picture we
        // fetched and could not compress would read as missing and be fetched
        // again every round for ever.
        .filter(|(slug, _)| !bg_ingest::mirror::held(slug))
        .collect();
    if missing.is_empty() {
        return Vec::new();
    }
    let detail = format!(
        "{} published stories have a publisher photograph we have not copied; \
         shares of those show our drawn card instead",
        missing.len()
    );
    if !apply {
        return vec![Finding::noted("image-mirror", detail)];
    }

    let mut got = 0usize;
    for (slug, url) in missing.iter().take(PER_ROUND as usize) {
        let Some(url) = bg_core::media::as_image(url) else {
            continue;
        };
        if bg_ingest::mirror::store_lead_image(&ctx.http, slug, &url).await {
            got += 1;
        }
    }
    vec![Finding::fixed(
        "image-mirror",
        detail,
        format!("copied {got} this round"),
    )]
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// What a share of a story actually weighs, fetched over the public URL.
///
/// **The check that was missing.** Six versions of the share-card work shipped
/// with the code doing exactly what it said and the outcome still wrong: a
/// photograph that reached `og:image` and took 28 seconds to fetch; a 60 KB
/// target set without measuring the 7 KB/s link; a 120 KB file stored under a
/// 60 KB target; two encoder passes that compressed nothing over 150 KB. Every
/// one was found by fetching the artefact and weighing it, and none by reading
/// the code or by any test written beforehand.
///
/// Nothing inside the database can catch that class. A row cannot tell you a
/// file is too heavy for the wire it has to cross. So this one goes and looks.
///
/// It measures rather than judges the network: the host cannot see its own
/// uplink the way a reader in another country does, and pretending otherwise
/// would be the same mistake again. What it *can* state exactly is how many
/// bytes a crawler is asked to take, which is the half that is ours.
/// Agents whose calls to the provider are mostly failing.
///
/// This is the check that should have existed first, and its absence cost
/// weeks. Every symptom was visible from outside — desks that never published,
/// a corroboration rate stuck at 98% single-source, a queue growing faster than
/// it drained — and every diagnosis was wrong, because the pass reported what
/// it *completed* and never what it had been refused. Meanwhile:
///
/// * 1,241 of 1,503 Skein calls returned `json_validate_failed`,
/// * every Scribe call in the table returned HTTP 413,
/// * the Gander declined 279 of 290 framings,
///
/// and the log line at the end of each pass said `analysed 0`, which reads like
/// an idle newsroom rather than a failing one.
///
/// Reported rather than fixed: a failure rate says something is wrong, not what
/// to change, and the answers here were three different things — a model
/// parameter, a reservation that could not fit, and a question worth asking
/// less often.
async fn check_call_failures(ctx: &Ctx) -> Vec<Finding> {
    let rows = match bg_db::agents::failure_rates(&ctx.db, FAILURE_WINDOW_HOURS).await {
        Ok(r) => r,
        Err(e) => return vec![Finding::noted("call-failures", format!("could not read: {e}"))],
    };
    let mut out = Vec::new();
    for (role, ok, failed, sample) in rows {
        let total = ok + failed;
        if total < MIN_CALLS_TO_JUDGE {
            continue;
        }
        let rate = failed as f64 * 100.0 / total as f64;
        if rate < FAILURE_ALARM {
            continue;
        }
        // The provider's own words, trimmed: the difference between a 413 and a
        // truncated JSON body is the whole diagnosis, and paraphrasing it here
        // would throw away the only part that says what to do.
        let why = sample.chars().take(160).collect::<String>();
        out.push(Finding::noted(
            "call-failures",
            format!(
                "{role}: {failed} of {total} model calls failed in the last \
                 {FAILURE_WINDOW_HOURS}h ({rate:.0}%) — {why}"
            ),
        ));
    }
    out
}

/// How far back to judge an agent's failure rate.
const FAILURE_WINDOW_HOURS: i64 = 24;

/// Below this, a run of bad luck looks like a broken agent.
const MIN_CALLS_TO_JUDGE: i64 = 8;

/// Above this share of failures, something is wrong with the agent rather than
/// with the stories it was given. Rate limiting alone can push a healthy agent
/// past a third, so the bar is set well clear of it.
const FAILURE_ALARM: f64 = 60.0;

async fn check_delivery(ctx: &Ctx) -> Vec<Finding> {
    // What a crawler will tolerate, in bytes, at the throughput this host has
    // actually delivered. Measured, not assumed: ~7 KB/s and a two-second
    // budget is 14 KB, which is roughly what the drawn card weighs — so the
    // card is the reference, and nothing we advertise should be far above it.
    // Overridable, and not only for tests: the right budget is a property of
    // the link a reader is on, and this host's is not the one it will always
    // have. A check whose thresholds cannot be moved is a check that will be
    // wrong the day the cable is plugged in.
    let doc_budget: usize = env_usize("BG_DELIVERY_DOC_BUDGET", 8_000);
    let image_budget: usize = env_usize("BG_DELIVERY_IMAGE_BUDGET", 45_000);
    // Two stories, so the probe is eight fetches at most rather than sixteen.
    // It is a spot check on what a crawler is handed, not a survey.
    const SAMPLE: i64 = 2;

    let base = std::env::var("BG_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("https://{}", bg_core::brand::DOMAIN));
    let base = base.trim_end_matches('/');

    let recent = match bg_db::stories::top_published(&ctx.db, SAMPLE).await {
        Ok(v) if !v.is_empty() => v,
        _ => return Vec::new(),
    };

    // Ask as WeChat's crawler does, so the measurement is of the document a
    // crawler is actually served rather than of the reader's page.
    let ua = "Mozilla/5.0 (iPhone) AppleWebKit/605.1.15 MicroMessenger/8.0.49";
    let client = match reqwest::Client::builder()
        // Per request, so one unreachable asset cannot consume the round.
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(ua)
        .build()
    {
        Ok(c) => c,
        Err(e) => return vec![Finding::noted("delivery", format!("no client: {e}"))],
    };

    let mut heavy: Vec<String> = Vec::new();
    for st in &recent {
        let url = format!("{base}/story/{}", st.slug);
        let Ok(resp) = client.get(&url).header("Accept", "*/*").send().await else {
            continue;
        };
        let Ok(body) = resp.text().await else {
            continue;
        };
        if body.len() > doc_budget {
            heavy.push(format!("{} document {} bytes", st.slug, body.len()));
        }
        // And the picture it points a crawler at — the part that went wrong
        // five times running.
        let Some(img) = body
            .split("og:image\" content=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
        else {
            continue;
        };
        if let Ok(r) = client.get(img).send().await {
            let n = r.bytes().await.map(|b| b.len()).unwrap_or(0);
            if n > image_budget {
                heavy.push(format!("{} share image {n} bytes", st.slug));
            }
        }
    }

    if heavy.is_empty() {
        return Vec::new();
    }
    // Reported, never acted on. The remedy depends on why — an encoder that
    // gave up, a document that grew, a publisher serving something enormous —
    // and guessing between them is how an agent makes things worse.
    vec![Finding::noted(
        "delivery",
        format!(
            "{} of the top {SAMPLE} stories are heavier than a crawler will wait for \
             (limits {doc_budget}B document, {image_budget}B image): {}",
            heavy.len(),
            heavy.join("; ")
        ),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_without_an_action_is_asking_for_help() {
        assert!(Finding::noted("k", "d").needs_a_human());
        assert!(!Finding::fixed("k", "d", "a").needs_a_human());
    }
}
