//! Binance、OKX、Hyperliquid 的 REST / WebSocket 客户端。
//!
//! 本库不依赖任何数据库、配置中心或 Web 框架：API 凭据和运行参数都由调用方显式传入。
//!
//! | 模块 | 内容 |
//! | --- | --- |
//! | [`binance::usdm`] | 币安 U 本位合约：下单（市价/限价/只做 Maker/条件单）、改单、撤单、查单、账户、行情、行情推送、用户数据流 |
//! | [`binance`] 其余 | 旧版接口（U 本位市价单、币本位、现货市价单、K 线 WebSocket），保留兼容 |
//! | [`okx`] | OKX REST 与 WebSocket |
//! | [`hyperliquid`] | Hyperliquid 下单、精度、WebSocket |
//!
//! 每个交易所对应一个同名 feature（默认全部启用）；只用币安时可以
//! `default-features = false, features = ["binance"]`，不会引入 ethers 等依赖。

#[cfg(feature = "binance")]
pub mod binance;
pub mod error;
#[cfg(feature = "hyperliquid")]
pub mod hyperliquid;
#[cfg(feature = "okx")]
pub mod okx;
mod signing;
pub mod types;

pub use error::{ExchangeClientError, Result};

// README 里的示例代码也作为文档测试编译，保证和接口一致。
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
