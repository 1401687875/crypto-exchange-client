//! 币安 U 本位合约（USDⓈ-M Futures）。
//!
//! ```no_run
//! use crypto_exchange_client::binance::usdm::*;
//!
//! # async fn demo() -> crypto_exchange_client::Result<()> {
//! let client = UsdmClient::new(UsdmConfig::testnet(), "api-key", "api-secret");
//! client.sync_time().await?;
//!
//! // 1. 交易规则和盘口
//! let info = client.exchange_info().await?;
//! let btc = info.symbol("BTCUSDT").unwrap();
//! let book = client.book_ticker("BTCUSDT").await?;
//!
//! // 2. 挂只做 Maker 的买单
//! let price = btc.post_only_price(&Side::Buy, book.bid_price, book.ask_price, 0);
//! let qty = btc.floor_qty("0.002".parse().unwrap());
//! let order = NewOrder::post_only("BTCUSDT", Side::Buy, qty, price).client_order_id("demo-1");
//! let placed = client.place_order(&order).await?;
//!
//! // 3. 查是否成交；没成交就撤单
//! let now = client.query_order("BTCUSDT", "demo-1").await?;
//! if now.status.is_open() {
//!     client.cancel_order("BTCUSDT", placed.order_id).await?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! 成交推送用 [`stream::spawn_user_stream`]，行情推送用 [`stream::spawn_market_stream`]。

mod client;
pub mod events;
pub mod exchange_info;
pub mod models;
mod rest;
pub mod stream;

pub use client::{
    Params, UsdmClient, UsdmConfig, USDM_REST_TESTNET_URL, USDM_REST_URL, USDM_WS_MARKET_URL,
    USDM_WS_PRIVATE_URL, USDM_WS_PUBLIC_URL, USDM_WS_TESTNET_URL,
};
pub use events::*;
pub use exchange_info::{round_to_step, ExchangeInfo, Rounding, SymbolFilter, SymbolInfo};
pub use models::*;
pub use rest::{AlgoId, Range};
pub use rust_decimal::Decimal;
pub use stream::{
    run_market_stream, run_user_stream, spawn_market_stream, spawn_user_stream, streams,
    MarketStreamMessage, StreamOptions, UserStreamMessage, UserStreamOptions,
};
