//! Gaggles — special topics, opened when coverage converges.

use crate::{Db, Result};
use bg_core::{
    domain::EditorialLanguage,
    ids::{RunId, StoryId},
};
use sqlx::Row;
use uuid::Uuid;

/// Headlines from the recent window, paired with the outlet that ran them.
///
/// The input to [`bg_core::trends::rank`]. Deliberately reads *raw items* rather
/// than published stories: a subject can be hot across the wires before the
/// pipeline has turned any of it into stories, and on a tier that triages a
/// fraction of intake it usually is.
pub async fn recent_headlines(
    db: &Db,
    language: EditorialLanguage,
    hours: i64,
    limit: i64,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT r.title, s.homepage
           FROM raw_items r
           JOIN sources s ON s.id = r.source_id
          WHERE r.published_at > now() - make_interval(hours => $1)
            AND s.robots_ok
            AND CASE
                WHEN $2 = 'zh-hant' THEN lower(r.lang) = 'zh-hant'
                WHEN $2 = 'zh' THEN lower(r.lang) = 'zh'
                WHEN $2 = 'fr' THEN lower(r.lang) LIKE 'fr%'
                WHEN $2 = 'es' THEN lower(r.lang) LIKE 'es%'
                WHEN $2 = 'ja' THEN lower(r.lang) LIKE 'ja%'
                WHEN $2 = 'ko' THEN lower(r.lang) LIKE 'ko%'
                ELSE lower(r.lang) NOT LIKE 'zh%' AND lower(r.lang) NOT LIKE 'fr%'
                     AND lower(r.lang) NOT LIKE 'es%'
                     AND lower(r.lang) NOT LIKE 'ja%' AND lower(r.lang) NOT LIKE 'ko%'
            END
          ORDER BY r.published_at DESC
          LIMIT $3",
    )
    .bind(hours as i32)
    .bind(language.as_str())
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("title"), r.get("homepage")))
        .collect())
}

/// Headlines from *before* the current window, for the baseline.
///
/// Everything between `skip_hours` and `back_hours` ago, so the comparison is
/// against a subject's history rather than against itself.
pub async fn baseline_headlines(
    db: &Db,
    language: EditorialLanguage,
    skip_hours: i64,
    back_hours: i64,
    limit: i64,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT r.title, s.homepage
           FROM raw_items r
           JOIN sources s ON s.id = r.source_id
          WHERE r.published_at <= now() - make_interval(hours => $1)
            AND r.published_at >  now() - make_interval(hours => $2)
            AND s.robots_ok
            AND CASE
                WHEN $3 = 'zh-hant' THEN lower(r.lang) = 'zh-hant'
                WHEN $3 = 'zh' THEN lower(r.lang) = 'zh'
                WHEN $3 = 'fr' THEN lower(r.lang) LIKE 'fr%'
                WHEN $3 = 'es' THEN lower(r.lang) LIKE 'es%'
                WHEN $3 = 'ja' THEN lower(r.lang) LIKE 'ja%'
                WHEN $3 = 'ko' THEN lower(r.lang) LIKE 'ko%'
                ELSE lower(r.lang) NOT LIKE 'zh%' AND lower(r.lang) NOT LIKE 'fr%'
                     AND lower(r.lang) NOT LIKE 'es%'
                     AND lower(r.lang) NOT LIKE 'ja%' AND lower(r.lang) NOT LIKE 'ko%'
            END
          ORDER BY r.published_at DESC
          LIMIT $4",
    )
    .bind(skip_hours as i32)
    .bind(back_hours as i32)
    .bind(language.as_str())
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("title"), r.get("homepage")))
        .collect())
}

/// Published stories whose headline carries a topic, newest first.
///
/// Matched on the title rather than a stored tag: the topic was *derived* from
/// titles, so matching anywhere else would put stories in a gaggle that do not
/// visibly belong to it, and a reader looking at the page would not see why.
pub async fn stories_for_topic(
    db: &Db,
    topic: &str,
    language: EditorialLanguage,
    limit: i64,
) -> Result<Vec<StoryId>> {
    let rows = sqlx::query(
        "SELECT id FROM stories
          WHERE status = 'published' AND title ILIKE '%' || $1 || '%'
            AND editorial_language = $2
          ORDER BY published_at DESC
          LIMIT $3",
    )
    .bind(topic)
    .bind(language.as_str())
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
    pub editorial_language: EditorialLanguage,
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
           (id, topic, slug, title, standfirst, source_count, story_count, model, run_id,
            editorial_language)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (topic, editorial_language) DO UPDATE SET
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
    .bind(g.editorial_language.as_str())
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

/// Refresh every permanent topic from its language-specific anchor and signal
/// terms. This is deliberately mechanical and cheap enough for the fast loop:
/// the models write stories and periodic briefs, but they do not get to decide
/// whether an already-published story visibly contains the tracked subject.
pub async fn refresh_tracked(db: &Db) -> Result<usize> {
    let tracked = sqlx::query(
        "SELECT id, editorial_language, anchor_terms, keywords
           FROM gaggles
          WHERE pinned",
    )
    .fetch_all(&db.pool)
    .await?;

    for topic in &tracked {
        let id: Uuid = topic.try_get("id")?;
        let language: String = topic.try_get("editorial_language")?;
        let anchors: Vec<String> = topic.try_get("anchor_terms")?;
        let signals: Vec<String> = topic.try_get("keywords")?;

        let rows = sqlx::query(
            "SELECT s.id
               FROM stories s
              WHERE s.status = 'published'
                AND s.editorial_language = $1
                AND EXISTS (
                    SELECT 1 FROM unnest($2::text[]) term
                     WHERE concat_ws(' ', s.title, s.summary) ILIKE '%' || term || '%'
                )
                AND EXISTS (
                    SELECT 1 FROM unnest($3::text[]) term
                     WHERE concat_ws(' ', s.title, s.summary) ILIKE '%' || term || '%'
                )
              ORDER BY s.published_at DESC
              LIMIT 200",
        )
        .bind(&language)
        .bind(&anchors)
        .bind(&signals)
        .fetch_all(&db.pool)
        .await?;
        let story_ids: Vec<StoryId> = rows
            .iter()
            .map(|r| r.try_get::<Uuid, _>("id").map(StoryId::from_uuid))
            .collect::<std::result::Result<_, _>>()?;
        set_stories(db, id, &story_ids).await?;

        let ids: Vec<Uuid> = story_ids.iter().map(|s| s.into_uuid()).collect();
        let source_count = if ids.is_empty() {
            0i64
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(DISTINCT r.source_id)
                   FROM story_items si
                   JOIN raw_items r ON r.id = si.raw_item_id
                  WHERE si.story_id = ANY($1)",
            )
            .bind(&ids)
            .fetch_one(&db.pool)
            .await?
        };
        sqlx::query(
            "UPDATE gaggles
                SET source_count = $2, story_count = $3, last_hot_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(source_count as i32)
        .bind(story_ids.len() as i32)
        .execute(&db.pool)
        .await?;
    }
    Ok(tracked.len())
}

/// Whether we already have a gaggle for this topic.
pub async fn exists(db: &Db, topic: &str, language: EditorialLanguage) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM gaggles WHERE topic = $1 AND editorial_language = $2",
    )
    .bind(topic)
    .bind(language.as_str())
    .fetch_one(&db.pool)
    .await?
        > 0)
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
    pub editorial_language: String,
    pub pinned: bool,
    pub analysis_md: String,
    pub watchpoints: Vec<String>,
    pub primary_source_names: Vec<String>,
    pub primary_source_urls: Vec<String>,
    pub last_briefed_at: Option<chrono::DateTime<chrono::Utc>>,
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
        editorial_language: r.try_get("editorial_language")?,
        pinned: r.try_get("pinned")?,
        analysis_md: r.try_get("analysis_md")?,
        watchpoints: r.try_get("watchpoints")?,
        primary_source_names: r.try_get("primary_source_names")?,
        primary_source_urls: r.try_get("primary_source_urls")?,
        last_briefed_at: r.try_get("last_briefed_at")?,
    })
}

const COLS: &str = "topic, slug, title, standfirst, source_count, story_count, model, \
                   editorial_language, pinned, analysis_md, watchpoints, \
                   primary_source_names, primary_source_urls, last_briefed_at";

/// Gaggles still being covered, hottest first.
///
/// Windowed rather than listing everything: a topic page nobody has written
/// about for a week is an archive entry, not a live special topic, and the
/// front page should not offer it as one.
pub async fn live(db: &Db, within_hours: i64, limit: i64) -> Result<Vec<GaggleRow>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM gaggles
          WHERE pinned OR last_hot_at > now() - make_interval(hours => $1)
          -- Heat, not size. Ordering by source_count alone meant a subject that
          -- once drew nine outlets outranked one drawing six an hour ago for as
          -- long as it kept qualifying at all, so the Special Topics row showed
          -- the same three entries for days while the wires moved underneath.
          -- A twelve-hour half-life puts a fresh convergence in front of a
          -- stale one without letting a single-outlet flurry displace a story
          -- the whole press is covering.
          ORDER BY pinned DESC,
                   source_count
                   * exp(-extract(epoch from (now() - last_hot_at)) / 43200.0)
                   DESC,
                   story_count DESC
          LIMIT $2"
    ))
    .bind(within_hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(row).collect()
}

/// Live topics that actually contain published stories for one independent
/// edition. A topic may be detected across the whole wire, but it must never
/// leak onto the other edition's front page merely because its framing exists.
pub async fn live_for_language(
    db: &Db,
    language: EditorialLanguage,
    within_hours: i64,
    limit: i64,
) -> Result<Vec<GaggleRow>> {
    let rows = crate::sql(format!(
        "SELECT {COLS}, g.last_hot_at
           FROM gaggles g
          WHERE (
                (g.pinned AND g.editorial_language = $2)
                OR (
                    NOT g.pinned
                    AND g.last_hot_at > now() - make_interval(hours => $1)
                    AND EXISTS (
                        SELECT 1
                          FROM gaggle_stories gs
                          JOIN stories s ON s.id = gs.story_id
                         WHERE gs.gaggle_id = g.id
                           AND s.status = 'published'
                           AND s.editorial_language = $2
                    )
                )
          )
          ORDER BY g.pinned DESC, g.last_hot_at DESC,
                   g.source_count DESC, g.story_count DESC
          LIMIT $3"
    ))
    .bind(within_hours as i32)
    .bind(language.as_str())
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(row).collect()
}

pub async fn by_slug(
    db: &Db,
    slug: &str,
    language: EditorialLanguage,
) -> Result<Option<GaggleRow>> {
    let r = crate::sql(format!(
        "SELECT {COLS} FROM gaggles
          WHERE slug = $1 AND editorial_language = $2"
    ))
    .bind(slug)
    .bind(language.as_str())
    .fetch_optional(&db.pool)
    .await?;
    r.as_ref().map(row).transpose()
}

/// The stories on a gaggle's page.
pub async fn story_ids(db: &Db, slug: &str, language: EditorialLanguage) -> Result<Vec<StoryId>> {
    let rows = sqlx::query(
        "SELECT gs.story_id
           FROM gaggle_stories gs
           JOIN gaggles g ON g.id = gs.gaggle_id
           JOIN stories s ON s.id = gs.story_id
          WHERE g.slug = $1
            AND g.editorial_language = $2
            AND s.status = 'published'
            AND s.editorial_language = $2
          ORDER BY s.published_at DESC",
    )
    .bind(slug)
    .bind(language.as_str())
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

/// Every gaggle's id, title and slug — for the Steward to audit.
pub async fn all_titles(db: &Db) -> Result<Vec<(uuid::Uuid, String, String)>> {
    let rows = sqlx::query("SELECT id, title, slug FROM gaggles")
        .fetch_all(&db.pool)
        .await?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get::<uuid::Uuid, _>("id")?,
                r.try_get("title")?,
                r.try_get("slug")?,
            ))
        })
        .collect()
}

/// Remove a special topic and its story links.
///
/// A gaggle is BitGoose's own furniture rather than reporting, so unlike a
/// story it can simply go: nothing was published under its URL that another
/// site would have linked to, and the stories it collected are untouched.
pub async fn delete(db: &Db, id: uuid::Uuid) -> Result<()> {
    let mut tx = db.pool.begin().await?;
    sqlx::query("DELETE FROM gaggle_stories WHERE gaggle_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM gaggles WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Special topics that stopped being topics — cold for longer than `hours`.
///
/// Returned rather than deleted so the caller decides, and so a read-only
/// Steward round can report what it would retire without touching anything.
pub async fn cold(db: &Db, hours: i64) -> Result<Vec<(uuid::Uuid, String, i64)>> {
    let rows: Vec<(uuid::Uuid, String, Option<f64>)> = sqlx::query_as(
        "SELECT id, title,
                extract(epoch from (now() - last_hot_at)) / 3600.0
           FROM gaggles
          WHERE NOT pinned
            AND last_hot_at < now() - make_interval(hours => $1::int)
          ORDER BY last_hot_at ASC",
    )
    .bind(hours)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, h)| (id, title, h.unwrap_or_default() as i64))
        .collect())
}

/// A permanent topic whose editorial brief is due for synthesis.
#[derive(Debug, Clone)]
pub struct TrackedBrief {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub standfirst: String,
    pub analysis_md: String,
    pub watchpoints: Vec<String>,
    pub editorial_language: EditorialLanguage,
    pub primary_source_names: Vec<String>,
    pub primary_source_urls: Vec<String>,
}

/// Permanent topics are re-synthesised on an editorial cadence. Their story
/// list still refreshes in the fast loop; prose does not need rewriting every
/// ninety seconds when no evidence has changed.
pub async fn tracked_due(db: &Db, hours: i64, limit: i64) -> Result<Vec<TrackedBrief>> {
    use std::str::FromStr;
    let rows = sqlx::query(
        "SELECT id, slug, title, standfirst, analysis_md, watchpoints,
                editorial_language, primary_source_names, primary_source_urls
           FROM gaggles
          WHERE pinned
            AND (last_briefed_at IS NULL
                 OR last_briefed_at < now() - make_interval(hours => $1::int))
          ORDER BY last_briefed_at ASC NULLS FIRST
          LIMIT $2",
    )
    .bind(hours)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            let language: String = r.try_get("editorial_language")?;
            Ok(TrackedBrief {
                id: r.try_get("id")?,
                slug: r.try_get("slug")?,
                title: r.try_get("title")?,
                standfirst: r.try_get("standfirst")?,
                analysis_md: r.try_get("analysis_md")?,
                watchpoints: r.try_get("watchpoints")?,
                editorial_language: EditorialLanguage::from_str(&language)
                    .unwrap_or(EditorialLanguage::En),
                primary_source_names: r.try_get("primary_source_names")?,
                primary_source_urls: r.try_get("primary_source_urls")?,
            })
        })
        .collect()
}

pub async fn update_tracked_brief(
    db: &Db,
    id: Uuid,
    standfirst: &str,
    analysis_md: &str,
    watchpoints: &[String],
    model: &str,
    run: RunId,
) -> Result<()> {
    sqlx::query(
        "UPDATE gaggles
            SET standfirst = $2, analysis_md = $3, watchpoints = $4,
                model = $5, run_id = $6, last_briefed_at = now()
          WHERE id = $1 AND pinned",
    )
    .bind(id)
    .bind(standfirst)
    .bind(analysis_md)
    .bind(watchpoints)
    .bind(model)
    .bind(run.into_uuid())
    .execute(&db.pool)
    .await?;
    Ok(())
}
