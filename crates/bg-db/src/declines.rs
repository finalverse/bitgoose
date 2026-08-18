//! Refusals worth remembering.
//!
//! A model saying "no" is a result, not an error, and it costs exactly as much
//! as a "yes". The Gander declined 279 of 290 topic framings — correctly, on
//! topics that had not yet earned one — but nothing wrote that down, so the
//! same subjects came round again every pass. On an allowance that buys a few
//! hundred calls a day, re-asking a question already answered is the difference
//! between a desk that publishes and one that does not.
//!
//! Backoff is exponential and capped: a topic that is not a story today may
//! well be one tomorrow, so this defers a question rather than closing it.

use crate::{Db, Result};

/// First refusal buys this long before we ask again.
const BASE_MINUTES: i64 = 90;

/// However many times it refuses, we come back within the day. Trends turn
/// over faster than that, and a permanent ban would quietly shrink the site.
const MAX_MINUTES: i64 = 60 * 18;

/// Record a refusal and push the next attempt out.
pub async fn note(db: &Db, kind: &str, subject: &str, reason: &str) -> Result<()> {
    sqlx::query(
        r#"
        insert into model_declines (kind, subject, attempts, retry_after, reason)
        values ($1, $2, 1, now() + make_interval(mins => $3), $4)
        on conflict (kind, subject) do update set
            attempts    = model_declines.attempts + 1,
            last_seen   = now(),
            reason      = excluded.reason,
            -- Double the wait each time, up to the cap.
            retry_after = now() + make_interval(mins =>
                least($5::bigint, $3::bigint * (2 ^ least(model_declines.attempts, 8))::bigint))
        "#,
    )
    .bind(kind)
    .bind(subject)
    .bind(BASE_MINUTES)
    .bind(reason)
    .bind(MAX_MINUTES)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Subjects of this kind that are still inside their backoff.
///
/// Returned as a set for the caller to filter a candidate list against, which
/// is one query per pass rather than one per candidate.
pub async fn resting(db: &Db, kind: &str) -> Result<std::collections::HashSet<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "select subject from model_declines where kind = $1 and retry_after > now()",
    )
    .bind(kind)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

/// Forget a refusal, for when the subject succeeds by another route.
pub async fn clear(db: &Db, kind: &str, subject: &str) -> Result<()> {
    sqlx::query("delete from model_declines where kind = $1 and subject = $2")
        .bind(kind)
        .bind(subject)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// What is currently being held off, newest refusal first — for `bg doctor`
/// and the Steward, so a backoff is never invisible.
pub async fn summary(db: &Db, limit: i64) -> Result<Vec<(String, String, i32, String)>> {
    let rows: Vec<(String, String, i32, Option<String>)> = sqlx::query_as(
        r#"select kind, subject, attempts, reason
           from model_declines where retry_after > now()
           order by last_seen desc limit $1"#,
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(k, s, a, r)| (k, s, a, r.unwrap_or_default()))
        .collect())
}

/// The kind used for trend topics the Gander would not frame.
pub const GAGGLE_FRAMING: &str = "gaggle.framing";
