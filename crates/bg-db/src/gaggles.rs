//! Gaggles — special topics, opened when coverage converges.

use crate::{Db, Result};
use bg_core::ids::{RunId, StoryId};
use sqlx::Row;
use uuid::Uuid;

/// Headlines from the recent window, paired with the outlet that ran them.
///
/// The input to [`bg_core::trends::rank`]. Deliberately reads *raw items* rather
/// than published stories: a subject can be hot across the wires before the
/// pipeline has turned any of it into stories, and on a tier that triages a
/// fraction of intake it usually is.
pub async fn recent_headlines(db: &Db, hours: i64, limit: i64) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT r.title, s.slug
           FROM raw_items r
           JOIN sources s ON s.id = r.source_id
          WHERE r.published_at > now() - make_interval(hours => $1)
            AND s.robots_ok
          ORDER BY r.published_at DESC
          LIMIT $2",
    )
    .bind(hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("title"), r.get("slug")))
        .collect())
}

/// Headlines from *before* the current window, for the baseline.
///
/// Everything between `skip_hours` and `back_hours` ago, so the comparison is
/// against a subject's history rather than against itself.
pub async fn baseline_headlines(
    db: &Db,
    skip_hours: i64,
    back_hours: i64,
    limit: i64,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT r.title, s.slug
           FROM raw_items r
           JOIN sources s ON s.id = r.source_id
          WHERE r.published_at <= now() - make_interval(hours => $1)
            AND r.published_at >  now() - make_interval(hours => $2)
            AND s.robots_ok
          ORDER BY r.published_at DESC
          LIMIT $3",
    )
    .bind(skip_hours as i32)
    .bind(back_hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("title"), r.get("slug")))
        .collect())
}

/// Published stories whose headline carries a topic, newest first.
///
/// Matched on the title rather than a stored tag: the topic was *derived* from
/// titles, so matching anywhere else would put stories in a gaggle that do not
/// visibly belong to it, and a reader looking at the page would not see why.
pub async fn stories_for_topic(db: &Db, topic: &str, limit: i64) -> Result<Vec<StoryId>> {
    let rows = sqlx::query(
        "SELECT id FROM stories
          WHERE status = 'published' AND title ILIKE '%' || $1 || '%'
          ORDER BY published_at DESC
          LIMIT $2",
    )
    .bind(topic)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| Ok(StoryId::from_uuid(r.try_get::<Uuid, _>("id")?)))
        .collect()
}

pub struct NewGaggle<'a> {
    pub topic: &'a str,
    pub slug: &'a str,
    pub title: &'a str,
    pub standfirst: &'a str,
    pub source_count: i32,
    pub story_count: i32,
    pub model: Option<String>,
}

/// Open a gaggle, or refresh one that is still hot.
///
/// Keyed on the topic so a subject that stays in the news updates in place.
/// Re-opening it as a second page every few hours would turn a live topic into
/// a pile of near-identical pages, which is the failure mode of every
/// auto-generated topic hub on the web.
pub async fn upsert(db: &Db, g: &NewGaggle<'_>, run: Option<RunId>) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO gaggles
           (id, topic, slug, title, standfirst, source_count, story_count, model, run_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (topic) DO UPDATE SET
            source_count = EXCLUDED.source_count,
            story_count  = EXCLUDED.story_count,
            last_hot_at  = now()
         RETURNING id",
    )
    .bind(id)
    .bind(g.topic)
    .bind(g.slug)
    .bind(g.title)
    .bind(g.standfirst)
    .bind(g.source_count)
    .bind(g.story_count)
    .bind(&g.model)
    .bind(run.map(|r| r.into_uuid()))
    .fetch_one(&db.pool)
    .await?;
    Ok(row.get::<Uuid, _>("id"))
}

/// Replace a gaggle's membership.
///
/// Cleared and rewritten rather than appended: a story that no longer matches
/// should leave, and a topic page that only ever grows accumulates everything
/// that once brushed against the subject.
pub async fn set_stories(db: &Db, gaggle: Uuid, stories: &[StoryId]) -> Result<()> {
    let mut tx = db.pool.begin().await?;
    sqlx::query("DELETE FROM gaggle_stories WHERE gaggle_id = $1")
        .bind(gaggle)
        .execute(&mut *tx)
        .await?;
    for s in stories {
        sqlx::query(
            "INSERT INTO gaggle_stories (gaggle_id, story_id) VALUES ($1,$2)
             ON CONFLICT DO NOTHING",
        )
        .bind(gaggle)
        .bind(s.into_uuid())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Whether we already have a gaggle for this topic.
pub async fn exists(db: &Db, topic: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM gaggles WHERE topic = $1")
            .bind(topic)
            .fetch_one(&db.pool)
            .await?
            > 0,
    )
}

/// A gaggle as the site renders it.
#[derive(Debug, Clone)]
pub struct GaggleRow {
    pub topic: String,
    pub slug: String,
    pub title: String,
    pub standfirst: String,
    pub source_count: i32,
    pub story_count: i32,
    pub model: Option<String>,
}

fn row(r: &sqlx::postgres::PgRow) -> Result<GaggleRow> {
    Ok(GaggleRow {
        topic: r.try_get("topic")?,
        slug: r.try_get("slug")?,
        title: r.try_get("title")?,
        standfirst: r.try_get("standfirst")?,
        source_count: r.try_get("source_count")?,
        story_count: r.try_get("story_count")?,
        model: r.try_get("model")?,
    })
}

const COLS: &str = "topic, slug, title, standfirst, source_count, story_count, model";

/// Gaggles still being covered, hottest first.
///
/// Windowed rather than listing everything: a topic page nobody has written
/// about for a week is an archive entry, not a live special topic, and the
/// front page should not offer it as one.
pub async fn live(db: &Db, within_hours: i64, limit: i64) -> Result<Vec<GaggleRow>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM gaggles
          WHERE last_hot_at > now() - make_interval(hours => $1)
          ORDER BY source_count DESC, story_count DESC
          LIMIT $2"
    ))
    .bind(within_hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(row).collect()
}

pub async fn by_slug(db: &Db, slug: &str) -> Result<Option<GaggleRow>> {
    let r = crate::sql(format!("SELECT {COLS} FROM gaggles WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(&db.pool)
        .await?;
    r.as_ref().map(row).transpose()
}

/// The stories on a gaggle's page.
pub async fn story_ids(db: &Db, slug: &str) -> Result<Vec<StoryId>> {
    let rows = sqlx::query(
        "SELECT gs.story_id
           FROM gaggle_stories gs
           JOIN gaggles g ON g.id = gs.gaggle_id
           JOIN stories s ON s.id = gs.story_id
          WHERE g.slug = $1 AND s.status = 'published'
          ORDER BY s.published_at DESC",
    )
    .bind(slug)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| Ok(StoryId::from_uuid(r.try_get::<Uuid, _>("story_id")?)))
        .collect()
}

pub async fn count(db: &Db) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM gaggles")
        .fetch_one(&db.pool)
        .await?)
}
