//! Market data.

use crate::{convert::*, Db, Result};
use bg_core::domain::{Asset, PriceTick};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

/// Upsert an asset by symbol.
///
/// `symbol` and `coingecko_id` are both UNIQUE, and a single `ON CONFLICT` can
/// only target one of them — so an upsert that conflicts on the *other* key
/// fails outright. This is not hypothetical: the seed list records Toncoin as
/// `TON`, while CoinGecko reports the same `the-open-network` id under the
/// symbol `GRAM`, which crashed the price refresh. Detaching the id from any
/// other row first makes the symbol the single identity and lets the id follow
/// it, which is the behaviour the rest of the system assumes.
pub async fn upsert_asset(
    db: &Db,
    symbol: &str,
    name: &str,
    coingecko_id: Option<&str>,
    rank: Option<i32>,
) -> Result<Asset> {
    let symbol = symbol.to_uppercase();
    let mut tx = db.pool.begin().await?;

    if let Some(gid) = coingecko_id {
        sqlx::query("UPDATE assets SET coingecko_id = NULL WHERE coingecko_id = $1 AND symbol <> $2")
            .bind(gid)
            .bind(&symbol)
            .execute(&mut *tx)
            .await?;
    }

    let row = sqlx::query(
        "INSERT INTO assets (id, symbol, name, coingecko_id, rank)
         VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (symbol) DO UPDATE SET
            name = EXCLUDED.name,
            coingecko_id = COALESCE(EXCLUDED.coingecko_id, assets.coingecko_id),
            rank = COALESCE(EXCLUDED.rank, assets.rank)
         RETURNING id, symbol, name, coingecko_id, rank",
    )
    .bind(Uuid::new_v4())
    .bind(&symbol)
    .bind(name)
    .bind(coingecko_id)
    .bind(rank)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Asset {
        id: asset_id(&row, "id")?,
        symbol: row.try_get("symbol")?,
        name: row.try_get("name")?,
        coingecko_id: row.try_get("coingecko_id")?,
        rank: row.try_get("rank")?,
    })
}

pub async fn assets(db: &Db) -> Result<Vec<Asset>> {
    let rows = sqlx::query("SELECT id, symbol, name, coingecko_id, rank FROM assets ORDER BY rank NULLS LAST, symbol")
        .fetch_all(&db.pool)
        .await?;
    rows.iter()
        .map(|r| {
            Ok(Asset {
                id: asset_id(r, "id")?,
                symbol: r.try_get("symbol")?,
                name: r.try_get("name")?,
                coingecko_id: r.try_get("coingecko_id")?,
                rank: r.try_get("rank")?,
            })
        })
        .collect()
}

/// Record a tick. Idempotent per `(symbol, ts)` so a retried poll does not
/// create a duplicate point in the series.
pub async fn insert_tick(db: &Db, t: &PriceTick) -> Result<()> {
    sqlx::query(
        "INSERT INTO price_ticks (symbol, ts, price_usd, change_24h_pct, volume_24h, market_cap)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (symbol, ts) DO UPDATE SET
            price_usd = EXCLUDED.price_usd,
            change_24h_pct = EXCLUDED.change_24h_pct,
            volume_24h = EXCLUDED.volume_24h,
            market_cap = EXCLUDED.market_cap",
    )
    .bind(t.symbol.to_uppercase())
    .bind(t.ts)
    .bind(t.price_usd)
    .bind(t.change_24h_pct)
    .bind(t.volume_24h)
    .bind(t.market_cap)
    .execute(&db.pool)
    .await?;
    Ok(())
}

fn tick_from_row(r: &sqlx::postgres::PgRow) -> Result<PriceTick> {
    Ok(PriceTick {
        symbol: r.try_get("symbol")?,
        ts: r.try_get("ts")?,
        price_usd: r.try_get("price_usd")?,
        change_24h_pct: r.try_get("change_24h_pct")?,
        volume_24h: r.try_get("volume_24h")?,
        market_cap: r.try_get("market_cap")?,
    })
}

/// Most recent tick per symbol — the ticker strip.
pub async fn latest_all(db: &Db) -> Result<Vec<PriceTick>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (p.symbol)
                p.symbol, p.ts, p.price_usd, p.change_24h_pct, p.volume_24h, p.market_cap
         FROM price_ticks p
         ORDER BY p.symbol, p.ts DESC",
    )
    .fetch_all(&db.pool)
    .await?;
    let mut ticks: Vec<PriceTick> = rows.iter().map(tick_from_row).collect::<Result<_>>()?;
    // Order by market cap so the strip leads with what matters, not alphabetically.
    ticks.sort_by(|a, b| b.market_cap.unwrap_or_default().cmp(&a.market_cap.unwrap_or_default()));
    Ok(ticks)
}

pub async fn latest(db: &Db, symbol: &str) -> Result<Option<PriceTick>> {
    let row = sqlx::query(
        "SELECT symbol, ts, price_usd, change_24h_pct, volume_24h, market_cap
         FROM price_ticks WHERE symbol = $1 ORDER BY ts DESC LIMIT 1",
    )
    .bind(symbol.to_uppercase())
    .fetch_optional(&db.pool)
    .await?;
    row.as_ref().map(tick_from_row).transpose()
}

pub async fn history(db: &Db, symbol: &str, hours: i32) -> Result<Vec<(DateTime<Utc>, Decimal)>> {
    let rows = sqlx::query(
        "SELECT ts, price_usd FROM price_ticks
         WHERE symbol = $1 AND ts > now() - make_interval(hours => $2)
         ORDER BY ts ASC",
    )
    .bind(symbol.to_uppercase())
    .bind(hours)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get("ts"), r.get("price_usd"))).collect())
}
