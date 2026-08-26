//! # bg-agents — the Flock
//!
//! Ten agents, one pipeline, no humans in the publishing path.
//!
//! ```text
//!   Scout ─▶ Gosling ─▶ Curator ─┬─▶ [Desk]  Scribe ─▶ Sentinel ─▶ Quant ─▶ Copydesk ─▶ Gander ─▶ published
//!   (poll)   (triage)   (cluster)│
//!                               └─▶ [Wire]  Herald ─▶ published
//!                                            Ombuds ─▶ corrections (post-publish)
//! ```
//!
//! Three rules hold across every stage:
//!
//! 1. **Every stage writes an `agent_runs` row**, LLM-backed or not, success or
//!    failure. The ledger is public on `/flock`; a stage that could fail
//!    silently would make the published error rate a lie. [`stage`] is the only
//!    way to run one, so this cannot be forgotten.
//! 2. **Only Gander publishes**, and only through [`bg_core::policy`]. No other
//!    module sets `status = published`.
//! 3. **Every stage is budget-checked.** A runaway loop is a spend incident, so
//!    the ceiling is enforced before the call, not measured after it.

pub mod copydesk;
pub mod curator;
pub mod gaggle;
pub mod gander;
pub mod gosling;
pub mod herald;
pub mod ombuds;
pub mod quant;
pub mod runner;
pub mod scout;
pub mod scribe;
pub mod sentinel;
pub mod skein;
pub mod steward;

use bg_core::domain::{AgentRole, RunStatus};
use bg_core::ids::{RunId, StoryId};
use bg_db::{agents as agents_repo, Db};
use bg_llm::{Completion, Llm};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use thiserror::Error;
use tracing::{info, warn};

pub type Result<T, E = FlockError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum FlockError {
    #[error(transparent)]
    Db(#[from] bg_db::DbError),

    #[error(transparent)]
    Llm(#[from] bg_llm::LlmError),

    #[error(transparent)]
    Ingest(#[from] bg_ingest::IngestError),

    /// A few agents issue an aggregate query directly rather than adding a
    /// single-caller method to the repository layer.
    #[error(transparent)]
    Sql(#[from] sqlx::Error),

    #[error("run budget of ${limit} exhausted (spent ${spent})")]
    BudgetExhausted { limit: Decimal, spent: Decimal },

    #[error("{0}")]
    Other(String),
}

impl FlockError {
    /// Whether this is the provider being unavailable rather than an answer.
    ///
    /// The distinction decides what a stage may conclude from a failure. A
    /// model that says "no" has decided something; a model that is rate limited
    /// has decided nothing, and treating the two alike is how the Curator came
    /// to seal thousands of items into single-source stories during the hours
    /// a free tier spends refusing requests.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Llm(e) => e.is_retryable(),
            Self::BudgetExhausted { .. } => true,
            Self::Db(_) | Self::Ingest(_) | Self::Sql(_) | Self::Other(_) => false,
        }
    }
}

/// Tuning knobs, all environment-overridable.
#[derive(Debug, Clone)]
pub struct FlockConfig {
    /// Stories at or above this newsworthiness get an original Desk story;
    /// everything else goes to the Wire.
    pub desk_threshold: i16,
    /// Cap on Desk drafts per run. The Desk is the expensive path.
    pub desk_max_per_run: usize,
    /// Spend ceiling per run in USD. Zero disables the check.
    pub run_budget_usd: Decimal,
    pub user_agent: String,
    /// Newsworthiness a Wire story must reach before a model writes its
    /// summary. Below it the card publishes as headline, outlet and link.
    pub wire_summary_floor: i16,
    /// Most stories the Skein will analyse in a day.
    ///
    /// The Skein is the single largest consumer of inference — 52% of a
    /// measured day, 42 analyses at ~3,000 tokens each — on a site that also
    /// wants to cover seven desks. A daily cap turns "analyse everything that
    /// clears the grounding floor" into "analyse the most newsworthy N", which
    /// is what `needing_analysis` already orders by.
    pub max_analyses_per_day: i64,
    /// What each agent may spend per day, in CCC-wei.
    ///
    /// Per agent, not shared: the point of a mandate is that a fault in one
    /// role cannot consume the whole newsroom's allowance.
    pub agent_budget_ccc: bg_core::mandate::Wei,
    /// CCC charged per million model tokens — the unit of account that lets one
    /// mandate span providers whose own prices differ by an order of magnitude.
    pub ccc_per_mtok: bg_core::mandate::Wei,
    pub ingest_concurrency: usize,
}

impl Default for FlockConfig {
    fn default() -> Self {
        Self {
            desk_threshold: 62,
            desk_max_per_run: 3,
            run_budget_usd: Decimal::from_str("2.00").unwrap(),
            user_agent: bg_ingest::http::DEFAULT_UA.to_string(),
            // A tenth of a CCC — a hundred thousand tokens a day, each.
            //
            // Sized to bind rather than to reassure. The whole newsroom's
            // allowance is about two hundred thousand tokens a day, so any
            // single agent reaching this has taken half of everything and is
            // almost certainly looping. A ceiling nothing can ever touch would
            // be a number on a page, not a control.
            wire_summary_floor: 65,
            max_analyses_per_day: 20,
            agent_budget_ccc: bg_core::mandate::CCC / 10,
            ccc_per_mtok: bg_core::mandate::DEFAULT_CCC_PER_MTOK,
            ingest_concurrency: 4,
        }
    }
}

impl FlockConfig {
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            desk_threshold: env_parse("BG_DESK_THRESHOLD").unwrap_or(d.desk_threshold),
            desk_max_per_run: env_parse("BG_DESK_MAX_PER_RUN").unwrap_or(d.desk_max_per_run),
            run_budget_usd: std::env::var("BG_RUN_BUDGET_USD")
                .ok()
                .and_then(|v| Decimal::from_str(&v).ok())
                .unwrap_or(d.run_budget_usd),
            user_agent: std::env::var("BG_USER_AGENT").unwrap_or(d.user_agent),
            // Parsed as a decimal CCC amount — "0.1", "2.5" — because the
            // useful settings here are fractions of a token and an integer-only
            // knob could not express any of them.
            wire_summary_floor: std::env::var("BG_WIRE_SUMMARY_FLOOR")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.wire_summary_floor),
            max_analyses_per_day: std::env::var("BG_MAX_ANALYSES_PER_DAY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.max_analyses_per_day),
            agent_budget_ccc: std::env::var("BG_AGENT_BUDGET_CCC")
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|c| *c >= 0.0 && c.is_finite())
                .map(|c| (c * bg_core::mandate::CCC as f64) as u128)
                .unwrap_or(d.agent_budget_ccc),
            ccc_per_mtok: d.ccc_per_mtok,
            ingest_concurrency: env_parse("BG_INGEST_CONCURRENCY").unwrap_or(d.ingest_concurrency),
        }
    }
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

/// Everything an agent needs. Cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    pub db: Db,
    pub llm: Llm,
    pub http: reqwest::Client,
    pub cfg: FlockConfig,
    /// One bounded spending authority per agent. See [`bg_core::mandate`].
    mandates: Mandates,
}

/// The Flock's mandates, shared across a pass.
///
/// In memory, and rebuilt from `agent_runs` at startup the same way the pacer's
/// daily ledger is — a restart that forgets what has been spent is a restart
/// that spends it again.
#[derive(Clone, Default)]
pub struct Mandates(
    std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<AgentRole, bg_core::mandate::Mandate>>,
    >,
);

impl Mandates {
    fn seed(cfg: &FlockConfig) -> Self {
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        let map = AgentRole::ALL
            .iter()
            .map(|r| {
                (
                    *r,
                    bg_core::mandate::Mandate::new(*r, cfg.agent_budget_ccc, now),
                )
            })
            .collect();
        Self(std::sync::Arc::new(std::sync::Mutex::new(map)))
    }
}

impl Ctx {
    pub fn new(db: Db, llm: Llm, cfg: FlockConfig) -> Result<Self> {
        let http = bg_ingest::http::client(&cfg.user_agent)?;
        let mandates = Mandates::seed(&cfg);
        Ok(Self {
            db,
            llm,
            http,
            cfg,
            mandates,
        })
    }

    /// The mandate covering one agent.
    ///
    /// Every role has one; a role without a mandate would be a role that can
    /// spend without a ceiling, so the map is built from `AgentRole::ALL`
    /// rather than from configuration that could omit an entry.
    pub fn mandate(&self, role: AgentRole) -> bg_core::mandate::Mandate {
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        let mut g = self.mandates.0.lock().expect("mandate lock");
        let m = g.entry(role).or_insert_with(|| {
            bg_core::mandate::Mandate::new(role, self.cfg.agent_budget_ccc, now)
        });
        m.roll(now);
        m.clone()
    }

    /// Record what a stage actually spent against its mandate.
    ///
    /// Tokens served by our own hardware are not spending. A mandate is a
    /// commitment about *buying* inference — it exists so an agent cannot run
    /// up an unbounded bill on someone's API — and charging it for a GPU we
    /// already own turns a spending control into a throughput limit on free
    /// compute.
    ///
    /// This was not theoretical. With triage moved to the local model, Gosling
    /// reached 0.1016 CCC against a 0.1000 budget and then refused every
    /// triage call for the rest of the window. The newsroom went quiet for
    /// fifteen hours with 5,128 items queued and a GPU sitting at 0%,
    /// rationing compute that costs nothing.
    fn settle_mandate(&self, role: AgentRole, tokens: u64) {
        if tokens == 0 || self.llm.tier_is_local(role.tier()) {
            return;
        }
        let cost = bg_core::mandate::tokens_to_ccc(tokens, self.cfg.ccc_per_mtok);
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        if let Ok(mut g) = self.mandates.0.lock() {
            let m = g.entry(role).or_insert_with(|| {
                bg_core::mandate::Mandate::new(role, self.cfg.agent_budget_ccc, now)
            });
            m.roll(now);
            m.settle(cost);
        }
    }

    /// Every mandate, for `/flock`.
    pub fn all_mandates(&self) -> Vec<bg_core::mandate::Mandate> {
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        let mut g = match self.mandates.0.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        for role in AgentRole::ALL {
            g.entry(*role).or_insert_with(|| {
                bg_core::mandate::Mandate::new(*role, self.cfg.agent_budget_ccc, now)
            });
        }
        let mut out: Vec<_> = g
            .values_mut()
            .map(|m| {
                m.roll(now);
                m.clone()
            })
            .collect();
        out.sort_by_key(|m| m.agent);
        out
    }

    /// As [`Ctx::new`], with the day's token spend restored from the run ledger.
    ///
    /// Prefer this anywhere long-running. Without it a restart resets the
    /// pacer's daily count to zero while the provider's own count carries on,
    /// so the newsroom immediately overruns a quota it had already spent and
    /// spends the rest of the day taking `retry in 291s`.
    pub async fn resumed(db: Db, llm: Llm, cfg: FlockConfig) -> Result<Self> {
        let ctx = Self::new(db, llm, cfg)?;
        match agents_repo::tokens_by_tier_24h(&ctx.db).await {
            Ok(used) => {
                for (tier, toks) in used {
                    let toks = toks.clamp(0, u32::MAX as i64) as u32;
                    ctx.llm.seed_daily(tier, toks);
                    if toks > 0 {
                        info!(
                            ?tier,
                            tokens = toks,
                            "restored today's spend from the run ledger"
                        );
                    }
                }
            }
            Err(e) => warn!(error = %e, "could not restore today's token spend; pacing from zero"),
        }
        Ok(ctx)
    }

    /// Spend in the last hour, the window the run budget is measured over.
    pub async fn spent_recently(&self) -> Decimal {
        agents_repo::cost_since(&self.db, 60)
            .await
            .unwrap_or_default()
    }

    async fn check_budget(&self) -> Result<()> {
        if self.cfg.run_budget_usd.is_zero() {
            return Ok(());
        }
        let spent = self.spent_recently().await;
        if spent >= self.cfg.run_budget_usd {
            return Err(FlockError::BudgetExhausted {
                limit: self.cfg.run_budget_usd,
                spent,
            });
        }
        Ok(())
    }
}

/// The task label a mandate is matched against.
///
/// Stages name themselves loosely — `"analyse"`, `"draft"` — so the role is
/// prefixed here rather than relying on every call site to remember. Deriving
/// it means a new stage cannot accidentally fall outside its own allowlist.
fn stage_name_qualified(role: AgentRole, stage: &str) -> String {
    if stage.starts_with(&format!("{}.", role.as_str())) {
        stage.to_string()
    } else {
        format!("{}.{stage}", role.as_str())
    }
}

/// What a stage produced.
pub struct StageOutput<T> {
    pub value: T,
    /// Present when the stage called a model; drives the cost ledger.
    pub completion: Option<Completion>,
    /// One line for `/flock` and the story's provenance trail.
    pub note: Option<String>,
}

impl<T> StageOutput<T> {
    pub fn plain(value: T, note: impl Into<String>) -> Self {
        Self {
            value,
            completion: None,
            note: Some(note.into()),
        }
    }
    pub fn with(value: T, completion: Completion, note: impl Into<String>) -> Self {
        Self {
            value,
            completion: Some(completion),
            note: Some(note.into()),
        }
    }
}

/// Run one agent stage, recording it in the public ledger.
///
/// The single entry point for agent work. Opening the run row *before* the work
/// starts means a crash leaves a `running` row rather than no trace at all, and
/// routing every stage through here is what makes "every stage is recorded" a
/// structural property instead of a convention.
pub async fn stage<T, F, Fut>(
    ctx: &Ctx,
    role: AgentRole,
    story: Option<StoryId>,
    stage_name: &str,
    f: F,
) -> Result<T>
where
    F: FnOnce(RunId) -> Fut,
    Fut: std::future::Future<Output = Result<StageOutput<T>>>,
{
    let agent = agents_repo::by_role(&ctx.db, role).await?;

    // Budget is checked before the row is opened, so a refused stage is
    // recorded as `budgeted` rather than looking like a crash.
    if role.tier() != bg_core::domain::ModelTier::None {
        // The mandate first: it is local arithmetic, it says *which* agent and
        // *what for*, and it refuses before a request is built rather than
        // after the money is gone. The global budget below remains the
        // backstop — a mandate bounds one agent, the budget bounds the sum.
        let m = ctx.mandate(role);
        // Nothing has been spent yet, so the check is for a mandate that is
        // already exhausted or does not cover this work at all. Skipped
        // entirely for a tier we serve ourselves: see `settle_mandate` — there
        // is no bill to bound, and enforcing one stopped the newsroom dead.
        if !ctx.llm.tier_is_local(role.tier()) {
            if let Err(refusal) = m.check(
                stage_name_qualified(role, stage_name).as_str(),
                role.tier(),
                1,
            ) {
                warn!(
                    role = %role, stage = stage_name, spent = %bg_core::mandate::format_ccc(m.spent),
                    budget = %bg_core::mandate::format_ccc(m.budget),
                    "mandate refused: {}", refusal.reason()
                );
                let run =
                    agents_repo::start_run(&ctx.db, agent.id, role, story, stage_name).await?;
                agents_repo::finish_run(
                    &ctx.db,
                    run,
                    &agents_repo::RunOutcome {
                        status: Some(RunStatus::Budgeted),
                        note: Some(format!("mandate: {}", refusal.reason())),
                        ..Default::default()
                    },
                )
                .await?;
                return Err(FlockError::Other(format!(
                    "{role} mandate: {}",
                    refusal.reason()
                )));
            }
        }

        if let Err(e) = ctx.check_budget().await {
            warn!(role = %role, stage = stage_name, "{e}");
            let run = agents_repo::start_run(&ctx.db, agent.id, role, story, stage_name).await?;
            agents_repo::finish_run(
                &ctx.db,
                run,
                &agents_repo::RunOutcome {
                    status: Some(RunStatus::Budgeted),
                    note: Some(e.to_string()),
                    ..Default::default()
                },
            )
            .await?;
            return Err(e);
        }
    }

    let run = agents_repo::start_run(&ctx.db, agent.id, role, story, stage_name).await?;
    let started = std::time::Instant::now();

    match f(run).await {
        Ok(out) => {
            let c = out.completion.as_ref();
            // Against the mandate, before the row is written: what the model
            // actually returned, not what was estimated beforehand.
            ctx.settle_mandate(
                role,
                c.map(|c| (c.prompt_tokens + c.completion_tokens) as u64)
                    .unwrap_or(0),
            );
            agents_repo::finish_run(
                &ctx.db,
                run,
                &agents_repo::RunOutcome {
                    status: Some(RunStatus::Ok),
                    provider: c.map(|c| c.provider.clone()).unwrap_or_default(),
                    model: c.map(|c| c.model.clone()).unwrap_or_default(),
                    prompt_tokens: c.map(|c| c.prompt_tokens as i32).unwrap_or(0),
                    completion_tokens: c.map(|c| c.completion_tokens as i32).unwrap_or(0),
                    cost_usd: c.map(|c| c.cost_usd).unwrap_or_default(),
                    latency_ms: started.elapsed().as_millis() as i32,
                    note: out.note,
                    ..Default::default()
                },
            )
            .await?;
            Ok(out.value)
        }
        Err(e) => {
            warn!(role = %role, stage = stage_name, error = %e, "stage failed");
            agents_repo::finish_run(
                &ctx.db,
                run,
                &agents_repo::RunOutcome {
                    status: Some(RunStatus::Failed),
                    latency_ms: started.elapsed().as_millis() as i32,
                    error: Some(e.to_string().chars().take(500).collect()),
                    ..Default::default()
                },
            )
            .await?;
            Err(e)
        }
    }
}

/// House style, prepended to every agent's system prompt.
///
/// Consolidated so the voice cannot drift between agents — and so the
/// non-negotiables (never reproduce source wording, never invent a number) are
/// stated once, in front of every model call.
pub const HOUSE_STYLE: &str = include_str!("../../../prompts/master-system.md");

/// Independent edition requested for every language-capable model stage.
pub const fn output_language(lang: bg_core::domain::EditorialLanguage) -> &'static str {
    match lang {
        bg_core::domain::EditorialLanguage::En => "en",
        bg_core::domain::EditorialLanguage::Zh => "zh",
        bg_core::domain::EditorialLanguage::ZhHant => "zh-hant",
        bg_core::domain::EditorialLanguage::Fr => "fr",
        bg_core::domain::EditorialLanguage::Es => "es",
        bg_core::domain::EditorialLanguage::Ja => "ja",
        bg_core::domain::EditorialLanguage::Ko => "ko",
    }
}

/// How much latitude each role gets. Extraction and judgement run at zero;
/// only the roles that choose words are allowed any.
///
/// Exhaustive on purpose — see [`seed_roster`].
const fn temperature(role: AgentRole) -> f32 {
    match role {
        AgentRole::Scribe => 0.3,
        AgentRole::Copydesk => 0.4,
        AgentRole::Herald => 0.2,
        // Analysis is inference, but it is inference about evidence. Warmth
        // here buys speculation, which is the one thing this role must not add.
        AgentRole::Skein => 0.2,
        AgentRole::Scout
        | AgentRole::Gosling
        | AgentRole::Curator
        | AgentRole::Sentinel
        | AgentRole::Quant
        | AgentRole::Gander
        | AgentRole::Ombuds => 0.0,
    }
}

/// Seed the roster with each agent's name, tier and system prompt.
///
/// Driven off [`AgentRole::ALL`] rather than a list maintained here. The list
/// version silently omitted a newly added role — seeding reported success, and
/// the agent then failed at runtime with "agent not found", which points
/// nowhere near the actual cause. Both functions this calls match exhaustively
/// on the role, so a new variant is now a compile error instead.
pub async fn seed_roster(db: &Db) -> Result<usize> {
    for role in AgentRole::ALL {
        let full = format!("{HOUSE_STYLE}\n\n---\n\n{}", compiled_prompt(*role));
        agents_repo::upsert(db, *role, role.display_name(), &full, temperature(*role)).await?;
    }
    info!(count = AgentRole::ALL.len(), "flock roster seeded");
    Ok(AgentRole::ALL.len())
}

/// System prompt for a role, as stored (falls back to the compiled-in text).
pub async fn system_prompt(ctx: &Ctx, role: AgentRole) -> String {
    match agents_repo::by_role(&ctx.db, role).await {
        Ok(a) if !a.system_prompt.trim().is_empty() => a.system_prompt,
        _ => format!("{HOUSE_STYLE}\n\n---\n\n{}", compiled_prompt(role)),
    }
}

fn compiled_prompt(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Scout => scout::SYSTEM,
        AgentRole::Gosling => gosling::SYSTEM,
        AgentRole::Curator => curator::SYSTEM,
        AgentRole::Scribe => scribe::SYSTEM,
        AgentRole::Sentinel => sentinel::SYSTEM,
        AgentRole::Quant => quant::SYSTEM,
        AgentRole::Copydesk => copydesk::SYSTEM,
        AgentRole::Gander => gander::SYSTEM,
        AgentRole::Herald => herald::SYSTEM,
        AgentRole::Ombuds => ombuds::SYSTEM,
        AgentRole::Skein => skein::SYSTEM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_has_a_compiled_prompt() {
        for r in AgentRole::ALL {
            let p = compiled_prompt(*r);
            assert!(p.len() > 80, "{r} has a stub prompt");
        }
    }

    #[test]
    fn house_style_states_the_non_negotiables() {
        for must in ["25 words", "own words", "sources disagree"] {
            assert!(HOUSE_STYLE.contains(must), "house style missing: {must}");
        }
    }
}
