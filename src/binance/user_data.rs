use crate::error::{ExchangeClientError, Result};
use crate::types::TradeOrder;
use bigdecimal::BigDecimal;
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, Method};
use serde::Deserialize;
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

const BINANCE_USDT_PRIVATE_WS_URL: &str = "wss://fstream.binance.com/private/ws";
const BINANCE_USDT_PRIVATE_WS_TESTNET_URL: &str = "wss://stream.binancefuture.com/ws";
const BINANCE_USDT_API_URL: &str = "https://fapi.binance.com";
const BINANCE_USDT_API_TESTNET_URL: &str = "https://testnet.binancefuture.com";
const LISTEN_KEY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(50 * 60);
const SESSION_ROTATION_INTERVAL: Duration = Duration::from_secs(23 * 60 * 60 + 50 * 60);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const RECENT_EVENT_LIMIT: usize = 512;

#[derive(Debug, Clone, Deserialize)]
pub struct BinanceOrderTradeUpdate {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E", default)]
    pub event_time: u64,
    #[serde(rename = "T", default)]
    pub transaction_time: u64,
    #[serde(rename = "o")]
    pub order: BinanceOrderUpdate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BinanceOrderUpdate {
    #[serde(rename = "s", default)]
    pub symbol: String,
    #[serde(rename = "c", default)]
    pub client_order_id: String,
    #[serde(rename = "S", default)]
    pub side: String,
    #[serde(rename = "o", default)]
    pub order_type: String,
    #[serde(rename = "x", default)]
    pub execution_type: String,
    #[serde(rename = "X", default)]
    pub order_status: String,
    #[serde(rename = "i", default)]
    pub order_id: u64,
    #[serde(rename = "l", default)]
    pub last_filled_quantity: String,
    #[serde(rename = "z", default)]
    pub accumulated_filled_quantity: String,
    #[serde(rename = "L", default)]
    pub last_filled_price: String,
    #[serde(rename = "ap", default)]
    pub average_price: String,
    #[serde(rename = "n", default)]
    pub commission: String,
    #[serde(rename = "N", default)]
    pub commission_asset: String,
    #[serde(rename = "ps", default)]
    pub position_side: String,
    #[serde(rename = "T", default)]
    pub trade_time: u64,
    #[serde(rename = "t", default)]
    pub trade_id: i64,
}

#[derive(Debug, Deserialize)]
struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    listen_key: String,
}

#[derive(Debug, Clone)]
pub struct BinanceUserDataClient {
    api_key: String,
    use_testnet: bool,
    http: Client,
}

impl BinanceUserDataClient {
    pub fn new(api_key: impl Into<String>, use_testnet: bool) -> Self {
        let http = Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("valid Binance user data HTTP client configuration");
        Self {
            api_key: api_key.into(),
            use_testnet,
            http,
        }
    }

    pub async fn run_order_updates(&self) -> Result<()> {
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        self.run_order_updates_until(shutdown_receiver).await
    }

    pub async fn run_order_updates_until(
        &self,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> Result<()> {
        let listen_key = self.start_listen_key().await?;
        let session_result = self.run_session(&listen_key, shutdown_receiver).await;
        if let Err(error) = self.close_listen_key().await {
            log::warn!("[USDT UserData] close listenKey failed: {}", error);
        }
        session_result
    }

    async fn start_listen_key(&self) -> Result<String> {
        let raw = self.listen_key_request(Method::POST).await?;
        let response: ListenKeyResponse = serde_json::from_str(&raw)
            .map_err(|error| ExchangeClientError::ResponseParse(format!("{} - {}", error, raw)))?;
        if response.listen_key.is_empty() {
            return Err(ExchangeClientError::ResponseParse(
                "Binance start user data stream returned empty listenKey".to_string(),
            ));
        }
        Ok(response.listen_key)
    }

    async fn keepalive_listen_key(&self) -> Result<()> {
        self.listen_key_request(Method::PUT).await.map(|_| ())
    }

    async fn close_listen_key(&self) -> Result<()> {
        self.listen_key_request(Method::DELETE).await.map(|_| ())
    }

    async fn listen_key_request(&self, method: Method) -> Result<String> {
        let endpoint = format!("{}/fapi/v1/listenKey", self.rest_base_url());
        let response = self
            .http
            .request(method, endpoint)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|error| ExchangeClientError::Connection(error.to_string()))?;
        let status = response.status();
        let raw = response
            .text()
            .await
            .map_err(|error| ExchangeClientError::Connection(error.to_string()))?;
        if !status.is_success() {
            return Err(ExchangeClientError::Connection(format!(
                "Binance listenKey request failed: status={}, body={}",
                status, raw
            )));
        }
        Ok(raw)
    }

    async fn run_session(
        &self,
        listen_key: &str,
        mut shutdown_receiver: watch::Receiver<bool>,
    ) -> Result<()> {
        let url = format!("{}/{}", self.websocket_base_url(), listen_key);
        let (mut stream, response) = connect_async(&url)
            .await
            .map_err(|error| ExchangeClientError::Connection(error.to_string()))?;
        log::info!(
            "[USDT UserData] private websocket connected | status: {} | testnet: {}",
            response.status(),
            self.use_testnet
        );

        let mut keepalive = tokio::time::interval(LISTEN_KEY_KEEPALIVE_INTERVAL);
        keepalive.tick().await;
        let session_rotation = tokio::time::sleep(SESSION_ROTATION_INTERVAL);
        tokio::pin!(session_rotation);

        loop {
            tokio::select! {
                message = stream.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            if handle_user_data_message(&text)? {
                                return Err(ExchangeClientError::Connection(
                                    "Binance user data listenKey expired".to_string(),
                                ));
                            }
                        }
                        Some(Ok(Message::Binary(data))) => {
                            let text = String::from_utf8(data).map_err(|error| {
                                ExchangeClientError::ResponseParse(error.to_string())
                            })?;
                            if handle_user_data_message(&text)? {
                                return Err(ExchangeClientError::Connection(
                                    "Binance user data listenKey expired".to_string(),
                                ));
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            stream.send(Message::Pong(payload)).await.map_err(|error| {
                                ExchangeClientError::Connection(error.to_string())
                            })?;
                        }
                        Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                        Some(Ok(Message::Close(frame))) => {
                            return Err(ExchangeClientError::Connection(format!(
                                "Binance user data websocket closed: {:?}", frame
                            )));
                        }
                        Some(Err(error)) => {
                            return Err(ExchangeClientError::Connection(error.to_string()));
                        }
                        None => {
                            return Err(ExchangeClientError::Connection(
                                "Binance user data websocket ended".to_string(),
                            ));
                        }
                    }
                }
                _ = keepalive.tick() => {
                    self.keepalive_listen_key().await?;
                    log::debug!("[USDT UserData] listenKey keepalive succeeded");
                }
                _ = &mut session_rotation => {
                    log::info!("[USDT UserData] rotating private websocket before 24h limit");
                    return Ok(());
                }
                changed = shutdown_receiver.changed() => {
                    if changed.is_ok() && *shutdown_receiver.borrow() {
                        log::info!("[USDT UserData] stopping private websocket after configuration change");
                        return Ok(());
                    }
                }
            }
        }
    }

    fn rest_base_url(&self) -> &'static str {
        if self.use_testnet {
            BINANCE_USDT_API_TESTNET_URL
        } else {
            BINANCE_USDT_API_URL
        }
    }

    fn websocket_base_url(&self) -> &'static str {
        if self.use_testnet {
            BINANCE_USDT_PRIVATE_WS_TESTNET_URL
        } else {
            BINANCE_USDT_PRIVATE_WS_URL
        }
    }
}

fn handle_user_data_message(raw: &str) -> Result<bool> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| ExchangeClientError::ResponseParse(format!("{} - {}", error, raw)))?;
    match value.get("e").and_then(serde_json::Value::as_str) {
        Some("ORDER_TRADE_UPDATE") => {
            let update: BinanceOrderTradeUpdate = serde_json::from_value(value)
                .map_err(|error| ExchangeClientError::ResponseParse(error.to_string()))?;
            log::info!(
                "[USDT UserData] ORDER_TRADE_UPDATE | symbol: {} | clientOrderId: {} | orderId: {} | execution: {} | status: {} | lastQty: {} | lastPrice: {} | accumulatedQty: {} | avgPrice: {} | commission: {} {}",
                update.order.symbol,
                update.order.client_order_id,
                update.order.order_id,
                update.order.execution_type,
                update.order.order_status,
                update.order.last_filled_quantity,
                update.order.last_filled_price,
                update.order.accumulated_filled_quantity,
                update.order.average_price,
                update.order.commission,
                update.order.commission_asset
            );
            publish_order_update(update);
            Ok(false)
        }
        Some("listenKeyExpired") => Ok(true),
        _ => Ok(false),
    }
}

/// 订阅 `BinanceUserDataClient::run_order_updates` 收到的全部 `ORDER_TRADE_UPDATE`。
///
/// 只收到订阅之后的事件；处理太慢超过 1024 条积压时 `recv` 返回 `Lagged`，调用方应改用查单补齐。
/// 私有流没启动时不会有事件。
pub fn subscribe_order_updates() -> broadcast::Receiver<BinanceOrderTradeUpdate> {
    event_bus().subscribe()
}

fn event_bus() -> &'static broadcast::Sender<BinanceOrderTradeUpdate> {
    static BUS: OnceLock<broadcast::Sender<BinanceOrderTradeUpdate>> = OnceLock::new();
    BUS.get_or_init(|| broadcast::channel(1024).0)
}

fn recent_events() -> &'static Mutex<VecDeque<BinanceOrderTradeUpdate>> {
    static EVENTS: OnceLock<Mutex<VecDeque<BinanceOrderTradeUpdate>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(RECENT_EVENT_LIMIT)))
}

fn publish_order_update(update: BinanceOrderTradeUpdate) {
    if let Ok(mut events) = recent_events().lock() {
        if events.len() >= RECENT_EVENT_LIMIT {
            events.pop_front();
        }
        events.push_back(update.clone());
    }
    let _ = event_bus().send(update);
}

fn matches_order(
    update: &BinanceOrderTradeUpdate,
    symbol: &str,
    order_id: Option<u64>,
    client_order_id: Option<&str>,
) -> bool {
    update.order.symbol.eq_ignore_ascii_case(symbol)
        && (order_id
            .filter(|order_id| *order_id > 0)
            .map(|order_id| update.order.order_id == order_id)
            .unwrap_or(false)
            || client_order_id
                .filter(|client_order_id| !client_order_id.is_empty())
                .map(|client_order_id| update.order.client_order_id == client_order_id)
                .unwrap_or(false))
}

fn trade_result_from_update(update: &BinanceOrderTradeUpdate) -> Option<TradeOrder> {
    let terminal_fill = update.order.order_status == "FILLED"
        || (matches!(
            update.order.order_status.as_str(),
            "CANCELED" | "EXPIRED" | "EXPIRED_IN_MATCH"
        ) && BigDecimal::from_str(&update.order.accumulated_filled_quantity)
            .unwrap_or_default()
            > BigDecimal::from(0));
    if !terminal_fill {
        return None;
    }

    let amount = BigDecimal::from_str(&update.order.accumulated_filled_quantity).ok()?;
    let average_price = BigDecimal::from_str(&update.order.average_price).ok()?;
    if amount <= BigDecimal::from(0) || average_price <= BigDecimal::from(0) {
        return None;
    }

    Some(TradeOrder {
        fee: BigDecimal::from_str(&update.order.commission).unwrap_or_default(),
        sell_price: average_price,
        amount,
    })
}

fn find_recent_trade_result(
    symbol: &str,
    order_id: Option<u64>,
    client_order_id: Option<&str>,
) -> Option<TradeOrder> {
    let events = recent_events().lock().ok()?;
    let mut result = events.iter().rev().find_map(|update| {
        matches_order(update, symbol, order_id, client_order_id)
            .then(|| trade_result_from_update(update))
            .flatten()
    })?;
    result.fee = events
        .iter()
        .filter(|update| {
            matches_order(update, symbol, order_id, client_order_id)
                && update.order.execution_type == "TRADE"
                && !update.order.commission_asset.eq_ignore_ascii_case("BNB")
        })
        .filter_map(|update| BigDecimal::from_str(&update.order.commission).ok())
        .fold(BigDecimal::from(0), |total, fee| total + fee)
        .with_scale(8);
    Some(result)
}

pub async fn wait_for_usdt_trade_result(
    symbol: &str,
    order_id: Option<u64>,
    client_order_id: Option<&str>,
    wait: Duration,
) -> Option<TradeOrder> {
    let mut receiver = event_bus().subscribe();
    if let Some(result) = find_recent_trade_result(symbol, order_id, client_order_id) {
        return Some(result);
    }

    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(update)) if matches_order(&update, symbol, order_id, client_order_id) => {
                if trade_result_from_update(&update).is_some() {
                    return find_recent_trade_result(symbol, order_id, client_order_id)
                        .or_else(|| trade_result_from_update(&update));
                }
            }
            Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_update() -> BinanceOrderTradeUpdate {
        serde_json::from_str(
            r#"{
                "e":"ORDER_TRADE_UPDATE",
                "E":1787470000001,
                "T":1787470000000,
                "o":{
                    "s":"ETHUSDT",
                    "c":"7497142668339527680",
                    "S":"BUY",
                    "o":"MARKET",
                    "x":"TRADE",
                    "X":"FILLED",
                    "i":8389766260419795834,
                    "l":"0.010",
                    "z":"0.010",
                    "L":"2403.72",
                    "ap":"2403.72",
                    "n":"0.00961488",
                    "N":"USDT",
                    "ps":"LONG",
                    "T":1787470000000,
                    "t":123
                }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_filled_order_trade_update() {
        let update = filled_update();
        let trade = trade_result_from_update(&update).unwrap();

        assert_eq!(update.order.order_id, 8389766260419795834);
        assert_eq!(trade.amount, BigDecimal::from_str("0.010").unwrap());
        assert_eq!(trade.sell_price, BigDecimal::from_str("2403.72").unwrap());
    }

    #[test]
    fn matches_by_order_id_or_client_order_id() {
        let update = filled_update();
        assert!(matches_order(
            &update,
            "ethusdt",
            Some(8389766260419795834),
            None
        ));
        assert!(matches_order(
            &update,
            "ETHUSDT",
            None,
            Some("7497142668339527680")
        ));
        assert!(!matches_order(&update, "BTCUSDT", None, Some("other")));
    }

    #[tokio::test]
    async fn subscribers_receive_published_updates() {
        let mut receiver = subscribe_order_updates();
        let mut update = filled_update();
        update.order.client_order_id = "subscribe_test".to_string();
        publish_order_update(update);
        loop {
            let received = receiver.recv().await.unwrap();
            if received.order.client_order_id == "subscribe_test" {
                break;
            }
        }
    }

    #[tokio::test]
    async fn event_bus_returns_terminal_fill_and_aggregates_commission() {
        let mut partial = filled_update();
        partial.order.symbol = "EVENTTESTUSDT".to_string();
        partial.order.client_order_id = "event_bus_test".to_string();
        partial.order.order_id = 900001;
        partial.order.order_status = "PARTIALLY_FILLED".to_string();
        partial.order.accumulated_filled_quantity = "0.004".to_string();
        partial.order.commission = "0.00384595".to_string();

        let mut filled = partial.clone();
        filled.order.order_status = "FILLED".to_string();
        filled.order.accumulated_filled_quantity = "0.010".to_string();
        filled.order.commission = "0.00576893".to_string();

        publish_order_update(partial);
        publish_order_update(filled);
        let trade = wait_for_usdt_trade_result(
            "EVENTTESTUSDT",
            Some(900001),
            Some("event_bus_test"),
            Duration::ZERO,
        )
        .await
        .unwrap();

        assert_eq!(trade.amount, BigDecimal::from_str("0.010").unwrap());
        assert_eq!(trade.sell_price, BigDecimal::from_str("2403.72").unwrap());
        assert_eq!(trade.fee, BigDecimal::from_str("0.00961488").unwrap());
    }
}
