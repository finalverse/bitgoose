//! The editorial policy engine — the gate every publish must pass.
//!
//! BitGoose reads other people's journalism. That is only defensible if the
//! boundary between *reading* and *reproducing* is mechanical rather than
//! aspirational. Prompt instructions are not a control: a model told "don't
//! copy" will still occasionally emit a lifted clause, and by the time anyone
//! notices, it is on the public internet with our name on it.
//!
//! So the rules live here, in code, and [`review`] runs on the path to
//! `status = published` with no way around it:
//!
//! * quotes capped at [`MAX_QUOTE_WORDS`] words, attributed, with a link out;
//! * no run longer than [`MAX_VERBATIM_RUN`] words shared with any source, which
//!   catches lifted wording even when it was never marked as a quote;
//! * every claim carries at least one source, or it does not ship;
//! * refuted claims can never appear in published prose;
//! * Desk stories need genuine corroboration, not one outlet echoed;
//! * the AI-authorship disclosure is present.
//!
//! Every block is written to `policy_violations`, so a refusal is a record
//! rather than a silent retry.

use crate::domain::{StoryKind, Verification};
use crate::text::{longest_common_word_run, word_count};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Longest verbatim quotation we will publish from a source.
pub const MAX_QUOTE_WORDS: usize = 25;

/// Longest run of consecutive words a draft may share with any source text.
/// Slightly above `MAX_QUOTE_WORDS` so a legitimately quoted passage plus its
/// attribution does not self-trip.
pub const MAX_VERBATIM_RUN: usize = 28;

/// Independent sources a Desk story needs before it can publish.
pub const MIN_DESK_SOURCES: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub max_quote_words: usize,
    pub max_verbatim_run: usize,
    pub min_desk_sources: usize,
    pub require_linkout: bool,
    pub require_disclosure: bool,
    /// Fraction of claims allowed to sit at `Unverified`/`SingleSource` in a
    /// published Desk piece. Above this, the draft is not yet reported out.
    pub max_soft_claim_ratio: f32,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            max_quote_words: MAX_QUOTE_WORDS,
            max_verbatim_run: MAX_VERBATIM_RUN,
            min_desk_sources: MIN_DESK_SOURCES,
            require_linkout: true,
            require_disclosure: true,
            max_soft_claim_ratio: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Publication is refused.
    Block,
    /// Recorded and surfaced on `/flock`, but does not stop the presses.
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationCode {
    QuoteTooLong,
    VerbatimOverlap,
    ClaimWithoutSource,
    RefutedClaimPublished,
    InsufficientSources,
    MissingLinkOut,
    MissingDisclosure,
    DanglingCitation,
    EmptyHeadline,
    HeadlineTooLong,
    SoftClaimRatio,
    UncitedBody,
}

impl ViolationCode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::QuoteTooLong => "quote_too_long",
            Self::VerbatimOverlap => "verbatim_overlap",
            Self::ClaimWithoutSource => "claim_without_source",
            Self::RefutedClaimPublished => "refuted_claim_published",
            Self::InsufficientSources => "insufficient_sources",
            Self::MissingLinkOut => "missing_link_out",
            Self::MissingDisclosure => "missing_disclosure",
            Self::DanglingCitation => "dangling_citation",
            Self::EmptyHeadline => "empty_headline",
            Self::HeadlineTooLong => "headline_too_long",
            Self::SoftClaimRatio => "soft_claim_ratio",
            Self::UncitedBody => "uncited_body",
        }
    }
}

impl fmt::Display for ViolationCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub code: ViolationCode,
    pub severity: Severity,
    /// Human-readable, written straight into `policy_violations.detail` and
    /// shown to the Gander agent so it can decide whether to revise or kill.
    pub detail: String,
    /// What tripped it — a claim id, a source slug, a quote.
    pub subject: Option<String>,
}

impl Violation {
    fn block(code: ViolationCode, detail: impl Into<String>, subject: Option<String>) -> Self {
        Self {
            code,
            severity: Severity::Block,
            detail: detail.into(),
            subject,
        }
    }
    fn warn(code: ViolationCode, detail: impl Into<String>, subject: Option<String>) -> Self {
        Self {
            code,
            severity: Severity::Warn,
            detail: detail.into(),
            subject,
        }
    }
}

/// A claim as the policy engine needs to see it.
#[derive(Debug, Clone)]
pub struct ClaimView<'a> {
    pub id: String,
    pub text: &'a str,
    pub verification: Verification,
    /// Count of *distinct* sources backing it.
    pub source_count: usize,
    /// Verbatim excerpts attached to this claim.
    pub excerpts: Vec<&'a str>,
    /// True if the body cites this claim.
    pub cited_in_body: bool,
}

/// A source as the policy engine needs to see it.
#[derive(Debug, Clone)]
pub struct SourceView<'a> {
    pub slug: &'a str,
    pub url: &'a str,
    /// Private working text, if we hold it. Used only for overlap checking and
    /// never rendered.
    pub body: Option<&'a str>,
    /// Whether the rendered page links out to this source.
    pub linked_out: bool,
}

/// Everything about to be published.
#[derive(Debug, Clone)]
pub struct PublishCandidate<'a> {
    pub kind: StoryKind,
    pub headline: &'a str,
    pub dek: &'a str,
    pub body_md: &'a str,
    /// Citation markers found in `body_md`, e.g. `["c1", "c2"]`.
    pub body_markers: Vec<String>,
    pub claims: Vec<ClaimView<'a>>,
    pub sources: Vec<SourceView<'a>>,
    pub has_disclosure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyReport {
    pub violations: Vec<Violation>,
}

impl PolicyReport {
    pub fn blocks(&self) -> impl Iterator<Item = &Violation> {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Block)
    }
    pub fn warnings(&self) -> impl Iterator<Item = &Violation> {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Warn)
    }
    pub fn block_count(&self) -> usize {
        self.blocks().count()
    }
    /// True when nothing blocks publication. Warnings are still recorded.
    pub fn passed(&self) -> bool {
        self.block_count() == 0
    }
    /// Convert to a hard error for callers that want `?`.
    pub fn enforce(self) -> crate::Result<Self> {
        if self.passed() {
            Ok(self)
        } else {
            Err(crate::CoreError::PolicyBlocked(self.block_count()))
        }
    }
    /// One-line summary for logs and the editor agent.
    pub fn summary(&self) -> String {
        if self.violations.is_empty() {
            return "clean".into();
        }
        self.violations
            .iter()
            .map(|v| {
                format!(
                    "{}{}",
                    v.code,
                    if v.severity == Severity::Block {
                        "!"
                    } else {
                        "?"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Run every rule. Pure — same input, same report.
pub fn review(c: &PublishCandidate<'_>, cfg: &PolicyConfig) -> PolicyReport {
    let mut v = Vec::new();

    // --- headline sanity ---------------------------------------------------
    if c.headline.trim().is_empty() {
        v.push(Violation::block(
            ViolationCode::EmptyHeadline,
            "headline is empty",
            None,
        ));
    } else if c.headline.chars().count() > 130 {
        v.push(Violation::warn(
            ViolationCode::HeadlineTooLong,
            format!(
                "headline is {} chars; SEO truncates near 60",
                c.headline.chars().count()
            ),
            None,
        ));
    }

    // --- disclosure --------------------------------------------------------
    if cfg.require_disclosure && !c.has_disclosure {
        v.push(Violation::block(
            ViolationCode::MissingDisclosure,
            "AI-authorship disclosure is missing from the rendered page",
            None,
        ));
    }

    // --- claims ------------------------------------------------------------
    let mut soft = 0usize;
    for cl in &c.claims {
        if cl.source_count == 0 {
            v.push(Violation::block(
                ViolationCode::ClaimWithoutSource,
                format!(
                    "claim has no source: \"{}\"",
                    crate::text::truncate_words(cl.text, 12)
                ),
                Some(cl.id.clone()),
            ));
        }
        if !cl.verification.publishable() {
            v.push(Violation::block(
                ViolationCode::RefutedClaimPublished,
                format!(
                    "claim is {} and cannot appear in published prose: \"{}\"",
                    cl.verification.label(),
                    crate::text::truncate_words(cl.text, 12)
                ),
                Some(cl.id.clone()),
            ));
        }
        if matches!(
            cl.verification,
            Verification::Unverified | Verification::SingleSource
        ) {
            soft += 1;
        }
        for ex in &cl.excerpts {
            let n = word_count(ex);
            if n > cfg.max_quote_words {
                v.push(Violation::block(
                    ViolationCode::QuoteTooLong,
                    format!(
                        "quoted excerpt is {n} words, limit is {}: \"{}\"",
                        cfg.max_quote_words,
                        crate::text::truncate_words(ex, 12)
                    ),
                    Some(cl.id.clone()),
                ));
            }
        }
    }

    if c.kind == StoryKind::Desk && !c.claims.is_empty() {
        let ratio = soft as f32 / c.claims.len() as f32;
        if ratio > cfg.max_soft_claim_ratio {
            v.push(Violation::block(
                ViolationCode::SoftClaimRatio,
                format!(
                    "{:.0}% of claims are unverified or single-source (limit {:.0}%); \
                     the story is not reported out yet",
                    ratio * 100.0,
                    cfg.max_soft_claim_ratio * 100.0
                ),
                None,
            ));
        }
    }

    // --- corroboration -----------------------------------------------------
    // Counted over distinct sources, so one outlet cited five times is one source.
    if c.kind == StoryKind::Desk && c.sources.len() < cfg.min_desk_sources {
        v.push(Violation::block(
            ViolationCode::InsufficientSources,
            format!(
                "Desk stories need {} independent sources, found {}",
                cfg.min_desk_sources,
                c.sources.len()
            ),
            None,
        ));
    }

    // --- link-outs ---------------------------------------------------------
    if cfg.require_linkout {
        for s in &c.sources {
            if !s.linked_out {
                v.push(Violation::block(
                    ViolationCode::MissingLinkOut,
                    format!("source `{}` is used but not linked out", s.slug),
                    Some(s.slug.to_string()),
                ));
            }
        }
    }

    // --- verbatim overlap: the plagiarism tripwire -------------------------
    for s in &c.sources {
        let Some(body) = s.body else { continue };
        let run = longest_common_word_run(c.body_md, body);
        if run > cfg.max_verbatim_run {
            v.push(Violation::block(
                ViolationCode::VerbatimOverlap,
                format!(
                    "draft shares a {run}-word verbatim run with source `{}` (limit {})",
                    s.slug, cfg.max_verbatim_run
                ),
                Some(s.slug.to_string()),
            ));
        }
    }

    // --- citation integrity ------------------------------------------------
    for m in &c.body_markers {
        if !c.claims.iter().any(|cl| &cl.id == m) {
            v.push(Violation::block(
                ViolationCode::DanglingCitation,
                format!("body cites `{m}` but no such claim exists"),
                Some(m.clone()),
            ));
        }
    }
    if c.kind == StoryKind::Desk && c.body_markers.is_empty() && !c.claims.is_empty() {
        v.push(Violation::warn(
            ViolationCode::UncitedBody,
            "Desk body carries no inline citations",
            None,
        ));
    }

    PolicyReport { violations: v }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(slug: &'a str, body: Option<&'a str>) -> SourceView<'a> {
        SourceView {
            slug,
            url: "https://example.com/x",
            body,
            linked_out: true,
        }
    }

    fn claim<'a>(id: &str, text: &'a str, v: Verification, n: usize) -> ClaimView<'a> {
        ClaimView {
            id: id.to_string(),
            text,
            verification: v,
            source_count: n,
            excerpts: vec![],
            cited_in_body: true,
        }
    }

    fn candidate<'a>() -> PublishCandidate<'a> {
        PublishCandidate {
            kind: StoryKind::Desk,
            headline: "Exchange freezes attacker funds after $70M exploit",
            dek: "The venue moved within minutes, but the method is still unclear.",
            body_md: "An exchange confirmed it halted withdrawals.[^c1] Analysts \
                      put the loss near seventy million dollars.[^c2]",
            body_markers: vec!["c1".into(), "c2".into()],
            claims: vec![
                claim(
                    "c1",
                    "The exchange halted withdrawals.",
                    Verification::Corroborated,
                    3,
                ),
                claim(
                    "c2",
                    "Losses total about $70 million.",
                    Verification::Corroborated,
                    2,
                ),
            ],
            sources: vec![src("decrypt", None), src("theblock", None)],
            has_disclosure: true,
        }
    }

    #[test]
    fn a_well_sourced_draft_passes() {
        let r = review(&candidate(), &PolicyConfig::default());
        assert!(r.passed(), "expected clean, got: {}", r.summary());
    }

    #[test]
    fn an_overlong_quote_blocks_publication() {
        let long = "we have identified the root cause of the incident and are working \
                    around the clock with law enforcement and outside security firms to \
                    recover the affected customer assets as quickly as possible";
        let mut c = candidate();
        c.claims[0].excerpts = vec![long];
        let r = review(&c, &PolicyConfig::default());
        assert!(!r.passed());
        assert!(r.blocks().any(|v| v.code == ViolationCode::QuoteTooLong));
    }

    #[test]
    fn a_claim_with_no_source_blocks_publication() {
        let mut c = candidate();
        c.claims[1].source_count = 0;
        let r = review(&c, &PolicyConfig::default());
        assert!(!r.passed());
        assert!(r
            .blocks()
            .any(|v| v.code == ViolationCode::ClaimWithoutSource));
    }

    #[test]
    fn lifted_wording_is_caught_even_when_not_marked_as_a_quote() {
        // The draft reproduces a long run of the source without quoting it —
        // exactly the failure mode prompt instructions do not reliably prevent.
        let source_body = "The company said in a statement that it had identified the \
                           root cause of the incident and was working with law enforcement \
                           and outside security firms to recover the affected customer assets \
                           as quickly as possible across every jurisdiction involved.";
        let mut c = candidate();
        c.body_md = "Here is our report. It had identified the root cause of the incident \
                     and was working with law enforcement and outside security firms to \
                     recover the affected customer assets as quickly as possible across \
                     every jurisdiction involved.";
        c.sources = vec![src("decrypt", Some(source_body)), src("theblock", None)];
        let r = review(&c, &PolicyConfig::default());
        assert!(!r.passed());
        assert!(r.blocks().any(|v| v.code == ViolationCode::VerbatimOverlap));
    }

    #[test]
    fn a_refuted_claim_can_never_ship() {
        let mut c = candidate();
        c.claims[0].verification = Verification::Refuted;
        let r = review(&c, &PolicyConfig::default());
        assert!(r
            .blocks()
            .any(|v| v.code == ViolationCode::RefutedClaimPublished));
    }

    #[test]
    fn a_single_source_desk_story_is_held() {
        let mut c = candidate();
        c.sources = vec![src("decrypt", None)];
        let r = review(&c, &PolicyConfig::default());
        assert!(r
            .blocks()
            .any(|v| v.code == ViolationCode::InsufficientSources));
    }

    #[test]
    fn the_wire_is_allowed_to_run_on_one_source() {
        // The Wire points at someone else's reporting; it does not assert it.
        let mut c = candidate();
        c.kind = StoryKind::Wire;
        c.sources = vec![src("decrypt", None)];
        c.claims.clear();
        c.body_markers.clear();
        let r = review(&c, &PolicyConfig::default());
        assert!(r.passed(), "wire blocked: {}", r.summary());
    }

    #[test]
    fn an_unlinked_source_blocks_publication() {
        let mut c = candidate();
        c.sources[0].linked_out = false;
        let r = review(&c, &PolicyConfig::default());
        assert!(r.blocks().any(|v| v.code == ViolationCode::MissingLinkOut));
    }

    #[test]
    fn missing_disclosure_blocks_publication() {
        let mut c = candidate();
        c.has_disclosure = false;
        assert!(review(&c, &PolicyConfig::default())
            .blocks()
            .any(|v| v.code == ViolationCode::MissingDisclosure));
    }

    #[test]
    fn a_citation_pointing_nowhere_blocks_publication() {
        let mut c = candidate();
        c.body_markers.push("c9".into());
        let r = review(&c, &PolicyConfig::default());
        assert!(r
            .blocks()
            .any(|v| v.code == ViolationCode::DanglingCitation));
    }

    #[test]
    fn a_thinly_reported_desk_story_is_blocked() {
        let mut c = candidate();
        c.claims[0].verification = Verification::SingleSource;
        c.claims[1].verification = Verification::Unverified;
        let r = review(&c, &PolicyConfig::default());
        assert!(r.blocks().any(|v| v.code == ViolationCode::SoftClaimRatio));
    }

    #[test]
    fn enforce_maps_blocks_to_an_error_and_clean_to_ok() {
        let mut c = candidate();
        assert!(review(&c, &PolicyConfig::default()).enforce().is_ok());
        c.claims[0].source_count = 0;
        assert!(review(&c, &PolicyConfig::default()).enforce().is_err());
    }
}
