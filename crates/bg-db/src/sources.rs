//! Source registry and polite-polling bookkeeping.

use crate::{convert::*, Db, DbError, Result};
use bg_core::domain::{Source, SourceKind};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

fn from_row(r: &PgRow) -> Result<Source> {
    Ok(Source {
        id: source_id(r, "id")?,
        slug: r.try_get("slug")?,
        name: r.try_get("name")?,
        kind: enum_col::<SourceKind>(r, "kind")?,
        url: r.try_get("url")?,
        homepage: r.try_get("homepage")?,
        trust: r.try_get("trust")?,
        robots_ok: r.try_get("robots_ok")?,
        poll_interval_s: r.try_get("poll_interval_s")?,
        etag: r.try_get("etag")?,
        last_modified: r.try_get("last_modified")?,
        last_polled_at: r.try_get("last_polled_at")?,
        last_error: r.try_get("last_error")?,
        enabled: r.try_get("enabled")?,
        created_at: r.try_get("created_at")?,
    })
}

const COLS: &str = "id, slug, name, kind, url, homepage, trust, robots_ok, poll_interval_s, \
                    etag, last_modified, last_polled_at, last_error, enabled, created_at";

/// Insert or update a source by slug. Deliberately preserves `etag`,
/// `last_modified` and `last_polled_at` — re-running the seeder must not cause
/// a full re-fetch of every feed.
#[allow(clippy::too_many_arguments)]
pub async fn upsert(
    db: &Db,
    slug: &str,
    name: &str,
    kind: SourceKind,
    url: &str,
    homepage: &str,
    trust: i16,
    poll_interval_s: i32,
) -> Result<Source> {
    let row = crate::sql(format!(
        "INSERT INTO sources (id, slug, name, kind, url, homepage, trust, poll_interval_s)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (slug) DO UPDATE SET
            name = EXCLUDED.name,
            kind = EXCLUDED.kind,
            url = EXCLUDED.url,
            homepage = EXCLUDED.homepage,
            trust = EXCLUDED.trust,
            poll_interval_s = EXCLUDED.poll_interval_s
         RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(slug)
    .bind(name)
    .bind(kind.as_str())
    .bind(url)
    .bind(homepage)
    .bind(trust)
    .bind(poll_interval_s)
    .fetch_one(&db.pool)
    .await?;
    from_row(&row)
}

pub async fn all(db: &Db) -> Result<Vec<Source>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM sources ORDER BY trust DESC, slug"
    ))
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn by_slug(db: &Db, slug: &str) -> Result<Source> {
    let row = crate::sql(format!("SELECT {COLS} FROM sources WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound("source"))?;
    from_row(&row)
}

/// Sources whose `poll_interval_s` has elapsed. Robots-blocked and disabled
/// sources are excluded here rather than filtered by the caller, so there is
/// one place that decides what we are allowed to fetch.
pub async fn due_for_poll(db: &Db, limit: i64) -> Result<Vec<Source>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM sources
         WHERE enabled AND robots_ok
           AND (last_polled_at IS NULL
                OR last_polled_at < now() - make_interval(secs => poll_interval_s))
         ORDER BY last_polled_at ASC NULLS FIRST
         LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Record a successful poll, storing conditional-GET validators so the next
/// request can be a cheap `304 Not Modified`.
pub async fn record_success(
    db: &Db,
    id: bg_core::SourceId,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE sources
         SET last_polled_at = now(), last_error = NULL,
             etag = COALESCE($2, etag),
             last_modified = COALESCE($3, last_modified)
         WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(etag)
    .bind(last_modified)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Record a failed poll. `last_polled_at` still advances so one broken feed
/// cannot monopolise the scheduler by staying permanently "due".
pub async fn record_failure(db: &Db, id: bg_core::SourceId, err: &str) -> Result<()> {
    sqlx::query("UPDATE sources SET last_polled_at = now(), last_error = $2 WHERE id = $1")
        .bind(id.into_uuid())
        .bind(err.chars().take(500).collect::<String>())
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn set_robots_ok(db: &Db, id: bg_core::SourceId, ok: bool) -> Result<()> {
    sqlx::query("UPDATE sources SET robots_ok = $2 WHERE id = $1")
        .bind(id.into_uuid())
        .bind(ok)
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn set_enabled(db: &Db, slug: &str, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE sources SET enabled = $2 WHERE slug = $1")
        .bind(slug)
        .bind(enabled)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Lightweight health view for `bg doctor` and the `/developers` page.
pub struct SourceHealth {
    pub slug: String,
    pub name: String,
    pub enabled: bool,
    pub robots_ok: bool,
    pub items: i64,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub async fn health(db: &Db) -> Result<Vec<SourceHealth>> {
    let rows = sqlx::query(
        "SELECT s.slug, s.name, s.enabled, s.robots_ok, s.last_polled_at, s.last_error,
                count(r.id) AS items
         FROM sources s
         LEFT JOIN raw_items r ON r.source_id = s.id
         GROUP BY s.id
         ORDER BY s.trust DESC, s.slug",
    )
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| SourceHealth {
            slug: r.get("slug"),
            name: r.get("name"),
            enabled: r.get("enabled"),
            robots_ok: r.get("robots_ok"),
            items: r.get("items"),
            last_polled_at: r.get("last_polled_at"),
            last_error: r.get("last_error"),
        })
        .collect())
}
