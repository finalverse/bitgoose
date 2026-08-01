//! **Copydesk** — headlines, deks and house style.
//!
//! Cheap tier: this is a well-specified rewriting task, not a judgement call.
//! Its one hard constraint is that the headline may not assert anything the
//! claim set does not support — the most common way an otherwise careful story
//! ends up misleading is a headline written for the click rather than the
//! evidence.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier};
use bg_core::ids::StoryId;
use bg_llm::{schema as sch, Request};
use serde::Deserialize;

pub const SYSTEM: &str = "Copydesk writes the headline, dek and SEO metadata.

headline: 6-14 words. Active voice, present or past tense, no colons, no \
questions, no clickbait, no 'here's why'. State what happened. It must be \
supported by the claims — if the claims say a figure is disputed, the headline \
may not state it as settled.

dek: one sentence, 15-30 words, adding what the headline could not fit. Never a \
restatement of the headline.

seo_title: up to 60 characters, including ' | BitGoose' if it fits.
seo_desc: 140-160 characters, plain description, no hype.
slug: lowercase, hyphenated, 3-8 words, no stopwords at the edges.

If the claim set is thin, write a plainer headline. Never compensate for weak \
reporting with a stronger headline.";

#[derive(Debug, Deserialize)]
pub struct Copy {
    pub headline: String,
    pub dek: String,
    pub seo_title: String,
    pub seo_desc: String,
    pub slug: String,
}

fn schema() -> serde_json::Value {
    sch::object(
        vec![
            ("headline", sch::string_hinted("6-14 words", "headline")),
            ("dek", sch::string_hinted("one sentence", "dek")),
            ("seo_title", sch::string_hinted("<=60 chars", "headline")),
            ("seo_desc", sch::string_hinted("140-160 chars", "dek")),
            ("slug", sch::string_hinted("hyphenated", "slug")),
        ],
        &["headline", "dek", "seo_title", "seo_desc", "slug"],
    )
}

pub async fn run(ctx: &Ctx, story: StoryId, body_md: &str) -> Result<Copy> {
    let claims = bg_db::claims::with_sources(&ctx.db, story).await?;
    let system = crate::system_prompt(ctx, AgentRole::Copydesk).await;
    let body = body_md.to_string();

    stage(
        ctx,
        AgentRole::Copydesk,
        Some(story),
        "copy",
        |_run| async move {
            let mut prompt = String::from("Verified claims:\n");
            for c in &claims {
                prompt.push_str(&format!(
                    "- [{}] {}\n",
                    c.claim.verification.label(),
                    c.claim.text
                ));
            }
            prompt.push_str(&format!(
                "\nStory body:\n{}\n\nWrite the headline, dek and metadata.",
                bg_core::text::truncate_words(&body, 400)
            ));

            let req = Request::new("copydesk.copy", ModelTier::Fast, system, prompt)
                .with_schema(schema())
                .with_max_tokens(1_500);
            let (mut copy, completion) = ctx.llm.complete_json::<Copy>(&req).await?;

            copy.headline = copy.headline.trim().to_string();
            copy.dek = copy.dek.trim().to_string();
            // Normalized rather than trusted: the slug is a permanent public URL,
            // and a model-supplied string with a space or slash in it would produce
            // a broken one.
            copy.slug = bg_core::slug::slugify(&if copy.slug.trim().is_empty() {
                copy.headline.clone()
            } else {
                copy.slug.clone()
            });
            if copy.seo_title.trim().is_empty() {
                copy.seo_title = copy.headline.clone();
            }
            if copy.seo_desc.trim().is_empty() {
                copy.seo_desc = copy.dek.clone();
            }

            let note = format!("\"{}\"", bg_core::text::truncate_words(&copy.headline, 10));
            Ok(StageOutput::with(copy, completion, note))
        },
    )
    .await
}
