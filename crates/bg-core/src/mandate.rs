//! What an agent is permitted to spend, and on what.
//!
//! ## Why this shape
//!
//! CreditChain's primitive for machine spending is a **policy-bound agent
//! wallet**: a budget, a time window, and an allowlist, so "machines transact
//! under rules humans can verify". The Flock is eleven agents already spending
//! real money on inference, metered per run. Those are the same object, and
//! BitGoose had the weaker half of it — one global budget, checked before each
//! stage, with no notion of which agent was spending or what for.
//!
//! That gap is not theoretical. A loop in the Skein could exhaust the day's
//! entire allowance before anything noticed, and every other agent would then
//! fail for the rest of the day with no indication of why. A mandate per agent
//! contains the blast radius to the agent that has the bug.
//!
//! ## Why it is worth denominating in CCC
//!
//! `/flock` publishes what each agent cost. Today that is BitGoose reporting on
//! BitGoose — the reader is asked to take it on trust, which is precisely the
//! thing this site exists not to ask. A mandate is a *public commitment made in
//! advance*: this agent may spend this much, on these tasks, until this time.
//! Settled against CreditChain, the commitment and the receipts are checkable
//! by someone who does not trust us at all.
//!
//! That is the honest reason to put a token near a newsroom. Not to charge
//! readers, not to mint something to sell — to make an accountability claim
//! falsifiable.
//!
//! ## What this module does and does not do
//!
//! It decides and it records. It holds no keys, signs nothing and sends
//! nothing. Settlement is a separate, explicitly authorised step, and **CCC is
//! a test network token with no monetary value** — the numbers here are a unit
//! of account for metering, and the code says so wherever a reader might
//! reasonably assume otherwise.

use crate::domain::{AgentRole, ModelTier};
use serde::{Deserialize, Serialize};

/// CCC is denominated to 18 places, as on any EVM chain. Everything here is in
/// the smallest unit, so no arithmetic touches a float.
pub type Wei = u128;

/// One CCC.
pub const CCC: Wei = 1_000_000_000_000_000_000;

/// How many CCC one million model tokens costs.
///
/// A unit of account, not a price. It exists so a mandate can be written in one
/// currency across providers whose own prices differ by an order of magnitude,
/// and so the number on `/flock` means the same thing next month.
///
/// One CCC per million tokens, chosen so the figures are legible at the scale
/// this newsroom actually runs at. The first attempt used a hundredth of that
/// and every agent on the page read `0.0000` — a mandate nobody can see the
/// state of is not a commitment, it is decoration.
pub const DEFAULT_CCC_PER_MTOK: Wei = CCC;

/// Convert a token count to the metering unit.
pub fn tokens_to_ccc(tokens: u64, ccc_per_mtok: Wei) -> Wei {
    // Multiply first: at 18 decimals there is ample headroom, and dividing
    // first would round every small call to zero and meter nothing at all.
    (tokens as u128).saturating_mul(ccc_per_mtok) / 1_000_000
}

/// Human-readable CCC, for display only.
pub fn format_ccc(w: Wei) -> String {
    let whole = w / CCC;
    let frac = (w % CCC) / (CCC / 10_000); // four places is plenty on a card
    format!("{whole}.{frac:04}")
}

/// A bounded authority to spend, of the kind CreditChain calls a mandate.
///
/// Deliberately not "a balance". A balance answers *can this go through*; a
/// mandate answers *was this within what was agreed*, which is the question a
/// reader of a glass newsroom is actually asking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Mandate {
    pub agent: AgentRole,
    /// Ceiling for the window, in CCC-wei.
    pub budget: Wei,
    /// Spent so far in the current window.
    pub spent: Wei,
    /// Length of the window in seconds. A day, in practice: long enough to
    /// absorb a burst, short enough that a mistake is not permanent.
    pub window_secs: u64,
    /// Unix seconds at which the current window began.
    pub window_started: u64,
    /// Task labels this agent may spend on, matched as prefixes.
    ///
    /// Empty means any task. Present, it is the "allowlist" half of the
    /// primitive: the Skein's mandate covers `skein.` and nothing else, so a
    /// mistake that points it at another stage is refused rather than paid for.
    pub tasks: Vec<String>,
    /// Highest model tier this agent may reach for.
    ///
    /// The cheapest effective guard there is. Tier is roughly a twentyfold
    /// price range, so a triage agent that starts calling the top tier costs
    /// twenty times what it should long before it exhausts a budget.
    pub max_tier: ModelTier,
}

/// Why a spend was refused. Recorded rather than only logged: a mandate that
/// silently declines is as opaque as no mandate at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// Would exceed the window's ceiling.
    OverBudget,
    /// The task is not on this agent's allowlist.
    TaskNotPermitted,
    /// A tier above what this agent is authorised to use.
    TierNotPermitted,
}

impl Refusal {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::OverBudget => "would exceed the agent's budget for this window",
            Self::TaskNotPermitted => "task is outside the agent's mandate",
            Self::TierNotPermitted => "model tier is above the agent's mandate",
        }
    }
}

impl Mandate {
    /// A mandate for one agent, sized to its role.
    pub fn new(agent: AgentRole, budget: Wei, now: u64) -> Self {
        Self {
            agent,
            budget,
            spent: 0,
            window_secs: 86_400,
            window_started: now,
            // Each agent's own prefix. Derived rather than configured so a new
            // role cannot be added with an accidentally empty allowlist, which
            // is the failure that would make the whole mechanism decorative.
            tasks: vec![format!("{}.", agent.as_str())],
            max_tier: agent.tier(),
        }
    }

    /// Roll the window forward if it has elapsed. Idempotent.
    pub fn roll(&mut self, now: u64) {
        if now.saturating_sub(self.window_started) >= self.window_secs {
            // Snap to the window boundary rather than to `now`, so windows do
            // not drift later every time a pass happens to run late.
            let elapsed = now - self.window_started;
            let windows = elapsed / self.window_secs;
            self.window_started += windows * self.window_secs;
            self.spent = 0;
        }
    }

    /// What remains in this window.
    pub fn remaining(&self) -> Wei {
        self.budget.saturating_sub(self.spent)
    }

    /// Whether a spend is within the mandate. Does not record it.
    pub fn check(&self, task: &str, tier: ModelTier, cost: Wei) -> Result<(), Refusal> {
        if !self.tasks.is_empty() && !self.tasks.iter().any(|p| task.starts_with(p.as_str())) {
            return Err(Refusal::TaskNotPermitted);
        }
        if tier > self.max_tier {
            return Err(Refusal::TierNotPermitted);
        }
        if cost > self.remaining() {
            return Err(Refusal::OverBudget);
        }
        Ok(())
    }

    /// Record a spend that has already happened.
    ///
    /// Separate from [`check`] and deliberately unable to fail: the estimate
    /// before a call and the tokens actually returned are different numbers,
    /// and an overrun must land in the ledger rather than being discarded for
    /// not fitting. The next check sees the true figure and refuses then.
    pub fn settle(&mut self, cost: Wei) {
        self.spent = self.spent.saturating_add(cost);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Mandate {
        Mandate::new(AgentRole::Skein, CCC, 1_000_000)
    }

    #[test]
    fn an_agent_may_only_spend_on_its_own_work() {
        let m = m();
        assert!(m.check("skein.analyse", ModelTier::Top, CCC / 10).is_ok());
        assert_eq!(
            m.check("scribe.draft", ModelTier::Top, CCC / 10),
            Err(Refusal::TaskNotPermitted)
        );
    }

    #[test]
    fn a_cheap_agent_cannot_reach_for_an_expensive_model() {
        // The guard that catches a misconfiguration before it costs anything:
        // tier spans roughly twentyfold in price.
        let g = Mandate::new(AgentRole::Gosling, CCC, 0);
        assert_eq!(g.max_tier, ModelTier::Fast);
        assert_eq!(
            g.check("gosling.triage", ModelTier::Top, 1),
            Err(Refusal::TierNotPermitted)
        );
        assert!(g.check("gosling.triage", ModelTier::Fast, 1).is_ok());
    }

    #[test]
    fn the_ceiling_holds() {
        let mut m = m();
        m.settle(CCC - 10);
        assert!(m.check("skein.analyse", ModelTier::Top, 10).is_ok());
        assert_eq!(
            m.check("skein.analyse", ModelTier::Top, 11),
            Err(Refusal::OverBudget)
        );
    }

    #[test]
    fn an_overrun_is_recorded_rather_than_discarded() {
        // The estimate before a call and the tokens it returns are different
        // numbers. Refusing to record the difference would leave the ledger
        // permanently understating what was spent.
        let mut m = m();
        m.settle(CCC * 2);
        assert_eq!(m.remaining(), 0);
        assert!(m.spent > m.budget);
        assert_eq!(
            m.check("skein.analyse", ModelTier::Top, 1),
            Err(Refusal::OverBudget)
        );
    }

    #[test]
    fn the_window_rolls_without_drifting() {
        let mut m = Mandate::new(AgentRole::Skein, CCC, 0);
        m.settle(CCC);
        // Two and a half days later.
        m.roll(86_400 * 2 + 43_200);
        assert_eq!(m.spent, 0);
        // Snapped to a boundary, not to the moment we happened to look.
        assert_eq!(m.window_started, 86_400 * 2);
    }

    #[test]
    fn rolling_early_changes_nothing() {
        let mut m = Mandate::new(AgentRole::Skein, CCC, 0);
        m.settle(CCC / 2);
        m.roll(86_399);
        assert_eq!(m.spent, CCC / 2);
        assert_eq!(m.window_started, 0);
    }

    #[test]
    fn every_role_gets_an_allowlist_that_is_not_empty() {
        // An empty allowlist permits everything, which would make the mechanism
        // decorative for any role someone adds later.
        for role in AgentRole::ALL {
            let m = Mandate::new(*role, CCC, 0);
            assert!(!m.tasks.is_empty(), "{role:?} has no allowlist");
            assert!(m
                .check(&format!("{}.x", role.as_str()), role.tier(), 1)
                .is_ok());
        }
    }

    #[test]
    fn small_calls_are_metered_rather_than_rounded_away() {
        // Dividing before multiplying would meter a 500-token call as zero and
        // the ledger would quietly undercount every cheap stage.
        let c = tokens_to_ccc(500, DEFAULT_CCC_PER_MTOK);
        assert!(c > 0, "500 tokens metered as nothing");
        assert_eq!(tokens_to_ccc(1_000_000, DEFAULT_CCC_PER_MTOK), CCC);
    }

    /// The failure that made the first version pointless: at a hundredth of a
    /// CCC per million tokens, a day's real usage rendered as `0.0000` on every
    /// agent and the bar never moved.
    #[test]
    fn a_typical_day_is_visible_on_the_page() {
        let daily = tokens_to_ccc(18_195, DEFAULT_CCC_PER_MTOK);
        assert_ne!(format_ccc(daily), "0.0000", "a real day rounds to nothing");
        let budget = CCC / 10;
        let pct = (daily * 100) / budget;
        assert!((1..=100).contains(&pct), "bar would sit at {pct}%");
    }

    #[test]
    fn display_does_not_lie_about_small_amounts() {
        assert_eq!(format_ccc(CCC), "1.0000");
        assert_eq!(format_ccc(CCC / 2), "0.5000");
        assert_eq!(format_ccc(0), "0.0000");
    }
}
