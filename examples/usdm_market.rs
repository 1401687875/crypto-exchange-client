//! 公开行情冒烟测试（不需要 API Key，不下单）。
//!
//! ```bash
//! cargo run --example usdm_market            # 主网
//! BINANCE_TESTNET=1 cargo run --example usdm_market
//! ```
use crypto_exchange_client::binance::usdm::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = if std::env::var("BINANCE_TESTNET").is_ok() {
        UsdmConfig::testnet()
    } else {
        UsdmConfig::mainnet()
    };
    let client = UsdmClient::public(config.clone());

    client.ping().await?;
    println!("时钟偏差 {} ms", client.sync_time().await?);
    let info = client.exchange_info().await?;
    let btc = info.symbol("BTCUSDT").expect("没有 BTCUSDT");
    println!(
        "BTCUSDT 价格步长 {:?}，数量步长 {:?}，最小名义 {:?}；共 {} 个合约",
        btc.tick_size(),
        btc.step_size(),
        btc.min_notional(),
        info.symbols.len()
    );
    let book = client.book_ticker("BTCUSDT").await?;
    println!("盘口 买一 {} / 卖一 {}", book.bid_price, book.ask_price);
    let depth = client.depth("BTCUSDT", Some(5)).await?;
    println!("深度 {} 档 / {} 档", depth.bids.len(), depth.asks.len());
    let premium = client.premium_index("BTCUSDT").await?;
    println!(
        "标记价格 {}，资金费率 {}",
        premium.mark_price, premium.last_funding_rate
    );
    println!(
        "资金费调整过的合约 {} 个",
        client.funding_info().await?.len()
    );
    println!("本分钟已用权重 {}", client.used_weight_1m());

    // 盘口流（public）和标记价格流（market）走不同地址，分两个连接。
    for streams in [
        vec![streams::book_ticker("BTCUSDT")],
        vec![streams::mark_price("BTCUSDT", true)],
    ] {
        let mut count = 0;
        run_market_stream(&config, &streams, StreamOptions::default(), |message| {
            println!("{message:?}");
            if matches!(message, MarketStreamMessage::Event { .. }) {
                count += 1;
            }
            count < 3
        })
        .await?;
    }
    Ok(())
}
