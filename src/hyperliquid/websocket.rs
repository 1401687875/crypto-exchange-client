use crate::hyperliquid::precision::{
    is_spot_contract, normalize_perp_symbol, normalize_spot_symbol,
};
use futures_util::SinkExt;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub const HYPERLIQUID_MAINNET_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
pub const HYPERLIQUID_TESTNET_WS_URL: &str = "wss://api.hyperliquid-testnet.xyz/ws";

pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone, Copy)]
pub enum HyperliquidInterval {
    Minutes1,
    Minutes3,
    Minutes5,
    Minutes15,
    Minutes30,
    Hours1,
    Hours2,
    Hours4,
    Hours8,
    Hours12,
    Days1,
}

impl HyperliquidInterval {
    pub fn as_str(&self) -> &'static str {
        match self {
            HyperliquidInterval::Minutes1 => "1m",
            HyperliquidInterval::Minutes3 => "3m",
            HyperliquidInterval::Minutes5 => "5m",
            HyperliquidInterval::Minutes15 => "15m",
            HyperliquidInterval::Minutes30 => "30m",
            HyperliquidInterval::Hours1 => "1h",
            HyperliquidInterval::Hours2 => "2h",
            HyperliquidInterval::Hours4 => "4h",
            HyperliquidInterval::Hours8 => "8h",
            HyperliquidInterval::Hours12 => "12h",
            HyperliquidInterval::Days1 => "1d",
        }
    }
}

pub fn build_ws_url(use_testnet: bool) -> &'static str {
    if use_testnet {
        HYPERLIQUID_TESTNET_WS_URL
    } else {
        HYPERLIQUID_MAINNET_WS_URL
    }
}

pub fn build_candle_subscribe_message(
    symbol: &str,
    interval: &str,
    contract_type: Option<&str>,
) -> String {
    let coin = if is_spot_contract(contract_type) {
        normalize_spot_symbol(symbol)
    } else {
        normalize_perp_symbol(symbol)
    };

    json!({
        "method": "subscribe",
        "subscription": {
            "type": "candle",
            "coin": coin,
            "interval": interval
        }
    })
    .to_string()
}

pub fn build_candle_unsubscribe_message(
    symbol: &str,
    interval: &str,
    contract_type: Option<&str>,
) -> String {
    let coin = if is_spot_contract(contract_type) {
        normalize_spot_symbol(symbol)
    } else {
        normalize_perp_symbol(symbol)
    };

    json!({
        "method": "unsubscribe",
        "subscription": {
            "type": "candle",
            "coin": coin,
            "interval": interval
        }
    })
    .to_string()
}

pub async fn connect_candle_stream(
    symbol: &str,
    interval: &str,
    contract_type: Option<&str>,
    use_testnet: bool,
) -> anyhow::Result<WsStream> {
    let url = build_ws_url(use_testnet);
    let (mut ws_stream, response) = connect_async(url).await?;
    log::info!(
        "[HYPERLIQUID] websocket connected, status: {}",
        response.status()
    );

    let subscribe_message = build_candle_subscribe_message(symbol, interval, contract_type);
    ws_stream.send(Message::Text(subscribe_message)).await?;

    Ok(ws_stream)
}
