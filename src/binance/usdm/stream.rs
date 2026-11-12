//! WebSocket：行情推送和用户数据流，断线自动重连。
//!
//! 两种用法：
//! - 回调：[`run_market_stream`] / [`run_user_stream`]，回调返回 `false` 时结束；
//! - 通道：[`spawn_market_stream`] / [`spawn_user_stream`]，返回接收端，丢掉接收端即结束。
//!
//! **重连后要用 REST 重新同步**：断线期间的推送不会补发。每次连上都会先收到 `Connected`，
//! 收到它时应查一次挂单 / 持仓（用户数据流）或重新拉深度快照（深度流）。

use super::client::{UsdmClient, UsdmConfig};
use super::events::{unwrap_combined, MarketEvent, UserDataEvent};
use crate::error::{ExchangeClientError, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 行情流名称。合约代码会自动转小写。
pub mod streams {
    use super::super::models::Interval;

    /// 单个合约的最优买卖价（实时）。
    pub fn book_ticker(symbol: &str) -> String {
        format!("{}@bookTicker", symbol.to_lowercase())
    }
    /// 全部合约的最优买卖价。
    pub fn all_book_tickers() -> String {
        "!bookTicker".into()
    }
    /// 有限档深度，`levels` 为 5/10/20，`speed_ms` 为 100/250/500。
    pub fn partial_depth(symbol: &str, levels: u32, speed_ms: u32) -> String {
        with_speed(
            format!("{}@depth{}", symbol.to_lowercase(), levels),
            speed_ms,
        )
    }
    /// 增量深度，`speed_ms` 为 100/250/500。
    pub fn diff_depth(symbol: &str, speed_ms: u32) -> String {
        with_speed(format!("{}@depth", symbol.to_lowercase()), speed_ms)
    }
    /// 归集成交。
    pub fn agg_trade(symbol: &str) -> String {
        format!("{}@aggTrade", symbol.to_lowercase())
    }
    /// 标记价格和资金费，`every_second` 为 true 时每秒推送，否则每 3 秒。
    pub fn mark_price(symbol: &str, every_second: bool) -> String {
        format!(
            "{}@markPrice{}",
            symbol.to_lowercase(),
            if every_second { "@1s" } else { "" }
        )
    }
    /// 全部合约的标记价格。
    pub fn all_mark_prices(every_second: bool) -> String {
        format!("!markPrice@arr{}", if every_second { "@1s" } else { "" })
    }
    /// K 线。
    pub fn kline(symbol: &str, interval: Interval) -> String {
        format!("{}@kline_{}", symbol.to_lowercase(), interval.as_str())
    }
    /// 强平订单。
    pub fn force_order(symbol: &str) -> String {
        format!("{}@forceOrder", symbol.to_lowercase())
    }

    fn with_speed(stream: String, speed_ms: u32) -> String {
        if speed_ms == 250 {
            stream
        } else {
            format!("{stream}@{speed_ms}ms")
        }
    }
}

/// 行情流走哪个地址：盘口和深度走 public，其他走 market。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamRoute {
    Public,
    Market,
}

pub fn route_of(stream: &str) -> StreamRoute {
    if stream.contains("bookTicker") || stream.contains("@depth") {
        StreamRoute::Public
    } else {
        StreamRoute::Market
    }
}

/// 组合行情流的地址。所有流必须同属一个路由（盘口/深度和其他行情要分两个连接）。
pub fn market_stream_url(config: &UsdmConfig, streams: &[String]) -> Result<String> {
    if streams.is_empty() || streams.len() > 1024 {
        return Err(ExchangeClientError::Validation(
            "一个连接订阅 1–1024 个流".into(),
        ));
    }
    let route = route_of(&streams[0]);
    if streams.iter().any(|s| route_of(s) != route) {
        return Err(ExchangeClientError::Validation(
            "盘口/深度流（public）和其他行情流（market）地址不同，请分成两个连接".into(),
        ));
    }
    let base = match route {
        StreamRoute::Public => &config.ws_public_url,
        StreamRoute::Market => &config.ws_market_url,
    };
    Ok(format!(
        "{}/stream?streams={}",
        base.trim_end_matches('/'),
        streams.join("/")
    ))
}

/// 用户数据流的地址。主网走 `/private/ws?listenKey=..&events=..`；测试网等没有路由的地址走 `/ws/<listenKey>`。
pub fn user_stream_url(config: &UsdmConfig, listen_key: &str, events: &[String]) -> String {
    let base = config.ws_private_url.trim_end_matches('/');
    if base.ends_with("/private") {
        let mut url = format!("{base}/ws?listenKey={listen_key}");
        if !events.is_empty() {
            url.push_str("&events=");
            url.push_str(&events.join("/"));
        }
        url
    } else {
        format!("{base}/ws/{listen_key}")
    }
}

/// 连接参数。
#[derive(Clone, Debug)]
pub struct StreamOptions {
    /// 断线后多久重连。
    pub reconnect_delay: Duration,
    /// 多久没有任何消息（含交易所的 ping）就主动重连。
    pub max_silence: Duration,
    /// 连接存活多久主动重连（交易所 24 小时强制断开）。
    pub max_connection_age: Duration,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            reconnect_delay: Duration::from_secs(3),
            max_silence: Duration::from_secs(300),
            max_connection_age: Duration::from_secs(23 * 3600 + 50 * 60),
        }
    }
}

/// 行情流推送给调用方的消息。
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum MarketStreamMessage {
    /// 已连接（含重连）。深度流应在此时重新拉快照。
    Connected,
    /// `stream` 是流名称，如 `btcusdt@bookTicker`。
    Event { stream: String, event: MarketEvent },
    /// 连接断开，原因；之后会自动重连。
    Disconnected(String),
}

/// 用户数据流推送给调用方的消息。
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum UserStreamMessage {
    /// 已连接（含重连）。应在此时用 REST 查一次挂单和持仓，补上断线期间的变化。
    Connected,
    Event(UserDataEvent),
    /// 连接断开，原因；之后会自动重连。
    Disconnected(String),
}

/// 用户数据流参数。
#[derive(Clone, Debug)]
pub struct UserStreamOptions {
    pub stream: StreamOptions,
    /// 多久续期一次 listenKey（有效期 60 分钟）。
    pub keepalive_interval: Duration,
    /// 订阅的事件（主网 `events` 参数）；为空时不带这个参数。
    pub events: Vec<String>,
    /// 停止时是否关闭 listenKey。同一个 API Key 只有一个 listenKey，别的程序也在用这个 Key 订阅时设为 false。
    pub close_listen_key_on_stop: bool,
}

impl Default for UserStreamOptions {
    fn default() -> Self {
        Self {
            stream: StreamOptions {
                // 用户数据流没有成交时可以很久没有业务消息，只靠交易所每 3 分钟的 ping 判断存活。
                max_silence: Duration::from_secs(600),
                ..StreamOptions::default()
            },
            keepalive_interval: Duration::from_secs(30 * 60),
            events: [
                "ORDER_TRADE_UPDATE",
                "ACCOUNT_UPDATE",
                "TRADE_LITE",
                "MARGIN_CALL",
                "ACCOUNT_CONFIG_UPDATE",
                "ALGO_UPDATE",
                "CONDITIONAL_ORDER_TRIGGER_REJECT",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            close_listen_key_on_stop: true,
        }
    }
}

enum Incoming {
    Text(String),
    Closed(String),
}

async fn connect(url: &str) -> Result<Socket> {
    let (socket, _) = connect_async(url)
        .await
        .map_err(|error| ExchangeClientError::Connection(format!("WebSocket 连接失败: {error}")))?;
    Ok(socket)
}

/// 读一条消息；自动回 pong。返回 None 表示这条不是业务消息（ping/pong 等）。
async fn read(socket: &mut Socket) -> Option<Incoming> {
    match socket.next().await {
        Some(Ok(Message::Text(text))) => Some(Incoming::Text(text)),
        Some(Ok(Message::Binary(data))) => String::from_utf8(data).ok().map(Incoming::Text),
        Some(Ok(Message::Ping(payload))) => {
            let _ = socket.send(Message::Pong(payload)).await;
            None
        }
        Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => None,
        Some(Ok(Message::Close(frame))) => {
            Some(Incoming::Closed(format!("交易所关闭连接: {frame:?}")))
        }
        Some(Err(error)) => Some(Incoming::Closed(format!("读取失败: {error}"))),
        None => Some(Incoming::Closed("连接已结束".into())),
    }
}

/// 订阅行情流，断线自动重连；回调返回 `false` 时结束并返回 `Ok(())`。
pub async fn run_market_stream<F>(
    config: &UsdmConfig,
    streams: &[String],
    options: StreamOptions,
    mut on_message: F,
) -> Result<()>
where
    F: FnMut(MarketStreamMessage) -> bool,
{
    let url = market_stream_url(config, streams)?;
    loop {
        let reason = match connect(&url).await {
            Err(error) => error.to_string(),
            Ok(mut socket) => {
                if !on_message(MarketStreamMessage::Connected) {
                    let _ = socket.close(None).await;
                    return Ok(());
                }
                let opened = Instant::now();
                let mut last_seen = Instant::now();
                let mut check = tokio::time::interval(Duration::from_secs(5));
                loop {
                    tokio::select! {
                        incoming = read(&mut socket) => {
                            last_seen = Instant::now();
                            match incoming {
                                None => {}
                                Some(Incoming::Closed(reason)) => break reason,
                                Some(Incoming::Text(text)) => {
                                    let Some((stream, event)) = parse_market(&text) else { continue };
                                    if !on_message(MarketStreamMessage::Event { stream, event }) {
                                        let _ = socket.close(None).await;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        _ = check.tick() => {
                            if last_seen.elapsed() > options.max_silence {
                                break format!("{} 秒没有消息", options.max_silence.as_secs());
                            }
                            if opened.elapsed() > options.max_connection_age {
                                break "连接接近 24 小时上限，主动重连".to_string();
                            }
                        }
                    }
                }
            }
        };
        log::warn!(
            "币安行情流断开：{reason}，{} 秒后重连",
            options.reconnect_delay.as_secs()
        );
        if !on_message(MarketStreamMessage::Disconnected(reason)) {
            return Ok(());
        }
        tokio::time::sleep(options.reconnect_delay).await;
    }
}

fn parse_market(text: &str) -> Option<(String, MarketEvent)> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("币安行情推送无法解析: {error}");
            return None;
        }
    };
    // 订阅确认 {"result":null,"id":..} 之类的消息忽略。
    if value.get("result").is_some() && value.get("id").is_some() {
        return None;
    }
    let stream = value
        .get("stream")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    match MarketEvent::from_value(unwrap_combined(value)) {
        Ok(event) => Some((stream, event)),
        Err(error) => {
            log::warn!("币安行情推送字段不符合预期: {error}");
            None
        }
    }
}

/// 在后台任务里订阅行情流，返回接收端。丢掉接收端后任务自动结束。
pub fn spawn_market_stream(
    config: UsdmConfig,
    streams: Vec<String>,
    options: StreamOptions,
    buffer: usize,
) -> Result<(
    mpsc::Receiver<MarketStreamMessage>,
    tokio::task::JoinHandle<()>,
)> {
    market_stream_url(&config, &streams)?;
    let (sender, receiver) = mpsc::channel(buffer.max(1));
    let handle = tokio::spawn(async move {
        let result = run_market_stream(&config, &streams, options, |message| {
            match sender.try_send(message) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    log::warn!("币安行情流接收端处理不过来，丢弃一条推送");
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        })
        .await;
        if let Err(error) = result {
            log::error!("币安行情流退出: {error}");
        }
    });
    Ok((receiver, handle))
}

/// 订阅用户数据流（订单、成交、余额、持仓变化），自动创建/续期/重建 listenKey、断线重连；
/// 回调返回 `false` 时关闭 listenKey 并返回 `Ok(())`。
pub async fn run_user_stream<F>(
    client: &UsdmClient,
    options: UserStreamOptions,
    mut on_message: F,
) -> Result<()>
where
    F: FnMut(UserStreamMessage) -> bool,
{
    client.api_key()?;
    loop {
        let reason = match session(client, &options, &mut on_message).await {
            SessionEnd::Stop => {
                if options.close_listen_key_on_stop {
                    let _ = client.close_user_stream().await;
                }
                return Ok(());
            }
            SessionEnd::Reconnect(reason) => reason,
        };
        log::warn!(
            "币安用户数据流断开：{reason}，{} 秒后重连",
            options.stream.reconnect_delay.as_secs()
        );
        if !on_message(UserStreamMessage::Disconnected(reason)) {
            if options.close_listen_key_on_stop {
                let _ = client.close_user_stream().await;
            }
            return Ok(());
        }
        tokio::time::sleep(options.stream.reconnect_delay).await;
    }
}

enum SessionEnd {
    Stop,
    Reconnect(String),
}

async fn session<F>(
    client: &UsdmClient,
    options: &UserStreamOptions,
    on_message: &mut F,
) -> SessionEnd
where
    F: FnMut(UserStreamMessage) -> bool,
{
    let listen_key = match client.start_user_stream().await {
        Ok(key) => key,
        Err(error) => return SessionEnd::Reconnect(format!("创建 listenKey 失败: {error}")),
    };
    let url = user_stream_url(client.config(), &listen_key, &options.events);
    let mut socket = match connect(&url).await {
        Ok(socket) => socket,
        Err(error) => return SessionEnd::Reconnect(error.to_string()),
    };
    if !on_message(UserStreamMessage::Connected) {
        let _ = socket.close(None).await;
        return SessionEnd::Stop;
    }
    let opened = Instant::now();
    let mut last_seen = Instant::now();
    let mut check = tokio::time::interval(Duration::from_secs(5));
    let mut keepalive = tokio::time::interval_at(
        Instant::now() + options.keepalive_interval,
        options.keepalive_interval,
    );
    loop {
        tokio::select! {
            incoming = read(&mut socket) => {
                last_seen = Instant::now();
                match incoming {
                    None => {}
                    Some(Incoming::Closed(reason)) => return SessionEnd::Reconnect(reason),
                    Some(Incoming::Text(text)) => {
                        let event = match UserDataEvent::parse(&text) {
                            Ok(event) => event,
                            Err(error) => {
                                log::warn!("币安用户数据推送字段不符合预期: {error}; {text}");
                                continue;
                            }
                        };
                        let expired = matches!(event, UserDataEvent::ListenKeyExpired { .. });
                        if !on_message(UserStreamMessage::Event(event)) {
                            let _ = socket.close(None).await;
                            return SessionEnd::Stop;
                        }
                        if expired {
                            let _ = socket.close(None).await;
                            return SessionEnd::Reconnect("listenKey 已过期".into());
                        }
                    }
                }
            }
            _ = keepalive.tick() => {
                if let Err(error) = client.keepalive_user_stream().await {
                    if error.api_code() == Some(-1125) {
                        let _ = socket.close(None).await;
                        return SessionEnd::Reconnect("listenKey 已失效".into());
                    }
                    // 网络抖动：下一轮再续，listenKey 有 60 分钟有效期。
                    log::warn!("续期 listenKey 失败: {error}");
                }
            }
            _ = check.tick() => {
                if last_seen.elapsed() > options.stream.max_silence {
                    let _ = socket.close(None).await;
                    return SessionEnd::Reconnect(format!("{} 秒没有消息", options.stream.max_silence.as_secs()));
                }
                if opened.elapsed() > options.stream.max_connection_age {
                    let _ = socket.close(None).await;
                    return SessionEnd::Reconnect("连接接近 24 小时上限，主动重连".into());
                }
            }
        }
    }
}

/// 在后台任务里订阅用户数据流，返回接收端。丢掉接收端后任务自动结束。
///
/// 用户数据流的消息不能丢，所以接收端处理不过来时会等待（背压），不会丢弃。
pub fn spawn_user_stream(
    client: UsdmClient,
    options: UserStreamOptions,
    buffer: usize,
) -> Result<(
    mpsc::Receiver<UserStreamMessage>,
    tokio::task::JoinHandle<()>,
)> {
    client.api_key()?;
    let (sender, receiver) = mpsc::channel(buffer.max(1));
    let handle = tokio::spawn(async move {
        // 回调是同步的，这里用一个无界的中转队列把消息交给异步发送，保证不丢。
        let (relay_tx, mut relay_rx) = mpsc::unbounded_channel::<UserStreamMessage>();
        let forward = tokio::spawn(async move {
            while let Some(message) = relay_rx.recv().await {
                if sender.send(message).await.is_err() {
                    break;
                }
            }
        });
        let result = run_user_stream(&client, options, |message| {
            relay_tx.send(message).is_ok() && !relay_tx.is_closed()
        })
        .await;
        drop(relay_tx);
        let _ = forward.await;
        if let Err(error) = result {
            log::error!("币安用户数据流退出: {error}");
        }
    });
    Ok((receiver, handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binance::usdm::models::Interval;

    #[test]
    fn stream_names_and_routes() {
        assert_eq!(streams::book_ticker("BTCUSDT"), "btcusdt@bookTicker");
        assert_eq!(
            streams::partial_depth("BTCUSDT", 5, 100),
            "btcusdt@depth5@100ms"
        );
        assert_eq!(streams::diff_depth("BTCUSDT", 250), "btcusdt@depth");
        assert_eq!(streams::mark_price("ETHUSDT", true), "ethusdt@markPrice@1s");
        assert_eq!(streams::kline("ETHUSDT", Interval::H4), "ethusdt@kline_4h");
        assert_eq!(route_of("!bookTicker"), StreamRoute::Public);
        assert_eq!(route_of("btcusdt@depth5@100ms"), StreamRoute::Public);
        assert_eq!(route_of("!markPrice@arr"), StreamRoute::Market);
    }

    #[test]
    fn stream_urls() {
        let config = UsdmConfig::mainnet();
        let url = market_stream_url(
            &config,
            &[
                streams::book_ticker("BTCUSDT"),
                streams::book_ticker("ETHUSDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            url,
            "wss://fstream.binance.com/public/stream?streams=btcusdt@bookTicker/ethusdt@bookTicker"
        );
        let url = market_stream_url(&config, &[streams::all_mark_prices(false)]).unwrap();
        assert_eq!(
            url,
            "wss://fstream.binance.com/market/stream?streams=!markPrice@arr"
        );
        assert!(market_stream_url(
            &config,
            &[
                streams::book_ticker("BTCUSDT"),
                streams::agg_trade("BTCUSDT")
            ]
        )
        .is_err());
        assert_eq!(
            user_stream_url(&config, "abc", &["ORDER_TRADE_UPDATE".into(), "ACCOUNT_UPDATE".into()]),
            "wss://fstream.binance.com/private/ws?listenKey=abc&events=ORDER_TRADE_UPDATE/ACCOUNT_UPDATE"
        );
        assert_eq!(
            user_stream_url(&UsdmConfig::testnet(), "abc", &[]),
            "wss://fstream.binancefuture.com/ws/abc"
        );
        assert_eq!(
            market_stream_url(&UsdmConfig::testnet(), &[streams::book_ticker("BTCUSDT")]).unwrap(),
            "wss://fstream.binancefuture.com/stream?streams=btcusdt@bookTicker"
        );
    }

    #[test]
    fn ignores_subscription_acks() {
        assert!(parse_market(r#"{"result":null,"id":1}"#).is_none());
        let (stream, event) = parse_market(
            r#"{"stream":"btcusdt@bookTicker","data":{"e":"bookTicker","u":1,"E":1,"T":1,"s":"BTCUSDT","b":"1","B":"1","a":"2","A":"1"}}"#,
        )
        .unwrap();
        assert_eq!(stream, "btcusdt@bookTicker");
        assert!(matches!(event, MarketEvent::BookTicker(_)));
    }
}
