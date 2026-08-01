//! Anthropic Messages API provider.
//!
//! Raw HTTP rather than an SDK: there is no official Anthropic Rust SDK, and
//! the Messages API surface we need is one POST.

use crate::{http_client, pricing, Completion, LlmError, LlmProvider, ModelSpec, Request, Result};
use async_trait::async_trait;
use bg_core::domain::ModelTier;
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

const API_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
    /// Per-tier model overrides from `BG_MODEL_FAST` / `_MID` / `_TOP`.
    overrides: [Option<String>; 3],
}

impl AnthropicProvider {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| LlmError::NotConfigured {
                provider: "anthropic",
                reason: "ANTHROPIC_API_KEY is unset or empty".into(),
            })?;
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".into())
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            api_key,
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

    fn resolved_model(&self, tier: ModelTier) -> String {
        let idx = match tier {
            ModelTier::Fast | ModelTier::None => 0,
            ModelTier::Mid => 1,
            ModelTier::Top => 2,
        };
        self.overrides[idx]
            .clone()
            .unwrap_or_else(|| pricing::anthropic_spec(tier).id.to_string())
    }
}

// -- wire types -------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    stop_details: Option<StopDetails>,
    #[serde(default)]
    model: Option<String>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct StopDetails {
    #[serde(default)]
    category: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    /// Thinking blocks arrive alongside text on models where thinking is on by
    /// default. They are skipped, not treated as the answer.
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        #[allow(dead_code)]
        thinking: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn spec(&self, tier: ModelTier) -> ModelSpec {
        pricing::anthropic_spec(tier)
    }

    async fn complete(&self, req: &Request) -> Result<Completion> {
        let spec = pricing::anthropic_spec(req.tier);
        let model = self.resolved_model(req.tier);

        let mut body = json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": [{ "role": "user", "content": req.user }],
        });

        // Sampling parameters were REMOVED on the frontier models — sending
        // `temperature` to Opus 5 or Sonnet 5 is a hard 400, not a warning.
        // Agents all carry a temperature, so it is filtered here rather than
        // at every call site.
        if spec.sampling {
            body["temperature"] = json!(req.temperature);
        }

        // Structured output. `output_config.format` is the current parameter;
        // the older top-level `output_format` is deprecated API-wide.
        if let Some(schema) = &req.json_schema {
            body["output_config"] = json!({
                "format": { "type": "json_schema", "schema": schema }
            });
        }

        let started = std::time::Instant::now();
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                provider: "anthropic",
                status: status.as_u16(),
                body: body.chars().take(500).collect(),
            });
        }

        let parsed: MessagesResponse = resp.json().await?;
        let latency_ms = started.elapsed().as_millis() as u32;

        // A safety decline is a successful HTTP 200 with `stop_reason:
        // "refusal"` and possibly empty content — checked before reading
        // content, or an empty array would surface as a confusing parse error.
        if parsed.stop_reason.as_deref() == Some("refusal") {
            return Err(LlmError::Refused {
                category: parsed
                    .stop_details
                    .and_then(|d| d.category)
                    .unwrap_or_else(|| "unspecified".into()),
            });
        }

        let text = parsed
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        if text.trim().is_empty() {
            return Err(LlmError::BadJson {
                detail: format!(
                    "no text content (stop_reason={:?})",
                    parsed.stop_reason.as_deref().unwrap_or("none")
                ),
                raw: String::new(),
            });
        }

        // Truncation produces invalid JSON downstream with a confusing message;
        // name the real cause here instead.
        if parsed.stop_reason.as_deref() == Some("max_tokens") && req.json_schema.is_some() {
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

        let cost = pricing::cost_usd(&spec, parsed.usage.input_tokens, parsed.usage.output_tokens);
        debug!(
            task = %req.task, %model, latency_ms,
            in_tok = parsed.usage.input_tokens, out_tok = parsed.usage.output_tokens,
            cost = %cost, "anthropic completion"
        );

        Ok(Completion {
            text,
            provider: "anthropic".into(),
            model: parsed.model.unwrap_or(model),
            prompt_tokens: parsed.usage.input_tokens,
            completion_tokens: parsed.usage.output_tokens,
            cost_usd: cost,
            latency_ms,
        })
    }

    async fn health(&self) -> Result<()> {
        // One-token ping. Cheaper than a real call and exercises auth.
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&json!({
                "model": pricing::ANTHROPIC_FAST.id,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }],
            }))
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(LlmError::Api {
                provider: "anthropic",
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
    fn thinking_blocks_are_skipped_and_text_blocks_are_joined() {
        let raw = r#"{
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world"}
            ],
            "stop_reason": "end_turn",
            "model": "claude-opus-5",
            "usage": {"input_tokens": 10, "output_tokens": 2}
        }"#;
        let parsed: MessagesResponse = serde_json::from_str(raw).unwrap();
        let text = parsed
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn unknown_block_types_do_not_break_parsing() {
        // Forward compatibility: a new block type must not fail the whole response.
        let raw = r#"{
            "content": [{"type": "some_future_block"}, {"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }"#;
        let parsed: MessagesResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.content.len(), 2);
    }

    #[test]
    fn a_refusal_response_parses_with_its_category() {
        let raw = r#"{
            "content": [],
            "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": "cyber"},
            "usage": {"input_tokens": 5, "output_tokens": 0}
        }"#;
        let parsed: MessagesResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.stop_reason.as_deref(), Some("refusal"));
        assert_eq!(
            parsed.stop_details.unwrap().category.as_deref(),
            Some("cyber")
        );
    }

    #[test]
    fn missing_stop_details_on_a_refusal_is_tolerated() {
        // stop_details is documented as possibly absent even on a refusal.
        let raw = r#"{"content": [], "stop_reason": "refusal", "usage": {"input_tokens": 1, "output_tokens": 0}}"#;
        let parsed: MessagesResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.stop_details.is_none());
    }
}
