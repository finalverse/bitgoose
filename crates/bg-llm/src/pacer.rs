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

use bg_core::domain::ModelTier;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Tokens per minute to plan against. `0` disables pacing entirely, which is
/// what a paid tier or a local model wants.
pub fn limit_from_env() -> u32 {
    std::env::var("BG_LLM_TOKENS_PER_MIN")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Tokens per **day**, which on a free tier is the limit that actually stops
/// the newsroom — and the only one no header reports.
///
/// Groq publishes `x-ratelimit-*-tokens` and `-requests` on every response, and
/// both can read perfectly healthy (`8000/8000`, reset `1ms`) while the daily
/// allowance is spent. It surfaces only in the body of the 429:
///
/// ```text
/// Rate limit reached ... on tokens per day (TPD):
/// Limit 200000, Used 196276, Requested 5572. Please try again in 13m18.336s.
/// ```
///
/// So this one has to be tracked locally. `0` disables it.
pub fn daily_limit_from_env() -> u32 {
    std::env::var("BG_LLM_TOKENS_PER_DAY")
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
const DAY: Duration = Duration::from_secs(86_400);

fn prune_day(day: &mut VecDeque<(Instant, u32)>) {
    let now = Instant::now();
    while let Some((t, _)) = day.front() {
        if now.duration_since(*t) >= DAY {
            day.pop_front();
        } else {
            break;
        }
    }
}

#[derive(Default)]
struct Ledger {
    spent: VecDeque<(Instant, u32)>,
    /// When the provider will next accept work on this tier.
    ///
    /// Set from the `retry_after` in a refusal, and it is the difference
    /// between one refusal and a whole wasted pass. Measured on production: 83
    /// rate-limit refusals in six hours, 69 of them saying "retry in 300s", and
    /// **28 of 28 Gander runs failed** — because each stage tried
    /// independently, slept, and was refused in turn. The provider had already
    /// said, on the first one, that nothing would work for five minutes.
    ///
    /// Per tier rather than global: the tiers have separate allowances, and a
    /// top-tier refusal says nothing about whether a cheap triage call would
    /// go through.
    cooldown_until: Option<Instant>,
    /// The provider's own account of what is left, and when it refills.
    ///
    /// Authoritative when present. Our own tally is an estimate built from
    /// character counts; this is the meter that actually decides whether the
    /// next call is refused, and it accounts for anything else sharing the key.
    observed: Option<(Instant, u32, Duration)>,
    /// Requests left and the refill interval for one, as the provider reports.
    ///
    /// On a free tier this is usually the *binding* limit. Groq allows 1,000 a
    /// day, refilling one every 86.4 seconds, and a pipeline pass can want
    /// eighty — so pacing tokens alone leaves the newsroom stalling on a quota
    /// it never looked at.
    observed_requests: Option<(Instant, u32, Duration)>,
    /// Every spend in the last 24 hours, for the daily ceiling.
    ///
    /// Kept separately from `spent` because that one is pruned to a minute.
    day: VecDeque<(Instant, u32)>,
}

/// Rolling token ledgers, one per model, corrected by what the provider reports.
///
/// Per model because that is how the limit is actually enforced. Measured
/// against Groq with identical ~977-token prompts: `gpt-oss-120b` dropped to
/// 7023 of 8000 while `gpt-oss-20b` stayed at 8000 throughout. A single shared
/// budget therefore throttled the cheap Fast-tier traffic — triage, clustering,
/// wire summaries, the bulk of every pass — against a ceiling that only really
/// binds the Skein on the Top tier.
///
/// Keyed by [`ModelTier`] rather than by model name, since each tier resolves to
/// one model per provider. Two tiers configured to the same model share a real
/// bucket, and that corrects itself: both observe the same low remaining figure
/// from the provider and both back off.
pub struct Pacer {
    limit: u32,
    daily_limit: u32,
    ledgers: Mutex<[Ledger; 3]>,
}

fn slot(tier: ModelTier) -> usize {
    match tier {
        ModelTier::Fast | ModelTier::None => 0,
        ModelTier::Mid => 1,
        ModelTier::Top => 2,
    }
}

impl Pacer {
    pub fn new(limit: u32) -> Self {
        Self::with_daily(limit, daily_limit_from_env())
    }

    pub fn with_daily(limit: u32, daily_limit: u32) -> Self {
        Self {
            limit,
            daily_limit,
            ledgers: Mutex::new(Default::default()),
        }
    }

    /// Tokens spent on this model in the last 24 hours, and the ceiling.
    pub fn day_usage(&self, tier: ModelTier) -> (u32, u32) {
        let mut ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        let l = &mut ledgers[slot(tier)];
        prune_day(&mut l.day);
        (l.day.iter().map(|(_, n)| n).sum(), self.daily_limit)
    }

    pub fn enabled(&self) -> bool {
        self.limit > 0
    }

    /// How long to wait before spending `cost` tokens on `tier`.
    ///
    /// Separate from the sleeping so it can be tested without a clock: the
    /// caller sleeps for whatever this returns.
    fn delay_for(&self, tier: ModelTier, cost: u32) -> Duration {
        if !self.enabled() {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        let l = &mut ledgers[slot(tier)];

        // Prefer the provider's own figure while it is still describing the
        // present. Once older than the refill it described, our own tally is
        // the better guess.
        if let Some((seen, remaining, reset)) = l.observed {
            let age = now.duration_since(seen);
            if age < reset.max(Duration::from_secs(1)) {
                if remaining >= cost {
                    return Duration::ZERO;
                }
                return reset.saturating_sub(age).min(WINDOW);
            }
        }

        // A request quota binds independently of the token one. When the
        // provider says only a handful of calls are left, spacing them by the
        // stated refill is the difference between degrading gracefully and
        // spending the rest of the day taking 429s.
        if let Some((seen, remaining, reset)) = l.observed_requests {
            let age = now.duration_since(seen);
            if remaining == 0 {
                return reset.saturating_sub(age).min(WINDOW);
            }
        }

        // The daily ceiling. Nothing announces this one in advance, so if we
        // do not count it ourselves the first sign is a 429 asking for a
        // thirteen-minute wait — repeatedly, for the rest of the day.
        if self.daily_limit > 0 {
            prune_day(&mut l.day);
            let day_used: u32 = l.day.iter().map(|(_, n)| n).sum();
            if day_used + cost > self.daily_limit {
                warn!(
                    day_used,
                    limit = self.daily_limit,
                    cost,
                    "daily token allowance spent; holding until it rolls over"
                );
                // Wait for the oldest spend to fall out of the 24h window.
                return l
                    .day
                    .front()
                    .map(|(t, _)| DAY.saturating_sub(now.duration_since(*t)))
                    .unwrap_or(DAY)
                    .min(WINDOW);
            }
        }

        let budget = (f64::from(self.limit) * SAFETY) as u32;
        while let Some((t, _)) = l.spent.front() {
            if now.duration_since(*t) >= WINDOW {
                l.spent.pop_front();
            } else {
                break;
            }
        }

        let used: u32 = l.spent.iter().map(|(_, n)| n).sum();

        // A single request larger than the whole budget can never fit, so
        // waiting for room would deadlock. It goes out unpaced and the provider
        // decides — but say so, loudly. This is a caller bug, not a condition
        // to absorb: raising the triage batch to thirty pushed one request past
        // the whole minute's allowance, every triage call then skipped pacing,
        // and the symptom was `retry in 286s` with the pacer apparently
        // enabled and working. Two deploys went by before anyone could see it,
        // because it looked exactly like the limit being too low.
        if cost > budget {
            warn!(
                cost,
                budget, "request exceeds the entire per-minute budget; sending unpaced"
            );
            return Duration::ZERO;
        }
        if used + cost <= budget {
            return Duration::ZERO;
        }

        let mut freed = 0u32;
        for (t, n) in l.spent.iter() {
            freed += n;
            if used - freed + cost <= budget {
                return WINDOW.saturating_sub(now.duration_since(*t));
            }
        }
        Duration::ZERO
    }

    /// Record what the provider says is left of this model's budget.
    ///
    /// Groq returns `x-ratelimit-remaining-tokens` and
    /// `x-ratelimit-reset-tokens` on every response. Reading them beats
    /// modelling the bucket ourselves: the real one refills continuously rather
    /// than in the sixty-second steps our own tally assumes, and it counts
    /// usage from anything else holding the same key.
    pub fn observe(&self, tier: ModelTier, remaining: Option<u32>, reset: Option<Duration>) {
        let (Some(remaining), Some(reset)) = (remaining, reset) else {
            return;
        };
        let mut ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        ledgers[slot(tier)].observed = Some((Instant::now(), remaining, reset));
    }

    /// Record the request allowance the provider reports.
    pub fn observe_requests(
        &self,
        tier: ModelTier,
        remaining: Option<u32>,
        reset: Option<Duration>,
    ) {
        let (Some(remaining), Some(reset)) = (remaining, reset) else {
            return;
        };
        let mut ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        ledgers[slot(tier)].observed_requests = Some((Instant::now(), remaining, reset));
    }

    /// How close to exhausted the request allowance is, 0.0-1.0, if known.
    ///
    /// Exposed so the pipeline can shrink its own appetite — the honest answer
    /// to a daily request quota is to make fewer, larger calls, not to wait
    /// longer between the same number of them.
    pub fn request_headroom(&self, tier: ModelTier) -> Option<f32> {
        let ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        ledgers[slot(tier)]
            .observed_requests
            .map(|(_, remaining, _)| remaining as f32)
            .map(|r| (r / 1000.0).clamp(0.0, 1.0))
    }

    /// Record tokens spent against a model.
    /// Note that the provider has refused this tier, and for how long.
    ///
    /// Called on the way out of a refusal so the rest of the pass does not
    /// spend itself discovering the same thing.
    pub fn cooling(&self, tier: ModelTier, retry_after: Duration) {
        // Capped. A provider that asks for an hour is reporting an outage, and
        // sitting out an hour of passes on its say-so would turn a bad twenty
        // minutes into a bad morning — the next pass can find out cheaply.
        let wait = retry_after.min(Duration::from_secs(600));
        if let Ok(mut g) = self.ledgers.lock() {
            g[slot(tier)].cooldown_until = Some(Instant::now() + wait);
        }
    }

    /// How long until this tier is worth trying again, if it is cooling.
    pub fn cooling_for(&self, tier: ModelTier) -> Option<Duration> {
        let g = self.ledgers.lock().ok()?;
        let until = g[slot(tier)].cooldown_until?;
        let now = Instant::now();
        (until > now).then(|| until - now)
    }

    pub fn record(&self, tier: ModelTier, tokens: u32) {
        if !self.enabled() || tokens == 0 {
            return;
        }
        let mut ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        let l = &mut ledgers[slot(tier)];
        let now = Instant::now();
        l.spent.push_back((now, tokens));
        // The same spend counts against the daily ceiling, which is pruned on a
        // 24-hour window rather than a one-minute one.
        l.day.push_back((now, tokens));
    }

    /// Block until `cost` tokens fit inside this model's budget.
    ///
    /// Returns the reservation, which the caller must hand to [`settle`] with
    /// the real figure once the call returns.
    ///
    /// [`settle`]: Self::settle
    pub async fn acquire(&self, tier: ModelTier, cost: u32, task: &str) -> Reservation {
        let wait = self.delay_for(tier, cost);
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
        self.record(tier, cost);
        Reservation { tier, cost }
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
        if !self.enabled() || actual == reservation.cost {
            return;
        }
        let mut ledgers = self.ledgers.lock().unwrap_or_else(|e| e.into_inner());
        let l = &mut ledgers[slot(reservation.tier)];
        // Newest first: our own reservation is the most recent entry of that
        // size, and matching the newest keeps concurrent callers from
        // correcting each other's.
        if let Some(e) = l
            .spent
            .iter_mut()
            .rev()
            .find(|(_, n)| *n == reservation.cost)
        {
            e.1 = actual;
        }
        // The daily ledger holds the same reservation and needs the same
        // correction; leaving it would drift the day's total high and stop the
        // newsroom well before the real ceiling.
        if let Some(e) = l.day.iter_mut().rev().find(|(_, n)| *n == reservation.cost) {
            e.1 = actual;
        }
    }
}

/// A reserved slice of the token budget, to be reconciled by
/// [`Pacer::settle`].
#[must_use = "a reservation that is never settled over-counts the budget"]
#[derive(Debug, Clone, Copy)]
pub struct Reservation {
    tier: ModelTier,
    cost: u32,
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
        assert_eq!(p.delay_for(ModelTier::Fast, 1_000), Duration::ZERO);
    }

    #[test]
    fn spending_the_minute_forces_a_wait() {
        let p = Pacer::new(8_000);
        // 0.9 * 8000 = 7200 usable.
        p.record(ModelTier::Fast, 7_000);
        assert_eq!(
            p.delay_for(ModelTier::Fast, 100),
            Duration::ZERO,
            "still fits"
        );
        assert!(
            p.delay_for(ModelTier::Fast, 1_000) > Duration::ZERO,
            "should wait"
        );
    }

    #[test]
    fn a_request_bigger_than_the_whole_budget_is_let_through() {
        // Otherwise it waits forever for room that can never exist.
        let p = Pacer::new(8_000);
        p.record(ModelTier::Fast, 7_000);
        assert_eq!(p.delay_for(ModelTier::Fast, 50_000), Duration::ZERO);
    }

    #[test]
    fn a_zero_limit_disables_pacing() {
        let p = Pacer::new(0);
        assert!(!p.enabled());
        p.record(ModelTier::Fast, 1_000_000);
        assert_eq!(p.delay_for(ModelTier::Fast, 1_000_000), Duration::ZERO);
    }

    #[test]
    fn the_wait_never_exceeds_the_window() {
        let p = Pacer::new(8_000);
        p.record(ModelTier::Fast, 7_200);
        assert!(p.delay_for(ModelTier::Fast, 7_200) <= WINDOW);
    }

    #[tokio::test]
    async fn an_overestimate_is_given_back() {
        // The estimate must include the full max_tokens ceiling; the reply is
        // usually a fraction of it. Without a refund the budget drains four
        // times faster than the tier requires.
        let p = Pacer::new(8_000);
        let r = p.acquire(ModelTier::Fast, 2_500, "t").await;
        assert!(
            p.delay_for(ModelTier::Fast, 5_000) > Duration::ZERO,
            "reserved, so tight"
        );
        p.settle(r, 500);
        assert_eq!(
            p.delay_for(ModelTier::Fast, 5_000),
            Duration::ZERO,
            "refund should leave room again"
        );
    }

    #[tokio::test]
    async fn an_underestimate_is_charged_in_full() {
        // The correction has to work in both directions, or a run of
        // longer-than-expected replies silently blows the budget.
        let p = Pacer::new(8_000);
        let r = p.acquire(ModelTier::Fast, 500, "t").await;
        p.settle(r, 7_000);
        assert!(
            p.delay_for(ModelTier::Fast, 1_000) > Duration::ZERO,
            "should now be tight"
        );
    }

    #[tokio::test]
    async fn settling_touches_only_its_own_reservation() {
        let p = Pacer::new(20_000);
        let a = p.acquire(ModelTier::Fast, 1_000, "a").await;
        let _b = p.acquire(ModelTier::Fast, 1_000, "b").await;
        p.settle(a, 10);
        // One of the two 1,000s became 10; the other must be untouched.
        let ledgers = p.ledgers.lock().unwrap();
        let spent = &ledgers[slot(ModelTier::Fast)].spent;
        let total: u32 = spent.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 1_010, "settle adjusted more than one entry");
    }

    /// Measured against Groq: with identical ~977-token prompts, gpt-oss-120b
    /// fell to 7023 of 8000 while gpt-oss-20b stayed at 8000. The buckets are
    /// per model, so draining one must not stall the other — a single shared
    /// budget throttled all the cheap Fast-tier traffic behind the Skein.
    #[test]
    fn draining_one_model_does_not_stall_another() {
        let p = Pacer::new(8_000);
        p.record(ModelTier::Top, 7_200);
        assert!(
            p.delay_for(ModelTier::Top, 2_000) > Duration::ZERO,
            "the drained tier should wait"
        );
        assert_eq!(
            p.delay_for(ModelTier::Fast, 2_000),
            Duration::ZERO,
            "a different model has its own budget"
        );
    }

    /// The provider's figure is per model too, so an observation on one tier
    /// must not silently license spending on another.
    #[test]
    fn an_observation_applies_only_to_its_own_model() {
        let p = Pacer::new(8_000);
        p.observe(ModelTier::Top, Some(10), Some(Duration::from_secs(30)));
        assert!(p.delay_for(ModelTier::Top, 5_000) > Duration::ZERO);
        assert_eq!(p.delay_for(ModelTier::Mid, 5_000), Duration::ZERO);
    }

    #[test]
    fn estimates_scale_with_the_prompt() {
        let small = estimate_tokens("sys", "hi", 100);
        let big = estimate_tokens("sys", &"word ".repeat(4_000), 100);
        assert!(big > small * 10, "estimate should track prompt size");
    }
}

#[cfg(test)]
mod oversize_tests {
    use super::*;

    /// A request bigger than the whole minute is sent unpaced — it has to be,
    /// or it waits for room that cannot exist. The danger is that this looks
    /// identical to working correctly: pacing reports enabled, and every call
    /// silently bypasses it. Callers must size requests under the budget.
    #[test]
    fn an_oversize_request_is_not_scheduled() {
        let p = Pacer::new(8_000);
        // 0.9 * 8000 = 7200 usable.
        assert_eq!(
            p.delay_for(ModelTier::Fast, 8_000),
            Duration::ZERO,
            "cannot wait for room that will never exist"
        );
        // And the fix for that is on the caller's side: keep under the budget.
        p.record(ModelTier::Fast, 7_000);
        assert!(
            p.delay_for(ModelTier::Fast, 5_700) > Duration::ZERO,
            "a request that fits must actually be scheduled"
        );
    }
}

#[cfg(test)]
mod request_quota_tests {
    use super::*;

    /// On Groq's free tier the request quota binds long before the token one:
    /// 1,000 calls a day against ~11.5 million tokens. Pacing tokens alone let
    /// the worker stall on a limit it had never looked at.
    #[tokio::test]
    async fn an_exhausted_request_quota_forces_a_wait() {
        let p = Pacer::new(8_000);
        p.observe_requests(ModelTier::Fast, Some(0), Some(Duration::from_secs(45)));
        assert!(
            p.delay_for(ModelTier::Fast, 10) > Duration::ZERO,
            "no requests left, so a tiny call must still wait"
        );
    }

    #[tokio::test]
    async fn requests_still_available_do_not_delay_a_small_call() {
        let p = Pacer::new(8_000);
        p.observe_requests(ModelTier::Fast, Some(500), Some(Duration::from_secs(45)));
        assert_eq!(p.delay_for(ModelTier::Fast, 10), Duration::ZERO);
    }

    /// Per model here too — draining the Top tier's calls must not block Fast.
    #[tokio::test]
    async fn the_request_quota_is_tracked_per_model() {
        let p = Pacer::new(8_000);
        p.observe_requests(ModelTier::Top, Some(0), Some(Duration::from_secs(45)));
        assert_eq!(p.delay_for(ModelTier::Fast, 10), Duration::ZERO);
    }
}

#[cfg(test)]
mod daily_budget_tests {
    use super::*;

    /// The limit that actually stopped the newsroom, and the only one no
    /// header reports. Groq's per-minute figures read 8000/8000 with a 1ms
    /// reset while the daily allowance was 98% spent; it appears solely in the
    /// body of the 429. If we do not count it ourselves, the first sign is a
    /// request for a thirteen-minute wait, repeated for the rest of the day.
    #[test]
    fn a_spent_daily_allowance_stops_further_calls() {
        let p = Pacer::with_daily(8_000, 200_000);
        p.record(ModelTier::Fast, 196_000);
        assert!(
            p.delay_for(ModelTier::Fast, 5_600) > Duration::ZERO,
            "should hold rather than take a 429"
        );
        let (used, limit) = p.day_usage(ModelTier::Fast);
        assert_eq!((used, limit), (196_000, 200_000));
    }

    #[test]
    fn the_daily_ceiling_is_per_model_as_well() {
        let p = Pacer::with_daily(8_000, 200_000);
        p.record(ModelTier::Fast, 199_000);
        assert_eq!(p.delay_for(ModelTier::Top, 5_600), Duration::ZERO);
    }

    #[tokio::test]
    async fn settling_corrects_the_day_not_just_the_minute() {
        // Otherwise the day drifts high on every over-reservation and the
        // newsroom stops well short of the real ceiling.
        let p = Pacer::with_daily(8_000, 200_000);
        let r = p.acquire(ModelTier::Fast, 5_000, "t").await;
        p.settle(r, 800);
        assert_eq!(p.day_usage(ModelTier::Fast).0, 800);
    }

    #[test]
    fn zero_disables_the_daily_ceiling() {
        let p = Pacer::with_daily(8_000, 0);
        p.record(ModelTier::Fast, 10_000_000);
        // Only the per-minute rule applies; a small call is unaffected by the
        // huge historical total once the minute window has moved on.
        assert_eq!(p.day_usage(ModelTier::Fast).1, 0);
    }
}

#[cfg(test)]
mod cooldown_tests {
    use super::*;

    /// One refusal should stop the rest of the pass, not be rediscovered by
    /// every agent in it.
    ///
    /// Measured on production before this existed: 83 refusals in six hours, 69
    /// saying "retry in 300s", 28 of 28 Gander runs failed and 29 of 32
    /// Gosling. The provider had already said on the first one that nothing
    /// would work for five minutes.
    #[test]
    fn a_refusal_cools_the_whole_tier() {
        let p = Pacer::new(8_000);
        assert!(p.cooling_for(ModelTier::Top).is_none(), "starts clear");

        p.cooling(ModelTier::Top, Duration::from_secs(300));
        let left = p.cooling_for(ModelTier::Top).expect("now cooling");
        assert!(left.as_secs() > 280 && left.as_secs() <= 300, "{left:?}");
    }

    #[test]
    fn the_other_tiers_are_unaffected() {
        // Tiers have separate allowances. A top-tier refusal says nothing about
        // whether a cheap triage call would go through, and treating it as
        // global would idle the newsroom on the strength of one expensive call.
        let p = Pacer::new(8_000);
        p.cooling(ModelTier::Top, Duration::from_secs(300));
        assert!(p.cooling_for(ModelTier::Fast).is_none());
        assert!(p.cooling_for(ModelTier::Mid).is_none());
    }

    #[test]
    fn an_outage_length_wait_is_capped() {
        // A provider asking for an hour is reporting an outage. Sitting out an
        // hour of passes on its say-so turns a bad twenty minutes into a bad
        // morning; the next pass can find out cheaply instead.
        let p = Pacer::new(8_000);
        p.cooling(ModelTier::Mid, Duration::from_secs(3_600));
        let left = p.cooling_for(ModelTier::Mid).expect("cooling");
        assert!(left.as_secs() <= 600, "waited {left:?} on one refusal");
    }
}
