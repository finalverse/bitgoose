//! Policy-violation ledger.
//!
//! Every refused publish is recorded. A block that is logged to stderr and
//! forgotten is a block that recurs silently; these rows are the evidence trail
//! behind the copyright posture and the failure counters on `/flock`.

use crate::{Db, Result};
use bg_core::ids::{ArticleId, RunId, StoryId};
use bg_core::policy::{PolicyReport, Severity};
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

/// Persist every violation in a report, blocks and warnings alike.
pub async fn record(
    db: &Db,
    report: &PolicyReport,
    story: Option<StoryId>,
    article: Option<ArticleId>,
    run: Option<RunId>,
) -> Result<usize> {
    let mut n = 0usize;
    for v in &report.violations {
        sqlx::query(
            "INSERT INTO policy_violations
               (id, story_id, article_id, run_id, code, severity, detail, subject)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(Uuid::new_v4())
        .bind(story.map(|s| s.into_uuid()))
        .bind(article.map(|a| a.into_uuid()))
        .bind(run.map(|r| r.into_uuid()))
        .bind(v.code.as_str())
        .bind(match v.severity {
            Severity::Block => "block",
            Severity::Warn => "warn",
        })
        .bind(&v.detail)
        .bind(&v.subject)
        .execute(&db.pool)
        .await?;
        n += 1;
    }
    Ok(n)
}

#[derive(Debug, Clone)]
pub struct ViolationRow {
    pub code: String,
    pub severity: String,
    pub detail: String,
    pub subject: Option<String>,
    pub story_slug: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn recent(db: &Db, limit: i64) -> Result<Vec<ViolationRow>> {
    let rows = sqlx::query(
        "SELECT v.code, v.severity, v.detail, v.subject, v.created_at, s.slug AS story_slug
         FROM policy_violations v
         LEFT JOIN stories s ON s.id = v.story_id
         ORDER BY v.created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| ViolationRow {
            code: r.get("code"),
            severity: r.get("severity"),
            detail: r.get("detail"),
            subject: r.get("subject"),
            story_slug: r.get("story_slug"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn count_blocks_24h(db: &Db) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM policy_violations
         WHERE severity = 'block' AND created_at > now() - interval '24 hours'",
    )
    .fetch_one(&db.pool)
    .await?)
}

/// Counts by code over a window, for the standards page.
pub async fn tally(db: &Db, days: i32) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query(
        "SELECT code, count(*) AS n FROM policy_violations
         WHERE created_at > now() - make_interval(days => $1)
         GROUP BY code ORDER BY n DESC",
    )
    .bind(days)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get("code"), r.get("n"))).collect())
}
