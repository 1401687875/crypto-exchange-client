use super::websocket::*;
use super::*;

#[test]
fn order_matrix_and_fractional_gtc() {
    assert!(PlaceOrderRequest::market_buy("AAPL", "10.25")
        .validate()
        .is_ok());
    assert!(PlaceOrderRequest::market_sell("AAPL", "0.01")
        .validate()
        .is_ok());
    let mut buy = PlaceOrderRequest::market_buy("AAPL", "10");
    buy.quantity = Some("1".into());
    assert!(buy.validate().is_err());
    let mut limit =
        PlaceOrderRequest::limit("AAPL", Side::Buy, "0.5", "180.50", TradingSession::Rth);
    limit.time_in_force = Some(TimeInForce::Gtc);
    assert!(limit.validate().is_err());
    limit.trading_session = Some(TradingSession::Extended);
    assert!(limit.validate().is_ok());
    limit.price = Some("180.501".into());
    assert!(limit.validate().is_err());
    assert!(PlaceOrderRequest::market_sell("AAPL", "-1")
        .validate()
        .is_err());
}

#[test]
fn wire_signature_covers_encoded_query_and_auth_modes() {
    let client = BinanceStocksClient::new("test-key", "secret");
    let request = client
        .request(
            Method::GET,
            "order/history",
            vec![("orderStatus".into(), "FILLED,CANCELED".into())],
            true,
            true,
        )
        .unwrap();
    let query = request.url().query().unwrap();
    let (payload, signature) = query.rsplit_once("&signature=").unwrap();
    assert!(payload.contains("FILLED%2CCANCELED"));
    assert_eq!(signature, sign_hmac_sha256_hex("secret", payload));
    assert_eq!(request.headers()["X-MBX-APIKEY"], "test-key");
    let stream = client
        .request(Method::POST, "listenKey", vec![], false, true)
        .unwrap();
    assert!(stream.url().query().unwrap().contains("timestamp="));
    assert!(!stream.url().query().unwrap().contains("signature="));
    let market = client
        .request(Method::GET, "market/exchangeInfo", vec![], false, false)
        .unwrap();
    assert!(!market.url().query().unwrap_or("").contains("timestamp"));
}

#[test]
fn stock_streams_preserve_case_and_supported_intervals() {
    assert_eq!(
        build_ws_url(&[StockStream::Kline("aapl".into(), KlineInterval::Month1)]).unwrap(),
        "wss://nbstream.binance.com/equity/ws/AAPL@kline_1M"
    );
    assert_eq!(
        StockStream::OrderReport("aBc123".into()).name().unwrap(),
        "aBc123@orderReport"
    );
    assert!(build_ws_url(&[]).is_err());
    assert!(StockStream::Quote("AAPL/price".into()).name().is_err());
}

#[tokio::test]
async fn http_empty_quote_and_error_response() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for (status, body) in [
            ("200 OK", ""),
            (
                "400 Bad Request",
                r#"{"code":-26004,"msg":"Unknown symbol"}"#,
            ),
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            assert!(String::from_utf8_lossy(&buffer[..n])
                .contains("/sapi/v1/equity/market/quote?symbol=AAPL"));
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let mut client = BinanceStocksClient::new("test", "secret");
    client.base_url = format!("http://{address}");
    client.http = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert!(client.quote("aapl").await.unwrap().is_none());
    assert!(client
        .quote("AAPL")
        .await
        .unwrap_err()
        .to_string()
        .contains("-26004"));
    server.join().unwrap();
}
