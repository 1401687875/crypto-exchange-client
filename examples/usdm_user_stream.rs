//! 账户查询 + 用户数据流冒烟测试（只读，不下单）。只读权限的 API Key 即可。
//!
//! ```bash
//! BINANCE_API_KEY=.. BINANCE_API_SECRET=.. cargo run --example usdm_user_stream
//! # 加 BINANCE_TESTNET=1 用测试网；LISTEN_SECONDS=600 设定监听时长（默认 120 秒）
//! ```
use crypto_exchange_client::binance::usdm::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let config = if std::env::var("BINANCE_TESTNET").is_ok() {
        UsdmConfig::testnet()
    } else {
        UsdmConfig::mainnet()
    };
    let client = UsdmClient::new(
        config,
        std::env::var("BINANCE_API_KEY")?,
        std::env::var("BINANCE_API_SECRET")?,
    );
    client.sync_time().await?;

    let account = client.account().await?;
    println!(
        "总权益 {}，可用 {}",
        account.total_margin_balance, account.available_balance
    );
    for balance in client
        .balances()
        .await?
        .iter()
        .filter(|b| !b.balance.is_zero())
    {
        println!("余额 {} {}", balance.asset, balance.balance);
    }
    for position in client
        .positions(None)
        .await?
        .iter()
        .filter(|p| !p.position_amt.is_zero())
    {
        println!(
            "持仓 {} {:?} {} @ {}",
            position.symbol, position.position_side, position.position_amt, position.entry_price
        );
    }
    println!(
        "挂单 {} 个，条件单 {} 个",
        client.open_orders(None).await?.len(),
        client.open_algo_orders(None).await?.len()
    );
    println!("双向持仓 {}", client.dual_side_position().await?);
    let fee = client.commission_rate("BTCUSDT").await?;
    println!(
        "BTCUSDT 费率 maker {} / taker {}",
        fee.maker_commission_rate, fee.taker_commission_rate
    );

    let seconds: u64 = std::env::var("LISTEN_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    println!("监听用户数据流 {seconds} 秒（可以在网页上下单/撤单观察推送）");
    let (mut receiver, handle) = spawn_user_stream(client, UserStreamOptions::default(), 1024)?;
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            message = receiver.recv() => match message {
                Some(message) => println!("{message:?}"),
                None => break,
            },
            _ = &mut deadline => break,
        }
    }
    drop(receiver);
    let _ = handle.await;
    Ok(())
}
