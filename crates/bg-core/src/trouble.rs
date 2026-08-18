//! Turning a provider's error into something a reader can act on.
//!
//! `/flock` promises the reader "how often it failed", and for weeks it kept
//! that promise in the least useful way available: a number. The Skein tile
//! read 262 runs, 1,241 failed, and nothing on the page — or in the API, or in
//! the pass summary — said that every one of those was
//! `json_validate_failed`, which is a two-line fix, or that Scribe's were
//! HTTP 413, which is a one-line fix and the reason the Desk had never
//! published anything at all.
//!
//! A glass newsroom that shows you the failure count and hides the reason is
//! frosted glass. So the raw provider string is classified here, once, and the
//! same sentence is used by the page, the JSON API and the Steward — because
//! three descriptions of one fault is how a fault stays unfixed.
//!
//! Deliberately conservative: an error this does not recognise returns `None`
//! and the caller shows the original text. Inventing a friendly summary for
//! something unrecognised is how the real message gets buried.

/// A short, plain-English reading of a failed run's error.
///
/// Returns `None` when the text is not a known failure mode — say the raw
/// string instead, unabridged.
pub fn explain(error: &str) -> Option<&'static str> {
    let e = error.to_lowercase();

    // Reservation larger than the per-minute window. Waiting cannot fix it:
    // the request has to get smaller.
    if e.contains("request too large") || e.contains("413") {
        return Some("asked for more tokens than the provider allows in one request");
    }
    // Structured output that never closed. Nearly always the reasoning budget
    // eating the room the answer needed.
    if e.contains("json_validate_failed") || e.contains("failed to generate json") {
        return Some("ran out of room before it finished writing its answer");
    }
    if e.contains("tokens per day") || e.contains("tpd") {
        return Some("the day's token allowance is spent");
    }
    if e.contains("rate limited") || e.contains("rate_limit") || e.contains("tokens per minute") {
        return Some("waiting for the provider's rate limit to clear");
    }
    if e.contains("declined") || e.contains("no viable") {
        return Some("declined the job: not enough coverage to work from yet");
    }
    if e.contains("timed out") || e.contains("timeout") {
        return Some("the provider did not answer in time");
    }
    if e.contains("401") || e.contains("unauthorized") || e.contains("invalid api key") {
        return Some("the provider rejected our credentials");
    }
    if e.contains("model_not_found") || e.contains("does not exist") {
        return Some("the configured model is not available on this account");
    }
    if e.contains("503") || e.contains("502") || e.contains("overloaded") {
        return Some("the provider is unavailable");
    }
    None
}

/// Whether a failure rate is bad enough to show as trouble rather than noise.
///
/// Rate limiting alone pushes a healthy agent past a third on a free tier, so
/// the bar sits well clear of that: this marks an agent that is *mostly*
/// failing, which is a different thing from a busy one.
pub fn is_troubled(ok: i64, failed: i64) -> bool {
    let total = ok + failed;
    total >= 8 && (failed as f64 * 100.0 / total as f64) >= 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these is a real error string taken from `agent_runs`.
    #[test]
    fn the_failures_that_actually_happened_are_explained() {
        let cases = [
            (
                "openai returned HTTP 413: {\"error\":{\"message\":\"Request too large for model \
                 `openai/gpt-oss-120b` … on tokens per minute (TPM): Limit 8000, Requested 9080",
                "asked for more tokens",
            ),
            (
                "openai returned HTTP 400: {\"error\":{\"message\":\"Failed to generate JSON. \
                 Please adjust your prompt.\",\"code\":\"json_validate_failed\"",
                "ran out of room",
            ),
            ("pacer rate limited; retry in 299s", "rate limit"),
            (
                "the model declined to frame this topic (\"No viable story\")",
                "declined the job",
            ),
        ];
        for (raw, expected) in cases {
            let got = explain(raw).unwrap_or_else(|| panic!("unexplained: {raw}"));
            assert!(got.contains(expected), "{raw}\n  got: {got}");
        }
    }

    #[test]
    fn an_unrecognised_error_is_not_papered_over() {
        // The caller shows the raw text in this case. A cheerful summary of
        // something we do not understand is worse than the original.
        assert_eq!(explain("segmentation fault in the goose"), None);
        assert_eq!(explain(""), None);
    }

    #[test]
    fn the_daily_allowance_is_not_confused_with_a_busy_minute() {
        // Different waits and different fixes: one clears in seconds, the
        // other needs the day to turn over or the budget to be spent better.
        assert_eq!(
            explain("on tokens per day (TPD): Limit 200000, Used 196664"),
            Some("the day's token allowance is spent")
        );
        assert_eq!(
            explain("on tokens per minute (TPM): Limit 8000, Used 7666"),
            Some("waiting for the provider's rate limit to clear")
        );
    }

    #[test]
    fn a_busy_agent_is_not_reported_as_a_broken_one() {
        assert!(!is_troubled(100, 20), "a fifth failing is a free tier");
        assert!(!is_troubled(2, 3), "too few runs to judge");
        // Skein, as it actually stood: 262 ok against 1,241 failed.
        assert!(is_troubled(262, 1_241));
    }
}
