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
fn parse_retry_hint(body: &str) -> Option<f64> {
    let i = body.find("try again in")? + "try again in".len();
    let rest = body[i..].trim_start();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().ok()
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

        let mut body = json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });

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
                .clamp(1.0, 300.0);
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
                    req.max_tokens
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
        // Nothing to read: the caller falls back to its own default.
        assert_eq!(parse_retry_hint("slow down"), None);
        assert_eq!(parse_retry_hint(""), None);
        assert_eq!(parse_retry_hint("try again in soon"), None);
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
