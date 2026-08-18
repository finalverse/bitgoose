//! The Flock roster and its public run ledger.

use crate::{convert::*, Db, DbError, Result};
use bg_core::domain::{Agent, AgentRole, AgentRunSummary, FlockStats, ModelTier, RunStatus};
use bg_core::ids::{AgentId, RunId, StoryId};
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

fn agent_from_row(r: &PgRow) -> Result<Agent> {
    Ok(Agent {
        id: agent_id(r, "id")?,
        slug: r.try_get("slug")?,
        name: r.try_get("name")?,
        role: enum_col::<AgentRole>(r, "role")?,
        tier: enum_col::<ModelTier>(r, "tier")?,
        system_prompt: r.try_get("system_prompt")?,
        temperature: r.try_get("temperature")?,
        enabled: r.try_get("enabled")?,
    })
}

const COLS: &str = "id, slug, name, role, tier, system_prompt, temperature, enabled";

/// Insert or refresh one agent. `enabled` is preserved on conflict so an
/// operator who disabled an agent does not have it silently switched back on by
/// the next seed run.
pub async fn upsert(
    db: &Db,
    role: AgentRole,
    name: &str,
    system_prompt: &str,
    temperature: f32,
) -> Result<Agent> {
    let row = crate::sql(format!(
        "INSERT INTO agents (id, slug, name, role, tier, system_prompt, temperature)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (role) DO UPDATE SET
            name = EXCLUDED.name,
            tier = EXCLUDED.tier,
            system_prompt = EXCLUDED.system_prompt,
            temperature = EXCLUDED.temperature
         RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(role.as_str())
    .bind(name)
    .bind(role.as_str())
    .bind(role.tier().as_str())
    .bind(system_prompt)
    .bind(temperature)
    .fetch_one(&db.pool)
    .await?;
    agent_from_row(&row)
}

pub async fn by_role(db: &Db, role: AgentRole) -> Result<Agent> {
    let row = crate::sql(format!("SELECT {COLS} FROM agents WHERE role = $1"))
        .bind(role.as_str())
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound("agent"))?;
    agent_from_row(&row)
}

pub async fn all(db: &Db) -> Result<Vec<Agent>> {
    let rows = crate::sql(format!("SELECT {COLS} FROM agents"))
        .fetch_all(&db.pool)
        .await?;
    rows.iter().map(agent_from_row).collect()
}

// -- run ledger -------------------------------------------------------------

/// Open a run row before the work starts.
///
/// Written up front rather than on completion so a crashed or hung agent leaves
/// a `running` row behind. A ledger that only records successes would show a
/// crash as though it never happened.
pub async fn start_run(
    db: &Db,
    agent: AgentId,
    role: AgentRole,
    story: Option<StoryId>,
    stage: &str,
) -> Result<RunId> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_runs (id, agent_id, role, story_id, stage, status)
         VALUES ($1,$2,$3,$4,$5,'running')",
    )
    .bind(id)
    .bind(agent.into_uuid())
    .bind(role.as_str())
    .bind(story.map(|s| s.into_uuid()))
    .bind(stage)
    .execute(&db.pool)
    .await?;
    Ok(RunId::from_uuid(id))
}

#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub status: Option<RunStatus>,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cost_usd: Decimal,
    pub latency_ms: i32,
    pub input_hash: Option<String>,
    pub output_hash: Option<String>,
    pub note: Option<String>,
    pub error: Option<String>,
}

pub async fn finish_run(db: &Db, run: RunId, o: &RunOutcome) -> Result<()> {
    let status = o.status.unwrap_or(RunStatus::Ok);
    sqlx::query(
        "UPDATE agent_runs SET
            status = $2, provider = $3, model = $4,
            prompt_tokens = $5, completion_tokens = $6, cost_usd = $7, latency_ms = $8,
            input_hash = $9, output_hash = $10, note = $11, error = $12,
            finished_at = now()
         WHERE id = $1",
    )
    .bind(run.into_uuid())
    .bind(status.as_str())
    .bind(&o.provider)
    .bind(&o.model)
    .bind(o.prompt_tokens)
    .bind(o.completion_tokens)
    .bind(o.cost_usd)
    .bind(o.latency_ms)
    .bind(&o.input_hash)
    .bind(&o.output_hash)
    .bind(&o.note)
    .bind(&o.error)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Total spend in a window. Feeds the per-run budget ceiling.
pub async fn cost_since(db: &Db, since_minutes: i32) -> Result<Decimal> {
    let v: Option<Decimal> = sqlx::query_scalar(
        "SELECT sum(cost_usd) FROM agent_runs
         WHERE started_at > now() - make_interval(mins => $1)",
    )
    .bind(since_minutes)
    .fetch_one(&db.pool)
    .await?;
    Ok(v.unwrap_or_default())
}

/// Per-agent 24-hour rollup for `/flock`.
///
/// LEFT JOIN from `agents`, so an agent that has not run today still appears
/// with zeroes instead of vanishing from the roster.
pub async fn flock_stats(db: &Db) -> Result<Vec<FlockStats>> {
    let rows = sqlx::query(
        "SELECT a.role, a.name, a.enabled,
                count(r.id)                                    AS runs_24h,
                count(r.id) FILTER (WHERE r.status = 'ok')     AS ok_24h,
                count(r.id) FILTER (WHERE r.status = 'failed') AS failed_24h,
                COALESCE(sum(r.cost_usd), 0)                   AS cost_24h,
                COALESCE(avg(r.latency_ms), 0)::bigint         AS avg_latency,
                COALESCE(sum(r.prompt_tokens + r.completion_tokens), 0)::bigint AS tokens_24h,
                max(r.started_at)                              AS last_run_at,
                (array_remove(array_agg(r.note ORDER BY r.started_at DESC), NULL))[1] AS last_note
         FROM agents a
         LEFT JOIN agent_runs r
                ON r.agent_id = a.id AND r.started_at > now() - interval '24 hours'
         GROUP BY a.id
         ORDER BY a.name",
    )
    .fetch_all(&db.pool)
    .await?;

    rows.iter()
        .map(|r| {
            Ok(FlockStats {
                role: enum_col::<AgentRole>(r, "role")?,
                name: r.try_get("name")?,
                runs_24h: r.try_get("runs_24h")?,
                ok_24h: r.try_get("ok_24h")?,
                failed_24h: r.try_get("failed_24h")?,
                cost_24h_usd: r.try_get("cost_24h")?,
                avg_latency_ms: r.try_get("avg_latency")?,
                tokens_24h: r.try_get("tokens_24h")?,
                last_run_at: r.try_get("last_run_at")?,
                last_note: r.try_get("last_note")?,
                enabled: r.try_get("enabled")?,
            })
        })
        .collect()
}

fn summary_from_row(r: &PgRow) -> Result<AgentRunSummary> {
    Ok(AgentRunSummary {
        role: enum_col::<AgentRole>(r, "role")?,
        status: enum_col::<RunStatus>(r, "status")?,
        model: r.try_get("model")?,
        cost_usd: r.try_get("cost_usd")?,
        latency_ms: r.try_get("latency_ms")?,
        started_at: r.try_get("started_at")?,
        note: r.try_get("note")?,
    })
}

/// The newsroom activity ticker.
pub async fn recent_runs(db: &Db, limit: i64) -> Result<Vec<AgentRunSummary>> {
    let rows = sqlx::query(
        "SELECT role, status, model, cost_usd, latency_ms, started_at, note
         FROM agent_runs ORDER BY started_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(summary_from_row).collect()
}

/// Which agents touched one story — the provenance trail on the story page.
pub async fn runs_for_story(db: &Db, story: StoryId) -> Result<Vec<AgentRunSummary>> {
    let rows = sqlx::query(
        "SELECT role, status, model, cost_usd, latency_ms, started_at, note
         FROM agent_runs WHERE story_id = $1 ORDER BY started_at ASC",
    )
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(summary_from_row).collect()
}

/// Newsroom-wide totals for the `/flock` header.
pub struct NewsroomTotals {
    pub runs_24h: i64,
    pub cost_24h: Decimal,
    pub tokens_24h: i64,
    pub failures_24h: i64,
    pub stories_published_24h: i64,
    pub claims_24h: i64,
}

pub async fn newsroom_totals(db: &Db) -> Result<NewsroomTotals> {
    let r = sqlx::query(
        "SELECT
           (SELECT count(*) FROM agent_runs WHERE started_at > now() - interval '24 hours') AS runs,
           (SELECT COALESCE(sum(cost_usd),0) FROM agent_runs
              WHERE started_at > now() - interval '24 hours') AS cost,
           (SELECT COALESCE(sum(prompt_tokens + completion_tokens),0)::bigint FROM agent_runs
              WHERE started_at > now() - interval '24 hours') AS tokens,
           (SELECT count(*) FROM agent_runs
              WHERE started_at > now() - interval '24 hours' AND status = 'failed') AS failures,
           (SELECT count(*) FROM stories
              WHERE published_at > now() - interval '24 hours') AS pubs,
           (SELECT count(*) FROM claims
              WHERE created_at > now() - interval '24 hours') AS claims",
    )
    .fetch_one(&db.pool)
    .await?;
    Ok(NewsroomTotals {
        runs_24h: r.try_get("runs")?,
        cost_24h: r.try_get("cost")?,
        tokens_24h: r.try_get("tokens")?,
        failures_24h: r.try_get("failures")?,
        stories_published_24h: r.try_get("pubs")?,
        claims_24h: r.try_get("claims")?,
    })
}

/// Tokens spent per model tier in the last 24 hours.
///
/// The pacer's daily ledger lives in memory, so a worker restart forgot
/// everything and the newsroom went straight back to hammering a quota it had
/// already spent — Groq answers those with `retry in 291s`, repeatedly, for the
/// rest of the day. The ledger of record is right here: every call writes its
/// token counts to `agent_runs`, so the pacer can be seeded from it at startup
/// instead of starting each restart in blissful ignorance.
///
/// Keyed by the role's tier rather than the model name, matching how the pacer
/// buckets its budgets.
pub async fn tokens_by_tier_24h(db: &Db) -> Result<Vec<(bg_core::domain::ModelTier, i64)>> {
    let rows = sqlx::query(
        "SELECT a.role, COALESCE(sum(r.prompt_tokens + r.completion_tokens), 0)::bigint AS toks
           FROM agent_runs r JOIN agents a ON a.id = r.agent_id
          WHERE r.started_at > now() - interval '24 hours'
          GROUP BY a.role",
    )
    .fetch_all(&db.pool)
    .await?;

    let mut out: std::collections::HashMap<bg_core::domain::ModelTier, i64> = Default::default();
    for r in &rows {
        let role: String = r.try_get("role")?;
        let Ok(role) = <bg_core::domain::AgentRole as std::str::FromStr>::from_str(&role) else {
            continue;
        };
        *out.entry(role.tier()).or_default() += r.try_get::<i64, _>("toks")?;
    }
    Ok(out.into_iter().collect())
}

/// Per-role success and failure counts over a window, with one real error.
///
/// The pass summary counts what finished; this counts what did not. Those were
/// wildly different numbers for weeks — `analysed 0` in the log while 83% of
/// Skein's calls were being rejected — and only the second one says why.
///
/// Returns `(role, ok, failed, one_error)` for every role that ran.
pub async fn failure_rates(
    db: &Db,
    hours: i64,
) -> Result<Vec<(String, i64, i64, String)>> {
    let rows: Vec<(String, i64, i64, Option<String>)> = sqlx::query_as(
        r#"
        select role,
               count(*) filter (where status = 'ok')     as ok,
               count(*) filter (where status = 'failed') as failed,
               -- The most recent failure, which is the one worth quoting: an
               -- agent that started failing an hour ago is not best explained
               -- by whatever went wrong yesterday.
               (array_agg(error order by started_at desc)
                    filter (where status = 'failed' and error is not null))[1]
        from agent_runs
        where started_at > now() - make_interval(hours => $1)
        group by role
        order by count(*) desc
        "#,
    )
    .bind(hours)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(r, ok, failed, e)| (r, ok, failed, e.unwrap_or_default()))
        .collect())
}
