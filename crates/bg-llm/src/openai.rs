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
    fn spec_for(&self, tier: ModelTier) -> ModelSpec {
        if self.is_local {
            pricing::LOCAL
        } else {
            pricing::openai_spec(tier)
        }
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

    /// `/flock` publishes the cost ledger as fact, so a locally served model
    /// must cost nothing there. Pricing an Ollama call at OpenAI's rates would
    /// put an invented number on the one page whose whole premise is that its
    /// numbers are real.
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
