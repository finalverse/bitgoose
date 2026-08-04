//! A token-per-minute pacer.
//!
//! Hosted free tiers meter tokens per minute, not requests. Groq's is 8,000.
//! Without pacing, the newsroom fires a pass's worth of calls as fast as it
//! can, spends the minute in the first few seconds, and then takes 429s for the
//! rest — which the retry loop absorbs by sleeping 75 seconds at a time. On a
//! live worker that looked like this, every pass:
//!
//! ```text
//! WARN rate limited; waiting task=gosling.triage attempt=1 wait_s=75
//! WARN rate limited; waiting task=gosling.triage attempt=2 wait_s=75
//! WARN rate limited; waiting task=gosling.triage attempt=3 wait_s=75
//! ```
//!
//! Three blind waits, then the stage gives up and everything downstream —
//! clustering, publishing, analysis — is starved. The site stops updating while
//! the budget goes unspent.
//!
//! Waiting *before* the call instead of after the rejection costs the same
//! wall-clock time and gets the work done. The retry loop stays as a backstop
//! for when our estimate is wrong or another process shares the key.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::debug;

/// Tokens per minute to plan against. `0` disables pacing entirely, which is
/// what a paid tier or a local model wants.
pub fn limit_from_env() -> u32 {
    std::env::var("BG_LLM_TOKENS_PER_MIN")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Fraction of the stated limit we actually plan to use.
///
/// Estimates are rough and the provider's accounting is not ours, so aiming at
/// the exact ceiling guarantees periodic overshoot. Nine tenths leaves room for
/// a mis-estimate without wasting much of the tier.
const SAFETY: f64 = 0.9;

const WINDOW: Duration = Duration::from_secs(60);

/// Rolling one-minute token ledger.
pub struct Pacer {
    limit: u32,
    spent: Mutex<VecDeque<(Instant, u32)>>,
}

impl Pacer {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            spent: Mutex::new(VecDeque::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.limit > 0
    }

    /// How long to wait before spending `cost` tokens.
    ///
    /// Separate from the sleeping so it can be tested without a clock: the
    /// caller sleeps for whatever this returns.
    fn delay_for(&self, cost: u32) -> Duration {
        if !self.enabled() {
            return Duration::ZERO;
        }
        let budget = (f64::from(self.limit) * SAFETY) as u32;
        let now = Instant::now();
        let mut spent = self.spent.lock().unwrap_or_else(|e| e.into_inner());

        while let Some((t, _)) = spent.front() {
            if now.duration_since(*t) >= WINDOW {
                spent.pop_front();
            } else {
                break;
            }
        }

        let used: u32 = spent.iter().map(|(_, n)| n).sum();
        // A single request larger than the whole budget can never fit. Let it
        // through rather than deadlocking; the provider will decide.
        if used + cost <= budget || cost > budget {
            return Duration::ZERO;
        }

        // Wait until enough of the oldest spend ages out of the window.
        let mut freed = 0u32;
        for (t, n) in spent.iter() {
            freed += n;
            if used - freed + cost <= budget {
                return WINDOW.saturating_sub(now.duration_since(*t));
            }
        }
        Duration::ZERO
    }

    /// Record tokens actually spent.
    pub fn record(&self, tokens: u32) {
        if !self.enabled() || tokens == 0 {
            return;
        }
        let mut spent = self.spent.lock().unwrap_or_else(|e| e.into_inner());
        spent.push_back((Instant::now(), tokens));
    }

    /// Block until `cost` tokens fit inside the rolling minute.
    pub async fn acquire(&self, cost: u32, task: &str) {
        let wait = self.delay_for(cost);
        if wait > Duration::ZERO {
            debug!(
                task,
                cost,
                wait_ms = wait.as_millis(),
                "pacing to stay inside the token budget"
            );
            tokio::time::sleep(wait).await;
        }
        // Reserve the estimate now. The real figure replaces it via `record`
        // once the call returns; reserving first stops concurrent callers from
        // all seeing an empty budget and piling in together.
        self.record(cost);
    }
}

/// Rough token count for a prompt.
///
/// Four characters per token is the usual English approximation and is close
/// enough for budgeting — we are deciding whether to wait 200ms or 3s, not
/// billing anyone.
pub fn estimate_tokens(system: &str, prompt: &str, max_output: u32) -> u32 {
    let input = (system.len() + prompt.len()) as u32 / 4;
    input + max_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_budget_never_waits() {
        let p = Pacer::new(8_000);
        assert_eq!(p.delay_for(1_000), Duration::ZERO);
    }

    #[test]
    fn spending_the_minute_forces_a_wait() {
        let p = Pacer::new(8_000);
        // 0.9 * 8000 = 7200 usable.
        p.record(7_000);
        assert_eq!(p.delay_for(100), Duration::ZERO, "still fits");
        assert!(p.delay_for(1_000) > Duration::ZERO, "should wait");
    }

    #[test]
    fn a_request_bigger_than_the_whole_budget_is_let_through() {
        // Otherwise it waits forever for room that can never exist.
        let p = Pacer::new(8_000);
        p.record(7_000);
        assert_eq!(p.delay_for(50_000), Duration::ZERO);
    }

    #[test]
    fn a_zero_limit_disables_pacing() {
        let p = Pacer::new(0);
        assert!(!p.enabled());
        p.record(1_000_000);
        assert_eq!(p.delay_for(1_000_000), Duration::ZERO);
    }

    #[test]
    fn the_wait_never_exceeds_the_window() {
        let p = Pacer::new(8_000);
        p.record(7_200);
        assert!(p.delay_for(7_200) <= WINDOW);
    }

    #[test]
    fn estimates_scale_with_the_prompt() {
        let small = estimate_tokens("sys", "hi", 100);
        let big = estimate_tokens("sys", &"word ".repeat(4_000), 100);
        assert!(big > small * 10, "estimate should track prompt size");
    }
}
