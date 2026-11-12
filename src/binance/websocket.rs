use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub const BINANCE_USDT_FUTURES_WS_URL: &str = "wss://fstream.binance.com/market/ws";
pub const BINANCE_USDT_FUTURES_WS_TESTNET_URL: &str = "wss://stream.binancefuture.com/ws";
pub const BINANCE_COIN_FUTURES_WS_URL: &str = "wss://dstream.binance.com/ws";
pub const BINANCE_COIN_FUTURES_WS_TESTNET_URL: &str = "wss://dstream.binancefuture.com/ws";
pub const BINANCE_SPOT_WS_URL: &str = "wss://stream.binance.com:9443/ws";
pub const BINANCE_SPOT_WS_TESTNET_URL: &str = "wss://stream.testnet.binance.vision/ws";
pub const BINANCE_FUTURES_WS_URL: &str = BINANCE_USDT_FUTURES_WS_URL;
pub const BINANCE_FUTURES_WS_TESTNET_URL: &str = BINANCE_USDT_FUTURES_WS_TESTNET_URL;

const HEARTBEAT_CHECK_INTERVAL: u64 = 300;
const MAX_SILENCE_DURATION: u64 = 600;
const RECONNECT_DELAY: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContractType {
    #[default]
    UsdtMargined,
    CoinMargined,
    Spot,
}

impl ContractType {
    pub fn get_ws_url(&self, use_testnet: bool) -> &'static str {
        match (self, use_testnet) {
            (ContractType::UsdtMargined, false) => BINANCE_USDT_FUTURES_WS_URL,
            (ContractType::UsdtMargined, true) => BINANCE_USDT_FUTURES_WS_TESTNET_URL,
            (ContractType::CoinMargined, false) => BINANCE_COIN_FUTURES_WS_URL,
            (ContractType::CoinMargined, true) => BINANCE_COIN_FUTURES_WS_TESTNET_URL,
            (ContractType::Spot, false) => BINANCE_SPOT_WS_URL,
            (ContractType::Spot, true) => BINANCE_SPOT_WS_TESTNET_URL,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "usdt" | "usdt_margined" | "u" => Some(ContractType::UsdtMargined),
            "coin" | "coin_margined" | "c" => Some(ContractType::CoinMargined),
            "spot" | "s" => Some(ContractType::Spot),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ContractType::UsdtMargined => "USDT",
            ContractType::CoinMargined => "COIN",
            ContractType::Spot => "SPOT",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum KlineInterval {
    Minutes1,
    Minutes3,
    Minutes5,
    Minutes15,
    Minutes30,
    Hours1,
    Hours2,
    Hours4,
    Hours6,
    Hours8,
    Hours12,
    Days1,
    Days3,
    Weeks1,
    Months1,
}

impl KlineInterval {
    pub fn as_str(&self) -> &'static str {
        match self {
            KlineInterval::Minutes1 => "1m",
            KlineInterval::Minutes3 => "3m",
            KlineInterval::Minutes5 => "5m",
            KlineInterval::Minutes15 => "15m",
            KlineInterval::Minutes30 => "30m",
            KlineInterval::Hours1 => "1h",
            KlineInterval::Hours2 => "2h",
            KlineInterval::Hours4 => "4h",
            KlineInterval::Hours6 => "6h",
            KlineInterval::Hours8 => "8h",
            KlineInterval::Hours12 => "12h",
            KlineInterval::Days1 => "1d",
            KlineInterval::Days3 => "3d",
            KlineInterval::Weeks1 => "1w",
            KlineInterval::Months1 => "1M",
        }
    }
}

pub fn build_kline_stream_name(symbol: &str, interval: KlineInterval) -> String {
    format!("{}@kline_{}", symbol.to_lowercase(), interval.as_str())
}

pub fn build_ws_url(symbol: &str, interval: KlineInterval, use_testnet: bool) -> String {
    build_ws_url_with_type(symbol, interval, ContractType::UsdtMargined, use_testnet)
}

pub fn build_ws_url_with_type(
    symbol: &str,
    interval: KlineInterval,
    contract_type: ContractType,
    use_testnet: bool,
) -> String {
    let base_url = contract_type.get_ws_url(use_testnet);
    let stream_name = build_kline_stream_name(symbol, interval);
    format!("{}/{}", base_url, stream_name)
}

pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub async fn connect_kline(
    symbol: &str,
    interval: KlineInterval,
    use_testnet: bool,
) -> anyhow::Result<WsStream> {
    connect_kline_with_type(symbol, interval, ContractType::UsdtMargined, use_testnet).await
}

pub async fn connect_kline_with_type(
    symbol: &str,
    interval: KlineInterval,
    contract_type: ContractType,
    use_testnet: bool,
) -> anyhow::Result<WsStream> {
    let url = build_ws_url_with_type(symbol, interval, contract_type, use_testnet);
    log::info!(
        "[{}] connect Binance websocket: {}",
        contract_type.as_str(),
        url
    );
    let (ws_stream, response) = connect_async(&url).await?;
    log::info!(
        "[{}] Binance websocket connected, status: {}",
        contract_type.as_str(),
        response.status()
    );
    Ok(ws_stream)
}

pub async fn run_kline_client<F>(
    symbol: &str,
    interval: KlineInterval,
    use_testnet: bool,
    on_message: F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> bool,
{
    run_kline_client_with_type(
        symbol,
        interval,
        ContractType::UsdtMargined,
        use_testnet,
        on_message,
    )
    .await
}

pub async fn run_kline_client_with_type<F>(
    symbol: &str,
    interval: KlineInterval,
    contract_type: ContractType,
    use_testnet: bool,
    mut on_message: F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> bool,
{
    loop {
        match run_kline_session_with_type(
            symbol,
            interval,
            contract_type,
            use_testnet,
            &mut on_message,
        )
        .await
        {
            Ok(should_continue) => {
                if !should_continue {
                    log::info!("[{}] websocket session ended", contract_type.as_str());
                    break;
                }
                log::warn!(
                    "[{}] websocket session disconnected, reconnecting in {}s",
                    contract_type.as_str(),
                    RECONNECT_DELAY
                );
            }
            Err(e) => {
                log::error!(
                    "[{}] websocket session error: {:?}, reconnecting in {}s",
                    contract_type.as_str(),
                    e,
                    RECONNECT_DELAY
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY)).await;
    }
    Ok(())
}

async fn run_kline_session_with_type<F>(
    symbol: &str,
    interval: KlineInterval,
    contract_type: ContractType,
    use_testnet: bool,
    on_message: &mut F,
) -> anyhow::Result<bool>
where
    F: FnMut(&str) -> bool,
{
    let mut ws_stream =
        connect_kline_with_type(symbol, interval, contract_type, use_testnet).await?;
    let mut last_message_time = Instant::now();
    let mut heartbeat_timer = tokio::time::interval(Duration::from_secs(HEARTBEAT_CHECK_INTERVAL));

    loop {
        tokio::select! {
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_message_time = Instant::now();
                        if !on_message(&text) {
                            return Ok(false);
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        last_message_time = Instant::now();
                        if let Ok(text) = String::from_utf8(data) {
                            if !on_message(&text) {
                                return Ok(false);
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws_stream.send(Message::Pong(payload)).await?;
                        last_message_time = Instant::now();
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_message_time = Instant::now();
                    }
                    Some(Ok(Message::Close(frame))) => {
                        log::warn!("[{}] websocket closed: {:?}", contract_type.as_str(), frame);
                        return Ok(true);
                    }
                    Some(Ok(Message::Frame(_))) => {
                        last_message_time = Instant::now();
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(true),
                }
            }
            _ = heartbeat_timer.tick() => {
                if last_message_time.elapsed() > Duration::from_secs(MAX_SILENCE_DURATION) {
                    log::warn!(
                        "[{}] no messages for {}s, reconnecting",
                        contract_type.as_str(),
                        MAX_SILENCE_DURATION
                    );
                    return Ok(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_kline_stream_name() {
        assert_eq!(
            build_kline_stream_name("BTCUSDT", KlineInterval::Minutes1),
            "btcusdt@kline_1m"
        );
        assert_eq!(
            build_kline_stream_name("btcusd_perp", KlineInterval::Hours4),
            "btcusd_perp@kline_4h"
        );
    }

    #[test]
    fn test_build_ws_url_usdt() {
        let url = build_ws_url("btcusdt", KlineInterval::Minutes1, false);
        assert_eq!(url, "wss://fstream.binance.com/market/ws/btcusdt@kline_1m");
    }

    #[test]
    fn test_build_ws_url_usdt_testnet() {
        let url = build_ws_url("btcusdt", KlineInterval::Minutes1, true);
        assert_eq!(url, "wss://stream.binancefuture.com/ws/btcusdt@kline_1m");
    }

    #[test]
    fn test_build_ws_url_coin() {
        let url = build_ws_url_with_type(
            "btcusd_perp",
            KlineInterval::Minutes1,
            ContractType::CoinMargined,
            false,
        );
        assert_eq!(url, "wss://dstream.binance.com/ws/btcusd_perp@kline_1m");
    }

    #[test]
    fn test_build_ws_url_coin_testnet() {
        let url = build_ws_url_with_type(
            "btcusd_perp",
            KlineInterval::Minutes1,
            ContractType::CoinMargined,
            true,
        );
        assert_eq!(
            url,
            "wss://dstream.binancefuture.com/ws/btcusd_perp@kline_1m"
        );
    }

    #[test]
    fn test_build_ws_url_spot() {
        let url = build_ws_url_with_type(
            "btcusdt",
            KlineInterval::Minutes1,
            ContractType::Spot,
            false,
        );
        assert_eq!(url, "wss://stream.binance.com:9443/ws/btcusdt@kline_1m");
    }

    #[test]
    fn test_build_ws_url_spot_testnet() {
        let url =
            build_ws_url_with_type("btcusdt", KlineInterval::Minutes1, ContractType::Spot, true);
        assert_eq!(
            url,
            "wss://stream.testnet.binance.vision/ws/btcusdt@kline_1m"
        );
    }
}
