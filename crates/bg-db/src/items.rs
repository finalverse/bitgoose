//! Raw source items.
//!
//! Note the split between [`recent`] (public projection, safe to serialize) and
//! [`body_for_analysis`] / [`bodies_for_story`] (private working text). Keeping
//! them as separate functions with separate return types means "did we just
//! leak source text?" is answerable by grepping for two names.

use crate::{convert::*, Db, Result};
use bg_core::domain::{ItemRole, RawItem, RawItemPublic};
use bg_core::ids::{RawItemId, SourceId, StoryId};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

/// Columns for the full record. `body_raw` is included only because
/// [`from_row`] is used by the analysis paths; the public paths use [`PUB_COLS`].
const COLS: &str = "id, source_id, external_id, canonical_url, url_hash, title, dek, authors, \
                    published_at, fetched_at, summary_raw, body_raw, body_hash, simhash, lang, \
                    image_url, story_id, triaged";

const PUB_COLS: &str = "id, source_id, canonical_url, title, authors, published_at, image_url";

fn from_row(r: &PgRow) -> Result<RawItem> {
    Ok(RawItem {
        id: raw_item_id(r, "id")?,
        source_id: source_id(r, "source_id")?,
        external_id: r.try_get("external_id")?,
        canonical_url: r.try_get("canonical_url")?,
        url_hash: r.try_get("url_hash")?,
        title: r.try_get("title")?,
        dek: r.try_get("dek")?,
        authors: r.try_get("authors")?,
        published_at: r.try_get("published_at")?,
        fetched_at: r.try_get("fetched_at")?,
        summary_raw: r.try_get("summary_raw")?,
        body_raw: r.try_get("body_raw")?,
        body_hash: r.try_get("body_hash")?,
        simhash: r.try_get("simhash")?,
        lang: r.try_get("lang")?,
        image_url: r.try_get("image_url")?,
        story_id: story_id_opt(r, "story_id")?,
        triaged: r.try_get("triaged")?,
    })
}

fn pub_from_row(r: &PgRow) -> Result<RawItemPublic> {
    Ok(RawItemPublic {
        id: raw_item_id(r, "id")?,
        source_id: source_id(r, "source_id")?,
        canonical_url: r.try_get("canonical_url")?,
        title: r.try_get("title")?,
        authors: r.try_get("authors")?,
        published_at: r.try_get("published_at")?,
        image_url: r.try_get("image_url")?,
    })
}

/// What the ingest layer produces before an ID is assigned.
#[derive(Debug, Clone)]
pub struct NewItem {
    pub source_id: SourceId,
    pub external_id: Option<String>,
    pub canonical_url: String,
    pub url_hash: String,
    pub title: String,
    pub dek: Option<String>,
    pub authors: Vec<String>,
    pub published_at: DateTime<Utc>,
    pub summary_raw: Option<String>,
    pub body_raw: Option<String>,
    pub body_hash: Option<String>,
    pub simhash: u64,
    pub lang: String,
    pub image_url: Option<String>,
}

/// Insert unless `url_hash` already exists.
///
/// Returns `None` on conflict, which is the common case — most of a feed on any
/// given poll is items we already have. Callers use the `None` count as the
/// "nothing new" signal rather than diffing.
pub async fn insert_new(db: &Db, it: &NewItem) -> Result<Option<RawItemId>> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO raw_items
           (id, source_id, external_id, canonical_url, url_hash, title, dek, authors,
            published_at, summary_raw, body_raw, body_hash, simhash, lang, image_url)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
         ON CONFLICT (url_hash) DO NOTHING
         RETURNING id",
    )
    .bind(id)
    .bind(it.source_id.into_uuid())
    .bind(&it.external_id)
    .bind(&it.canonical_url)
    .bind(&it.url_hash)
    .bind(&it.title)
    .bind(&it.dek)
    .bind(&it.authors)
    .bind(it.published_at)
    .bind(&it.summary_raw)
    .bind(&it.body_raw)
    .bind(&it.body_hash)
    .bind(simhash_to_db(it.simhash))
    .bind(&it.lang)
    .bind(&it.image_url)
    .fetch_optional(&db.pool)
    .await?;
    Ok(row.map(|r| RawItemId::from_uuid(r.get::<Uuid, _>("id"))))
}

pub async fn count(db: &Db) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM raw_items").fetch_one(&db.pool).await?)
}

/// Items Gosling has not yet read, newest first.
pub async fn untriaged(db: &Db, limit: i64) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items WHERE NOT triaged ORDER BY published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn mark_triaged(
    db: &Db,
    id: RawItemId,
    category: Option<&str>,
    assets: &[String],
    score: i16,
) -> Result<()> {
    sqlx::query(
        "UPDATE raw_items SET triaged = TRUE, category = $2, assets = $3, triage_score = $4
         WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(category)
    .bind(assets)
    .bind(score)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Triaged items not yet attached to a story — the clustering input.
pub async fn unclustered(db: &Db, limit: i64) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items
         WHERE triaged AND story_id IS NULL
         ORDER BY published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Recent items that already belong to a story — the clustering *candidates*.
///
/// Restricted to a time window because near-duplicate matching across the whole
/// archive would both cost more and be wrong: the same headline six months
/// apart is two events, not one.
pub async fn clustering_candidates(db: &Db, hours: i64, limit: i64) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items
         WHERE story_id IS NOT NULL
           AND published_at > now() - make_interval(hours => $1)
         ORDER BY published_at DESC LIMIT $2"
    ))
    .bind(hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Attach an item to a story and record how it relates to it.
pub async fn attach_to_story(
    db: &Db,
    item: RawItemId,
    story: StoryId,
    role: ItemRole,
) -> Result<()> {
    let mut tx = db.pool.begin().await?;
    sqlx::query("UPDATE raw_items SET story_id = $2 WHERE id = $1")
        .bind(item.into_uuid())
        .bind(story.into_uuid())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO story_items (story_id, raw_item_id, role) VALUES ($1,$2,$3)
         ON CONFLICT (story_id, raw_item_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(story.into_uuid())
    .bind(item.into_uuid())
    .bind(role.as_str())
    .execute(&mut *tx)
    .await?;
    // Denormalized so front-page queries never need the join.
    sqlx::query(
        "UPDATE stories SET
            source_count = (SELECT count(DISTINCT r.source_id)
                            FROM story_items si JOIN raw_items r ON r.id = si.raw_item_id
                            WHERE si.story_id = $1),
            updated_at = now()
         WHERE id = $1",
    )
    .bind(story.into_uuid())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn by_story(db: &Db, story: StoryId) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items WHERE story_id = $1 ORDER BY published_at ASC"
    ))
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn recent_public(db: &Db, limit: i64) -> Result<Vec<RawItemPublic>> {
    let rows = crate::sql(format!(
        "SELECT {PUB_COLS} FROM raw_items ORDER BY published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(pub_from_row).collect()
}

// -- private working text ---------------------------------------------------
// The two functions below are the ONLY ones that hand out `body_raw`. They
// return bare strings rather than a serializable struct so the text cannot
// accidentally ride along inside an API response type.

/// Source text for claim extraction. Never rendered, never serialized.
pub async fn body_for_analysis(db: &Db, id: RawItemId) -> Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT body_raw FROM raw_items WHERE id = $1")
        .bind(id.into_uuid())
        .fetch_optional(&db.pool)
        .await?
        .flatten())
}

/// `(source_slug, body)` for every item on a story, for the policy engine's
/// verbatim-overlap check.
pub async fn bodies_for_story(db: &Db, story: StoryId) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT s.slug AS slug, r.body_raw AS body
         FROM raw_items r JOIN sources s ON s.id = r.source_id
         WHERE r.story_id = $1 AND r.body_raw IS NOT NULL",
    )
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get("slug"), r.get("body"))).collect())
}
