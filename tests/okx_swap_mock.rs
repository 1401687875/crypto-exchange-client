//! 用本地模拟服务器检查 OKX 永续合约客户端：请求路径和参数、签名、模拟盘请求头、业务错误码、翻页。不访问 OKX。

use crypto_exchange_client::okx::swap::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

type Routes = Arc<dyn Fn(&str, &str, &str) -> String + Send + Sync>;

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
                let head_end = loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_string();
                let target = first.next().unwrap().to_string();
                let headers: HashMap<String, String> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while buffer.len() < head_end + length {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buffer[head_end..]).to_string();
                let reply = routes(&method, &target, &body);
                log.lock().unwrap().push(Recorded {
                    method,
                    target,
                    headers,
                    body,
                });
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (address, log)
}

fn ok(data: &str) -> String {
    format!(r#"{{"code":"0","msg":"","data":{data}}}"#)
}

fn d(text: &str) -> Decimal {
    text.parse().unwrap()
}

const ORDER: &str = r#"{"instId":"BTC-USDT-SWAP","ordId":"312269865356374016","clOrdId":"xsZ1","px":"65000.1","sz":"2","ordType":"post_only","side":"buy","posSide":"long","tdMode":"cross","accFillSz":"2","avgPx":"65000.1","state":"filled","fee":"-0.026","feeCcy":"USDT","reduceOnly":"false","cTime":"1","uTime":"2"}"#;

#[tokio::test]
async fn requests_are_signed_routed_and_flagged_as_demo() {
    let routes: Routes = Arc::new(|method, target, _body| {
        let path = target.split('?').next().unwrap();
        match (method, path) {
            ("GET", "/api/v5/public/instruments") => ok(
                r#"[{"instId":"BTC-USDT-SWAP","settleCcy":"USDT","ctType":"linear","ctVal":"0.01","lotSz":"0.01","minSz":"0.01","tickSz":"0.1","state":"live"}]"#,
            ),
            ("GET", "/api/v5/market/tickers") => ok(
                r#"[{"instId":"BTC-USDT-SWAP","last":"65000","bidPx":"64999.9","bidSz":"10","askPx":"65000","askSz":"3","ts":"1"}]"#,
            ),
            ("POST", "/api/v5/trade/order") => {
                ok(r#"[{"ordId":"312269865356374016","clOrdId":"xsZ1","sCode":"0","sMsg":""}]"#)
            }
            ("GET", "/api/v5/trade/order") => ok(&format!("[{ORDER}]")),
            ("GET", "/api/v5/trade/fills") => ok(
                r#"[{"instId":"BTC-USDT-SWAP","tradeId":"9","ordId":"312269865356374016","side":"buy","posSide":"long","fillPx":"65000.1","fillSz":"2","fee":"-0.026","feeCcy":"USDT","execType":"M","fillPnl":"0","ts":"3"}]"#,
            ),
            ("GET", "/api/v5/account/positions") => ok(
                r#"[{"instId":"BTC-USDT-SWAP","posSide":"short","pos":"3","avgPx":"65000","markPx":"64000","upl":"3","mgnMode":"cross","lever":"5"}]"#,
            ),
            ("GET", "/api/v5/account/balance") => ok(
                r#"[{"totalEq":"1001.5","details":[{"ccy":"USDT","eq":"1000.5","cashBal":"990","availEq":"800","availBal":"","upl":"10.5"}]}]"#,
            ),
            ("GET", "/api/v5/account/config") => {
                ok(r#"[{"uid":"1","posMode":"long_short_mode","acctLv":"2"}]"#)
            }
            ("POST", "/api/v5/account/set-leverage") => {
                ok(r#"[{"instId":"BTC-USDT-SWAP","lever":"5","mgnMode":"cross","posSide":""}]"#)
            }
            ("POST", "/api/v5/trade/order-algo") => {
                ok(r#"[{"algoId":"77","algoClOrdId":"xsZslZ1","sCode":"0","sMsg":""}]"#)
            }
            ("POST", "/api/v5/trade/cancel-algos") => {
                ok(r#"[{"algoId":"77","sCode":"0","sMsg":""}]"#)
            }
            _ => r#"{"code":"404","msg":"no route","data":[]}"#.into(),
        }
    });
    let (rest, log) = mock_http(routes).await;
    let config = SwapConfig {
        rest_url: rest,
        ..SwapConfig::demo()
    };
    let client = SwapClient::new(config, "my-key", "my-secret", "my-pass");

    let instruments = client.instruments().await.unwrap();
    assert_eq!(instruments[0].ct_val, d("0.01"));
    let tickers = client.tickers().await.unwrap();
    assert_eq!(tickers[0].bid_px, d("64999.9"));
    let order = NewOrder::post_only("BTC-USDT-SWAP", "buy", d("2"), d("65000.1"))
        .pos_side("long")
        .client_order_id("xsZ1");
    assert_eq!(
        client.place_order(&order).await.unwrap().ord_id,
        "312269865356374016"
    );
    let queried = client
        .order("BTC-USDT-SWAP", &OrderRef::Client("xsZ1".into()))
        .await
        .unwrap();
    assert!(queried.is_final());
    assert_eq!(queried.acc_fill_sz, d("2"));
    let fills = client
        .order_fills("BTC-USDT-SWAP", "312269865356374016")
        .await
        .unwrap();
    assert!(fills[0].is_maker());
    let positions = client.positions().await.unwrap();
    assert_eq!(positions[0].pos_side, "short");
    let balance = client.balance(Some("USDT")).await.unwrap();
    assert_eq!(balance.detail("USDT").unwrap().avail_eq, Some(d("800")));
    assert_eq!(balance.detail("USDT").unwrap().avail_bal, None);
    assert!(client.account_config().await.unwrap().is_long_short());
    client
        .set_leverage("BTC-USDT-SWAP", 5, "cross")
        .await
        .unwrap();
    let stop = NewStopOrder::stop_market("BTC-USDT-SWAP", "buy", d("3"), d("97500"))
        .pos_side("short")
        .client_id("xsZslZ1");
    assert_eq!(client.place_stop(&stop).await.unwrap().algo_id, "77");
    client.cancel_stop("BTC-USDT-SWAP", "77").await.unwrap();

    let log = log.lock().unwrap().clone();
    // 公开接口不签名；所有请求都带模拟盘标记。
    assert!(log
        .iter()
        .all(|r| r.headers.get("x-simulated-trading").map(String::as_str) == Some("1")));
    assert!(log[0].headers.get("ok-access-key").is_none());
    assert_eq!(log[0].target, "/api/v5/public/instruments?instType=SWAP");
    // 签名 = Base64(HMAC(时间戳 + 方法 + 路径含查询串 + 请求体))。
    for r in log
        .iter()
        .filter(|r| r.headers.contains_key("ok-access-key"))
    {
        assert_eq!(r.headers["ok-access-key"], "my-key");
        assert_eq!(r.headers["ok-access-passphrase"], "my-pass");
        let expected = sign(
            "my-secret",
            &r.headers["ok-access-timestamp"],
            &r.method,
            &r.target,
            &r.body,
        );
        assert_eq!(
            r.headers["ok-access-sign"], expected,
            "{} {}",
            r.method, r.target
        );
    }
    let placed = log
        .iter()
        .find(|r| r.method == "POST" && r.target == "/api/v5/trade/order")
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&placed.body).unwrap();
    assert_eq!(body["ordType"], "post_only");
    assert_eq!(body["clOrdId"], "xsZ1");
    let queried = log
        .iter()
        .find(|r| r.method == "GET" && r.target.starts_with("/api/v5/trade/order?"))
        .unwrap();
    assert_eq!(
        queried.target,
        "/api/v5/trade/order?instId=BTC-USDT-SWAP&clOrdId=xsZ1"
    );
    let cancel = log
        .iter()
        .find(|r| r.target == "/api/v5/trade/cancel-algos")
        .unwrap();
    assert_eq!(cancel.body, r#"[{"algoId":"77","instId":"BTC-USDT-SWAP"}]"#);
}

#[tokio::test]
async fn item_errors_and_fallbacks() {
    let routes: Routes = Arc::new(|method, target, _body| {
        let path = target.split('?').next().unwrap();
        match (method, path) {
            ("POST", "/api/v5/trade/order") => r#"{"code":"1","msg":"All operations failed","data":[{"ordId":"","clOrdId":"x","sCode":"51008","sMsg":"Order failed. Insufficient USDT margin in account"}]}"#.into(),
            ("GET", "/api/v5/trade/order") => r#"{"code":"51603","msg":"Order does not exist","data":[]}"#.into(),
            ("POST", "/api/v5/trade/cancel-order") => r#"{"code":"1","msg":"","data":[{"ordId":"1","clOrdId":"x","sCode":"51401","sMsg":"Cancellation failed as the order has been canceled."}]}"#.into(),
            ("GET", "/api/v5/trade/fills") => ok("[]"),
            ("GET", "/api/v5/trade/fills-history") => ok(r#"[{"instId":"ETH-USDT-SWAP","ordId":"5","side":"sell","fillPx":"3000","fillSz":"1","fee":"0.01","feeCcy":"USDT","execType":"T","ts":"1"}]"#),
            ("GET", "/api/v5/trade/orders-pending") => {
                // 第一页 100 条，第二页 1 条。
                if target.contains("after=") {
                    ok(&format!("[{ORDER}]"))
                } else {
                    ok(&format!("[{}]", vec![ORDER; 100].join(",")))
                }
            }
            _ => r#"{"code":"404","msg":"no route","data":[]}"#.into(),
        }
    });
    let (rest, log) = mock_http(routes).await;
    let client = SwapClient::new(
        SwapConfig {
            rest_url: rest,
            ..SwapConfig::mainnet()
        },
        "k",
        "s",
        "p",
    );
    let err = client
        .place_order(&NewOrder::market("BTC-USDT-SWAP", "buy", d("1")))
        .await
        .unwrap_err();
    assert_eq!(err.api_code(), Some(51008));
    assert!(!err.is_unknown_execution());
    let err = client
        .order("BTC-USDT-SWAP", &OrderRef::Client("x".into()))
        .await
        .unwrap_err();
    assert_eq!(err.api_code(), Some(51603));
    let err = client
        .cancel_order("BTC-USDT-SWAP", &OrderRef::Id("1".into()))
        .await
        .unwrap_err();
    assert_eq!(err.api_code(), Some(51401));
    let fills = client.order_fills("ETH-USDT-SWAP", "5").await.unwrap();
    assert_eq!(fills.len(), 1);
    assert!(!fills[0].is_maker());
    assert_eq!(client.pending_orders().await.unwrap().len(), 101);
    let log = log.lock().unwrap();
    // 实盘不带模拟盘标记。
    assert!(log
        .iter()
        .all(|r| !r.headers.contains_key("x-simulated-trading")));
    assert!(log.iter().any(|r| r
        .target
        .contains("orders-pending?instType=SWAP&after=312269865356374016&limit=100")));
}

#[test]
fn contract_conversion() {
    let instrument: Instrument = serde_json::from_str(
        r#"{"instId":"ETH-USDT-SWAP","ctVal":"0.1","lotSz":"0.01","minSz":"0.01","tickSz":"0.01","state":"live"}"#,
    )
    .unwrap();
    assert_eq!(coins_to_contracts(d("0.1234"), &instrument), d("1.23"));
    assert_eq!(contracts_to_coins(d("1.23"), &instrument), d("0.123"));
}
