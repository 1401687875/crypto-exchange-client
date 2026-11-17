//! 币安。新代码请用 [`usdm`]（U 本位合约的完整接口）；`order` / `account` / `websocket` / `user_data` 是旧版接口，保留兼容。
//! [`stocks`] 是币安股票交易（SAPI）接口。

pub mod account;
pub mod order;
pub mod stocks;
pub mod usdm;
pub mod user_data;
pub mod websocket;

pub use order::{
    BinanceOrderClient, BINANCE_COIN_FUTURES_API_TESTNET_URL, BINANCE_COIN_FUTURES_API_URL,
    BINANCE_SPOT_API_TESTNET_URL, BINANCE_SPOT_API_URL, BINANCE_USDT_FUTURES_API_TESTNET_URL,
    BINANCE_USDT_FUTURES_API_URL,
};
pub use user_data::{
    wait_for_usdt_trade_result, BinanceOrderTradeUpdate, BinanceOrderUpdate, BinanceUserDataClient,
};
pub use websocket::{
    build_kline_stream_name, build_ws_url, build_ws_url_with_type, connect_kline,
    connect_kline_with_type, run_kline_client, run_kline_client_with_type, ContractType,
    KlineInterval, WsStream, BINANCE_COIN_FUTURES_WS_TESTNET_URL, BINANCE_COIN_FUTURES_WS_URL,
    BINANCE_FUTURES_WS_TESTNET_URL, BINANCE_FUTURES_WS_URL, BINANCE_SPOT_WS_TESTNET_URL,
    BINANCE_SPOT_WS_URL, BINANCE_USDT_FUTURES_WS_TESTNET_URL, BINANCE_USDT_FUTURES_WS_URL,
};
