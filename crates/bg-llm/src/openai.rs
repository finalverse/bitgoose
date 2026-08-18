//! OpenAI-compatible provider.
//!
//! One implementation covers OpenAI, Together, vLLM, LM Studio and Ollama —
//! they all speak `/chat/completions`. That is the point of having it in the
//! chain: it is a genuinely independent second path, including a fully local
//! one, rather than a second endpoint at the same vendor.

use crate::{http_client, pricing, Completion, LlmError, LlmProvider, ModelSpec, Request, Result};
use async_trait::async_trait;
use bg_core::domain::ModelTier;
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

pub struct OpenAiProvider {
    /// Serving from localhost, so calls are free. See `pricing::LOCAL`.
    is_local: bool,
    api_key: String,
    base_url: String,
    http: reqwest::Client,
    overrides: [Option<String>; 3],
}

/// Pull "try again in 38.6025s" out of a rate-limit message.
///
/// Providers that omit Retry-After often still say the wait in prose. Reading
/// it beats guessing: waiting too little burns another attempt against the same
/// budget, and waiting too long stalls the pass.
///
/// The wait carries the same `2m52.8s` shape as the reset headers, so it is
/// read with the same parser. Doing it by hand here was a real outage: taking
/// digits until the first non-digit turned the daily limit's
/// `try again in 48m29.952s` into **48 seconds**, so the newsroom spent a
/// 48-minute lockout asking once a minute and being refused every time.
fn parse_retry_hint(body: &str) -> Option<f64> {
    let i = body.find("try again in")? + "try again in".len();
    let rest = body[i..].trim_start();
    // Up to the end of the duration: digits, a decimal point, and the unit
    // letters `m` and `s`. Stops at the space before "Need more tokens?".
    let span: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'm' || *c == 's')
        .collect();
    // ...but not the full stop that ends the sentence: `48m29.952s.` is not a
    // duration, and `parse_reset` is right to reject it.
    parse_reset(span.trim_end_matches('.')).map(|d| d.as_secs_f64())
}

fn header_u32(h: &reqwest::header::HeaderMap, name: &str) -> Option<u32> {
    h.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Parse Groq's rate-limit reset format: `577ms`, `7.66s`, `2m52.8s`.
///
/// Not a plain number of seconds — the unit varies with magnitude, and reading
/// `2m52.8s` as 2 seconds would be worse than not reading it at all.
fn header_duration(h: &reqwest::header::HeaderMap, name: &str) -> Option<std::time::Duration> {
    parse_reset(h.get(name)?.to_str().ok()?)
}

fn parse_reset(v: &str) -> Option<std::time::Duration> {
    let v = v.trim();
    if let Some(ms) = v.strip_suffix("ms") {
        return ms
            .parse::<f64>()
            .ok()
            .map(std::time::Duration::from_secs_f64)
            .map(|d| d / 1000);
    }
    let (mins, rest) = match v.split_once('m') {
        Some((m, r)) if !m.is_empty() && m.chars().all(|c| c.is_ascii_digit()) => {
            (Some(m.parse::<f64>().ok()?), r)
        }
        _ => (None, v),
    };
    let secs = rest.strip_suffix('s').unwrap_or(rest);
    let secs = if secs.is_empty() {
        // Empty is only meaningful as the tail of something like `1m`. On its
        // own it means we understood nothing, and answering "zero" would say
        // "retry immediately" — the worst possible reading of a header we
        // failed to parse.
        None
    } else {
        Some(secs.parse::<f64>().ok()?)
    };
    match (mins, secs) {
        (None, None) => None,
        (m, s) => Some(std::time::Duration::from_secs_f64(
            m.unwrap_or(0.0) * 60.0 + s.unwrap_or(0.0),
        )),
    }
}

/// A non-negative price per million tokens from the environment.
///
/// A negative or unparseable value is ignored rather than clamped: it means the
/// operator meant something we did not understand, and guessing at that is how
/// a wrong number reaches a published ledger.
fn env_price(key: &str) -> Option<f64> {
    std::env::var(key)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

impl OpenAiProvider {
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into())
            .trim_end_matches('/')
            .to_string();
        // Local servers ignore the key but the header must still be present;
        // defaulting keeps an Ollama setup from needing a meaningless value.
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        let is_local = base_url.contains("localhost") || base_url.contains("127.0.0.1");
        if api_key.trim().is_empty() && !is_local {
            return Err(LlmError::NotConfigured {
                provider: "openai",
                reason: "OPENAI_API_KEY is unset and OPENAI_BASE_URL is not local".into(),
            });
        }
        Ok(Self {
            is_local,
            api_key: if api_key.is_empty() {
                "local".into()
            } else {
                api_key
            },
            base_url,
            http: http_client(),
            overrides: [
                std::env::var("BG_MODEL_FAST")
                    .ok()
                    .filter(|s| !s.is_empty()),
                std::env::var("BG_MODEL_MID").ok().filter(|s| !s.is_empty()),
                std::env::var("BG_MODEL_TOP").ok().filter(|s| !s.is_empty()),
            ],
        })
    }

    /// Pricing for a tier. A locally served model is free, and saying
    /// otherwise would put a fabricated figure in the published cost ledger.
    /// Pricing for a tier.
    ///
    /// This provider fronts four quite different things — OpenAI itself, a
    /// model on localhost, a free tier like Groq or Cerebras, and any other
    /// OpenAI-compatible host — and only the first has prices we actually know.
    /// Applying OpenAI's table to all of them puts invented figures in the cost
    /// ledger that `/flock` publishes as fact.
    ///
    /// So: localhost is free, `api.openai.com` uses the real table, and
    /// anything else is priced from `BG_LLM_PRICE_IN` / `BG_LLM_PRICE_OUT` (USD
    /// per million tokens) if the operator sets them, or recorded at zero if
    /// not. Zero is the honest default for an unknown endpoint — under-reporting
    /// a figure nobody supplied is better than fabricating one, and the model
    /// name in the ledger still says exactly what ran.
    fn spec_for(&self, tier: ModelTier) -> ModelSpec {
        if self.is_local {
            return pricing::LOCAL;
        }
        if self.base_url.contains("api.openai.com") {
            return pricing::openai_spec(tier);
        }
        let mut spec = pricing::LOCAL;
        if let Some(v) = env_price("BG_LLM_PRICE_IN") {
            spec.input_per_mtok = v;
        }
        if let Some(v) = env_price("BG_LLM_PRICE_OUT") {
            spec.output_per_mtok = v;
        }
        spec
    }

    fn resolved_model(&self, tier: ModelTier) -> String {
        let idx = match tier {
            ModelTier::Fast | ModelTier::None => 0,
            ModelTier::Mid => 1,
            ModelTier::Top => 2,
        };
        self.overrides[idx]
            .clone()
            .unwrap_or_else(|| pricing::openai_spec(tier).id.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    /// Some servers surface a refusal in its own field rather than as content.
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

/// Trim a reservation so the request can physically be admitted.
///
/// The provider charges its rate limit for **what you reserve**, not what you
/// use: a 1,080-token prompt asking for 8,000 output is billed as 9,080 against
/// an 8,000-per-minute window, and comes back
///
///   HTTP 413: Request too large … Limit 8000, Requested 9080
///
/// every time, forever. Scribe asked for 8,000 and so the Desk — the original
/// reporting the whole site is for — had never once published. No amount of
/// waiting fixes that; the request cannot fit, so it must be made smaller.
///
/// Leaves a margin because the four-chars-per-token estimate is approximate and
/// erring low here costs the whole call. Never trims below [`MIN_OUTPUT`]: a
/// reply squeezed into a hundred tokens is a truncation, which for a structured
/// response fails validation and wastes the call just as completely.
fn fits_the_window(system: &str, user: &str, asked: u32) -> u32 {
    let window = std::env::var("BG_TPM_WINDOW")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_TPM_WINDOW);
    let prompt = (system.len() + user.len()) as u32 / 4;
    let room = window
        .saturating_sub(prompt)
        .saturating_sub(RESERVATION_MARGIN);
    asked.min(room.max(MIN_OUTPUT))
}

/// The per-minute token window a single request has to fit inside.
///
/// Groq's free tier. Overridable because it is a property of the account, not
/// of the code, and a paid tier moves it.
const DEFAULT_TPM_WINDOW: u32 = 8_000;

/// Slack for the estimate being wrong in the expensive direction.
const RESERVATION_MARGIN: u32 = 400;

/// Below this a structured reply truncates, which fails validation anyway.
const MIN_OUTPUT: u32 = 512;

/// How hard a reasoning model should think before answering.
///
/// `gpt-oss`, o-series and the Qwen "thinking" models emit a private chain of
/// thought that is billed as **output tokens** and then discarded. Measured on
/// the live key with a four-headline triage prompt:
///
/// | | total tokens | reasoning emitted |
/// |---|---|---|
/// | provider default | 393 | 1,094 chars |
/// | `low` | 184 | 148 chars |
///
/// Same prompt, same usable answer, **53% of the tokens** — and on
/// completion-heavy work like a twenty-item triage batch the gap is wider,
/// because the reasoning grows with the number of items and the answer does
/// not. Left at the default this quietly ate most of a 200,000-token daily
/// allowance: the provider recorded 196,664 used on a day our own ledger
/// counted 118,236, and four of the seven desks went unpublished for want of
/// the difference.
///
/// The Top tier keeps `medium`, because Gander and Sentinel are making
/// editorial judgements where the deliberation is the point. Everything else
/// gets `low`: triage, clustering and distribution copy are classification
/// tasks, and a classifier that deliberates is only an expensive classifier.
///
/// Returns `None` for models that do not take the parameter — sending it to
/// one that does not is a 400, so this is an allowlist rather than a blocklist.
fn reasoning_effort(model: &str, tier: ModelTier) -> Option<String> {
    if !takes_reasoning_effort(model) {
        return None;
    }
    let per_tier = match tier {
        ModelTier::Fast => "BG_REASONING_FAST",
        ModelTier::Mid => "BG_REASONING_MID",
        ModelTier::Top => "BG_REASONING_TOP",
        // Deterministic work never reaches a provider.
        ModelTier::None => return None,
    };
    let chosen = std::env::var(per_tier)
        .or_else(|_| std::env::var("BG_REASONING"))
        .unwrap_or_else(|_| {
            match tier {
                ModelTier::Top => "medium",
                _ => "low",
            }
            .to_string()
        });
    let chosen = chosen.trim().to_lowercase();
    // An explicit escape hatch: `off` restores the provider's own default,
    // which is what to reach for if a tier starts answering badly.
    if chosen.is_empty() || chosen == "off" || chosen == "default" {
        return None;
    }
    Some(chosen)
}

/// Models known to accept `reasoning_effort`.
fn takes_reasoning_effort(model: &str) -> bool {
    let lowered = model.to_lowercase();
    let m = lowered.rsplit('/').next().unwrap_or(&lowered);
    m.starts_with("gpt-oss")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.starts_with("gpt-5")
        || m.contains("thinking")
        || m.starts_with("qwen3")
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn spec(&self, tier: ModelTier) -> ModelSpec {
        self.spec_for(tier)
    }

    async fn complete(&self, req: &Request) -> Result<Completion> {
        let spec = self.spec_for(req.tier);
        let model = self.resolved_model(req.tier);

        let max_tokens = fits_the_window(&req.system, &req.user, req.max_tokens);

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": req.temperature,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });

        if let Some(effort) = reasoning_effort(&model, req.tier) {
            body["reasoning_effort"] = json!(effort);
        }

        if let Some(schema) = &req.json_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": "bitgoose_output", "strict": true, "schema": schema }
            });
        }

        let started = std::time::Instant::now();
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 429 {
            // Prefer the Retry-After header; fall back to the wait embedded in
            // the message body, which is where Groq puts it.
            let header = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<f64>().ok());
            let body = resp.text().await.unwrap_or_default();
            let secs = header
                .or_else(|| parse_retry_hint(&body))
                .unwrap_or(20.0)
                // Up to an hour, not five minutes. The per-minute limit asks
                // for tens of seconds, but the *daily* one asks for the rest of
                // the day — "Limit 200000, Used 196664 … try again in 48m29s".
                // Clamping that to 300s did not shorten the wait, it just meant
                // we spent the next 48 minutes asking every five and being told
                // the same thing.
                .clamp(1.0, 3600.0);
            return Err(LlmError::RateLimited {
                provider: "openai",
                retry_after: std::time::Duration::from_secs_f64(secs),
            });
        }
        if !status.is_success() {
            return Err(LlmError::Api {
                provider: "openai",
                status: status.as_u16(),
                body: resp
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect(),
            });
        }

        // Read the budget headers before consuming the body.
        let rate_remaining_tokens = header_u32(resp.headers(), "x-ratelimit-remaining-tokens");
        let rate_reset = header_duration(resp.headers(), "x-ratelimit-reset-tokens");
        let rate_remaining_requests = header_u32(resp.headers(), "x-ratelimit-remaining-requests");
        let rate_reset_requests = header_duration(resp.headers(), "x-ratelimit-reset-requests");

        let parsed: ChatResponse = resp.json().await?;
        let latency_ms = started.elapsed().as_millis() as u32;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::BadJson {
                detail: "response contained no choices".into(),
                raw: String::new(),
            })?;

        if let Some(r) = choice.message.refusal {
            return Err(LlmError::Refused { category: r });
        }
        if choice.finish_reason.as_deref() == Some("content_filter") {
            return Err(LlmError::Refused {
                category: "content_filter".into(),
            });
        }

        let text = choice.message.content.unwrap_or_default();
        if text.trim().is_empty() {
            return Err(LlmError::BadJson {
                detail: format!("empty content (finish_reason={:?})", choice.finish_reason),
                raw: String::new(),
            });
        }
        if choice.finish_reason.as_deref() == Some("length") && req.json_schema.is_some() {
            return Err(LlmError::BadJson {
                detail: format!(
                    "hit max_tokens ({}); structured output truncated",
                    max_tokens
                ),
                raw: text.chars().take(200).collect(),
            });
        }

        if let Some(schema) = &req.json_schema {
            let value = serde_json::from_str(&text).map_err(|e| LlmError::BadJson {
                detail: e.to_string(),
                raw: text.chars().take(400).collect(),
            })?;
            crate::schema::validate(&value, schema).map_err(LlmError::SchemaViolation)?;
        }

        let usage = parsed.usage.unwrap_or_default();
        let cost = pricing::cost_usd(&spec, usage.prompt_tokens, usage.completion_tokens);
        debug!(task = %req.task, %model, latency_ms, cost = %cost, "openai completion");

        Ok(Completion {
            text,
            provider: "openai".into(),
            model: parsed.model.unwrap_or(model),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cost_usd: cost,
            latency_ms,
            rate_remaining_tokens,
            rate_reset,
            rate_remaining_requests,
            rate_reset_requests,
        })
    }

    async fn health(&self) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(LlmError::Api {
                provider: "openai",
                status: resp.status().as_u16(),
                body: resp
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standard_response_parses() {
        let raw = r#"{
            "model": "gpt-4o",
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2}
        }"#;
        let p: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(p.choices[0].message.content.as_deref(), Some("hi"));
    }

    #[test]
    fn a_response_without_usage_still_parses() {
        // Ollama and several local servers omit `usage` entirely.
        let raw = r#"{"choices": [{"message": {"content": "hi"}}]}"#;
        let p: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(p.usage.is_none());
        assert_eq!(p.choices.len(), 1);
    }

    #[test]
    fn a_refusal_field_parses() {
        let raw = r#"{"choices": [{"message": {"content": null, "refusal": "I cannot help"}}]}"#;
        let p: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(p.choices[0].message.refusal.is_some());
    }
}

#[cfg(test)]
mod local_pricing_tests {
    use super::*;

    /// Serialises the tests that write `BG_LLM_PRICE_*`.
    ///
    /// Environment variables are process-global and cargo runs tests on
    /// parallel threads, so two tests touching the same variable interleave:
    /// one clears what the other just set, and the failure looks like a pricing
    /// bug. Every test below that mutates the environment takes this first.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// `/flock` publishes the cost ledger as fact, so a locally served model
    /// must cost nothing there. Pricing an Ollama call at OpenAI's rates would
    /// put an invented number on the one page whose whole premise is that its
    /// numbers are real.
    fn provider_at(url: &str) -> OpenAiProvider {
        OpenAiProvider {
            is_local: url.contains("127.0.0.1") || url.contains("localhost"),
            api_key: "k".into(),
            base_url: url.into(),
            http: http_client(),
            overrides: [None, None, None],
        }
    }

    /// The ledger on `/flock` is published as fact, so a price must be known,
    /// declared, or zero — never inferred from a different vendor's table.
    /// Groq omits Retry-After and puts the wait in the message. Reading it
    /// beats guessing: too short burns another attempt against the same budget,
    /// too long stalls the pass.
    #[test]
    fn the_wait_is_read_out_of_the_rate_limit_message() {
        let body = r#"{"error":{"message":"Rate limit reached for model `openai/gpt-oss-20b` on tokens per minute (TPM): Limit 8000, Used 7666, Requested 5481. Please try again in 38.6025s.","type":"tokens"}}"#;
        assert_eq!(parse_retry_hint(body), Some(38.6025));

        assert_eq!(parse_retry_hint("try again in 7s"), Some(7.0));

        // The daily limit, which is where this went wrong: read as 48 seconds,
        // the newsroom hammered a 48-minute lockout once a minute for an hour.
        let tpd = r#"{"error":{"message":"Rate limit reached for model `openai/gpt-oss-20b` in organization `org_x` service tier `on_demand` on tokens per day (TPD): Limit 200000, Used 196664, Requested 10072. Please try again in 48m29.952s. Need more tokens? Upgrade to Dev Tier today at https://console.groq.com/settings/billing","type":"tokens"}}"#;
        assert_eq!(parse_retry_hint(tpd), Some(48.0 * 60.0 + 29.952));

        // Whole minutes, no seconds part.
        assert_eq!(parse_retry_hint("try again in 2m"), Some(120.0));
        assert_eq!(parse_retry_hint("try again in 577ms"), Some(0.577));
        // Nothing to read: the caller falls back to its own default.
        assert_eq!(parse_retry_hint("slow down"), None);
        assert_eq!(parse_retry_hint(""), None);
        assert_eq!(parse_retry_hint("try again in soon"), None);
    }

    #[test]
    fn a_reservation_can_never_exceed_the_window() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard above makes this the only thread touching this.
        unsafe { std::env::remove_var("BG_TPM_WINDOW") }

        // Scribe's real request: ~1,080 tokens of prompt asking for 8,000 out.
        // Requested 9,080 against a window of 8,000 — a permanent HTTP 413, and
        // the reason the Desk had never published.
        let prompt = "x".repeat(1_080 * 4);
        let got = fits_the_window("", &prompt, 8_000);
        assert!(
            got + 1_080 <= DEFAULT_TPM_WINDOW,
            "still too large: {got} + 1080 > {DEFAULT_TPM_WINDOW}"
        );

        // A reservation that already fits is left alone — this must not become
        // a quiet across-the-board truncation of every agent's output.
        assert_eq!(fits_the_window("", "short", 2_000), 2_000);
    }

    #[test]
    fn a_prompt_that_fills_the_window_still_gets_room_to_answer() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard above makes this the only thread touching this.
        unsafe { std::env::remove_var("BG_TPM_WINDOW") }

        // Nothing sensible can be reserved here, but handing back 0 would ask
        // the model for an empty completion rather than fail honestly.
        let huge = "x".repeat(40_000 * 4);
        assert_eq!(fits_the_window("", &huge, 3_000), MIN_OUTPUT);
    }

    #[test]
    fn reasoning_models_are_told_not_to_ruminate() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard above makes this the only thread touching these.
        unsafe {
            for k in [
                "BG_REASONING",
                "BG_REASONING_FAST",
                "BG_REASONING_MID",
                "BG_REASONING_TOP",
            ] {
                std::env::remove_var(k);
            }
        }

        // Classification tiers think as little as the model allows.
        assert_eq!(
            reasoning_effort("openai/gpt-oss-20b", ModelTier::Fast).as_deref(),
            Some("low")
        );
        assert_eq!(
            reasoning_effort("openai/gpt-oss-120b", ModelTier::Mid).as_deref(),
            Some("low")
        );
        // Gander and Sentinel are paid to deliberate.
        assert_eq!(
            reasoning_effort("openai/gpt-oss-120b", ModelTier::Top).as_deref(),
            Some("medium")
        );
        // Deterministic work never reaches a provider.
        assert_eq!(reasoning_effort("openai/gpt-oss-20b", ModelTier::None), None);
    }

    #[test]
    fn models_that_would_reject_the_parameter_never_see_it() {
        // Sending `reasoning_effort` to a model that does not take it is a 400,
        // so an unknown model must be assumed not to take it.
        for m in [
            "llama-3.3-70b-versatile",
            "mixtral-8x7b",
            "claude-sonnet-5",
            "gpt-4o-mini",
        ] {
            assert!(!takes_reasoning_effort(m), "{m} would 400");
        }
        for m in [
            "openai/gpt-oss-20b",
            "gpt-oss-120b",
            "o3-mini",
            "qwen3-32b",
            "some/model-thinking",
        ] {
            assert!(takes_reasoning_effort(m), "{m} accepts it");
        }
    }

    #[test]
    fn a_tier_can_be_given_its_deliberation_back() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard above makes this the only thread touching these.
        unsafe {
            std::env::set_var("BG_REASONING_FAST", "off");
            std::env::set_var("BG_REASONING_MID", "HIGH");
        }
        // `off` means send nothing and let the provider decide — the knob to
        // reach for if a tier starts answering badly.
        assert_eq!(reasoning_effort("openai/gpt-oss-20b", ModelTier::Fast), None);
        assert_eq!(
            reasoning_effort("openai/gpt-oss-120b", ModelTier::Mid).as_deref(),
            Some("high"),
            "the value is normalised, not passed through verbatim"
        );
        // SAFETY: as above.
        unsafe {
            std::env::remove_var("BG_REASONING_FAST");
            std::env::remove_var("BG_REASONING_MID");
        }
    }

    #[test]
    fn only_openai_itself_is_priced_with_openais_table() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard above makes this the only thread touching these.
        unsafe {
            std::env::remove_var("BG_LLM_PRICE_IN");
            std::env::remove_var("BG_LLM_PRICE_OUT");
        }

        let openai = provider_at("https://api.openai.com/v1");
        assert!(
            openai.spec_for(ModelTier::Top).output_per_mtok > 0.0,
            "OpenAI's own endpoint must use the real price table"
        );

        // A free tier (Groq, Cerebras) or any other compatible host: unknown,
        // so zero rather than OpenAI's prices.
        for url in [
            "https://api.groq.com/openai/v1",
            "https://api.cerebras.ai/v1",
            "https://openrouter.ai/api/v1",
        ] {
            let p = provider_at(url);
            assert_eq!(
                p.spec_for(ModelTier::Top).output_per_mtok,
                0.0,
                "{url} must not inherit OpenAI's prices"
            );
        }
    }

    #[test]
    fn an_operator_can_declare_the_real_price() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard above makes this the only thread touching these.
        unsafe {
            std::env::set_var("BG_LLM_PRICE_IN", "0.59");
            std::env::set_var("BG_LLM_PRICE_OUT", "0.79");
        }
        let p = provider_at("https://api.groq.com/openai/v1");
        let s = p.spec_for(ModelTier::Mid);
        assert_eq!(s.input_per_mtok, 0.59);
        assert_eq!(s.output_per_mtok, 0.79);

        // Nonsense is ignored rather than clamped — it means the operator meant
        // something we did not understand.
        unsafe {
            std::env::set_var("BG_LLM_PRICE_IN", "-3");
            std::env::set_var("BG_LLM_PRICE_OUT", "banana");
        }
        let s = provider_at("https://api.groq.com/openai/v1").spec_for(ModelTier::Mid);
        assert_eq!(s.input_per_mtok, 0.0);
        assert_eq!(s.output_per_mtok, 0.0);

        unsafe {
            std::env::remove_var("BG_LLM_PRICE_IN");
            std::env::remove_var("BG_LLM_PRICE_OUT");
        }
    }

    #[test]
    fn local_models_are_never_billed() {
        let local = OpenAiProvider {
            is_local: true,
            api_key: "local".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            http: http_client(),
            overrides: [None, None, None],
        };
        for tier in [ModelTier::Fast, ModelTier::Mid, ModelTier::Top] {
            let s = local.spec_for(tier);
            assert_eq!(s.input_per_mtok, 0.0, "local input tokens must be free");
            assert_eq!(s.output_per_mtok, 0.0, "local output tokens must be free");
        }

        let hosted = OpenAiProvider {
            is_local: false,
            api_key: "sk-test".into(),
            base_url: "https://api.openai.com/v1".into(),
            http: http_client(),
            overrides: [None, None, None],
        };
        assert!(
            hosted.spec_for(ModelTier::Top).output_per_mtok > 0.0,
            "a hosted model must still be billed"
        );
    }
}

#[cfg(test)]
mod reset_header_tests {
    use super::parse_reset;
    use std::time::Duration;

    /// Groq varies the unit with magnitude. Reading `2m52.8s` as 2 seconds
    /// would be worse than not reading the header at all — we would hammer a
    /// limit we had been told, precisely, to wait out.
    #[test]
    fn every_unit_groq_actually_sends_is_understood() {
        assert_eq!(parse_reset("577ms"), Some(Duration::from_millis(577)));
        assert_eq!(parse_reset("7.66s"), Some(Duration::from_secs_f64(7.66)));
        assert_eq!(parse_reset("2m52.8s"), Some(Duration::from_secs_f64(172.8)));
        assert_eq!(parse_reset("1m"), Some(Duration::from_secs(60)));
    }

    #[test]
    fn nonsense_yields_nothing_rather_than_a_wrong_number() {
        assert_eq!(parse_reset(""), None);
        assert_eq!(parse_reset("soon"), None);
    }
}
