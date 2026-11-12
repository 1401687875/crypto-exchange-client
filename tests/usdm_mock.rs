//! 用本地模拟服务器检查 U 本位客户端发出的请求（方法、路径、参数、签名、API Key 请求头），
//! 以及用户数据流的完整流程（创建 listenKey → 连接 → 收事件 → 停止时关闭 listenKey）。不访问币安。
#![allow(clippy::result_large_err)]

use crypto_exchange_client::binance::usdm::*;
use futures_util::SinkExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    path: String,
    query: String,
    api_key: Option<String>,
}

type Routes = Arc<dyn Fn(&str, &str) -> (u16, String) + Send + Sync>;

/// 极简 HTTP 服务器：记录每个请求，按 (方法, 路径) 返回预设响应。
async fn mock_http(routes: Routes) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let log_inner = log.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let routes = routes.clone();
            let log = log_inner.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..n]);
                    if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buffer).to_string();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_string();
                let target = first.next().unwrap().to_string();
                let (path, query) = target
                    .split_once('?')
                    .map(|(p, q)| (p.to_string(), q.to_string()))
                    .unwrap_or((target.clone(), String::new()));
                let headers: HashMap<String, String> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                    .collect();
                log.lock().unwrap().push(Recorded {
                    method: method.clone(),
                    path: path.clone(),
                    query,
                    api_key: headers.get("x-mbx-apikey").cloned(),
                });
                let (status, body) = routes(&method, &path);
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nx-mbx-used-weight-1m: 7\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (address, log)
}

fn config(rest: &str, ws: &str) -> UsdmConfig {
    UsdmConfig {
        rest_url: rest.into(),
        ws_public_url: ws.into(),
        ws_market_url: ws.into(),
        ws_private_url: ws.into(),
        ..UsdmConfig::testnet()
    }
}

fn verify_signature(query: &str, secret: &str) {
    let (unsigned, signature) = query.rsplit_once("&signature=").expect("缺少签名");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(unsigned.as_bytes());
    assert_eq!(hex::encode(mac.finalize().into_bytes()), signature);
    assert!(unsigned.contains("timestamp="));
    assert!(unsigned.contains("recvWindow=5000"));
}

const ORDER: &str = r#"{"orderId":42,"symbol":"BTCUSDT","status":"NEW","clientOrderId":"mk-1","price":"65000.1","avgPrice":"0","origQty":"0.002","executedQty":"0","cumQuote":"0","timeInForce":"GTX","type":"LIMIT","reduceOnly":false,"closePosition":false,"side":"BUY","positionSide":"BOTH","updateTime":1}"#;

fn d(text: &str) -> Decimal {
    text.parse().unwrap()
}

#[tokio::test]
async fn trading_requests_are_signed_and_routed() {
    let routes: Routes = Arc::new(|method, path| {
        match (method, path) {
        ("POST", "/fapi/v1/order") | ("GET", "/fapi/v1/order") | ("DELETE", "/fapi/v1/order") | ("PUT", "/fapi/v1/order") => (200, ORDER.into()),
        ("GET", "/fapi/v1/openOrders") => (200, format!("[{ORDER}]")),
        ("DELETE", "/fapi/v1/allOpenOrders") => (200, r#"{"code":200,"msg":"The operation of cancel all open order is done."}"#.into()),
        ("POST", "/fapi/v1/batchOrders") => (200, format!(r#"[{ORDER},{{"code":-5022,"msg":"Due to the order could not be executed as maker, the Post Only order will be rejected."}}]"#)),
        ("GET", "/fapi/v1/userTrades") => (200, r#"[{"buyer":true,"commission":"0.026","commissionAsset":"USDT","id":698759,"maker":true,"orderId":42,"price":"65000.1","qty":"0.002","quoteQty":"130.0002","realizedPnl":"0","side":"BUY","positionSide":"BOTH","symbol":"BTCUSDT","time":1569514978020}]"#.into()),
        ("POST", "/fapi/v1/algoOrder") => (200, r#"{"algoId":7,"clientAlgoId":"stop-1","algoType":"CONDITIONAL","orderType":"STOP_MARKET","symbol":"BTCUSDT","side":"SELL","quantity":"","algoStatus":"NEW","triggerPrice":"60000","closePosition":true,"createTime":1}"#.into()),
        ("GET", "/fapi/v1/commissionRate") => (200, r#"{"symbol":"BTCUSDT","makerCommissionRate":"0.0002","takerCommissionRate":"0.0005"}"#.into()),
        _ => (404, r#"{"code":-1,"msg":"no route"}"#.into()),
    }
    });
    let (rest, log) = mock_http(routes).await;
    let client = UsdmClient::new(config(&rest, "ws://unused"), "my-key", "my-secret");

    let order = NewOrder::post_only("BTCUSDT", Side::Buy, d("0.002"), d("65000.10"))
        .client_order_id("mk-1");
    let placed = client.place_order(&order).await.unwrap();
    assert_eq!(placed.order_id, 42);
    assert_eq!(placed.time_in_force, Some(TimeInForce::Gtx));
    assert_eq!(client.used_weight_1m(), 7);

    client.query_order("BTCUSDT", "mk-1").await.unwrap();
    client.cancel_order("BTCUSDT", 42).await.unwrap();
    client
        .modify_order(&ModifyOrder::new(
            "BTCUSDT",
            OrderId::Exchange(42),
            Side::Buy,
            d("0.002"),
            d("65000.2"),
        ))
        .await
        .unwrap();
    assert_eq!(client.open_orders(Some("BTCUSDT")).await.unwrap().len(), 1);
    assert_eq!(client.cancel_all_orders("BTCUSDT").await.unwrap().code, 200);
    let batch = client
        .place_batch_orders(&[order.clone(), order.clone()])
        .await
        .unwrap();
    assert!(matches!(batch[1], BatchItem::Err { code: -5022, .. }));
    let trades = client.order_trades("BTCUSDT", 42).await.unwrap();
    assert!(trades[0].maker);
    let stop = client
        .place_algo_order(
            &NewAlgoOrder::stop_market("BTCUSDT", Side::Sell, d("60000"))
                .close_position()
                .client_algo_id("stop-1"),
        )
        .await
        .unwrap();
    assert_eq!(stop.quantity, None);
    assert_eq!(
        client
            .commission_rate("BTCUSDT")
            .await
            .unwrap()
            .maker_commission_rate,
        d("0.0002")
    );

    let log = log.lock().unwrap().clone();
    let calls: Vec<(String, String)> = log
        .iter()
        .map(|r| (r.method.clone(), r.path.clone()))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("POST".into(), "/fapi/v1/order".into()),
            ("GET".into(), "/fapi/v1/order".into()),
            ("DELETE".into(), "/fapi/v1/order".into()),
            ("PUT".into(), "/fapi/v1/order".into()),
            ("GET".into(), "/fapi/v1/openOrders".into()),
            ("DELETE".into(), "/fapi/v1/allOpenOrders".into()),
            ("POST".into(), "/fapi/v1/batchOrders".into()),
            ("GET".into(), "/fapi/v1/userTrades".into()),
            ("POST".into(), "/fapi/v1/algoOrder".into()),
            ("GET".into(), "/fapi/v1/commissionRate".into()),
        ]
    );
    for record in &log {
        assert_eq!(record.api_key.as_deref(), Some("my-key"));
        verify_signature(&record.query, "my-secret");
    }
    assert!(log[0].query.starts_with("symbol=BTCUSDT&side=BUY&type=LIMIT&timeInForce=GTX&quantity=0.002&price=65000.1&newClientOrderId=mk-1&"));
    assert!(log[1].query.contains("origClientOrderId=mk-1"));
    assert!(log[2].query.contains("orderId=42"));
    assert!(log[3].query.contains("price=65000.2"));
    assert!(
        log[6].query.contains("batchOrders=%5B%7B")
            && log[6].query.contains("%22symbol%22%3A%22BTCUSDT%22")
    );
    assert!(
        log[8].query.contains("algoType=CONDITIONAL")
            && log[8].query.contains("closePosition=true")
    );
}

#[tokio::test]
async fn api_errors_and_public_endpoints() {
    let routes: Routes = Arc::new(|method, path| {
        match (method, path) {
        ("POST", "/fapi/v1/order") => (400, r#"{"code":-5022,"msg":"Post Only order will be rejected."}"#.into()),
        ("GET", "/fapi/v1/ticker/bookTicker") => (200, r#"{"symbol":"BTCUSDT","bidPrice":"65000.0","bidQty":"1","askPrice":"65000.1","askQty":"2","time":1}"#.into()),
        ("GET", "/fapi/v1/time") => (200, r#"{"serverTime":4102444800000}"#.into()),
        ("POST", "/fapi/v1/marginType") => (400, r#"{"code":-4046,"msg":"No need to change margin type."}"#.into()),
        ("GET", "/fapi/v1/markPriceKlines") => (200, r#"[[1591256400000,"9653.69440000","9653.69640000","9651.38600000","9651.55200000","0",1591256459999,"0",60,"0","0","0"]]"#.into()),
        _ => (503, "Service Unavailable".into()),
    }
    });
    let (rest, log) = mock_http(routes).await;
    let client = UsdmClient::new(config(&rest, "ws://unused"), "k", "s");

    let error = client
        .place_order(&NewOrder::post_only("BTCUSDT", Side::Buy, d("1"), d("1")))
        .await
        .unwrap_err();
    assert_eq!(error.api_code(), Some(-5022));
    assert!(!error.is_unknown_execution());

    let error = client.query_order("BTCUSDT", 1).await.unwrap_err();
    assert!(error.is_unknown_execution(), "{error}");

    // 本地校验失败时不发请求。
    let before = log.lock().unwrap().len();
    assert!(client
        .place_order(&NewOrder::market("BTCUSDT", Side::Buy, d("0")))
        .await
        .is_err());
    assert_eq!(log.lock().unwrap().len(), before);

    let public = UsdmClient::public(config(&rest, "ws://unused"));
    let book = public.book_ticker("BTCUSDT").await.unwrap();
    assert_eq!(book.ask_price, d("65000.1"));
    assert!(log.lock().unwrap().last().unwrap().api_key.is_none());
    assert!(public.positions(None).await.is_err());

    let offset = client.sync_time().await.unwrap();
    assert!(offset > 1_000_000_000);
    client
        .set_margin_type("BTCUSDT", MarginType::Crossed)
        .await
        .unwrap();
    let klines = public
        .mark_price_klines("BTCUSDT", Interval::M1, Range::default())
        .await
        .unwrap();
    assert_eq!(klines[0].trades, 60);
}

#[tokio::test]
async fn user_stream_end_to_end() {
    // WebSocket 服务器：连上后推一个订单事件，然后保持连接。
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_address = format!("ws://{}", ws_listener.local_addr().unwrap());
    let paths = Arc::new(Mutex::new(Vec::<String>::new()));
    let paths_inner = paths.clone();
    tokio::spawn(async move {
        loop {
            let (socket, _) = ws_listener.accept().await.unwrap();
            let paths = paths_inner.clone();
            tokio::spawn(async move {
                let callback = |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                                response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    paths.lock().unwrap().push(request.uri().to_string());
                    Ok(response)
                };
                let mut ws = tokio_tungstenite::accept_hdr_async(socket, callback)
                    .await
                    .unwrap();
                let event = r#"{"e":"ORDER_TRADE_UPDATE","E":1,"T":1,"o":{"s":"BTCUSDT","c":"mk-1","S":"BUY","o":"LIMIT","f":"GTX","q":"0.002","p":"65000.1","ap":"65000.1","sp":"0","x":"TRADE","X":"FILLED","i":42,"l":"0.002","z":"0.002","L":"65000.1","N":"USDT","n":"0.026","T":1,"t":9,"b":"0","a":"0","m":true,"R":false,"wt":"CONTRACT_PRICE","ot":"LIMIT","ps":"BOTH","cp":false,"rp":"0"}}"#;
                ws.send(tokio_tungstenite::tungstenite::Message::Text(event.into()))
                    .await
                    .unwrap();
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            });
        }
    });
    let routes: Routes = Arc::new(|method, path| match (method, path) {
        ("POST", "/fapi/v1/listenKey") => (200, r#"{"listenKey":"LK123"}"#.into()),
        ("DELETE", "/fapi/v1/listenKey") => (200, "{}".into()),
        _ => (404, r#"{"code":-1,"msg":"no route"}"#.into()),
    });
    let (rest, log) = mock_http(routes).await;
    let client = UsdmClient::new(config(&rest, &ws_address), "key", "secret");

    let mut seen = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_user_stream(&client, UserStreamOptions::default(), |message| {
            let stop = matches!(message, UserStreamMessage::Event(_));
            seen.push(message);
            !stop
        }),
    )
    .await
    .expect("用户数据流超时")
    .unwrap();

    assert_eq!(seen[0], UserStreamMessage::Connected);
    let UserStreamMessage::Event(UserDataEvent::OrderTradeUpdate(update)) = &seen[1] else {
        panic!("{seen:?}")
    };
    assert!(update.order.is_fill() && update.order.is_maker);
    assert_eq!(paths.lock().unwrap()[0], "/ws/LK123");
    let calls: Vec<(String, String, Option<String>)> = log
        .lock()
        .unwrap()
        .iter()
        .map(|r| (r.method.clone(), r.path.clone(), r.api_key.clone()))
        .collect();
    assert_eq!(
        calls,
        vec![
            (
                "POST".into(),
                "/fapi/v1/listenKey".into(),
                Some("key".into())
            ),
            (
                "DELETE".into(),
                "/fapi/v1/listenKey".into(),
                Some("key".into())
            ),
        ]
    );
}

#[tokio::test]
async fn market_stream_reconnects() {
    // 第一次连接推一条后断开，第二次连接再推一条：应收到 Connected, Event, Disconnected, Connected, Event。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        for round in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let event = format!(
                r#"{{"stream":"btcusdt@bookTicker","data":{{"e":"bookTicker","u":{round},"E":1,"T":1,"s":"BTCUSDT","b":"1","B":"1","a":"2","A":"1"}}}}"#
            );
            ws.send(tokio_tungstenite::tungstenite::Message::Text(event))
                .await
                .unwrap();
            if round == 0 {
                ws.close(None).await.unwrap();
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    });
    let options = StreamOptions {
        reconnect_delay: std::time::Duration::from_millis(50),
        ..StreamOptions::default()
    };
    let mut seen = Vec::new();
    let mut events = 0;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_market_stream(
            &config("http://unused", &address),
            &[streams::book_ticker("BTCUSDT")],
            options,
            |message| {
                if matches!(message, MarketStreamMessage::Event { .. }) {
                    events += 1;
                }
                seen.push(std::mem::discriminant(&message));
                events < 2
            },
        ),
    )
    .await
    .expect("行情流超时")
    .unwrap();
    let connected = std::mem::discriminant(&MarketStreamMessage::Connected);
    let disconnected = std::mem::discriminant(&MarketStreamMessage::Disconnected(String::new()));
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[0], connected);
    assert_eq!(seen[2], disconnected);
    assert_eq!(seen[3], connected);
}
