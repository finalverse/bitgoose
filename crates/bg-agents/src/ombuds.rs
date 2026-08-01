//! **Ombuds** — re-reads what we published and corrects it.
//!
//! The agent nobody else has. A conventional newsroom corrects when a reader
//! complains; Ombuds re-checks published claims against sources that arrived
//! *after* publication, which is when most corrections actually become
//! necessary. Corrections are append-only: a new article version plus a
//! `corrections` row, never a silent edit to the page a reader already saw.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier, Verification};
use bg_core::ids::StoryId;
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

pub const SYSTEM: &str = "Ombuds re-checks published claims against source material \
that has arrived since publication.

For each claim, decide whether the new material changes its standing:
- unchanged — the new material is consistent with what we published
- strengthened — additional independent sources now confirm it
- weakened — the support is thinner than it appeared
- contradicted — new material directly contradicts it

Only recommend a correction for `weakened` or `contradicted`, and only when a \
reader acting on the story would be misled. A slightly revised figure that does \
not change the meaning is not a correction.

Corrections are expensive to a publication's credibility in both directions: \
issuing one for a triviality is noise, and failing to issue a needed one is a \
lie by omission. Judge whether the reader was misled.";

#[derive(Debug, Deserialize)]
struct Recheck {
    findings: Vec<Finding>,
    needs_correction: bool,
    correction_reason: String,
}

#[derive(Debug, Deserialize)]
struct Finding {
    claim_index: i64,
    standing: String,
    new_verification: String,
}

fn schema(n: usize) -> serde_json::Value {
    let v: Vec<&str> = Verification::ALL.iter().map(|v| v.as_str()).collect();
    sch::object(
        vec![
            (
                "findings",
                sch::array_n(
                    sch::object(
                        vec![
                            ("claim_index", sch::integer_index("index")),
                            (
                                "standing",
                                sch::enumeration_stub(
                                    &["unchanged", "strengthened", "weakened", "contradicted"],
                                    "standing",
                                    "unchanged",
                                ),
                            ),
                            ("new_verification", sch::enumeration(&v, "revised state")),
                        ],
                        &["claim_index", "standing", "new_verification"],
                    ),
                    "one per claim",
                    n,
                ),
            ),
            (
                "needs_correction",
                sch::boolean("would a reader have been misled"),
            ),
            (
                "correction_reason",
                sch::string_hinted("what changed, or empty", "reason"),
            ),
        ],
        &["findings", "needs_correction", "correction_reason"],
    )
}

/// Re-check one published story.
pub async fn recheck(ctx: &Ctx, story: StoryId) -> Result<bool> {
    let claims = bg_db::claims::with_sources(&ctx.db, story).await?;
    if claims.is_empty() {
        return Ok(false);
    }
    let items = bg_db::items::by_story(&ctx.db, story).await?;
    let Some(article) = bg_db::articles::latest_for_story(&ctx.db, story).await? else {
        return Ok(false);
    };

    // Only worth a model call if evidence arrived after we published.
    let published_at = article.published_at.unwrap_or(article.created_at);
    let fresh: Vec<_> = items
        .iter()
        .filter(|i| i.fetched_at > published_at)
        .collect();
    if fresh.is_empty() {
        return Ok(false);
    }

    let system = crate::system_prompt(ctx, AgentRole::Ombuds).await;
    let claim_list: Vec<String> = claims
        .iter()
        .enumerate()
        .map(|(i, c)| format!("[{i}] ({}) {}", c.claim.verification.label(), c.claim.text))
        .collect();
    let fresh_text: Vec<String> = fresh
        .iter()
        .map(|i| {
            format!(
                "- {}: {}",
                i.title,
                i.summary_raw
                    .as_deref()
                    .map(|s| bg_core::text::truncate_words(s, 100))
                    .unwrap_or_default()
            )
        })
        .collect();
    let claims_for_update = claims.clone();
    let article_version = article.version;
    let article_id = article.id;

    stage(
        ctx,
        AgentRole::Ombuds,
        Some(story),
        "recheck",
        |_run| async move {
            let prompt = format!(
                "Published claims:\n{}\n\nSource material that arrived after publication:\n{}\n\n\
             Does any claim need revising?",
                claim_list.join("\n"),
                fresh_text.join("\n")
            );
            let req = Request::new("ombuds.recheck", ModelTier::Mid, system, prompt)
                .with_schema(schema(claims_for_update.len()))
                .with_max_tokens(3_000);
            let (r, completion) = ctx.llm.complete_json::<Recheck>(&req).await?;

            let mut changed = 0usize;
            for f in &r.findings {
                let Some(c) = claims_for_update.get(f.claim_index as usize) else {
                    continue;
                };
                if f.standing == "unchanged" {
                    continue;
                }
                let nv =
                    Verification::from_str(&f.new_verification).unwrap_or(c.claim.verification);
                if nv != c.claim.verification {
                    bg_db::claims::set_verification(&ctx.db, c.claim.id, nv, c.claim.confidence)
                        .await?;
                    changed += 1;
                }
            }

            if r.needs_correction && !r.correction_reason.trim().is_empty() {
                // Append-only: the correction row records the change; the reader
                // can always see that the page moved and why.
                bg_db::articles::add_correction(
                    &ctx.db,
                    article_id,
                    article_version,
                    article_version + 1,
                    r.correction_reason.trim(),
                    "",
                    None,
                )
                .await?;
                warn!(story = %story, reason = %r.correction_reason, "CORRECTION ISSUED");
            }

            let note = if r.needs_correction {
                format!(
                    "correction issued: {}",
                    bg_core::text::truncate_words(&r.correction_reason, 15)
                )
            } else {
                format!("{changed} claim(s) revised, no correction needed")
            };
            Ok(StageOutput::with(r.needs_correction, completion, note))
        },
    )
    .await
}

/// Re-check recently published stories.
pub async fn run(ctx: &Ctx, limit: i64) -> Result<usize> {
    let recent = bg_db::stories::published(&ctx.db, None, limit, 0).await?;
    let mut corrections = 0usize;
    for s in recent {
        match recheck(ctx, s.id).await {
            Ok(true) => corrections += 1,
            Ok(false) => {}
            Err(e) => warn!(story = %s.id, error = %e, "recheck failed"),
        }
    }
    if corrections > 0 {
        info!(corrections, "ombuds issued corrections");
    }
    Ok(corrections)
}
