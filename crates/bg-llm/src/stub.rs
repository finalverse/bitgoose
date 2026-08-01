//! The offline provider.
//!
//! Not a mock in the usual sense: instead of returning canned strings, it reads
//! the caller's JSON Schema and synthesizes a conforming instance, seeded by a
//! hash of the prompt. That makes it useful for far more than unit tests — the
//! entire Flock pipeline runs on it with no API key, no network and no cost,
//! which is what allows clustering, the policy engine, the database writes and
//! the rendering to be exercised end to end before a single paid token.
//!
//! Two properties matter and are tested below: output always satisfies the
//! schema, and the same prompt always produces the same output.

use crate::{pricing, Completion, LlmProvider, ModelSpec, Request, Result};
use async_trait::async_trait;
use bg_core::domain::ModelTier;
use rust_decimal::Decimal;
use serde_json::{json, Map, Value};

#[derive(Debug, Default, Clone)]
pub struct StubProvider;

/// Deterministic 64-bit hash (FNV-1a) — stable across runs and toolchains,
/// unlike `DefaultHasher`.
fn seed_of(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A tiny xorshift PRNG so successive draws from one seed differ.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn pick(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// First sentence of the prompt's user text, for plausible echoed content.
fn first_sentence(s: &str) -> String {
    let t = s.trim();
    let end = t
        .find(['.', '\n'])
        .map(|i| i + 1)
        .unwrap_or(t.len().min(120));
    let out = t[..end].trim().to_string();
    if out.is_empty() {
        "Placeholder text generated offline by the BitGoose stub provider.".into()
    } else {
        out
    }
}

/// The real subject buried in an agent prompt.
///
/// Agent prompts start with framing ("Source material:", "Verified claims:")
/// before the actual headline. Echoing the framing produces offline pages that
/// read as broken rather than as placeholder, which makes it impossible to
/// judge the site's layout and typography without a live provider. Pulling the
/// genuine headline out gives realistic offline content.
fn subject_of(prompt: &str) -> String {
    for line in prompt.lines() {
        let l = line.trim();
        for prefix in ["Headline: ", "Story: ", "- ", "[0] "] {
            if let Some(rest) = l.strip_prefix(prefix) {
                // Several prompts annotate each line with a bracketed status
                // (`- [Corroborated] ...`); that is scaffolding, not subject.
                let rest = match rest.trim().split_once(']') {
                    Some((tag, after)) if tag.starts_with('[') => after.trim(),
                    _ => rest.trim(),
                };
                if rest.len() > 12 && !rest.starts_with("===") {
                    return rest.to_string();
                }
            }
        }
    }
    first_sentence(prompt)
}

/// Build a value satisfying `schema`.
///
/// `x-stub` hints select what a string field is filled with, so offline output
/// resembles the real shape (a headline looks like a headline) rather than a
/// wall of `lorem`.
pub fn synthesize(schema: &Value, rng: &mut Rng, ctx: &str, depth: usize) -> Value {
    synth(schema, rng, ctx, depth, 0)
}

/// `ordinal` is the element's position within its enclosing array, so index
/// fields (`x-stub: "ordinal"`) echo the input they refer to.

fn synth(schema: &Value, rng: &mut Rng, ctx: &str, depth: usize, ordinal: usize) -> Value {
    if depth > 8 {
        return Value::Null;
    }

    if let Some(branches) = schema.get("anyOf").and_then(|v| v.as_array()) {
        // Prefer the first non-null branch — a schema-conforming null
        // everywhere would exercise nothing downstream.
        let chosen = branches
            .iter()
            .find(|b| b.get("type").and_then(|t| t.as_str()) != Some("null"))
            .unwrap_or(&branches[0]);
        return synth(chosen, rng, ctx, depth + 1, ordinal);
    }

    if let Some(allowed) = schema.get("enum").and_then(|v| v.as_array()) {
        if !allowed.is_empty() {
            if let Some(want) = schema.get("x-stub-enum").and_then(|v| v.as_str()) {
                if let Some(hit) = allowed.iter().find(|v| v.as_str() == Some(want)) {
                    return hit.clone();
                }
            }
            return allowed[rng.pick(allowed.len())].clone();
        }
    }

    let ty = schema
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("string");
    match ty {
        "object" => {
            let mut out = Map::new();
            // Emit exactly the required fields: the smallest conforming object
            // is also the strictest test of downstream code, which must not
            // depend on optional fields being present.
            let required: Vec<String> = schema
                .get("required")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
                for name in &required {
                    if let Some(sub) = props.get(name) {
                        out.insert(name.clone(), synth(sub, rng, ctx, depth + 1, ordinal));
                    }
                }
            }
            Value::Object(out)
        }
        "array" => {
            let items = schema
                .get("items")
                .cloned()
                .unwrap_or_else(|| json!({"type": "string"}));
            // `x-stub-count` means the caller expects one entry per input (a
            // batched agent). Honouring it is what lets offline runs exercise
            // the pipeline at real volume instead of processing 1 item in 25.
            let n = match schema.get("x-stub-count").and_then(|v| v.as_u64()) {
                Some(exact) => exact as usize,
                // Otherwise 1-3: enough to exercise iteration, few enough to
                // keep offline output readable.
                None => 1 + rng.pick(3),
            };
            Value::Array(
                (0..n)
                    .map(|i| synth(&items, rng, ctx, depth + 1, i))
                    .collect(),
            )
        }
        "integer" => match schema.get("x-stub").and_then(|v| v.as_str()) {
            Some("ordinal") => json!(ordinal as i64),
            _ => {
                let lo = schema
                    .get("x-stub-min")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as i64;
                let hi = schema
                    .get("x-stub-max")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(99.0) as i64;
                json!(lo + rng.pick((hi - lo + 1).max(1) as usize) as i64)
            }
        },
        "number" => {
            // Honour an explicit range; otherwise 0.50..=0.99, which suits the
            // confidence fields that dominate this schema surface.
            let lo = schema
                .get("x-stub-min")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            let hi = schema
                .get("x-stub-max")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.99);
            let span = (hi - lo).max(0.0);
            let v = lo + (rng.pick(101) as f64 / 100.0) * span;
            json!((v * 100.0).round() / 100.0)
        }
        "boolean" => json!(rng.next() % 2 == 0),
        "null" => Value::Null,
        _ => {
            let hint = schema.get("x-stub").and_then(|v| v.as_str()).unwrap_or("");
            Value::String(stub_string(hint, ctx, rng, ordinal))
        }
    }
}

fn stub_string(hint: &str, ctx: &str, rng: &mut Rng, ordinal: usize) -> String {
    let subject = subject_of(ctx);
    match hint {
        "headline" => bg_core::text::truncate_words(&subject, 12),
        // Claims are the one field where identical values look obviously wrong:
        // a ledger showing the same sentence three times reads as a rendering
        // bug rather than as placeholder content.
        "claim" if ordinal > 0 => format!(
            "{} (supporting detail {}).",
            bg_core::text::truncate_words(&subject, 10).trim_end_matches('.'),
            ordinal + 1
        ),
        "dek" | "summary" => format!(
            "{} This summary was generated offline by the BitGoose stub provider.",
            bg_core::text::truncate_words(&subject, 22)
        ),
        "slug" => bg_core::slug::slugify(&bg_core::text::truncate_words(&subject, 7)),
        "claim" => format!(
            "{}.",
            bg_core::text::truncate_words(&subject, 14).trim_end_matches('.')
        ),
        "excerpt" => bg_core::text::truncate_words(&subject, 10),
        "body_md" => format!(
            "{}[^c1]\n\nThis story was assembled offline by the BitGoose stub provider, which \
             synthesizes schema-conforming output without calling a model.[^c2] No tokens were \
             billed and no network request was made.\n\nWith a live provider configured, this \
             body is original synthesis across every source listed below.",
            bg_core::text::truncate_words(&subject, 30)
        ),
        "reason" | "note" => "Deterministic offline rationale from the stub provider.".to_string(),
        "asset" => ["BTC", "ETH", "SOL", "XRP"][rng.pick(4)].to_string(),
        _ => bg_core::text::truncate_words(&subject, 25),
    }
}

#[async_trait]
impl LlmProvider for StubProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn spec(&self, _tier: ModelTier) -> ModelSpec {
        pricing::STUB
    }

    async fn complete(&self, req: &Request) -> Result<Completion> {
        let started = std::time::Instant::now();
        let mut rng = Rng(seed_of(&format!(
            "{}|{}|{}",
            req.task, req.system, req.user
        )));

        let text = match &req.json_schema {
            Some(schema) => {
                let v = synthesize(schema, &mut rng, &req.user, 0);
                // Self-check: a stub that emits non-conforming output would
                // send the pipeline chasing a phantom bug.
                crate::schema::validate(&v, schema).map_err(|e| {
                    crate::LlmError::SchemaViolation(format!("stub self-check: {e}"))
                })?;
                serde_json::to_string(&v).unwrap_or_else(|_| "{}".into())
            }
            None => format!(
                "[stub:{}] {}",
                req.task,
                bg_core::text::truncate_words(&first_sentence(&req.user), 40)
            ),
        };

        // Rough token estimate so the ledger and budget logic have realistic
        // numbers to work with offline.
        let prompt_tokens = ((req.system.len() + req.user.len()) / 4) as u32;
        let completion_tokens = (text.len() / 4) as u32;

        Ok(Completion {
            text,
            provider: "stub".into(),
            model: pricing::STUB.id.into(),
            prompt_tokens,
            completion_tokens,
            cost_usd: Decimal::ZERO,
            latency_ms: started.elapsed().as_millis() as u32,
        })
    }

    async fn health(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema as s;
    use bg_core::domain::ModelTier;

    fn draft_schema() -> Value {
        s::object(
            vec![
                ("headline", s::string_hinted("headline", "headline")),
                ("dek", s::string_hinted("dek", "dek")),
                (
                    "claims",
                    s::array(
                        s::object(
                            vec![
                                ("text", s::string_hinted("claim", "claim")),
                                (
                                    "kind",
                                    s::enumeration(&["fact", "figure", "quote", "forecast"], "k"),
                                ),
                                ("confidence", s::number("0-1")),
                                ("source_indices", s::array(s::integer("i"), "sources")),
                            ],
                            &["text", "kind", "confidence", "source_indices"],
                        ),
                        "claims",
                    ),
                ),
            ],
            &["headline", "dek", "claims"],
        )
    }

    #[tokio::test]
    async fn stub_output_always_satisfies_the_schema() {
        let p = StubProvider;
        let schema = draft_schema();
        // Many different prompts — the property must hold for all of them.
        for i in 0..200 {
            let req = Request::new(
                "scribe.draft",
                ModelTier::Mid,
                "sys",
                format!("story number {i} about an exchange hack"),
            )
            .with_schema(schema.clone());
            let out = p.complete(&req).await.unwrap();
            let v = out.json().unwrap();
            s::validate(&v, &schema).unwrap_or_else(|e| panic!("iteration {i}: {e}\n{}", out.text));
        }
    }

    #[tokio::test]
    async fn the_stub_is_deterministic() {
        let p = StubProvider;
        let req =
            Request::new("t", ModelTier::Mid, "sys", "same input").with_schema(draft_schema());
        let a = p.complete(&req).await.unwrap();
        let b = p.complete(&req).await.unwrap();
        assert_eq!(a.text, b.text);
    }

    #[tokio::test]
    async fn different_prompts_produce_different_output() {
        let p = StubProvider;
        let schema = draft_schema();
        let a = p
            .complete(
                &Request::new("t", ModelTier::Mid, "sys", "solana outage")
                    .with_schema(schema.clone()),
            )
            .await
            .unwrap();
        let b = p
            .complete(
                &Request::new("t", ModelTier::Mid, "sys", "sec approves etf").with_schema(schema),
            )
            .await
            .unwrap();
        assert_ne!(a.text, b.text);
    }

    #[tokio::test]
    async fn the_stub_never_charges() {
        let p = StubProvider;
        let out = p
            .complete(&Request::new(
                "t",
                ModelTier::Top,
                "sys",
                "expensive-looking prompt",
            ))
            .await
            .unwrap();
        assert_eq!(out.cost_usd, Decimal::ZERO);
        assert!(
            out.prompt_tokens > 0,
            "should still estimate tokens for the ledger"
        );
    }

    #[tokio::test]
    async fn plain_text_requests_return_prose_not_json() {
        let p = StubProvider;
        let out = p
            .complete(&Request::new(
                "copydesk.headline",
                ModelTier::Fast,
                "sys",
                "Exchange halts withdrawals.",
            ))
            .await
            .unwrap();
        assert!(out.text.contains("stub"));
        assert!(
            out.json().is_err(),
            "a schemaless request should not return JSON"
        );
    }

    #[test]
    fn nullable_fields_synthesize_the_real_branch_not_null() {
        let mut rng = Rng(1);
        let v = synthesize(&s::nullable(s::string("x")), &mut rng, "context", 0);
        assert!(v.is_string(), "should prefer the non-null branch, got {v}");
    }
}
