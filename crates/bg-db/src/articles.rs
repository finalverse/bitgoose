//! Articles: versioned renderings of a claim set, plus their corrections.

use crate::{convert::*, Db, DbError, Result};
use bg_core::domain::{Article, Correction, StoryStatus};
use bg_core::ids::{AgentId, ArticleId, ClaimId, RunId, StoryId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

const COLS: &str = "id, story_id, version, headline, dek, slug, body_md, seo_title, seo_desc, \
                    reading_time_s, status, published_at, content_hash, editor_run_id, created_at";

fn from_row(r: &PgRow) -> Result<Article> {
    Ok(Article {
        id: article_id(r, "id")?,
        story_id: story_id(r, "story_id")?,
        version: r.try_get("version")?,
        headline: r.try_get("headline")?,
        dek: r.try_get("dek")?,
        slug: r.try_get("slug")?,
        body_md: r.try_get("body_md")?,
        seo_title: r.try_get("seo_title")?,
        seo_desc: r.try_get("seo_desc")?,
        reading_time_s: r.try_get("reading_time_s")?,
        status: enum_col::<StoryStatus>(r, "status")?,
        published_at: r.try_get("published_at")?,
        content_hash: r.try_get("content_hash")?,
        editor_run_id: run_id_opt(r, "editor_run_id")?,
        created_at: r.try_get("created_at")?,
    })
}

#[derive(Debug, Clone)]
pub struct NewArticle {
    pub headline: String,
    pub dek: String,
    pub slug: String,
    pub body_md: String,
    pub seo_title: String,
    pub seo_desc: String,
    pub content_hash: String,
}

/// Insert the next version of a story's article.
///
/// Versions are allocated as `max + 1` inside the insert rather than read then
/// written, so two agents finishing at once cannot collide on a version number.
pub async fn insert_version(
    db: &Db,
    story: StoryId,
    a: &NewArticle,
    editor_run: Option<RunId>,
) -> Result<Article> {
    let row = crate::sql(format!(
        "INSERT INTO articles
           (id, story_id, version, headline, dek, slug, body_md, seo_title, seo_desc,
            reading_time_s, status, content_hash, editor_run_id)
         VALUES ($1, $2,
                 (SELECT COALESCE(max(version), 0) + 1 FROM articles WHERE story_id = $2),
                 $3,$4,$5,$6,$7,$8,$9,'review',$10,$11)
         RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(story.into_uuid())
    .bind(&a.headline)
    .bind(&a.dek)
    .bind(&a.slug)
    .bind(&a.body_md)
    .bind(&a.seo_title)
    .bind(&a.seo_desc)
    .bind(bg_core::text::reading_time_s(&a.body_md))
    .bind(&a.content_hash)
    .bind(editor_run.map(|r| r.into_uuid()))
    .fetch_one(&db.pool)
    .await?;
    from_row(&row)
}

pub async fn publish(db: &Db, id: ArticleId) -> Result<()> {
    sqlx::query(
        "UPDATE articles SET status = 'published', published_at = COALESCE(published_at, now())
         WHERE id = $1",
    )
    .bind(id.into_uuid())
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn latest_for_story(db: &Db, story: StoryId) -> Result<Option<Article>> {
    let row = crate::sql(format!(
        "SELECT {COLS} FROM articles WHERE story_id = $1 ORDER BY version DESC LIMIT 1"
    ))
    .bind(story.into_uuid())
    .fetch_optional(&db.pool)
    .await?;
    row.as_ref().map(from_row).transpose()
}

pub async fn by_id(db: &Db, id: ArticleId) -> Result<Article> {
    let row = crate::sql(format!("SELECT {COLS} FROM articles WHERE id = $1"))
        .bind(id.into_uuid())
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound("article"))?;
    from_row(&row)
}

pub async fn add_citations(db: &Db, article: ArticleId, pairs: &[(String, ClaimId)]) -> Result<()> {
    for (marker, claim) in pairs {
        sqlx::query(
            "INSERT INTO article_citations (article_id, marker, claim_id) VALUES ($1,$2,$3)
             ON CONFLICT (article_id, marker) DO UPDATE SET claim_id = EXCLUDED.claim_id",
        )
        .bind(article.into_uuid())
        .bind(marker)
        .bind(claim.into_uuid())
        .execute(&db.pool)
        .await?;
    }
    Ok(())
}

pub async fn citations(db: &Db, article: ArticleId) -> Result<Vec<(String, ClaimId)>> {
    let rows = sqlx::query(
        "SELECT marker, claim_id FROM article_citations WHERE article_id = $1 ORDER BY marker",
    )
    .bind(article.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(|r| Ok((r.try_get("marker")?, claim_id(r, "claim_id")?))).collect()
}

// -- corrections ------------------------------------------------------------

pub async fn add_correction(
    db: &Db,
    article: ArticleId,
    from_v: i32,
    to_v: i32,
    reason: &str,
    diff_md: &str,
    agent: Option<AgentId>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO corrections (id, article_id, from_version, to_version, reason, diff_md, agent_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(article.into_uuid())
    .bind(from_v)
    .bind(to_v)
    .bind(reason)
    .bind(diff_md)
    .bind(agent.map(|a| a.into_uuid()))
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Every correction ever issued against any version of a story's article.
///
/// Joined through `articles` on `story_id` rather than a single article id:
/// corrections create new versions, so a per-article lookup would show a reader
/// only the corrections issued before the version they happen to be reading.
pub async fn corrections_for_story(db: &Db, story: StoryId) -> Result<Vec<Correction>> {
    let rows = sqlx::query(
        "SELECT c.id, c.article_id, c.from_version, c.to_version, c.reason, c.diff_md,
                c.issued_at, c.agent_id
         FROM corrections c
         JOIN articles a ON a.id = c.article_id
         WHERE a.story_id = $1
         ORDER BY c.issued_at DESC",
    )
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(Correction {
                id: correction_id(r, "id")?,
                article_id: article_id(r, "article_id")?,
                from_version: r.try_get("from_version")?,
                to_version: r.try_get("to_version")?,
                reason: r.try_get("reason")?,
                diff_md: r.try_get("diff_md")?,
                issued_at: r.try_get("issued_at")?,
                agent_id: agent_id_opt(r, "agent_id")?,
            })
        })
        .collect()
}
