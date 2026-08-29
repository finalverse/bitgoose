//! Human editorial directions are discovery assignments, not publication
//! commands. Scout widens intake; every result still passes autonomous review.

use crate::{Db, DbError, Result};
use bg_core::domain::{Beat, EditorialLanguage};
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, Row};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct EditorialDirection {
    pub id: Uuid,
    pub title: String,
    pub briefing: String,
    pub anchor_terms: Vec<String>,
    pub keywords: Vec<String>,
    pub editorial_language: EditorialLanguage,
    pub beat: Beat,
    pub priority: i16,
    pub status: String,
    pub created_by: String,
    pub last_searched_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct NewEditorialDirection<'a> {
    pub title: &'a str,
    pub briefing: &'a str,
    pub anchor_terms: &'a [String],
    pub keywords: &'a [String],
    pub editorial_language: EditorialLanguage,
    pub beat: Beat,
    pub priority: i16,
    pub created_by: &'a str,
}

fn from_row(row: &PgRow) -> Result<EditorialDirection> {
    let language: String = row.try_get("editorial_language")?;
    let beat: String = row.try_get("beat")?;
    Ok(EditorialDirection {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        briefing: row.try_get("briefing")?,
        anchor_terms: row.try_get("anchor_terms")?,
        keywords: row.try_get("keywords")?,
        editorial_language: EditorialLanguage::from_str(&language).map_err(|source| {
            DbError::Decode {
                column: "editorial_language",
                source,
            }
        })?,
        beat: Beat::from_str(&beat).map_err(|source| DbError::Decode {
            column: "beat",
            source,
        })?,
        priority: row.try_get("priority")?,
        status: row.try_get("status")?,
        created_by: row.try_get("created_by")?,
        last_searched_at: row.try_get("last_searched_at")?,
        created_at: row.try_get("created_at")?,
    })
}

const COLS: &str = "id, title, briefing, anchor_terms, keywords, editorial_language, beat, \
                    priority, status, created_by, last_searched_at, created_at";

pub async fn create(db: &Db, direction: &NewEditorialDirection<'_>) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO editorial_directions
           (id,title,briefing,anchor_terms,keywords,editorial_language,beat,priority,created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(id)
    .bind(direction.title)
    .bind(direction.briefing)
    .bind(direction.anchor_terms)
    .bind(direction.keywords)
    .bind(direction.editorial_language.as_str())
    .bind(direction.beat.as_str())
    .bind(direction.priority.clamp(1, 100))
    .bind(direction.created_by)
    .execute(&db.pool)
    .await?;
    audit(
        db,
        direction.created_by,
        "direction.created",
        Some(id),
        serde_json::json!({
            "title": direction.title,
            "language": direction.editorial_language.as_str(),
            "beat": direction.beat.as_str(),
            "priority": direction.priority.clamp(1, 100)
        }),
    )
    .await?;
    Ok(id)
}

pub async fn list(db: &Db, limit: i64) -> Result<Vec<EditorialDirection>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM editorial_directions
          ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'paused' THEN 1 ELSE 2 END,
                   priority DESC, created_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn searches_due(db: &Db, minutes: i64, limit: i64) -> Result<Vec<EditorialDirection>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM editorial_directions
          WHERE status = 'active'
            AND (last_searched_at IS NULL
                 OR last_searched_at < now() - make_interval(mins => $1::int))
          ORDER BY priority DESC, last_searched_at ASC NULLS FIRST LIMIT $2"
    ))
    .bind(minutes as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn mark_searched(db: &Db, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE editorial_directions SET last_searched_at=now(), updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn set_status(db: &Db, id: Uuid, status: &str, actor: &str) -> Result<bool> {
    if !matches!(status, "active" | "paused" | "completed") {
        return Err(DbError::InvalidInput(format!("unknown status {status}")));
    }
    let result =
        sqlx::query("UPDATE editorial_directions SET status=$2, updated_at=now() WHERE id=$1")
            .bind(id)
            .bind(status)
            .execute(&db.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Ok(false);
    }
    audit(
        db,
        actor,
        "direction.status",
        Some(id),
        serde_json::json!({"status": status}),
    )
    .await?;
    Ok(true)
}

async fn audit(
    db: &Db,
    actor: &str,
    action: &str,
    direction_id: Option<Uuid>,
    detail: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO editorial_audit_log (id,actor,action,direction_id,detail) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(action)
    .bind(direction_id)
    .bind(detail)
    .execute(&db.pool)
    .await?;
    Ok(())
}
