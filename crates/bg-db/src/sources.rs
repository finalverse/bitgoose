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
        beat: enum_col_opt::<bg_core::domain::Beat>(r, "beat")?,
        robots_ok: r.try_get("robots_ok")?,
        ai_input_ok: r.try_get("ai_input_ok")?,
        ai_signal: r.try_get("ai_signal")?,
        poll_interval_s: r.try_get("poll_interval_s")?,
        etag: r.try_get("etag")?,
        last_modified: r.try_get("last_modified")?,
        last_polled_at: r.try_get("last_polled_at")?,
        last_error: r.try_get("last_error")?,
        enabled: r.try_get("enabled")?,
        created_at: r.try_get("created_at")?,
    })
}

const COLS: &str = "id, slug, name, kind, url, homepage, trust, beat, robots_ok, \
     ai_input_ok, ai_signal, poll_interval_s, \
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
    beat: Option<bg_core::domain::Beat>,
) -> Result<Source> {
    let row = crate::sql(format!(
        "INSERT INTO sources (id, slug, name, kind, url, homepage, trust, poll_interval_s, beat)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (slug) DO UPDATE SET
            name = EXCLUDED.name,
            kind = EXCLUDED.kind,
            url = EXCLUDED.url,
            homepage = EXCLUDED.homepage,
            trust = EXCLUDED.trust,
            poll_interval_s = EXCLUDED.poll_interval_s,
            beat = EXCLUDED.beat
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
    .bind(beat.map(|b| b.as_str()))
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

/// Record what a site says about putting its text into a model.
pub async fn set_ai_input(
    db: &Db,
    id: bg_core::SourceId,
    ok: bool,
    signal: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE sources SET ai_input_ok = $2, ai_signal = $3 WHERE id = $1")
        .bind(id.into_uuid())
        .bind(ok)
        .bind(signal)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Items whose source does not permit model input, so the Skein must not read
/// them and the extractor must not store their text.
pub async fn ai_input_denied(db: &Db) -> Result<Vec<bg_core::SourceId>> {
    let rows = sqlx::query("SELECT id FROM sources WHERE NOT ai_input_ok")
        .fetch_all(&db.pool)
        .await?;
    rows.iter()
        .map(|r| Ok(bg_core::SourceId::from(r.try_get::<uuid::Uuid, _>("id")?)))
        .collect()
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

/// Sources that are failing *and* producing nothing.
///
/// There is no failure counter — `last_error` is cleared on every success, so
/// it says only that the most recent poll failed. On this host a single failed
/// poll is noise: the uplink drops a large share of its packets and eleven of
/// fifteen polls failed at once on a bad afternoon with every feed healthy.
///
/// So failure alone is not the signal. Failing *and* having produced no item
/// for a long stretch is: that separates a feed that is genuinely gone from one
/// the network could not reach this minute.
pub async fn failing_and_barren(
    db: &Db,
    quiet_hours: i64,
) -> Result<Vec<(bg_core::SourceId, String, i64, String)>> {
    let rows = sqlx::query(
        "SELECT s.id, s.slug,
                coalesce(round(extract(epoch FROM (now() - max(r.fetched_at)))/3600)::bigint,
                         $1 * 10) AS quiet,
                coalesce(left(s.last_error, 80), '') AS err
           FROM sources s
           LEFT JOIN raw_items r ON r.source_id = s.id
          WHERE s.enabled AND s.last_error IS NOT NULL
          GROUP BY s.id, s.slug, s.last_error
         HAVING coalesce(max(r.fetched_at), 'epoch'::timestamptz)
                < now() - make_interval(hours => $1::int)
          ORDER BY 3 DESC",
    )
    .bind(quiet_hours as i32)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok((
                bg_core::SourceId::from(r.try_get::<uuid::Uuid, _>("id")?),
                r.try_get("slug")?,
                r.try_get("quiet")?,
                r.try_get("err")?,
            ))
        })
        .collect()
}
