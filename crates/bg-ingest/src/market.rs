//! Market data.
//!
//! CoinGecko is the primary: one request covers the whole ticker strip with
//! caps and 24h changes. Coinbase is the fallback for the majors — it needs one
//! request per pair and carries no volume or cap, but it is a different company
//! on different infrastructure, which is the point of a fallback. Binance is
//! excluded: it returns HTTP 451 to this region.

use crate::{IngestError, Result};
use bg_core::domain::PriceTick;
use bg_db::{prices, Db};
use chrono::Utc;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

/// The assets the ticker strip and asset hubs cover.
pub const TRACKED: &[(&str, &str, &str)] = &[
    // (symbol, display name, coingecko id)
    ("BTC", "Bitcoin", "bitcoin"),
    ("ETH", "Ethereum", "ethereum"),
    ("SOL", "Solana", "solana"),
    ("XRP", "XRP", "ripple"),
    ("BNB", "BNB", "binancecoin"),
    ("DOGE", "Dogecoin", "dogecoin"),
    ("ADA", "Cardano", "cardano"),
    ("AVAX", "Avalanche", "avalanche-2"),
    ("LINK", "Chainlink", "chainlink"),
    ("TON", "Toncoin", "the-open-network"),
    ("SUI", "Sui", "sui"),
    ("MATIC", "Polygon", "matic-network"),
];

#[derive(Debug, Deserialize)]
struct GeckoMarket {
    symbol: String,
    name: String,
    id: String,
    current_price: Option<f64>,
    market_cap: Option<f64>,
    total_volume: Option<f64>,
    price_change_percentage_24h: Option<f64>,
    market_cap_rank: Option<i32>,
}

fn dec(v: f64) -> Option<Decimal> {
    Decimal::from_str(&format!("{v:.8}")).ok()
}

/// Pull every tracked asset from CoinGecko in one request.
pub async fn fetch_coingecko(client: &reqwest::Client) -> Result<Vec<(PriceTick, String, String, Option<i32>)>> {
    let ids = TRACKED.iter().map(|(_, _, id)| *id).collect::<Vec<_>>().join(",");
    let url = format!(
        "https://api.coingecko.com/api/v3/coins/markets\
         ?vs_currency=usd&ids={ids}&order=market_cap_desc&per_page=250&page=1&sparkline=false"
    );
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(IngestError::Http { status: resp.status().as_u16(), url });
    }
    let markets: Vec<GeckoMarket> = resp.json().await?;
    let ts = Utc::now();

    Ok(markets
        .into_iter()
        .filter_map(|m| {
            let price = dec(m.current_price?)?;
            Some((
                PriceTick {
                    symbol: m.symbol.to_uppercase(),
                    ts,
                    price_usd: price,
                    change_24h_pct: m.price_change_percentage_24h,
                    volume_24h: m.total_volume.and_then(dec),
                    market_cap: m.market_cap.and_then(dec),
                },
                m.name,
                m.id,
                m.market_cap_rank,
            ))
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct CoinbaseSpot {
    data: CoinbaseSpotData,
}

#[derive(Debug, Deserialize)]
struct CoinbaseSpotData {
    amount: String,
    base: String,
}

/// Fallback: spot price per pair. No volume or market cap available.
pub async fn fetch_coinbase(client: &reqwest::Client, symbol: &str) -> Result<PriceTick> {
    let url = format!("https://api.coinbase.com/v2/prices/{symbol}-USD/spot");
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(IngestError::Http { status: resp.status().as_u16(), url });
    }
    let body: CoinbaseSpot = resp.json().await?;
    Ok(PriceTick {
        symbol: body.data.base.to_uppercase(),
        ts: Utc::now(),
        price_usd: Decimal::from_str(&body.data.amount)
            .map_err(|e| IngestError::Decode(e.to_string()))?,
        change_24h_pct: None,
        volume_24h: None,
        market_cap: None,
    })
}

/// Refresh prices, falling back to Coinbase for the majors if CoinGecko fails.
///
/// Returns how many symbols were written. A market strip stuck at yesterday's
/// numbers is worse than one showing fewer assets, so partial success counts.
pub async fn refresh(db: &Db, client: &reqwest::Client) -> usize {
    match fetch_coingecko(client).await {
        Ok(rows) if !rows.is_empty() => {
            let mut n = 0;
            for (tick, name, gecko_id, rank) in rows {
                if let Err(e) =
                    prices::upsert_asset(db, &tick.symbol, &name, Some(&gecko_id), rank).await
                {
                    warn!(symbol = %tick.symbol, error = %e, "asset upsert failed");
                    continue;
                }
                match prices::insert_tick(db, &tick).await {
                    Ok(()) => n += 1,
                    Err(e) => warn!(symbol = %tick.symbol, error = %e, "tick insert failed"),
                }
            }
            info!(count = n, source = "coingecko", "prices refreshed");
            n
        }
        other => {
            if let Err(e) = other {
                warn!(error = %e, "coingecko failed, falling back to coinbase");
            }
            let mut n = 0;
            for (sym, name, gecko_id) in TRACKED.iter().take(6) {
                match fetch_coinbase(client, sym).await {
                    Ok(tick) => {
                        let _ = prices::upsert_asset(db, sym, name, Some(gecko_id), None).await;
                        if prices::insert_tick(db, &tick).await.is_ok() {
                            n += 1;
                        }
                    }
                    Err(e) => warn!(symbol = %sym, error = %e, "coinbase fallback failed"),
                }
            }
            info!(count = n, source = "coinbase", "prices refreshed via fallback");
            n
        }
    }
}
