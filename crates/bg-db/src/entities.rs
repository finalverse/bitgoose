//! The entity graph behind topic and asset hub pages.

use crate::{convert::*, Db, DbError, Result};
use bg_core::domain::{Entity, EntityKind};
use bg_core::ids::{EntityId, StoryId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

const COLS: &str = "id, kind, name, slug, ticker, aliases, summary, created_at";

fn from_row(r: &PgRow) -> Result<Entity> {
    Ok(Entity {
        id: entity_id(r, "id")?,
        kind: enum_col::<EntityKind>(r, "kind")?,
        name: r.try_get("name")?,
        slug: r.try_get("slug")?,
        ticker: r.try_get("ticker")?,
        aliases: r.try_get("aliases")?,
        summary: r.try_get("summary")?,
        created_at: r.try_get("created_at")?,
    })
}

/// Upsert by slug. Aliases are unioned rather than replaced, so an extraction
/// pass that happens to see fewer names for an entity does not shrink what we
/// already know about it.
pub async fn upsert(
    db: &Db,
    kind: EntityKind,
    name: &str,
    slug: &str,
    ticker: Option<&str>,
    aliases: &[String],
) -> Result<Entity> {
    let row = crate::sql(format!(
        "INSERT INTO entities (id, kind, name, slug, ticker, aliases)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (slug) DO UPDATE SET
            name = EXCLUDED.name,
            kind = EXCLUDED.kind,
            ticker = COALESCE(EXCLUDED.ticker, entities.ticker),
            aliases = ARRAY(SELECT DISTINCT unnest(entities.aliases || EXCLUDED.aliases))
         RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(kind.as_str())
    .bind(name)
    .bind(slug)
    .bind(ticker.map(|t| t.to_uppercase()))
    .bind(aliases)
    .fetch_one(&db.pool)
    .await?;
    from_row(&row)
}

pub async fn by_slug(db: &Db, slug: &str) -> Result<Entity> {
    let row = crate::sql(format!("SELECT {COLS} FROM entities WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound("entity"))?;
    from_row(&row)
}

pub async fn all(db: &Db) -> Result<Vec<Entity>> {
    let rows = crate::sql(format!("SELECT {COLS} FROM entities ORDER BY name"))
        .fetch_all(&db.pool)
        .await?;
    rows.iter().map(from_row).collect()
}

pub async fn link_story(db: &Db, entity: EntityId, story: StoryId, salience: f32) -> Result<()> {
    sqlx::query(
        "INSERT INTO entity_mentions (entity_id, story_id, salience) VALUES ($1,$2,$3)
         ON CONFLICT (entity_id, story_id) DO UPDATE SET salience = EXCLUDED.salience",
    )
    .bind(entity.into_uuid())
    .bind(story.into_uuid())
    .bind(salience.clamp(0.0, 1.0))
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Entities most covered in a window — the `/flyway` "who is in the news" rail.
pub async fn trending(db: &Db, days: i32, limit: i64) -> Result<Vec<(Entity, i64)>> {
    let rows = crate::sql(format!(
        "SELECT {}, count(m.story_id) AS n
         FROM entities e
         JOIN entity_mentions m ON m.entity_id = e.id
         JOIN stories s ON s.id = m.story_id
         WHERE s.status = 'published' AND s.published_at > now() - make_interval(days => $1)
         GROUP BY e.id
         ORDER BY n DESC, e.name
         LIMIT $2",
        COLS.split(", ")
            .map(|c| format!("e.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(days)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| Ok((from_row(r)?, r.try_get("n")?)))
        .collect()
}
