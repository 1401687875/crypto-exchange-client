//! OKX 永续合约（U 本位 SWAP）。
//!
//! ```no_run
//! use crypto_exchange_client::okx::swap::*;
//!
//! # async fn demo() -> crypto_exchange_client::Result<()> {
//! // 模拟盘：SwapConfig::demo()，要用模拟盘创建的 API Key。
//! let client = SwapClient::new(SwapConfig::demo(), "api-key", "secret", "passphrase");
//!
//! // 1. 交易规则和盘口（数量单位是"张"，一张 = ctVal 个币）
//! let instruments = client.instruments().await?;
//! let btc = instruments.iter().find(|i| i.inst_id == "BTC-USDT-SWAP").unwrap();
//! let book = client.ticker("BTC-USDT-SWAP").await?;
//!
//! // 2. 双向持仓下挂只做 Maker 的开多单：0.002 BTC
//! let sz = coins_to_contracts("0.002".parse().unwrap(), btc);
//! let order = NewOrder::post_only("BTC-USDT-SWAP", "buy", sz, book.bid_px)
//!     .pos_side("long")
//!     .client_order_id("demo1");
//! client.place_order(&order).await?;
//!
//! // 3. 查是否成交；没成交就撤单
//! let now = client.order("BTC-USDT-SWAP", &OrderRef::Client("demo1".into())).await?;
//! if !now.is_final() {
//!     client.cancel_order("BTC-USDT-SWAP", &OrderRef::Client("demo1".into())).await?;
//! }
//! # Ok(())
//! # }
//! ```

mod client;
pub mod models;
mod rest;

pub use client::{sign, Query, SwapClient, SwapConfig, OKX_REST_URL};
pub use models::*;
pub use rest::{coins_to_contracts, contracts_to_coins};
pub use rust_decimal::Decimal;
