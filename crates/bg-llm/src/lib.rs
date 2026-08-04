//! # bg-llm
//!
//! One trait, three providers, and a cost ledger.
//!
//! Agents never name a model. They ask for a [`ModelTier`] and this crate
//! resolves it per provider, so switching the whole newsroom from Anthropic to
//! a local Ollama is an environment variable rather than a code change.
//!
//! ## The stub provider is not a mock
//!
//! [`stub::StubProvider`] generates deterministic output from the caller's JSON
//! schema. That makes the entire pipeline runnable with no API key and no cost
//! — which is what lets the policy engine, the clustering, the database writes
//! and the rendering all be tested end to end without spending anything or
//! depending on the network.

pub mod anthropic;
pub mod openai;
pub mod pacer;
pub mod pricing;
pub mod schema;
pub mod stub;

use async_trait::async_trait;
use bg_core::domain::ModelTier;
use pricing::ModelSpec;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

pub type Result<T, E = LlmError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("{provider} returned HTTP {status}: {body}")]
    Api {
        provider: &'static str,
        status: u16,
        body: String,
    },

    /// Rate limited, with the wait the provider asked for.
    ///
    /// Distinct from a generic 429 because the response tells us *how long* to
    /// wait, and honouring that is the difference between riding out a free
    /// tier's per-minute budget and failing the whole run. Falling through to
    /// another provider does not help when there is only one.
    #[error("{provider} rate limited; retry in {}s", retry_after.as_secs())]
    RateLimited {
        provider: &'static str,
        retry_after: std::time::Duration,
    },

    #[error("{provider} is not configured: {reason}")]
    NotConfigured {
        provider: &'static str,
        reason: String,
    },

    /// The model declined on safety grounds. A normal HTTP 200 — not a
    /// transport failure — so it is modelled as its own variant rather than
    /// being lumped in with API errors, and it is never retried.
    #[error("model refused the request ({category})")]
    Refused { category: String },

    #[error("could not parse model output as JSON: {detail}")]
    BadJson { detail: String, raw: String },

    #[error("model output did not satisfy the schema: {0}")]
    SchemaViolation(String),

    #[error("every provider in the failover chain failed; last error: {0}")]
    AllProvidersFailed(String),

    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}

impl LlmError {
    /// Whether retrying, or falling through to the next provider, could help.
    ///
    /// A refusal and a schema violation are *decisions*, not outages — retrying
    /// them burns money to get the same answer.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Api { status, .. } => *status == 429 || *status >= 500,
            Self::RateLimited { .. } => true,
            Self::Transport(_) => true,
            Self::Refused { .. }
            | Self::SchemaViolation(_)
            | Self::BadJson { .. }
            | Self::NotConfigured { .. }
            | Self::AllProvidersFailed(_) => false,
        }
    }
}

/// A completion request. Provider-agnostic.
#[derive(Debug, Clone)]
pub struct Request {
    pub system: String,
    pub user: String,
    pub tier: ModelTier,
    pub max_tokens: u32,
    pub temperature: f32,
    /// When set, the provider constrains output to this JSON Schema and the
    /// response is validated against it before returning.
    pub json_schema: Option<serde_json::Value>,
    /// Short label for logs and the run ledger, e.g. `"scribe.draft"`.
    pub task: String,
}

impl Request {
    pub fn new(
        task: impl Into<String>,
        tier: ModelTier,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
            tier,
            // 16k is the SDK-safe ceiling for a non-streaming request; beyond
            // that the connection can time out before the model finishes.
            max_tokens: 16_000,
            temperature: 0.2,
            json_schema: None,
            task: task.into(),
        }
    }

    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.json_schema = Some(schema);
        self
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = t;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: Decimal,
    pub latency_ms: u32,
    /// What the provider says is left of our per-minute token allowance, and
    /// when it refills. Groq returns both on every response, which is strictly
    /// better than our own estimate: it is their accounting, it covers anything
    /// else using the same key, and it needs no guessing about tokenisation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_remaining_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_reset: Option<std::time::Duration>,
}

impl Completion {
    /// Parse the response as JSON. Only meaningful when the request carried a
    /// schema.
    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.text).map_err(|e| LlmError::BadJson {
            detail: e.to_string(),
            raw: self.text.chars().take(400).collect(),
        })
    }

    pub fn parse_into<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.text).map_err(|e| LlmError::BadJson {
            detail: e.to_string(),
            raw: self.text.chars().take(400).collect(),
        })
    }
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Concrete model for a tier, plus its pricing and capabilities.
    fn spec(&self, tier: ModelTier) -> ModelSpec;

    async fn complete(&self, req: &Request) -> Result<Completion>;

    /// Cheap reachability check for `bg doctor`.
    async fn health(&self) -> Result<()>;
}

/// A provider chain with failover.
///
/// How many times to wait out a rate limit on one provider before giving up.
///
/// Three is enough to ride out a per-minute budget without letting a wedged
/// provider stall a pipeline pass indefinitely.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;

/// Longest wait we are willing to sit through. A quota that resets in an hour
/// is an outage to be reported, not slept through.
///
/// Raised from 75s after watching it fail on production. Groq was asking for
/// 91 seconds; we slept 75, retried, were refused again, and did that three
/// times — 225 seconds of waiting that could not have worked, because sleeping
/// *less* than the provider asked for guarantees the next call is refused too.
const MAX_RATE_LIMIT_WAIT: std::time::Duration = std::time::Duration::from_secs(180);

/// Ordered, tried left to right, skipping errors that retrying cannot fix. In
/// practice the chain ends in the stub, so the pipeline degrades to free,
/// offline operation instead of stopping when an upstream is down.
#[derive(Clone)]
pub struct Llm {
    chain: Vec<Arc<dyn LlmProvider>>,
    /// Keeps us inside a per-minute token allowance by waiting *before* a call
    /// rather than absorbing the rejection after it. See [`pacer`].
    pacer: Arc<pacer::Pacer>,
}

impl Llm {
    pub fn new(chain: Vec<Arc<dyn LlmProvider>>) -> Self {
        Self::with_pace(chain, pacer::limit_from_env())
    }

    /// `tokens_per_min` of 0 disables pacing — the right setting for a paid
    /// tier or a local model, where the only limit is the hardware.
    pub fn with_pace(chain: Vec<Arc<dyn LlmProvider>>, tokens_per_min: u32) -> Self {
        assert!(
            !chain.is_empty(),
            "LLM chain must have at least one provider"
        );
        if tokens_per_min > 0 {
            info!(tokens_per_min, "pacing LLM calls to a per-minute budget");
        }
        Self {
            chain,
            pacer: Arc::new(pacer::Pacer::new(tokens_per_min)),
        }
    }

    /// Build from environment: `BG_LLM_PROVIDER` then `BG_LLM_FALLBACK`.
    ///
    /// A provider that cannot be configured (no key) is dropped with a warning
    /// rather than failing startup — a missing OpenAI key should not stop a
    /// deployment that is running on Anthropic.
    pub fn from_env() -> Self {
        let primary = std::env::var("BG_LLM_PROVIDER").unwrap_or_else(|_| "stub".into());
        let fallback = std::env::var("BG_LLM_FALLBACK").unwrap_or_default();

        let mut names: Vec<String> = vec![primary];
        names.extend(
            fallback
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
        names.dedup();

        let mut chain: Vec<Arc<dyn LlmProvider>> = Vec::new();
        for n in &names {
            match n.as_str() {
                "anthropic" => match anthropic::AnthropicProvider::from_env() {
                    Ok(p) => chain.push(Arc::new(p)),
                    Err(e) => warn!(provider = "anthropic", error = %e, "skipping provider"),
                },
                "openai" | "openai_compat" | "ollama" => match openai::OpenAiProvider::from_env() {
                    Ok(p) => chain.push(Arc::new(p)),
                    Err(e) => warn!(provider = "openai", error = %e, "skipping provider"),
                },
                "stub" => chain.push(Arc::new(stub::StubProvider)),
                other => warn!(provider = %other, "unknown provider, ignoring"),
            }
        }

        if chain.is_empty() {
            warn!("no LLM provider could be configured; falling back to the offline stub");
            chain.push(Arc::new(stub::StubProvider));
        }
        info!(
            chain = %chain.iter().map(|p| p.name()).collect::<Vec<_>>().join(" -> "),
            "LLM chain ready"
        );
        Self::with_pace(chain, pacer::limit_from_env())
    }

    pub fn primary(&self) -> &dyn LlmProvider {
        self.chain[0].as_ref()
    }

    pub fn provider_names(&self) -> Vec<&'static str> {
        self.chain.iter().map(|p| p.name()).collect()
    }

    /// Run a request through the chain.
    pub async fn complete(&self, req: &Request) -> Result<Completion> {
        // Spend the minute deliberately. The retry loop below stays as a
        // backstop for when this estimate is wrong or something else is using
        // the same key, but it should now be the exception rather than the
        // mechanism by which we discover the limit.
        let reservation = if self.pacer.enabled() {
            let cost = pacer::estimate_tokens(&req.system, &req.user, req.max_tokens);
            Some(self.pacer.acquire(req.tier, cost, &req.task).await)
        } else {
            None
        };

        let mut last: Option<LlmError> = None;
        for p in &self.chain {
            // Rate limits are waited out on the same provider rather than
            // failed over. A free tier's per-minute token budget is a normal
            // operating condition, not an outage, and the response says exactly
            // how long to wait — moving to a different provider would neither
            // help nor be possible when the chain has one entry.
            let mut attempt = 0u32;
            let outcome = loop {
                match p.complete(req).await {
                    // Only retry when we intend to wait the *full* time asked.
                    // Truncating the wait and trying anyway is what turned one
                    // refusal into three: the provider said 91 seconds, we
                    // slept 75, and of course it refused again.
                    Err(LlmError::RateLimited {
                        provider,
                        retry_after,
                    }) if attempt < MAX_RATE_LIMIT_RETRIES
                        && retry_after <= MAX_RATE_LIMIT_WAIT =>
                    {
                        attempt += 1;
                        warn!(
                            provider, task = %req.task, attempt,
                            wait_s = retry_after.as_secs(), "rate limited; waiting"
                        );
                        tokio::time::sleep(retry_after).await;
                    }
                    other => break other,
                }
            };
            match outcome {
                Ok(c) => {
                    // Give back whatever the estimate over-reserved. The output
                    // ceiling is usually far above the real reply, so without
                    // this the budget drains several times faster than the tier
                    // requires and the newsroom paces itself to a crawl.
                    if let Some(r) = reservation {
                        self.pacer.settle(r, c.prompt_tokens + c.completion_tokens);
                    }
                    // The provider's own meter overrides our estimate.
                    self.pacer
                        .observe(req.tier, c.rate_remaining_tokens, c.rate_reset);
                    return Ok(c);
                }
                Err(e) if e.is_retryable() => {
                    warn!(provider = p.name(), task = %req.task, error = %e, "falling through");
                    last = Some(e);
                }
                // A refusal or a schema violation is the model's answer, not an
                // outage — return it rather than shopping for a provider that
                // says something else.
                Err(e) => return Err(e),
            }
        }
        Err(LlmError::AllProvidersFailed(
            last.map(|e| e.to_string())
                .unwrap_or_else(|| "empty chain".into()),
        ))
    }

    /// Request structured output and deserialize it.
    pub async fn complete_json<T: serde::de::DeserializeOwned>(
        &self,
        req: &Request,
    ) -> Result<(T, Completion)> {
        debug_assert!(req.json_schema.is_some(), "complete_json without a schema");
        let c = self.complete(req).await?;
        let v = c.parse_into::<T>()?;
        Ok((v, c))
    }
}

impl std::fmt::Debug for Llm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Llm")
            .field("chain", &self.provider_names())
            .finish()
    }
}

/// Shared HTTP client for the network-backed providers.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Generous: a top-tier model reasoning over a large claim set can take
        // well over a minute.
        .timeout(std::time::Duration::from_secs(180))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("http client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_and_schema_violations_are_not_retried() {
        assert!(!LlmError::Refused {
            category: "cyber".into()
        }
        .is_retryable());
        assert!(!LlmError::SchemaViolation("missing field".into()).is_retryable());
        assert!(!LlmError::BadJson {
            detail: "x".into(),
            raw: String::new()
        }
        .is_retryable());
    }

    #[test]
    fn rate_limits_and_server_errors_are_retried() {
        assert!(LlmError::Api {
            provider: "anthropic",
            status: 429,
            body: String::new()
        }
        .is_retryable());
        assert!(LlmError::Api {
            provider: "anthropic",
            status: 529,
            body: String::new()
        }
        .is_retryable());
        assert!(!LlmError::Api {
            provider: "anthropic",
            status: 400,
            body: String::new()
        }
        .is_retryable());
    }

    #[tokio::test]
    async fn the_chain_falls_through_to_the_stub() {
        // A provider that always fails with a retryable error, then the stub.
        struct Broken;
        #[async_trait]
        impl LlmProvider for Broken {
            fn name(&self) -> &'static str {
                "broken"
            }
            fn spec(&self, _: ModelTier) -> ModelSpec {
                pricing::STUB
            }
            async fn complete(&self, _: &Request) -> Result<Completion> {
                Err(LlmError::Api {
                    provider: "broken",
                    status: 503,
                    body: "down".into(),
                })
            }
            async fn health(&self) -> Result<()> {
                Ok(())
            }
        }

        let llm = Llm::new(vec![Arc::new(Broken), Arc::new(stub::StubProvider)]);
        let req = Request::new("t", ModelTier::Fast, "sys", "user");
        let out = llm.complete(&req).await.unwrap();
        assert_eq!(out.provider, "stub", "should have fallen through");
    }

    #[tokio::test]
    async fn a_refusal_stops_the_chain_instead_of_shopping_providers() {
        struct Refuser;
        #[async_trait]
        impl LlmProvider for Refuser {
            fn name(&self) -> &'static str {
                "refuser"
            }
            fn spec(&self, _: ModelTier) -> ModelSpec {
                pricing::STUB
            }
            async fn complete(&self, _: &Request) -> Result<Completion> {
                Err(LlmError::Refused {
                    category: "cyber".into(),
                })
            }
            async fn health(&self) -> Result<()> {
                Ok(())
            }
        }

        let llm = Llm::new(vec![Arc::new(Refuser), Arc::new(stub::StubProvider)]);
        let req = Request::new("t", ModelTier::Fast, "sys", "user");
        assert!(matches!(
            llm.complete(&req).await,
            Err(LlmError::Refused { .. })
        ));
    }
}

#[cfg(test)]
mod rate_limit_policy {
    use super::*;

    /// Sleeping less than the provider asked for guarantees the retry is
    /// refused too. Observed live: Groq asked 91s, the cap was 75s, and three
    /// attempts burned 225 seconds without a single one able to succeed.
    #[test]
    fn a_wait_longer_than_we_will_sit_through_is_not_retried() {
        let asked = std::time::Duration::from_secs(91);
        assert!(
            asked <= MAX_RATE_LIMIT_WAIT,
            "91s was a real production figure; the ceiling must accommodate it"
        );

        let too_long = std::time::Duration::from_secs(3_600);
        assert!(
            too_long > MAX_RATE_LIMIT_WAIT,
            "an hour-long quota reset is an outage, not something to sleep through"
        );
    }
}
