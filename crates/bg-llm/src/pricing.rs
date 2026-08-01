//! Model catalogue: tier resolution, capabilities and per-token pricing.
//!
//! Prices are USD per million tokens, current as of 2026-08-01. They live here
//! rather than in a config file because the cost ledger is published on
//! `/flock` — a stale price makes a public number wrong, and a wrong number in
//! code is at least reviewable in a diff.

use bg_core::domain::ModelTier;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    /// USD per million input tokens.
    pub input_per_mtok: f64,
    /// USD per million output tokens.
    pub output_per_mtok: f64,
    /// Whether the model accepts `temperature` / `top_p` / `top_k`.
    ///
    /// The frontier models removed sampling parameters entirely — sending one
    /// is a hard 400, not a warning. Every agent carries a temperature, so the
    /// request builder has to consult this rather than send it blindly.
    pub sampling: bool,
    /// Whether the model supports `output_config.format` structured output.
    pub structured_output: bool,
}

/// Anthropic models, by capability tier.
pub const ANTHROPIC_FAST: ModelSpec = ModelSpec {
    id: "claude-haiku-4-5",
    input_per_mtok: 1.00,
    output_per_mtok: 5.00,
    sampling: true,
    structured_output: true,
};

pub const ANTHROPIC_MID: ModelSpec = ModelSpec {
    id: "claude-sonnet-5",
    input_per_mtok: 3.00,
    output_per_mtok: 15.00,
    sampling: false,
    structured_output: true,
};

pub const ANTHROPIC_TOP: ModelSpec = ModelSpec {
    id: "claude-opus-5",
    input_per_mtok: 5.00,
    output_per_mtok: 25.00,
    sampling: false,
    structured_output: true,
};

/// OpenAI-compatible defaults. Overridable, since this provider also fronts
/// Ollama and other local servers where the model names are arbitrary.
pub const OPENAI_FAST: ModelSpec = ModelSpec {
    id: "gpt-4o-mini",
    input_per_mtok: 0.15,
    output_per_mtok: 0.60,
    sampling: true,
    structured_output: true,
};

pub const OPENAI_MID: ModelSpec = ModelSpec {
    id: "gpt-4o",
    input_per_mtok: 2.50,
    output_per_mtok: 10.00,
    sampling: true,
    structured_output: true,
};

pub const OPENAI_TOP: ModelSpec = ModelSpec {
    id: "gpt-4o",
    input_per_mtok: 2.50,
    output_per_mtok: 10.00,
    sampling: true,
    structured_output: true,
};

pub const STUB: ModelSpec = ModelSpec {
    id: "stub-deterministic",
    input_per_mtok: 0.0,
    output_per_mtok: 0.0,
    sampling: true,
    structured_output: true,
};

pub fn anthropic_spec(tier: ModelTier) -> ModelSpec {
    match tier {
        ModelTier::Top => ANTHROPIC_TOP,
        ModelTier::Mid => ANTHROPIC_MID,
        // A `None`-tier role should never reach the LLM layer; if one does,
        // bill it at the cheapest rate rather than the most expensive.
        ModelTier::Fast | ModelTier::None => ANTHROPIC_FAST,
    }
}

pub fn openai_spec(tier: ModelTier) -> ModelSpec {
    match tier {
        ModelTier::Top => OPENAI_TOP,
        ModelTier::Mid => OPENAI_MID,
        ModelTier::Fast | ModelTier::None => OPENAI_FAST,
    }
}

/// Cost of one call, in USD.
pub fn cost_usd(spec: &ModelSpec, prompt_tokens: u32, completion_tokens: u32) -> Decimal {
    let dollars = (prompt_tokens as f64 / 1_000_000.0) * spec.input_per_mtok
        + (completion_tokens as f64 / 1_000_000.0) * spec.output_per_mtok;
    // Six decimal places matches the NUMERIC(12,6) column; a single cheap call
    // can genuinely cost less than a hundredth of a cent, and truncating those
    // to zero would make the published daily total drift low over thousands of
    // runs.
    Decimal::from_f64(dollars).unwrap_or_default().round_dp(6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn cost_matches_published_rates() {
        // 1M in + 1M out on Opus 5 = $5 + $25.
        assert_eq!(
            cost_usd(&ANTHROPIC_TOP, 1_000_000, 1_000_000),
            Decimal::from_str("30").unwrap()
        );
        // A realistic Scribe call on Sonnet 5: 12k in, 1.5k out.
        // 12000/1e6*3 = 0.036 ; 1500/1e6*15 = 0.0225 → 0.0585
        assert_eq!(
            cost_usd(&ANTHROPIC_MID, 12_000, 1_500),
            Decimal::from_str("0.0585").unwrap()
        );
    }

    #[test]
    fn tiny_calls_do_not_round_to_zero() {
        // 100 in / 50 out on Haiku is a fraction of a cent but must still register.
        let c = cost_usd(&ANTHROPIC_FAST, 100, 50);
        assert!(c > Decimal::ZERO, "sub-cent call rounded away: {c}");
    }

    #[test]
    fn the_stub_is_free() {
        assert_eq!(cost_usd(&STUB, 999_999, 999_999), Decimal::ZERO);
    }

    #[test]
    fn frontier_models_are_marked_as_rejecting_sampling_params() {
        // Opus 5 and Sonnet 5 return 400 if temperature is sent at all.
        assert!(!ANTHROPIC_TOP.sampling);
        assert!(!ANTHROPIC_MID.sampling);
        assert!(
            ANTHROPIC_FAST.sampling,
            "Haiku 4.5 still accepts temperature"
        );
    }
}
