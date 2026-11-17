# crypto-exchange-client

Rust 写的加密货币交易所客户端：币安 U 本位合约（完整的下单、查单、撤单、账户、行情和推送）、OKX、Hyperliquid。
异步（tokio），金额用 `rust_decimal::Decimal`，不依赖任何数据库或配置中心，凭据由调用方传入。

Async Rust clients for Binance USDⓈ-M futures (orders incl. post-only & conditional/algo orders, account, market data,
market & user-data WebSocket streams), plus OKX and Hyperliquid.

## 引用

```toml
[dependencies]
crypto-exchange-client = { git = "https://github.com/1401687875/crypto-exchange-client", tag = "v0.3.0" }

# 只用币安（不引入 ethers / hyperliquid SDK）：
# crypto-exchange-client = { git = "...", tag = "v0.3.0", default-features = false, features = ["binance"] }
```

| feature | 内容 | 默认 |
| --- | --- | --- |
| `binance` | 币安：`binance::usdm`（U 本位合约，推荐）和旧版接口 | 是 |
| `okx` | OKX REST 和 WebSocket | 是 |
| `hyperliquid` | Hyperliquid 下单、精度、WebSocket（依赖 ethers） | 是 |

## 币安 U 本位合约（`binance::usdm`）

### 接口一览

| 类别 | 方法 | 币安接口 |
| --- | --- | --- |
| 下单 | `place_order`（市价 / 限价 / 只做 Maker）、`test_order`、`place_batch_orders`（≤5） | `POST /fapi/v1/order`、`/order/test`、`/batchOrders` |
| 改单 | `modify_order`（限价单改价/改量） | `PUT /fapi/v1/order` |
| 撤单 | `cancel_order`、`cancel_batch_orders`（≤10）、`cancel_all_orders`、`countdown_cancel_all`（死人开关） | `DELETE /fapi/v1/order`、`/batchOrders`、`/allOpenOrders`、`POST /countdownCancelAll` |
| 查单 | `query_order`、`query_open_order`、`open_orders`、`all_orders`、`user_trades`、`order_trades` | `GET /fapi/v1/order`、`/openOrder`、`/openOrders`、`/allOrders`、`/userTrades` |
| 条件单 | `place_algo_order`、`cancel_algo_order`、`cancel_all_algo_orders`、`query_algo_order`、`open_algo_orders`、`all_algo_orders` | `/fapi/v1/algoOrder` 等（止损/止盈已从普通下单接口迁出） |
| 账户 | `account`、`balances`、`positions`、`commission_rate`、`set_leverage`、`set_margin_type`、`dual_side_position`、`set_dual_side_position`、`modify_isolated_margin`、`income` | `/fapi/v3/account`、`/v3/balance`、`/v3/positionRisk`、`/v1/commissionRate` 等 |
| 行情 | `exchange_info`、`depth`、`book_ticker(s)`、`price(s)`、`ticker_24h(s)`、`klines`、`mark_price_klines`、`premium_index(es)`、`funding_rates`、`funding_info`、`open_interest`、`agg_trades`、`server_time`、`ping` | `/fapi/v1/...` |
| 行情推送 | `run_market_stream` / `spawn_market_stream`；流名称见 `streams::*` | `wss://fstream.binance.com/public`（盘口、深度）、`/market`（其他） |
| 用户数据流 | `run_user_stream` / `spawn_user_stream`（自动创建、续期、重建 listenKey，断线重连） | `wss://fstream.binance.com/private` |
| 精度 | `SymbolInfo::round_price` / `floor_qty` / `check_order` / `post_only_price` | 来自 `exchange_info` |

### 只做 Maker 挂单 → 查成交 → 撤单

```rust,no_run
use crypto_exchange_client::binance::usdm::*;

# async fn demo() -> crypto_exchange_client::Result<()> {
let client = UsdmClient::new(UsdmConfig::mainnet(), "api-key", "api-secret");
client.sync_time().await?;                       // 校准时钟，避免 -1021

let info = client.exchange_info().await?;        // 建议缓存
let rules = info.symbol("BTCUSDT").unwrap();
let book = client.book_ticker("BTCUSDT").await?;

let price = rules.post_only_price(&Side::Buy, book.bid_price, book.ask_price, 0);
let qty = rules.floor_qty("0.002".parse().unwrap());
let order = NewOrder::post_only("BTCUSDT", Side::Buy, qty, price).client_order_id("rebal-0001");

match client.place_order(&order).await {
    Ok(placed) => println!("挂单 {}", placed.order_id),
    Err(e) if e.api_code() == Some(-5022) => println!("会立即成交，被拒：换个价格再挂"),
    Err(e) if e.is_unknown_execution() => {
        // 超时 / 5xx：订单可能已经下了。用同一个客户端 ID 查，不要直接重发。
        let order = client.query_order("BTCUSDT", "rebal-0001").await?;
        println!("实际状态 {}", order.status);
    }
    Err(e) => return Err(e),
}

let now = client.query_order("BTCUSDT", "rebal-0001").await?;
if now.status.is_open() {
    let canceled = client.cancel_order("BTCUSDT", "rebal-0001").await?;
    println!("撤单，已成交 {}，剩余 {}", canceled.executed_qty, canceled.remaining_qty());
}
# Ok(())
# }
```

按盘口自动定价也可以：`NewOrder::post_only_matched("BTCUSDT", Side::Buy, qty, PriceMatch::Queue)` 挂在己方一档。

### 用户数据流（成交推送）

```rust,no_run
use crypto_exchange_client::binance::usdm::*;

# async fn demo(client: UsdmClient) -> crypto_exchange_client::Result<()> {
let (mut rx, _task) = spawn_user_stream(client.clone(), UserStreamOptions::default(), 1024)?;
while let Some(message) = rx.recv().await {
    match message {
        // 每次（重）连上都会先收到它：此时用 REST 查一次挂单和持仓，补上断线期间的变化。
        UserStreamMessage::Connected => { let _ = client.open_orders(None).await?; }
        UserStreamMessage::Event(UserDataEvent::OrderTradeUpdate(u)) if u.order.is_fill() => {
            println!("{} 成交 {} @ {}，maker={}", u.order.client_order_id, u.order.last_filled_qty, u.order.last_filled_price, u.order.is_maker);
        }
        _ => {}
    }
}
# Ok(())
# }
```

### 行情推送

```rust,no_run
use crypto_exchange_client::binance::usdm::*;

# async fn demo() -> crypto_exchange_client::Result<()> {
let config = UsdmConfig::mainnet();
// 盘口/深度（public）和其他行情（market）地址不同，要分两个连接。
let streams = vec![streams::book_ticker("BTCUSDT"), streams::book_ticker("ETHUSDT")];
let (mut rx, _task) = spawn_market_stream(config, streams, StreamOptions::default(), 4096)?;
while let Some(MarketStreamMessage::Event { event: MarketEvent::BookTicker(b), .. }) = rx.recv().await {
    println!("{} {} / {}", b.symbol, b.bid_price, b.ask_price);
}
# Ok(())
# }
```

### 注意事项

- **超时不重发**：下单/撤单返回 `is_unknown_execution()` 为真的错误时，交易所是否执行未知，先按客户端订单 ID 查询。
- **条件单走 algoOrder**：币安已把 `STOP_MARKET` / `TAKE_PROFIT_MARKET` 等迁到 `/fapi/v1/algoOrder`，用 `NewAlgoOrder`。
  `NewOrder` 遇到条件单类型会在本地直接报错。
- **WebSocket 地址**：币安 2026-04-23 起下线了旧的 `/ws`、`/stream`，本库用新地址 `/public`、`/market`、`/private`。
  测试网仍是 `wss://fstream.binancefuture.com`（不分路由）。
- **限频**：`used_weight_1m()`、`order_count()` 返回最近一次响应头里的用量；`is_rate_limited()` 判断 429/418。
- 程序宕机防挂单残留：定时调用 `countdown_cancel_all(symbol, 120_000)`。

### 冒烟测试

```bash
cargo run --example usdm_market                                                  # 公开行情，无需 Key
BINANCE_API_KEY=.. BINANCE_API_SECRET=.. cargo run --example usdm_user_stream    # 账户 + 用户数据流，只读
BINANCE_API_KEY=.. BINANCE_API_SECRET=.. cargo run --example usdm_order_lifecycle # 测试网：挂单→查→改→撤、条件单
```

## 旧版币安接口

`binance::BinanceOrderClient`（U 本位市价单、币本位限价/条件单、现货市价单、按客户端 ID 查单/撤单、账户流水）和
`binance::run_kline_client`（K 线 WebSocket）保留不变，新代码建议用 `binance::usdm`。

U 本位市价单 `create_usdt_market_order` 会核对成交：下单结果不明（超时、5xx）时只按客户端 ID 查询、不重发；
响应里没有成交均价时先等私有流的 `ORDER_TRADE_UPDATE`，再轮询订单和成交明细；时间戳被拒（-1021）时同步服务器时间后重签一次。
等私有流需要调用方先启动 `binance::BinanceUserDataClient::run_order_updates`（不启动也能用，只是改走轮询）。

### 币安股票（`binance::stocks`）

`BinanceStocksClient`：币安股票交易 SAPI（股票列表、精度、下单、查单、撤单、账户）和行情 WebSocket，价格和数量保持十进制字符串。

## OKX 与 Hyperliquid

### OKX

```rust
use crypto_exchange_client::okx::OkxApiClient;
use crypto_exchange_client::types::PlaceOrderRequest;

# async fn example() -> anyhow::Result<()> {
let client = OkxApiClient::new_with_credentials(
    "api-key".to_string(),
    "secret-key".to_string(),
    "passphrase".to_string(),
);

let response = client
    .place_order(PlaceOrderRequest {
        inst_id: "BTC-USDT-SWAP".to_string(),
        td_mode: "cross".to_string(),
        side: "buy".to_string(),
        ord_type: "market".to_string(),
        sz: "1".to_string(),
        pos_side: "long".to_string(),
        tgt_ccy: None,
        cl_ord_id: None,
        px: None,
    })
    .await?;
# Ok(())
# }
```

### Hyperliquid 精度

```rust
use crypto_exchange_client::hyperliquid::fetch_precision;

# async fn example() -> crypto_exchange_client::Result<()> {
let info = fetch_precision("ETH", Some("usdt"), false).await?;
println!(
    "symbol={} quantity_precision={} price_precision={}",
    info.symbol, info.quantity_precision, info.price_precision
);
# Ok(())
# }
```

### Hyperliquid K 线 WebSocket

```rust
use futures_util::StreamExt;
use crypto_exchange_client::hyperliquid::connect_candle_stream;
use tokio_tungstenite::tungstenite::protocol::Message;

# async fn example() -> anyhow::Result<()> {
let mut ws = connect_candle_stream("ETH", "5m", Some("usdt"), false).await?;
while let Some(msg) = ws.next().await {
    match msg? {
        Message::Text(text) => println!("{text}"),
        Message::Binary(data) => println!("{}", String::from_utf8_lossy(&data)),
        _ => {}
    }
}
# Ok(())
# }
```

### Hyperliquid 市价单

```rust
use bigdecimal::BigDecimal;
use crypto_exchange_client::hyperliquid::HyperliquidOrderClient;
use std::str::FromStr;

# async fn example() -> crypto_exchange_client::Result<()> {
let client = HyperliquidOrderClient::new(
    "0xYOUR_PRIVATE_KEY",
    false,
    None, // Some("0x...") when using vault address
)
.await?;

let quantity = BigDecimal::from_str("0.01").unwrap();
let order = client
    .place_market_order("ETH", true, &quantity, Some("usdt"), Some(0.01), None)
    .await?;
println!("filled amount={} avg_price={}", order.amount, order.sell_price);
# Ok(())
# }
```

## 开发

```bash
cargo test            # 单元测试 + 本地模拟服务器测试，不访问交易所
cargo clippy --all-targets
cargo fmt
```

## 许可

MIT
