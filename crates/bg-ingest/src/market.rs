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

/// Equities and indices carried alongside the crypto strip.
///
/// A crypto-only ticker says the newsroom covers crypto. BitGoose runs a
/// Markets desk, a Business desk and an AI desk, and on any given day the
/// story is as likely to be Nasdaq or Nvidia as it is Bitcoin — the two move
/// together often enough that showing one without the other hides the
/// interesting part.
///
/// The tickers are chosen to match what the desks actually write about: the
/// three headline indices, then the companies that recur in AI and crypto
/// coverage.
/// `(vendor symbol, our symbol, display name)`.
///
/// The vendor's symbol stays at the edge. Yahoo writes indices as `^GSPC`, and
/// storing that put a caret in our database, in `/asset/^GSPC` URLs and in the
/// ticker strip, where it read as a typo. A vendor's notation is an artefact of
/// the vendor, and the boundary is the place to leave it.
pub const EQUITIES: &[(&str, &str, &str)] = &[
    ("^GSPC", "SPX", "S&P 500"),
    ("^IXIC", "NDAQ", "Nasdaq"),
    ("^DJI", "DJIA", "Dow Jones"),
    ("NVDA", "NVDA", "Nvidia"),
    ("COIN", "COIN", "Coinbase"),
    ("MSTR", "MSTR", "Strategy"),
    ("TSLA", "TSLA", "Tesla"),
];

/// Symbols that are stock indices rather than tradeable assets.
///
/// They lead the ticker — an index is the context every other number on the
/// strip is read against — and they carry no market cap, so without this they
/// sort to the very end behind twelve coins and fall off a strip that shows
/// fourteen.
pub const INDICES: &[&str] = &["SPX", "NDAQ", "DJIA"];

#[derive(Debug, Deserialize)]
struct YahooChart {
    chart: YahooChartBody,
}

#[derive(Debug, Deserialize)]
struct YahooChartBody {
    result: Option<Vec<YahooResult>>,
}

#[derive(Debug, Deserialize)]
struct YahooResult {
    meta: YahooMeta,
}

#[derive(Debug, Deserialize)]
struct YahooMeta {
    #[serde(rename = "regularMarketPrice")]
    price: Option<f64>,
    /// The previous session's close, which is what a day's move is measured
    /// against. Yahoo names it differently on the chart endpoint than on the
    /// quote endpoint, and the quote endpoint now answers 401 without a
    /// crumb — so this is the one that works.
    #[serde(rename = "chartPreviousClose")]
    previous_close: Option<f64>,
}

/// One index or ticker from Yahoo's chart endpoint.
///
/// One request per symbol: the batch quote endpoint returns 401 without a
/// session crumb, and seven small requests on a 20 MB/s link cost less than
/// reverse-engineering an auth handshake that Yahoo can change at will.
pub async fn fetch_equity(
    client: &reqwest::Client,
    vendor_symbol: &str,
    symbol: &str,
    name: &str,
) -> Result<PriceTick> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1d&range=2d",
        urlencoding_min(vendor_symbol)
    );
    let body: YahooChart = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let meta = body
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .map(|r| r.meta)
        .ok_or_else(|| IngestError::Parse {
            source_slug: symbol.to_string(),
            detail: format!("no chart result for {name}"),
        })?;
    let price = meta
        .price
        .ok_or_else(|| IngestError::Parse {
            source_slug: symbol.to_string(),
            detail: format!("no price for {name}"),
        })?;
    // A change of "0%" and "unknown" are different claims and the strip shows
    // them differently, so an absent previous close stays absent.
    let change = meta
        .previous_close
        .filter(|p| *p > 0.0)
        .map(|prev| (price - prev) / prev * 100.0);
    Ok(PriceTick {
        symbol: symbol.to_string(),
        ts: Utc::now(),
        price_usd: dec(price)
            .ok_or_else(|| IngestError::Parse {
            source_slug: symbol.to_string(),
            detail: format!("unrepresentable price for {name}"),
        })?,
        change_24h_pct: change,
        volume_24h: None,
        market_cap: None,
    })
}

/// Percent-encode the few characters an index symbol actually contains.
///
/// `^GSPC` needs its caret escaped and nothing else does; pulling in a whole
/// encoding crate for one character would be the larger change.
fn urlencoding_min(s: &str) -> String {
    s.replace('^', "%5E")
}

/// Refresh the equities strip. Never fails the pass — a missing index is a
/// gap in a ticker, not a broken newsroom.
pub async fn refresh_equities(db: &Db, client: &reqwest::Client) -> usize {
    let mut n = 0;
    for (vendor, symbol, name) in EQUITIES {
        match fetch_equity(client, vendor, symbol, name).await {
            Ok(tick) => {
                let _ = prices::upsert_asset(db, symbol, name, None, None).await;
                if prices::insert_tick(db, &tick).await.is_ok() {
                    n += 1;
                }
            }
            Err(e) => warn!(symbol = %symbol, error = %e, "equity fetch failed"),
        }
    }
    if n > 0 {
        info!(count = n, source = "yahoo", "equities refreshed");
    }
    n
}

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
pub async fn fetch_coingecko(
    client: &reqwest::Client,
) -> Result<Vec<(PriceTick, String, String, Option<i32>)>> {
    let ids = TRACKED
        .iter()
        .map(|(_, _, id)| *id)
        .collect::<Vec<_>>()
        .join(",");
    let url = format!(
        "https://api.coingecko.com/api/v3/coins/markets\
         ?vs_currency=usd&ids={ids}&order=market_cap_desc&per_page=250&page=1&sparkline=false"
    );
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(IngestError::Http {
            status: resp.status().as_u16(),
            url,
        });
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
        return Err(IngestError::Http {
            status: resp.status().as_u16(),
            url,
        });
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
            info!(
                count = n,
                source = "coinbase",
                "prices refreshed via fallback"
            );
            n
        }
    }
}
