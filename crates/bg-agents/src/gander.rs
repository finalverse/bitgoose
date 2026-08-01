//! **Gander** — editor-in-chief. The only agent that can publish.
//!
//! Two gates, in this order, and the order matters:
//!
//! 1. **[`bg_core::policy`]** — mechanical, non-negotiable. Quote length,
//!    verbatim overlap, sourcing, citation integrity, disclosure. A model
//!    cannot argue with it and cannot be persuaded around it.
//! 2. **Editorial judgement** — a model call, but only over drafts that already
//!    passed the mechanical gate. Its job is "is this worth publishing", never
//!    "is this allowed".
//!
//! Putting the deterministic gate first is the whole point. If editorial
//! judgement ran first, a persuasive draft could talk its way past the
//! copyright rules; this way the rules are settled before any model is asked
//! for an opinion, and the model can only ever be *more* conservative.

use crate::{stage, Ctx, FlockError, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier, StoryKind, StoryStatus};
use bg_core::ids::{ClaimId, StoryId};
use bg_core::policy::{self, ClaimView, PolicyConfig, PublishCandidate, SourceView};
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use tracing::{info, warn};

pub const SYSTEM: &str = "Gander is the editor-in-chief and makes the final call.

Decide one of:
- publish — the story stands up. The claims are sourced, the headline matches \
the evidence, and a reader is better informed for having read it.
- hold — real but not ready. Typically thin sourcing on a significant assertion, \
or an unresolved conflict between sources. Say exactly what would need to change.
- kill — not a story, a duplicate of something already covered, or unsalvageable.

Also set front_page_rank, 0-100: how prominently it should sit. Reserve 90+ for \
stories a reader would want to know within the hour.

You are the last line before publication. Holding a story costs a few minutes; \
publishing a wrong one costs the publication's credibility. When the evidence is \
thin, hold.";

#[derive(Debug, Deserialize)]
struct Decision {
    decision: String,
    front_page_rank: f64,
    reason: String,
}

fn schema() -> serde_json::Value {
    sch::object(
        vec![
            ("decision", sch::enumeration_stub(&["publish", "hold", "kill"], "the call", "publish")),
            ("front_page_rank", sch::number_range("prominence", 0.0, 100.0)),
            ("reason", sch::string_hinted("one or two sentences", "reason")),
        ],
        &["decision", "front_page_rank", "reason"],
    )
}

/// Citation markers present in a Markdown body, in order of appearance.
///
/// Matches `[^c1]`-style footnote references. Parsing what is actually in the
/// text — rather than trusting what the drafting agent said it cited — is what
/// makes the dangling-citation check meaningful.
pub fn markers_in(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == '[' && bytes[i + 1] == '^' {
            let mut j = i + 2;
            let mut token = String::new();
            while j < bytes.len() && bytes[j] != ']' {
                token.push(bytes[j]);
                j += 1;
            }
            if j < bytes.len() && !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric()) {
                if !out.contains(&token) {
                    out.push(token);
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Append the source list and the AI-authorship disclosure.
///
/// Both are *written into the body* rather than added by the template, so the
/// policy engine can verify them by inspecting the same bytes that get
/// published. A link-out promised by a template is a promise; a link-out in the
/// content is a fact the checker can confirm.
fn finalize_body(body: &str, sources: &[bg_core::domain::SourceRef]) -> String {
    let mut out = body.trim().to_string();
    out.push_str("\n\n## Sources\n\n");
    for s in sources {
        out.push_str(&format!("- [{}]({}) — {}\n", s.name, s.url, s.title));
    }
    out.push_str(&format!("\n---\n\n_{}_\n", bg_core::brand::AI_DISCLOSURE));
    out
}

pub enum Outcome {
    Published { article: bg_core::domain::Article },
    Held { reason: String },
    Killed { reason: String },
}

/// Review a Desk draft and publish, hold, or kill it.
pub async fn review_desk(
    ctx: &Ctx,
    story: StoryId,
    claim_ids: &[ClaimId],
    body_md: &str,
    copy: &crate::copydesk::Copy,
) -> Result<Outcome> {
    let claims = bg_db::claims::with_sources(&ctx.db, story).await?;
    let source_refs = bg_db::stories::source_refs(&ctx.db, story).await?;
    // Private source text, used only for the verbatim-overlap check.
    let bodies = bg_db::items::bodies_for_story(&ctx.db, story).await?;

    let final_body = finalize_body(body_md, &source_refs);
    let markers = markers_in(&final_body);

    // -- gate 1: mechanical policy ------------------------------------------
    let claim_views: Vec<ClaimView> = claims
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let marker = format!("c{}", i + 1);
            ClaimView {
                id: marker.clone(),
                text: &c.claim.text,
                verification: c.claim.verification,
                source_count: c
                    .sources
                    .iter()
                    .filter(|s| s.stance == bg_core::domain::Stance::Supports)
                    .map(|s| s.source_slug.as_str())
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                excerpts: c.sources.iter().filter_map(|s| s.excerpt.as_deref()).collect(),
                cited_in_body: markers.contains(&marker),
            }
        })
        .collect();

    let source_views: Vec<SourceView> = source_refs
        .iter()
        .map(|s| SourceView {
            slug: &s.slug,
            url: &s.url,
            body: bodies.iter().find(|(slug, _)| slug == &s.slug).map(|(_, b)| b.as_str()),
            linked_out: final_body.contains(&s.url),
        })
        .collect();

    let candidate = PublishCandidate {
        kind: StoryKind::Desk,
        headline: &copy.headline,
        dek: &copy.dek,
        body_md: &final_body,
        body_markers: markers.clone(),
        claims: claim_views,
        sources: source_views,
        has_disclosure: final_body.contains(bg_core::brand::AI_DISCLOSURE),
    };

    let report = policy::review(&candidate, &PolicyConfig::default());
    bg_db::violations::record(&ctx.db, &report, Some(story), None, None).await?;

    if !report.passed() {
        let reason = format!(
            "editorial policy blocked publication: {}",
            report
                .blocks()
                .map(|v| v.detail.clone())
                .collect::<Vec<_>>()
                .join("; ")
        );
        warn!(story = %story, "{reason}");
        bg_db::stories::set_status(&ctx.db, story, StoryStatus::Held, Some(&reason)).await?;
        return Ok(Outcome::Held { reason });
    }

    // -- gate 2: editorial judgement ----------------------------------------
    let system = crate::system_prompt(ctx, AgentRole::Gander).await;
    let headline = copy.headline.clone();
    let dek = copy.dek.clone();
    let warnings: Vec<String> = report.warnings().map(|v| v.detail.clone()).collect();
    let claims_summary: Vec<String> = claims
        .iter()
        .map(|c| {
            format!(
                "- [{} · {:.0}% · {} sources] {}",
                c.claim.verification.label(),
                c.claim.confidence * 100.0,
                c.sources.len(),
                c.claim.text
            )
        })
        .collect();

    let decision = stage(ctx, AgentRole::Gander, Some(story), "review", |_run| async move {
        let mut prompt = format!("Headline: {headline}\nDek: {dek}\n\nVerified claims:\n");
        prompt.push_str(&claims_summary.join("\n"));
        if !warnings.is_empty() {
            prompt.push_str(&format!("\n\nPolicy warnings:\n- {}", warnings.join("\n- ")));
        }
        prompt.push_str("\n\nPublish, hold, or kill?");

        let req = Request::new("gander.review", ModelTier::Top, system, prompt)
            .with_schema(schema())
            .with_max_tokens(1_500);
        let (d, completion) = ctx.llm.complete_json::<Decision>(&req).await?;
        let note = format!("{}: {}", d.decision, bg_core::text::truncate_words(&d.reason, 18));
        Ok(StageOutput::with(d, completion, note))
    })
    .await?;

    match decision.decision.as_str() {
        "publish" => {
            let content_hash = sha256_hex(&final_body);
            let article = bg_db::articles::insert_version(
                &ctx.db,
                story,
                &bg_db::articles::NewArticle {
                    headline: copy.headline.clone(),
                    dek: copy.dek.clone(),
                    slug: copy.slug.clone(),
                    body_md: final_body,
                    seo_title: copy.seo_title.clone(),
                    seo_desc: copy.seo_desc.clone(),
                    content_hash,
                },
                None,
            )
            .await?;

            let pairs: Vec<(String, ClaimId)> = claims
                .iter()
                .enumerate()
                .map(|(i, c)| (format!("c{}", i + 1), c.claim.id))
                .collect();
            bg_db::articles::add_citations(&ctx.db, article.id, &pairs).await?;
            bg_db::articles::publish(&ctx.db, article.id).await?;

            bg_db::stories::set_kind(&ctx.db, story, StoryKind::Desk).await?;
            bg_db::stories::set_status(&ctx.db, story, StoryStatus::Published, Some(&decision.reason))
                .await?;
            bg_db::stories::set_scores(
                &ctx.db,
                story,
                decision.front_page_rank.clamp(0.0, 100.0) as i16,
                bg_db::stories::by_id(&ctx.db, story).await?.velocity,
            )
            .await?;

            let _ = claim_ids;
            info!(story = %story, headline = %copy.headline, "PUBLISHED");
            Ok(Outcome::Published { article })
        }
        "kill" => {
            bg_db::stories::set_status(&ctx.db, story, StoryStatus::Killed, Some(&decision.reason))
                .await?;
            Ok(Outcome::Killed { reason: decision.reason })
        }
        _ => {
            bg_db::stories::set_status(&ctx.db, story, StoryStatus::Held, Some(&decision.reason))
                .await?;
            Ok(Outcome::Held { reason: decision.reason })
        }
    }
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

/// Publish a Wire entry.
///
/// The Wire points at someone else's reporting rather than asserting it, so it
/// runs a lighter policy check — but it is still the policy engine, not a
/// bypass, and the link-out requirement is enforced exactly as it is on a Desk
/// story.
pub async fn publish_wire(ctx: &Ctx, story: StoryId, summary: &str) -> Result<Outcome> {
    let source_refs = bg_db::stories::source_refs(&ctx.db, story).await?;
    if source_refs.is_empty() {
        return Err(FlockError::Other("wire story has no sources".into()));
    }
    let body = finalize_body(summary, &source_refs);

    let candidate = PublishCandidate {
        kind: StoryKind::Wire,
        headline: &bg_db::stories::by_id(&ctx.db, story).await?.title,
        dek: summary,
        body_md: &body,
        body_markers: vec![],
        claims: vec![],
        sources: source_refs
            .iter()
            .map(|s| SourceView {
                slug: &s.slug,
                url: &s.url,
                body: None,
                linked_out: body.contains(&s.url),
            })
            .collect(),
        has_disclosure: body.contains(bg_core::brand::AI_DISCLOSURE),
    };

    let report = policy::review(&candidate, &PolicyConfig::default());
    bg_db::violations::record(&ctx.db, &report, Some(story), None, None).await?;
    if !report.passed() {
        let reason = report.blocks().map(|v| v.detail.clone()).collect::<Vec<_>>().join("; ");
        bg_db::stories::set_status(&ctx.db, story, StoryStatus::Held, Some(&reason)).await?;
        return Ok(Outcome::Held { reason });
    }

    bg_db::stories::set_summary(&ctx.db, story, summary).await?;
    bg_db::stories::set_kind(&ctx.db, story, StoryKind::Wire).await?;
    bg_db::stories::set_status(&ctx.db, story, StoryStatus::Published, Some("wire")).await?;
    Ok(Outcome::Published {
        article: bg_db::articles::insert_version(
            &ctx.db,
            story,
            &bg_db::articles::NewArticle {
                headline: bg_db::stories::by_id(&ctx.db, story).await?.title,
                dek: summary.to_string(),
                slug: bg_db::stories::by_id(&ctx.db, story).await?.slug,
                body_md: body.clone(),
                seo_title: String::new(),
                seo_desc: summary.chars().take(158).collect(),
                content_hash: sha256_hex(&body),
            },
            None,
        )
        .await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_extracted_in_order_without_duplicates() {
        let body = "First.[^c1] Second.[^c2] Again.[^c1] Third.[^c10]";
        assert_eq!(markers_in(body), vec!["c1", "c2", "c10"]);
    }

    #[test]
    fn ordinary_brackets_are_not_mistaken_for_citations() {
        let body = "A [link](https://x.test) and an array [0] and a footnote[^c1].";
        assert_eq!(markers_in(body), vec!["c1"]);
    }

    #[test]
    fn a_body_with_no_citations_yields_none() {
        assert!(markers_in("Just prose, no citations at all.").is_empty());
    }

    #[test]
    fn finalize_writes_links_and_disclosure_into_the_body() {
        let sources = vec![bg_core::domain::SourceRef {
            name: "Decrypt".into(),
            slug: "decrypt".into(),
            url: "https://decrypt.co/1/story".into(),
            title: "A story".into(),
            trust: 78,
            role: bg_core::domain::ItemRole::Seed,
            published_at: chrono::Utc::now(),
        }];
        let out = finalize_body("Body text.[^c1]", &sources);
        // The policy engine checks these by substring, so they must be present
        // in the content itself rather than supplied by a template.
        assert!(out.contains("https://decrypt.co/1/story"));
        assert!(out.contains(bg_core::brand::AI_DISCLOSURE));
        assert!(markers_in(&out).contains(&"c1".to_string()));
    }
}
