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
    ///
    /// Returns the reservation, which the caller must hand to [`settle`] with
    /// the real figure once the call returns.
    ///
    /// [`settle`]: Self::settle
    pub async fn acquire(&self, cost: u32, task: &str) -> Reservation {
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
        // Reserve up front, before the call: otherwise concurrent callers all
        // see an empty budget and pile in together.
        self.record(cost);
        Reservation(cost)
    }

    /// Replace a reservation with what the call actually cost.
    ///
    /// This matters more than it looks. The estimate has to include the full
    /// `max_tokens` because we cannot know how long a reply will be, but most
    /// replies are a fraction of it — a 2,000-token ceiling against a 400-token
    /// answer. Without giving the difference back, every call over-reserves by
    /// four fifths of its output allowance and the newsroom paces itself far
    /// slower than the tier actually requires.
    pub fn settle(&self, reservation: Reservation, actual: u32) {
        if !self.enabled() {
            return;
        }
        let estimate = reservation.0;
        if actual == estimate {
            return;
        }
        let mut spent = self.spent.lock().unwrap_or_else(|e| e.into_inner());
        // Newest first: our own reservation is the most recent entry of that
        // size, and matching the newest keeps concurrent callers from
        // correcting each other's.
        if let Some(slot) = spent.iter_mut().rev().find(|(_, n)| *n == estimate) {
            slot.1 = actual;
        }
    }
}

/// A reserved slice of the token budget, to be reconciled by
/// [`Pacer::settle`].
#[must_use = "a reservation that is never settled over-counts the budget"]
#[derive(Debug, Clone, Copy)]
pub struct Reservation(u32);

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

    #[tokio::test]
    async fn an_overestimate_is_given_back() {
        // The estimate must include the full max_tokens ceiling; the reply is
        // usually a fraction of it. Without a refund the budget drains four
        // times faster than the tier requires.
        let p = Pacer::new(8_000);
        let r = p.acquire(2_500, "t").await;
        assert!(p.delay_for(5_000) > Duration::ZERO, "reserved, so tight");
        p.settle(r, 500);
        assert_eq!(
            p.delay_for(5_000),
            Duration::ZERO,
            "refund should leave room again"
        );
    }

    #[tokio::test]
    async fn an_underestimate_is_charged_in_full() {
        // The correction has to work in both directions, or a run of
        // longer-than-expected replies silently blows the budget.
        let p = Pacer::new(8_000);
        let r = p.acquire(500, "t").await;
        p.settle(r, 7_000);
        assert!(p.delay_for(1_000) > Duration::ZERO, "should now be tight");
    }

    #[tokio::test]
    async fn settling_touches_only_its_own_reservation() {
        let p = Pacer::new(20_000);
        let a = p.acquire(1_000, "a").await;
        let _b = p.acquire(1_000, "b").await;
        p.settle(a, 10);
        // One of the two 1,000s became 10; the other must be untouched.
        let spent = p.spent.lock().unwrap();
        let total: u32 = spent.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 1_010, "settle adjusted more than one entry");
    }

    #[test]
    fn estimates_scale_with_the_prompt() {
        let small = estimate_tokens("sys", "hi", 100);
        let big = estimate_tokens("sys", &"word ".repeat(4_000), 100);
        assert!(big > small * 10, "estimate should track prompt size");
    }
}
