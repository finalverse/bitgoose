//! **Sentinel** — checks every claim against every source.
//!
//! Top tier, and the most important agent in the newsroom. Everything BitGoose
//! claims to offer over a conventional aggregator rests on the confidence
//! numbers this agent assigns, so it runs on the strongest model available and
//! is the one place where being slow and expensive is the correct trade.
//!
//! Its output is deliberately *lowered* by the deterministic floor in
//! [`apply_floor`]: a model that says "corroborated" about a claim only one
//! outlet reported is overruled by the source count. Confidence is a fact about
//! the evidence, not a feeling about the sentence.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier, Verification};
use bg_core::ids::StoryId;
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use tracing::info;

pub const SYSTEM: &str = "Sentinel verifies claims against the source material.

For each claim, decide:
- verification:
  * primary_verified — confirmed by a primary document (a filing, an official \
statement from the party itself, on-chain data)
  * corroborated — two or more independent outlets report it consistently
  * single_source — only one outlet reports it
  * disputed — sources materially disagree about it
  * refuted — a source directly contradicts it and is more authoritative
  * unverified — the sources do not actually establish it
- confidence: 0.0-1.0. Be conservative. 0.95+ means you would stake the \
publication's reputation on it.
- note: one sentence on what settled it. For disputed or refuted claims, say \
specifically what the disagreement is.

Two outlets reprinting the same wire copy are ONE source, not two. Look at \
whether the reporting is independent, not at how many URLs mention it.

A claim that merely sounds plausible is unverified. Your job is to be the reason \
a reader can trust the number, so err toward the lower rating.";

#[derive(Debug, Deserialize)]
struct Verdicts {
    verdicts: Vec<Verdict>,
}

#[derive(Debug, Deserialize)]
struct Verdict {
    claim_index: i64,
    verification: String,
    confidence: f64,
    note: String,
}

fn schema(n: usize) -> serde_json::Value {
    let v: Vec<&str> = Verification::ALL.iter().map(|v| v.as_str()).collect();
    sch::object(
        vec![(
            "verdicts",
            sch::array_n(
                sch::object(
                    vec![
                        ("claim_index", sch::integer_index("index of the claim")),
                        (
                            "verification",
                            sch::enumeration_stub(&v, "verification state", "corroborated"),
                        ),
                        ("confidence", sch::number_range("confidence", 0.0, 1.0)),
                        ("note", sch::string_hinted("one sentence", "reason")),
                    ],
                    &["claim_index", "verification", "confidence", "note"],
                ),
                "one verdict per claim",
                n,
            ),
        )],
        &["verdicts"],
    )
}

/// A deterministic ceiling derived from the evidence, applied over the model's
/// verdict.
///
/// The model can only ever *lower* a rating from here. Nothing with one
/// independent source may be called corroborated no matter how confident the
/// prose sounded — that is exactly the failure mode that makes aggregators look
/// authoritative about things nobody actually confirmed.
fn apply_floor(model_verdict: Verification, confidence: f32, sources: i64) -> (Verification, f32) {
    let capped = match (model_verdict, sources) {
        // Disagreement and refutation are findings, not counts — they stand.
        (Verification::Disputed, _) | (Verification::Refuted, _) => model_verdict,
        (_, 0) => Verification::Unverified,
        (Verification::PrimaryVerified, _) => Verification::PrimaryVerified,
        (_, 1) => Verification::SingleSource,
        (Verification::Unverified, _) => Verification::Unverified,
        (_, _) => model_verdict,
    };

    let ceiling = match capped {
        Verification::Unverified => 0.4,
        Verification::SingleSource => 0.75,
        Verification::Disputed => 0.5,
        Verification::Refuted => 0.2,
        Verification::Corroborated => 0.95,
        Verification::PrimaryVerified => 1.0,
    };
    (capped, confidence.clamp(0.0, 1.0).min(ceiling))
}

/// Verify every claim on a story.
pub async fn run(ctx: &Ctx, story: StoryId) -> Result<usize> {
    let claims = bg_db::claims::by_story(&ctx.db, story).await?;
    if claims.is_empty() {
        return Ok(0);
    }
    let items = bg_db::items::by_story(&ctx.db, story).await?;
    let counts: HashMap<_, _> = bg_db::claims::source_counts(&ctx.db, story)
        .await?
        .into_iter()
        .collect();
    let system = crate::system_prompt(ctx, AgentRole::Sentinel).await;

    stage(
        ctx,
        AgentRole::Sentinel,
        Some(story),
        "verify",
        |_run| async move {
            let mut prompt = String::from("Source material:\n\n");
            for (i, it) in items.iter().enumerate() {
                prompt.push_str(&format!("=== SOURCE [{i}] ===\n{}\n", it.title));
                if let Some(b) = it.body_raw.as_deref().or(it.summary_raw.as_deref()) {
                    prompt.push_str(&format!("{}\n", bg_core::text::truncate_words(b, 400)));
                }
                prompt.push('\n');
            }
            prompt.push_str("\nClaims to verify:\n");
            for (i, c) in claims.iter().enumerate() {
                prompt.push_str(&format!("[{i}] {}\n", c.text));
            }

            let req = Request::new("sentinel.verify", ModelTier::Top, system, prompt)
                .with_schema(schema(claims.len()))
                .with_max_tokens(6_000);
            let (parsed, completion) = ctx.llm.complete_json::<Verdicts>(&req).await?;

            let mut n = 0usize;
            let mut disputed = 0usize;
            for v in parsed.verdicts {
                let Some(claim) = claims.get(v.claim_index as usize) else {
                    continue;
                };
                let model_verdict =
                    Verification::from_str(&v.verification).unwrap_or(Verification::Unverified);
                let sources = counts.get(&claim.id).copied().unwrap_or(0);
                let (final_v, conf) = apply_floor(model_verdict, v.confidence as f32, sources);

                if matches!(final_v, Verification::Disputed | Verification::Refuted) {
                    disputed += 1;
                }
                bg_db::claims::set_verification(&ctx.db, claim.id, final_v, conf).await?;
                let _ = &v.note;
                n += 1;
            }

            let note = format!("{n} claims verified, {disputed} disputed or refuted");
            info!(story = %story, verified = n, disputed, "sentinel pass");
            Ok(StageOutput::with(n, completion, note))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_source_can_never_be_called_corroborated() {
        let (v, c) = apply_floor(Verification::Corroborated, 0.99, 1);
        assert_eq!(v, Verification::SingleSource);
        assert!(c <= 0.75, "confidence must be capped too, got {c}");
    }

    #[test]
    fn a_claim_with_no_sources_is_unverified_regardless() {
        let (v, c) = apply_floor(Verification::PrimaryVerified, 1.0, 0);
        assert_eq!(v, Verification::Unverified);
        assert!(c <= 0.4);
    }

    #[test]
    fn genuine_corroboration_survives() {
        let (v, c) = apply_floor(Verification::Corroborated, 0.9, 3);
        assert_eq!(v, Verification::Corroborated);
        assert!((c - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn disagreement_is_preserved_even_with_many_sources() {
        // Several outlets can all report, and still contradict one another.
        let (v, _) = apply_floor(Verification::Disputed, 0.8, 5);
        assert_eq!(v, Verification::Disputed);
        let (v, _) = apply_floor(Verification::Refuted, 0.8, 5);
        assert_eq!(v, Verification::Refuted);
    }

    #[test]
    fn the_model_can_lower_but_not_raise() {
        // Model says unverified despite four sources — its caution stands.
        let (v, _) = apply_floor(Verification::Unverified, 0.3, 4);
        assert_eq!(v, Verification::Unverified);
    }

    #[test]
    fn confidence_is_always_within_range() {
        for sources in 0..6 {
            for v in Verification::ALL {
                let (_, c) = apply_floor(*v, 5.0, sources);
                assert!((0.0..=1.0).contains(&c), "out of range: {c}");
            }
        }
    }
}
