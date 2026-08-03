//! The Skein's analyses.
//!
//! One row per story, replaced rather than versioned: an analysis is a current
//! read, and two contradictory takes on one page would be worse than either.
//! Corrections to *reporting* are append-only ([`crate::corrections`]) because
//! the record of what we asserted matters; a superseded forecast has no such
//! claim on the reader.

use crate::{convert::*, Db, Result};
use bg_core::domain::Analysis;
use bg_core::ids::{AnalysisId, RunId, StoryId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

const COLS: &str = "id, story_id, significance, direction, horizon, confidence, watch, model, \
                    run_id, grounded_chars, created_at";

fn from_row(r: &PgRow) -> Result<Analysis> {
    Ok(Analysis {
        id: AnalysisId::from_uuid(r.try_get("id")?),
        story_id: story_id(r, "story_id")?,
        significance: r.try_get("significance")?,
        direction: r.try_get("direction")?,
        horizon: r.try_get("horizon")?,
        confidence: r.try_get("confidence")?,
        watch: r.try_get("watch")?,
        model: r.try_get("model")?,
        run_id: run_id_opt(r, "run_id")?,
        grounded_chars: r.try_get("grounded_chars")?,
        created_at: r.try_get("created_at")?,
    })
}

pub struct NewAnalysis {
    pub significance: String,
    pub direction: String,
    pub horizon: String,
    pub confidence: i16,
    pub watch: Vec<String>,
    pub model: Option<String>,
    pub grounded_chars: i32,
}

/// Insert or replace the analysis for a story.
pub async fn upsert(
    db: &Db,
    story: StoryId,
    a: &NewAnalysis,
    run: Option<RunId>,
) -> Result<AnalysisId> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO analyses
           (id, story_id, significance, direction, horizon, confidence, watch, model,
            run_id, grounded_chars)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (story_id) DO UPDATE SET
            significance   = EXCLUDED.significance,
            direction      = EXCLUDED.direction,
            horizon        = EXCLUDED.horizon,
            confidence     = EXCLUDED.confidence,
            watch          = EXCLUDED.watch,
            model          = EXCLUDED.model,
            run_id         = EXCLUDED.run_id,
            grounded_chars = EXCLUDED.grounded_chars,
            created_at     = now()
         RETURNING id",
    )
    .bind(id)
    .bind(story.into_uuid())
    .bind(&a.significance)
    .bind(&a.direction)
    .bind(&a.horizon)
    .bind(a.confidence)
    .bind(&a.watch)
    .bind(&a.model)
    .bind(run.map(|r| r.into_uuid()))
    .bind(a.grounded_chars)
    .fetch_one(&db.pool)
    .await?;
    Ok(AnalysisId::from_uuid(row.get::<Uuid, _>("id")))
}

pub async fn for_story(db: &Db, story: StoryId) -> Result<Option<Analysis>> {
    let row = crate::sql(format!("SELECT {COLS} FROM analyses WHERE story_id = $1"))
        .bind(story.into_uuid())
        .fetch_optional(&db.pool)
        .await?;
    row.as_ref().map(from_row).transpose()
}

/// Published stories with no analysis yet, richest source material first.
///
/// Ordering by available text rather than recency is deliberate: the Skein's
/// output is only as good as what it read, so when there is a budget for N
/// analyses they should go to the N stories we can actually support. Newest-
/// first would spend the budget on whatever happened to land last, which on a
/// feed-driven site is usually the thinnest item of the hour.
pub async fn needing_analysis(db: &Db, min_chars: i64, limit: i64) -> Result<Vec<StoryId>> {
    let rows = sqlx::query(
        "SELECT s.id, sum(length(coalesce(r.body_raw, r.summary_raw, ''))) AS chars
           FROM stories s
           JOIN raw_items r ON r.story_id = s.id
           LEFT JOIN analyses a ON a.story_id = s.id
          WHERE s.status = 'published' AND a.id IS NULL
          GROUP BY s.id
         HAVING sum(length(coalesce(r.body_raw, r.summary_raw, ''))) >= $1
          ORDER BY chars DESC
          LIMIT $2",
    )
    .bind(min_chars)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| Ok(StoryId::from_uuid(r.try_get::<Uuid, _>("id")?)))
        .collect()
}

pub async fn count(db: &Db) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM analyses")
        .fetch_one(&db.pool)
        .await?)
}

/// Drop every analysis. Used when the model or the prompt changes materially —
/// a mixed archive of old and new takes is not something a reader can reason
/// about, and the analyses are cheap to regenerate.
pub async fn clear(db: &Db) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM analyses")
        .execute(&db.pool)
        .await?
        .rows_affected())
}
