//! 下单全流程冒烟测试：挂一个**不会成交**的只做 Maker 买单（买一价下方 3%）→ 查单 → 改价 → 撤单；
//! 再下一个远离市价的条件单 → 查询 → 撤销。最后核对没有残留挂单。
//!
//! 默认用测试网。要在主网跑必须同时设置 `BINANCE_MAINNET=1`（会占用约 1 笔最小名义金额的保证金，几秒后撤掉）。
//!
//! ```bash
//! BINANCE_API_KEY=.. BINANCE_API_SECRET=.. cargo run --example usdm_order_lifecycle
//! SYMBOL=ETHUSDT ...   # 换合约（默认 BTCUSDT）
//! ```
use crypto_exchange_client::binance::usdm::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let mainnet = std::env::var("BINANCE_MAINNET").ok().as_deref() == Some("1");
    let config = if mainnet {
        UsdmConfig::mainnet()
    } else {
        UsdmConfig::testnet()
    };
    println!("环境：{}", if mainnet { "主网" } else { "测试网" });
    let client = UsdmClient::new(
        config,
        std::env::var("BINANCE_API_KEY")?,
        std::env::var("BINANCE_API_SECRET")?,
    );
    let symbol = std::env::var("SYMBOL").unwrap_or_else(|_| "BTCUSDT".into());
    client.sync_time().await?;

    let info = client.exchange_info().await?;
    let rules = info.symbol(&symbol).expect("合约不存在").clone();
    let dual = client.dual_side_position().await?;
    let book = client.book_ticker(&symbol).await?;
    let price = rules.round_price(book.bid_price * Decimal::new(97, 2), Rounding::Down);
    let min_notional = rules.min_notional().unwrap_or(Decimal::new(100, 0));
    let step = rules.step_size().unwrap_or(Decimal::new(1, 3));
    let mut qty = rules.floor_qty(min_notional * Decimal::new(11, 1) / price);
    if qty * price < min_notional {
        qty += step;
    }
    rules
        .check_order(qty, price, false)
        .map_err(anyhow::Error::msg)?;
    let tag = chrono::Utc::now().timestamp_millis();
    let client_id = format!("smoke-{tag}");

    let mut order = NewOrder::post_only(&symbol, Side::Buy, qty, price).client_order_id(&client_id);
    if dual {
        order = order.position_side(PositionSide::Long);
    }
    client.test_order(&order).await?;
    println!("1. 测试下单通过");

    let placed = client.place_order(&order).await?;
    println!(
        "2. 已挂单 {} {} @ {}，状态 {}",
        placed.order_id, placed.orig_qty, placed.price, placed.status
    );

    let queried = client.query_order(&symbol, client_id.as_str()).await?;
    println!(
        "3. 按客户端 ID 查到：状态 {}，已成交 {}",
        queried.status, queried.executed_qty
    );

    let new_price = rules.round_price(price * Decimal::new(999, 3), Rounding::Down);
    let modified = client
        .modify_order(&ModifyOrder::new(
            &symbol,
            OrderId::Exchange(placed.order_id),
            Side::Buy,
            qty,
            new_price,
        ))
        .await?;
    println!("4. 改价到 {}，状态 {}", modified.price, modified.status);

    let open = client.open_orders(Some(&symbol)).await?;
    println!("5. 当前挂单 {} 个", open.len());

    let canceled = client.cancel_order(&symbol, placed.order_id).await?;
    println!("6. 撤单后状态 {}", canceled.status);

    let mut stop = NewAlgoOrder::stop_market(
        &symbol,
        Side::Sell,
        rules.round_price(book.bid_price * Decimal::new(50, 2), Rounding::Down),
    )
    .quantity(qty)
    .working_type(WorkingType::MarkPrice)
    .client_algo_id(format!("smoke-stop-{tag}"));
    if dual {
        stop = stop.position_side(PositionSide::Long);
    } else {
        stop = stop.reduce_only(true);
    }
    match client.place_algo_order(&stop).await {
        Ok(algo) => {
            println!("7. 条件单 {} 状态 {:?}", algo.algo_id, algo.algo_status);
            let queried = client
                .query_algo_order(AlgoId::Exchange(algo.algo_id))
                .await?;
            println!("8. 查到条件单 触发价 {:?}", queried.trigger_price);
            client
                .cancel_algo_order(AlgoId::Exchange(algo.algo_id))
                .await?;
            println!("9. 条件单已撤");
        }
        // 没有仓位时只减仓的条件单可能被拒，这不影响前面的挂单流程。
        Err(error) => println!("7. 条件单被拒（没有仓位时可能如此）：{error}"),
    }

    let left = client.open_orders(Some(&symbol)).await?;
    println!(
        "10. 剩余挂单 {} 个；最近权重 {}",
        left.len(),
        client.used_weight_1m()
    );
    anyhow::ensure!(
        left.iter().all(|o| o.client_order_id != client_id),
        "测试挂单没有撤掉，请手动处理"
    );
    Ok(())
}
