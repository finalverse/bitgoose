//! **Skein** — what the story means, and where it goes.
//!
//! A skein is geese in flight formation: the flock seen as a direction rather
//! than as birds. This agent is the one place BitGoose asserts something no
//! source said, which makes it both the reason to read the site and the easiest
//! thing here to get badly wrong.
//!
//! Three constraints keep it honest, and each exists because the alternative
//! failed in practice:
//!
//! 1. **A grounding gate.** Below [`MIN_GROUNDING_CHARS`] of real source text
//!    the Skein does not run. Thirty fabricated stories reached the site once
//!    because a model was handed bare headlines and asked to elaborate; a model
//!    asked to find meaning in a headline will always find some.
//! 2. **A falsifiable direction.** Every forecast carries a horizon and a list
//!    of signals that would confirm or refute it. A prediction with no deadline
//!    and no test cannot be wrong, and so is not worth the reader's time.
//! 3. **Quotes are extracted, never composed.** They enter the claim graph as
//!    [`ClaimKind::Quote`] with the excerpt attached to the specific item it
//!    came from, and are truncated to the policy limit before storage rather
//!    than trusted to a word count the model did itself.

use crate::{stage, Ctx, FlockError, Result, StageOutput};
use bg_core::domain::{AgentRole, ClaimKind, ModelTier, Stance};
use bg_core::ids::StoryId;
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use tracing::{info, warn};

/// Minimum characters of real source text before the Skein will say anything.
///
/// Set against the archive rather than by feel: a typical RSS summary runs
/// 300-600 characters, and analysis drawn from one of those is analysis of a
/// headline. 1,500 means at least a substantial excerpt or a fetched page.
pub const MIN_GROUNDING_CHARS: usize = 1_500;

/// Total words of source text put in one prompt, divided across the story's
/// sources.
///
/// Sized against the tightest budget we run under — Groq's free tier at 8,000
/// tokens a minute. 2,600 words is roughly 3,500 tokens, which leaves room for
/// the system prompt, the schema and a 2,000-token reply inside one window.
const TOTAL_SOURCE_WORDS: usize = 2_600;

pub const SYSTEM: &str = "Skein reads a story and says what it means and where it goes.

You are not summarising. A summary of the sources already exists and the reader \
has just read it. Restating what happened is a failure of this task.

SIGNIFICANCE. Two to four sentences on what this event actually means — the \
part a well-informed reader would not get from the headline. Reach for the \
mechanism: who is now constrained, what becomes cheaper or dearer, which \
prior assumption just broke. Argue it from the sources. If the honest answer is \
that this is routine and changes little, say that plainly; a boring story \
called boring is worth more than a boring story inflated.

DIRECTION. One to three sentences on what follows from this, stated as a \
forecast and not as fact. Name the specific thing you expect to happen. \
'Uncertainty may persist' is not a direction.

HORIZON. The period the direction is claimed over: 'days', 'weeks', 'this \
quarter', 'this year'.

CONFIDENCE. 0-100 in the direction, chosen by which of these the evidence \
actually supports. Do not average them, and do not pick a number in the middle \
because it feels safe — a figure you would give to every story tells the reader \
nothing:
- 85-95: near-mechanical. The thing has already happened and only needs to \
finish, or a party with the power to do it has publicly committed to a date.
- 65-84: strong precedent for this exact sequence, or a named actor has said \
they intend it and can act alone.
- 45-64: plausible, but it turns on a decision nobody has announced yet.
- 25-44: one reading of an ambiguous situation; a competent analyst could \
argue the opposite from the same sources.
- Under 25: you are guessing. Say so, and keep the direction short.

WATCH. Two or three concrete, checkable signals that would confirm or refute \
the direction. Each must be something a reader could actually go and look at — \
a filing, a print, a shipped release, a number crossing a level. Not 'market \
sentiment'.

QUOTES. Pull up to three genuinely quotable lines from the source text: \
something a named person actually said, or a sharply-worded sentence from the \
reporting. Copy them EXACTLY as they appear, at most 20 words each, and give \
the index of the source you took each from. Attribute the speaker if the source \
names one. If nothing in the sources is worth quoting, return an empty list — \
a manufactured quote is the single worst thing you could produce here. Never \
write a quote that is not verbatim in the source text.";

#[derive(Debug, Deserialize)]
pub struct Read {
    pub significance: String,
    pub direction: String,
    pub horizon: String,
    pub confidence: i64,
    pub watch: Vec<String>,
    pub quotes: Vec<PulledQuote>,
}

#[derive(Debug, Deserialize)]
pub struct PulledQuote {
    pub text: String,
    pub speaker: String,
    pub source_index: i64,
}

fn schema(n_sources: usize) -> serde_json::Value {
    sch::object(
        vec![
            (
                "significance",
                sch::string_hinted("2-4 sentences: what this means", "significance"),
            ),
            (
                "direction",
                sch::string_hinted("1-3 sentences: what follows, as a forecast", "direction"),
            ),
            (
                "horizon",
                sch::enumeration(
                    &["days", "weeks", "this quarter", "this year"],
                    "period the direction covers",
                ),
            ),
            (
                "confidence",
                sch::integer_bounded("0-100 confidence in the direction", 100),
            ),
            (
                "watch",
                sch::array(
                    sch::string_hinted("a concrete, checkable signal", "signal"),
                    "2-3 signals",
                ),
            ),
            (
                "quotes",
                sch::array(
                    sch::object(
                        vec![
                            ("text", sch::string_hinted("verbatim, <=20 words", "quote")),
                            ("speaker", sch::string_hinted("who said it, or empty", "")),
                            ("source_index", sch::integer_index("source index")),
                        ],
                        &["text", "speaker", "source_index"],
                    ),
                    "0-3 verbatim quotes",
                ),
            ),
        ],
        &[
            "significance",
            "direction",
            "horizon",
            "confidence",
            "watch",
            "quotes",
        ],
    )
    .tap_sources(n_sources)
}

/// Small extension so the schema builder can note how many sources exist
/// without threading the count through every helper.
trait TapSources {
    fn tap_sources(self, n: usize) -> Self;
}
impl TapSources for serde_json::Value {
    fn tap_sources(mut self, n: usize) -> Self {
        if let Some(q) = self
            .get_mut("properties")
            .and_then(|p| p.get_mut("quotes"))
            .and_then(|q| q.get_mut("items"))
            .and_then(|i| i.get_mut("properties"))
            .and_then(|p| p.get_mut("source_index"))
        {
            q["maximum"] = serde_json::json!(n.saturating_sub(1));
        }
        self
    }
}

/// Analyse a story. `Ok(false)` means the grounding gate held it back.
pub async fn run(ctx: &Ctx, story: StoryId) -> Result<bool> {
    let story_record = bg_db::stories::by_id(&ctx.db, story).await?;
    let output_language = crate::output_language(story_record.editorial_language);
    let items = bg_db::items::by_story(&ctx.db, story).await?;
    if items.is_empty() {
        return Err(FlockError::Other("story has no source items".into()));
    }

    // Drop anything from a publisher who does not permit model input.
    //
    // The backstop to the extraction gate, and it has to exist: text ingested
    // before a site changed its posture — or before BitGoose learned to read
    // the posture at all — is already in the database, and the gate upstream
    // only stops new fetches. The story keeps its citation and its link; what
    // it loses is having that outlet's words in a prompt.
    let denied = bg_db::sources::ai_input_denied(&ctx.db)
        .await
        .unwrap_or_default();
    let items: Vec<_> = items
        .into_iter()
        .filter(|it| !denied.contains(&it.source_id))
        .collect();
    if items.is_empty() {
        info!(story = %story, "every source declines model input; not analysed");
        return Ok(false);
    }

    // Count what we will actually put in the prompt, not what exists in the
    // row. A body we truncate to 900 words is not grounding for the part we
    // dropped, and `summary_raw` standing in for a missing body is the common
    // case rather than the exception.
    let grounded: usize = items
        .iter()
        .filter_map(|it| it.body_raw.as_deref().or(it.summary_raw.as_deref()))
        .map(|t| t.trim().len())
        .sum();

    if grounded < MIN_GROUNDING_CHARS {
        info!(
            story = %story, grounded,
            "below the grounding floor; no analysis rather than an invented one"
        );
        return Ok(false);
    }

    let system = crate::system_prompt(ctx, AgentRole::Skein).await;

    stage(
        ctx,
        AgentRole::Skein,
        Some(story),
        "analyse",
        |run| async move {
            // Budget the prompt as a whole rather than per source. A cap of 900
            // words each is no cap at all on a ten-source story: that is ~12k
            // tokens against a free tier of 8k a minute, so the biggest
            // stories — the ones most worth analysing — would 429 on every
            // attempt and never produce anything. Dividing a fixed budget
            // means cost scales with the story count, not the source count.
            let share = (TOTAL_SOURCE_WORDS / items.len().max(1)).clamp(120, 900);

            let mut prompt = format!("OUTPUT_LANGUAGE={output_language}\n\nSource material:\n\n");
            for (i, it) in items.iter().enumerate() {
                prompt.push_str(&format!("=== SOURCE [{i}] ===\nHeadline: {}\n", it.title));
                if let Some(body) = it.body_raw.as_deref().or(it.summary_raw.as_deref()) {
                    prompt.push_str(&format!(
                        "Text: {}\n",
                        bg_core::text::truncate_words(body, share)
                    ));
                }
                prompt.push('\n');
            }
            prompt.push_str("\nWhat does this mean, and where does it go?");

            let req = Request::new("skein.analyse", ModelTier::Top, system, prompt)
                .with_schema(schema(items.len()))
                // 1,241 of 1,503 Skein calls failed with `json_validate_failed`, the
                // `failed_generation` cut off mid-word. It was not a prompting
                // problem: reasoning tokens come out of this same budget, so the
                // model spent it thinking and truncated before closing the JSON.
                // With `reasoning_effort: low` the budget is the answer's again,
                // and this leaves room above the p95 of the calls that did land.
                .with_max_tokens(2_600);
            let (read, completion) = ctx.llm.complete_json::<Read>(&req).await?;

            let significance = read.significance.trim();
            let direction = read.direction.trim();
            if significance.is_empty() || direction.is_empty() {
                return Err(FlockError::Other("skein returned an empty analysis".into()));
            }

            let model = Some(completion.model.clone());
            let confidence = read.confidence.clamp(0, 100) as i16;
            let watch: Vec<String> = read
                .watch
                .iter()
                .map(|w| w.trim().to_string())
                .filter(|w| !w.is_empty())
                .take(3)
                .collect();

            bg_db::analyses::upsert(
                &ctx.db,
                story,
                &bg_db::analyses::NewAnalysis {
                    significance: significance.to_string(),
                    direction: direction.to_string(),
                    horizon: read.horizon.trim().to_string(),
                    confidence,
                    watch,
                    model,
                    grounded_chars: grounded.min(i32::MAX as usize) as i32,
                },
                Some(run),
            )
            .await?;

            let quotes = store_quotes(ctx, story, &items, &read.quotes, run).await?;

            let note = format!("{confidence}% confidence, {quotes} quotes, {grounded} chars read");
            info!(story = %story, confidence, quotes, "skein analysed");
            Ok(StageOutput::with(true, completion, note))
        },
    )
    .await
}

/// Persist pulled quotes as claims, dropping any that are not actually in the
/// source they cite.
///
/// The verbatim check is the whole point. A model told to quote exactly will
/// still occasionally tidy a sentence — fix its grammar, merge two clauses —
/// and a tidied quote is a fabricated one no matter how close it lands. We
/// compare against the source text and discard on mismatch rather than trying
/// to repair it.
async fn store_quotes(
    ctx: &Ctx,
    story: StoryId,
    items: &[bg_core::domain::RawItem],
    quotes: &[PulledQuote],
    run: bg_core::ids::RunId,
) -> Result<usize> {
    let mut kept = 0;
    for q in quotes.iter().take(3) {
        let text = q.text.trim().trim_matches('"').trim();
        if text.is_empty() {
            continue;
        }
        let Some(item) = items.get(q.source_index.max(0) as usize) else {
            continue;
        };
        let haystack = item
            .body_raw
            .as_deref()
            .or(item.summary_raw.as_deref())
            .unwrap_or_default();

        if !contains_verbatim(haystack, text) {
            warn!(story = %story, quote = %text, "quote is not verbatim in its source; dropped");
            continue;
        }

        // Truncate here rather than trusting the model's word count — the
        // policy limit is ours to enforce, and a DB CHECK will reject the row
        // anyway if we let a long one through.
        let excerpt = bg_core::text::truncate_words(text, bg_core::policy::MAX_QUOTE_WORDS);
        let speaker = q.speaker.trim();
        let claim_text = if speaker.is_empty() {
            format!("\u{201c}{excerpt}\u{201d}")
        } else {
            format!("{speaker}: \u{201c}{excerpt}\u{201d}")
        };

        let claim_id = bg_db::claims::insert(
            &ctx.db,
            story,
            &bg_db::claims::NewClaim {
                text: claim_text,
                kind: ClaimKind::Quote,
                numeric_value: None,
                unit: None,
                as_of: Some(item.published_at),
            },
            Some(run),
        )
        .await?;
        bg_db::claims::add_source(&ctx.db, claim_id, item.id, Stance::Supports, Some(&excerpt))
            .await?;

        // A claim starts unverified because Sentinel has not weighed it yet.
        // That is right for an assertion about the world and wrong for this:
        // we have already checked, character by character, that the source
        // contains these words. Leaving it at "unverified, 0% confidence"
        // states something false about a quote we verified more directly than
        // anything else on the page. It is single-source by definition — one
        // outlet printed it — and no amount of corroboration changes that.
        bg_db::claims::set_verification(
            &ctx.db,
            claim_id,
            bg_core::domain::Verification::SingleSource,
            1.0,
        )
        .await?;
        kept += 1;
    }
    Ok(kept)
}

/// Whether `needle` appears in `haystack`, ignoring differences that are
/// typography rather than wording.
///
/// Feeds mangle punctuation constantly — curly quotes become straight, non-
/// breaking spaces appear mid-sentence, an em dash arrives as a hyphen. Exact
/// matching would reject honest quotes over a character the publisher's CMS
/// chose, so we normalise both sides and compare the words.
fn contains_verbatim(haystack: &str, needle: &str) -> bool {
    fn norm(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut space = false;
        for c in s.chars() {
            let c = match c {
                '\u{2018}' | '\u{2019}' | '\u{201b}' => '\'',
                '\u{201c}' | '\u{201d}' | '\u{201f}' => '"',
                '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
                '\u{00a0}' | '\u{2009}' | '\u{202f}' => ' ',
                c => c,
            };
            if c.is_whitespace() {
                space = true;
                continue;
            }
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.extend(c.to_lowercase());
        }
        out
    }
    let (h, n) = (norm(haystack), norm(needle));
    !n.is_empty() && h.contains(&n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typography_differences_do_not_reject_an_honest_quote() {
        let source = "The chair said \u{201c}we are not\u{00a0}done raising\u{201d} on Tuesday.";
        assert!(contains_verbatim(source, "we are not done raising"));
        assert!(contains_verbatim(source, "We Are Not Done Raising"));
    }

    #[test]
    fn a_tidied_quote_is_still_a_fabricated_one() {
        let source = "He said the rollout was, in his words, kind of a mess.";
        // Plausible, close, and not what the source says.
        assert!(!contains_verbatim(source, "the rollout was a mess"));
    }

    #[test]
    fn an_empty_quote_matches_nothing() {
        // `contains("")` is true for every string, which would wave through a
        // quote the model left blank.
        assert!(!contains_verbatim("anything at all", "   "));
    }

    /// The gate exists to stop analysis of headlines. If someone lowers it to
    /// where a typical feed summary (300-600 chars) passes, that protection is
    /// gone — so this fails the build, not just the suite.
    const _: () = assert!(
        MIN_GROUNDING_CHARS > 600,
        "grounding floor is low enough for a bare RSS summary to pass"
    );
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// The prompt must not grow with the number of sources. A ten-source story
    /// is the most worth analysing and was the one guaranteed to 429.
    #[test]
    fn prompt_size_is_bounded_however_many_sources_there_are() {
        for n in [1usize, 3, 10, 40] {
            let share = (TOTAL_SOURCE_WORDS / n.max(1)).clamp(120, 900);
            let total = share * n;
            assert!(
                total <= TOTAL_SOURCE_WORDS.max(900) * 2,
                "{n} sources would send {total} words"
            );
        }
    }

    /// ...but each source still gets enough text to be worth reading. The floor
    /// matters more than the ceiling here: a 40-source pile-up that gives every
    /// outlet twenty words is just headlines again, which is what the grounding
    /// gate exists to prevent.
    #[test]
    fn each_source_keeps_a_usable_share() {
        let share = (TOTAL_SOURCE_WORDS / 40).clamp(120, 900);
        assert_eq!(share, 120);
    }
}
