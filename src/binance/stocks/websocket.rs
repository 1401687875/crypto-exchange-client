use super::{invalid, BinanceStocksClient};
use crate::{ExchangeClientError, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub const BINANCE_STOCKS_WS_URL: &str = "wss://nbstream.binance.com/equity";
#[derive(Debug, Clone, Copy)]
pub enum KlineInterval {
    Minutes5,
    Hour1,
    Day1,
    Week1,
    Month1,
}
impl KlineInterval {
    fn as_str(self) -> &'static str {
        match self {
            Self::Minutes5 => "5m",
            Self::Hour1 => "1h",
            Self::Day1 => "1d",
            Self::Week1 => "1w",
            Self::Month1 => "1M",
        }
    }
}
#[derive(Clone)]
pub enum StockStream {
    Price,
    Quote(String),
    Kline(String, KlineInterval),
    Calendar,
    TradingStatus(String),
    Tradability(String),
    OrderReport(String),
}
impl StockStream {
    pub fn name(&self) -> Result<String> {
        let (symbol, suffix) = match self {
            Self::Price => return Ok("price".into()),
            Self::Calendar => return Ok("calendar".into()),
            Self::Quote(s) => (s, "quote".into()),
            Self::Kline(s, i) => (s, format!("kline_{}", i.as_str())),
            Self::TradingStatus(s) => (s, "tradingStatus".into()),
            Self::Tradability(s) => (s, "tradability".into()),
            Self::OrderReport(key) => {
                if key.is_empty()
                    || !key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return invalid("invalid listenKey");
                }
                return Ok(format!("{key}@orderReport"));
            }
        };
        if symbol.is_empty()
            || !symbol
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return invalid("invalid stock ticker");
        }
        Ok(format!("{}@{suffix}", symbol.to_ascii_uppercase()))
    }
}
pub fn build_ws_url(streams: &[StockStream]) -> Result<String> {
    let names = streams
        .iter()
        .map(StockStream::name)
        .collect::<Result<Vec<_>>>()?;
    match names.len() {
        0 => invalid("at least one stock stream is required"),
        1 => Ok(format!("{BINANCE_STOCKS_WS_URL}/ws/{}", names[0])),
        _ => Ok(format!(
            "{BINANCE_STOCKS_WS_URL}/stream?streams={}",
            names.join("/")
        )),
    }
}

/// Delivers the original event object (combined-stream envelopes are unwrapped).
/// Unknown event fields are preserved. Returns on close/error so callers can reconnect.
pub async fn run_streams_until<F>(
    streams: &[StockStream],
    shutdown: watch::Receiver<bool>,
    on_event: F,
) -> Result<()>
where
    F: FnMut(Value) -> bool,
{
    run_session(streams, None, shutdown, on_event).await
}

impl BinanceStocksClient {
    /// Creates a user stream and renews its 60-minute TTL every 30 minutes.
    pub async fn run_order_reports_until<F>(
        &self,
        shutdown: watch::Receiver<bool>,
        on_event: F,
    ) -> Result<()>
    where
        F: FnMut(Value) -> bool,
    {
        if *shutdown.borrow() {
            return Ok(());
        }
        let key = self.create_or_renew_listen_key().await?.listen_key;
        run_session(
            &[StockStream::OrderReport(key)],
            Some(self),
            shutdown,
            on_event,
        )
        .await
    }
}

async fn run_session<F>(
    streams: &[StockStream],
    client: Option<&BinanceStocksClient>,
    mut shutdown: watch::Receiver<bool>,
    mut on_event: F,
) -> Result<()>
where
    F: FnMut(Value) -> bool,
{
    if *shutdown.borrow() {
        return Ok(());
    }
    let (mut socket, _) = connect_async(build_ws_url(streams)?)
        .await
        .map_err(ws_error)?;
    let mut renewal = tokio::time::interval(Duration::from_secs(30 * 60));
    renewal.tick().await;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { let _ = socket.close(None).await; return Ok(()); }
            }
            _ = renewal.tick(), if client.is_some() => {
                let renewed = client.unwrap().create_or_renew_listen_key().await?;
                if !streams.iter().any(|s| matches!(s, StockStream::OrderReport(key) if key == &renewed.listen_key)) {
                    return Err(ExchangeClientError::Connection("stock listenKey changed; reconnect required".into()));
                }
            }
            message = socket.next() => {
                let raw = match message {
                    Some(Ok(Message::Text(s))) => s,
                    Some(Ok(Message::Binary(bytes))) => String::from_utf8(bytes).map_err(|e| ExchangeClientError::ResponseParse(e.to_string()))?,
                    Some(Ok(Message::Ping(p))) => { socket.send(Message::Pong(p)).await.map_err(ws_error)?; continue; }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => return Err(ExchangeClientError::Connection("stock websocket closed; reconnect required".into())),
                    Some(Err(e)) => return Err(ws_error(e)),
                };
                let mut event: Value = serde_json::from_str(&raw)?;
                if event.get("stream").is_some() && event.get("data").is_some() { event = event["data"].take(); }
                if !on_event(event) { let _ = socket.close(None).await; return Ok(()); }
            }
        }
    }
}
fn ws_error(_: tokio_tungstenite::tungstenite::Error) -> ExchangeClientError {
    // The connection URL can contain a private listenKey.
    ExchangeClientError::Connection("stock websocket transport error; reconnect required".into())
}
