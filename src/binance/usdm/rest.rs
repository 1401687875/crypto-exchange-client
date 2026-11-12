//! U 本位合约 REST 接口：行情、下单/改单/撤单/查单、条件单、账户、listenKey。

use super::client::{Params, UsdmClient};
use super::exchange_info::ExchangeInfo;
use super::models::*;
use crate::error::{ExchangeClientError, Result};
use reqwest::Method;
use rust_decimal::Decimal;

/// 时间区间和条数（历史查询用）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Range {
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub limit: Option<u32>,
}

impl Range {
    pub fn new(start_time: Option<i64>, end_time: Option<i64>, limit: Option<u32>) -> Self {
        Self {
            start_time,
            end_time,
            limit,
        }
    }

    fn push_to(&self, params: &mut Params) {
        params
            .opt("startTime", self.start_time)
            .opt("endTime", self.end_time)
            .opt("limit", self.limit);
    }
}

// ---------------------------------------------------------------------------
// 行情（不需要 API Key）
// ---------------------------------------------------------------------------
impl UsdmClient {
    /// 连通性测试。
    pub async fn ping(&self) -> Result<()> {
        let _: serde_json::Value = self.get_public("/fapi/v1/ping", Params::new()).await?;
        Ok(())
    }

    /// 交易所服务器时间（毫秒）。
    pub async fn server_time(&self) -> Result<i64> {
        let value: serde_json::Value = self.get_public("/fapi/v1/time", Params::new()).await?;
        value["serverTime"]
            .as_i64()
            .ok_or_else(|| ExchangeClientError::ResponseParse("serverTime 缺失".into()))
    }

    /// 全部合约的交易规则（价格/数量步长、最小名义金额等）。权重 1，建议缓存。
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        self.get_public("/fapi/v1/exchangeInfo", Params::new())
            .await
    }

    /// 深度快照。`limit` 可选 5/10/20/50/100/500/1000。
    pub async fn depth(&self, symbol: &str, limit: Option<u32>) -> Result<OrderBook> {
        self.get_public(
            "/fapi/v1/depth",
            Params::new()
                .with("symbol", symbol)
                .with_opt("limit", limit),
        )
        .await
    }

    /// 单个合约的最优买卖价。
    pub async fn book_ticker(&self, symbol: &str) -> Result<BookTicker> {
        self.get_public(
            "/fapi/v1/ticker/bookTicker",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 全部合约的最优买卖价。
    pub async fn book_tickers(&self) -> Result<Vec<BookTicker>> {
        self.get_public("/fapi/v1/ticker/bookTicker", Params::new())
            .await
    }

    /// 单个合约的最新价。
    pub async fn price(&self, symbol: &str) -> Result<PriceTicker> {
        self.get_public(
            "/fapi/v2/ticker/price",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 全部合约的最新价。
    pub async fn prices(&self) -> Result<Vec<PriceTicker>> {
        self.get_public("/fapi/v2/ticker/price", Params::new())
            .await
    }

    /// 单个合约的 24 小时行情。
    pub async fn ticker_24h(&self, symbol: &str) -> Result<Ticker24h> {
        self.get_public("/fapi/v1/ticker/24hr", Params::new().with("symbol", symbol))
            .await
    }

    /// 全部合约的 24 小时行情（权重 40）。
    pub async fn tickers_24h(&self) -> Result<Vec<Ticker24h>> {
        self.get_public("/fapi/v1/ticker/24hr", Params::new()).await
    }

    /// K 线，`limit` 默认 500、最多 1500。
    pub async fn klines(
        &self,
        symbol: &str,
        interval: Interval,
        range: Range,
    ) -> Result<Vec<Kline>> {
        let mut params = Params::new()
            .with("symbol", symbol)
            .with("interval", interval);
        range.push_to(&mut params);
        self.get_public("/fapi/v1/klines", params).await
    }

    /// 标记价格 K 线（返回结构同 K 线，成交量类字段为 0）。
    pub async fn mark_price_klines(
        &self,
        symbol: &str,
        interval: Interval,
        range: Range,
    ) -> Result<Vec<Kline>> {
        let mut params = Params::new()
            .with("symbol", symbol)
            .with("interval", interval);
        range.push_to(&mut params);
        let rows: Vec<serde_json::Value> =
            self.get_public("/fapi/v1/markPriceKlines", params).await?;
        rows.into_iter().map(pad_kline).collect()
    }

    /// 单个合约的标记价格、指数价格和当期资金费率。
    pub async fn premium_index(&self, symbol: &str) -> Result<PremiumIndex> {
        self.get_public(
            "/fapi/v1/premiumIndex",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 全部合约的标记价格和当期资金费率。
    pub async fn premium_indexes(&self) -> Result<Vec<PremiumIndex>> {
        self.get_public("/fapi/v1/premiumIndex", Params::new())
            .await
    }

    /// 资金费历史，`limit` 默认 100、最多 1000。
    pub async fn funding_rates(
        &self,
        symbol: Option<&str>,
        range: Range,
    ) -> Result<Vec<FundingRate>> {
        let mut params = Params::new().with_opt("symbol", symbol);
        range.push_to(&mut params);
        self.get_public("/fapi/v1/fundingRate", params).await
    }

    /// 被调整过资金费上下限或结算间隔的合约。
    pub async fn funding_info(&self) -> Result<Vec<FundingInfo>> {
        self.get_public("/fapi/v1/fundingInfo", Params::new()).await
    }

    /// 当前持仓量。
    pub async fn open_interest(&self, symbol: &str) -> Result<OpenInterest> {
        self.get_public(
            "/fapi/v1/openInterest",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 归集成交，`limit` 默认 500、最多 1000。
    pub async fn agg_trades(
        &self,
        symbol: &str,
        from_id: Option<i64>,
        range: Range,
    ) -> Result<Vec<AggTrade>> {
        let mut params = Params::new()
            .with("symbol", symbol)
            .with_opt("fromId", from_id);
        range.push_to(&mut params);
        self.get_public("/fapi/v1/aggTrades", params).await
    }
}

fn pad_kline(row: serde_json::Value) -> Result<Kline> {
    // 标记价格 K 线的成交量类字段是 "0" 或缺失，补齐后复用 K 线解析。
    let mut items = row.as_array().cloned().unwrap_or_default();
    while items.len() < 12 {
        items.push(serde_json::Value::String("0".into()));
    }
    for index in [6, 8] {
        if let Some(text) = items[index].as_str().map(str::to_string) {
            items[index] = serde_json::Value::from(text.parse::<i64>().unwrap_or(0));
        }
    }
    Ok(serde_json::from_value(serde_json::Value::Array(items))?)
}

// ---------------------------------------------------------------------------
// 下单、改单、撤单、查单
// ---------------------------------------------------------------------------
impl UsdmClient {
    /// 下单（市价 / 限价 / 只做 Maker）。
    ///
    /// 返回错误且 [`ExchangeClientError::is_unknown_execution`] 为真时，订单是否已下**无法确定**：
    /// 用同一个客户端订单 ID 调 [`UsdmClient::query_order`] 确认，不要直接重发。
    /// 只做 Maker 单如果会立即成交，交易所返回错误码 `-5022`。
    pub async fn place_order(&self, order: &NewOrder) -> Result<Order> {
        order.validate()?;
        self.request_signed(Method::POST, "/fapi/v1/order", order.to_params())
            .await
    }

    /// 测试下单：交易所校验参数但不进撮合。
    pub async fn test_order(&self, order: &NewOrder) -> Result<()> {
        order.validate()?;
        let _: serde_json::Value = self
            .request_signed(Method::POST, "/fapi/v1/order/test", order.to_params())
            .await?;
        Ok(())
    }

    /// 批量下单，一次最多 5 个；每个订单单独成功或失败。
    pub async fn place_batch_orders(&self, orders: &[NewOrder]) -> Result<Vec<BatchItem>> {
        if orders.is_empty() || orders.len() > 5 {
            return Err(ExchangeClientError::Validation(
                "批量下单一次 1–5 个".into(),
            ));
        }
        for order in orders {
            order.validate()?;
        }
        let list: Vec<serde_json::Value> = orders.iter().map(NewOrder::to_json).collect();
        let params = Params::new().with("batchOrders", serde_json::to_string(&list)?);
        self.request_signed(Method::POST, "/fapi/v1/batchOrders", params)
            .await
    }

    /// 改单（只支持限价单，改价格/数量）。
    pub async fn modify_order(&self, modify: &ModifyOrder) -> Result<Order> {
        self.request_signed(Method::PUT, "/fapi/v1/order", modify.to_params())
            .await
    }

    /// 撤单。成功返回撤单后的订单状态（已成交部分见 `executed_qty`）。
    pub async fn cancel_order(&self, symbol: &str, id: impl Into<OrderId>) -> Result<Order> {
        let mut params = Params::new().with("symbol", symbol);
        id.into().push_to(&mut params);
        self.request_signed(Method::DELETE, "/fapi/v1/order", params)
            .await
    }

    /// 批量撤单，一次最多 10 个。
    pub async fn cancel_batch_orders(
        &self,
        symbol: &str,
        ids: &[OrderId],
    ) -> Result<Vec<BatchItem>> {
        if ids.is_empty() || ids.len() > 10 {
            return Err(ExchangeClientError::Validation(
                "批量撤单一次 1–10 个".into(),
            ));
        }
        let mut params = Params::new().with("symbol", symbol);
        let exchange: Vec<i64> = ids
            .iter()
            .filter_map(|id| match id {
                OrderId::Exchange(id) => Some(*id),
                _ => None,
            })
            .collect();
        let client: Vec<&str> = ids
            .iter()
            .filter_map(|id| match id {
                OrderId::Client(id) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        if !exchange.is_empty() && !client.is_empty() {
            return Err(ExchangeClientError::Validation(
                "批量撤单不能混用交易所订单 ID 和客户端订单 ID".into(),
            ));
        }
        if !exchange.is_empty() {
            params.push("orderIdList", serde_json::to_string(&exchange)?);
        } else {
            params.push("origClientOrderIdList", serde_json::to_string(&client)?);
        }
        self.request_signed(Method::DELETE, "/fapi/v1/batchOrders", params)
            .await
    }

    /// 撤掉某个合约的全部挂单（不含条件单，条件单用 [`UsdmClient::cancel_all_algo_orders`]）。
    pub async fn cancel_all_orders(&self, symbol: &str) -> Result<CodeMsg> {
        self.request_signed(
            Method::DELETE,
            "/fapi/v1/allOpenOrders",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 倒计时撤单（死人开关）：`countdown_ms` 毫秒内没有再次调用，就撤掉这个合约的全部挂单；传 0 取消倒计时。
    /// 程序宕机时防止挂单留在盘口。建议每 30 秒用 120000 续一次。
    pub async fn countdown_cancel_all(
        &self,
        symbol: &str,
        countdown_ms: u64,
    ) -> Result<CountdownCancel> {
        let params = Params::new()
            .with("symbol", symbol)
            .with("countdownTime", countdown_ms);
        self.request_signed(Method::POST, "/fapi/v1/countdownCancelAll", params)
            .await
    }

    /// 查询订单（任何状态）。交易所只保留最近一段时间的已撤单/过期单。
    pub async fn query_order(&self, symbol: &str, id: impl Into<OrderId>) -> Result<Order> {
        let mut params = Params::new().with("symbol", symbol);
        id.into().push_to(&mut params);
        self.request_signed(Method::GET, "/fapi/v1/order", params)
            .await
    }

    /// 查询一个当前挂单；订单已不在盘口时返回错误 `-2013`。
    pub async fn query_open_order(&self, symbol: &str, id: impl Into<OrderId>) -> Result<Order> {
        let mut params = Params::new().with("symbol", symbol);
        id.into().push_to(&mut params);
        self.request_signed(Method::GET, "/fapi/v1/openOrder", params)
            .await
    }

    /// 当前全部挂单；`symbol` 为空时查所有合约（权重 40）。
    pub async fn open_orders(&self, symbol: Option<&str>) -> Result<Vec<Order>> {
        self.request_signed(
            Method::GET,
            "/fapi/v1/openOrders",
            Params::new().with_opt("symbol", symbol),
        )
        .await
    }

    /// 历史订单（含已成交、已撤销），`limit` 默认 500、最多 1000；时间区间最长 7 天。
    pub async fn all_orders(
        &self,
        symbol: &str,
        order_id: Option<i64>,
        range: Range,
    ) -> Result<Vec<Order>> {
        let mut params = Params::new()
            .with("symbol", symbol)
            .with_opt("orderId", order_id);
        range.push_to(&mut params);
        self.request_signed(Method::GET, "/fapi/v1/allOrders", params)
            .await
    }

    /// 成交明细（含是否 Maker、手续费），`limit` 默认 500、最多 1000；时间区间最长 7 天。
    pub async fn user_trades(&self, symbol: &str, range: Range) -> Result<Vec<UserTrade>> {
        let mut params = Params::new().with("symbol", symbol);
        range.push_to(&mut params);
        self.request_signed(Method::GET, "/fapi/v1/userTrades", params)
            .await
    }

    /// 某个订单的全部成交明细。
    pub async fn order_trades(&self, symbol: &str, order_id: i64) -> Result<Vec<UserTrade>> {
        let params = Params::new()
            .with("symbol", symbol)
            .with("orderId", order_id);
        self.request_signed(Method::GET, "/fapi/v1/userTrades", params)
            .await
    }
}

// ---------------------------------------------------------------------------
// 条件单（algoOrder）
// ---------------------------------------------------------------------------
impl UsdmClient {
    /// 下条件单（止损 / 止盈 / 跟踪止损）。
    pub async fn place_algo_order(&self, order: &NewAlgoOrder) -> Result<AlgoOrder> {
        order.validate()?;
        self.request_signed(Method::POST, "/fapi/v1/algoOrder", order.to_params())
            .await
    }

    /// 撤条件单，`id` 为交易所 algoId 或客户端 clientAlgoId。
    pub async fn cancel_algo_order(&self, id: AlgoId) -> Result<serde_json::Value> {
        self.request_signed(Method::DELETE, "/fapi/v1/algoOrder", id.to_params())
            .await
    }

    /// 撤掉某个合约的全部条件单。
    pub async fn cancel_all_algo_orders(&self, symbol: &str) -> Result<CodeMsg> {
        self.request_signed(
            Method::DELETE,
            "/fapi/v1/algoOpenOrders",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 查询条件单。
    pub async fn query_algo_order(&self, id: AlgoId) -> Result<AlgoOrder> {
        self.request_signed(Method::GET, "/fapi/v1/algoOrder", id.to_params())
            .await
    }

    /// 当前未触发的条件单；`symbol` 为空时查所有合约。
    pub async fn open_algo_orders(&self, symbol: Option<&str>) -> Result<Vec<AlgoOrder>> {
        let value: serde_json::Value = self
            .request_signed(
                Method::GET,
                "/fapi/v1/openAlgoOrders",
                Params::new().with_opt("symbol", symbol),
            )
            .await?;
        parse_algo_list(value)
    }

    /// 历史条件单。
    pub async fn all_algo_orders(&self, symbol: &str, range: Range) -> Result<Vec<AlgoOrder>> {
        let mut params = Params::new().with("symbol", symbol);
        range.push_to(&mut params);
        let value: serde_json::Value = self
            .request_signed(Method::GET, "/fapi/v1/allAlgoOrders", params)
            .await?;
        parse_algo_list(value)
    }
}

/// 条件单标识。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlgoId {
    Exchange(i64),
    Client(String),
}

impl AlgoId {
    fn to_params(&self) -> Params {
        match self {
            AlgoId::Exchange(id) => Params::new().with("algoId", id),
            AlgoId::Client(id) => Params::new().with("clientAlgoId", id),
        }
    }
}

fn parse_algo_list(value: serde_json::Value) -> Result<Vec<AlgoOrder>> {
    // 列表可能直接是数组，也可能包在 {"orders": [...]} 里。
    let list = match value {
        serde_json::Value::Array(_) => value,
        serde_json::Value::Object(mut map) => map
            .remove("orders")
            .or_else(|| map.remove("data"))
            .unwrap_or(serde_json::Value::Array(vec![])),
        _ => serde_json::Value::Array(vec![]),
    };
    Ok(serde_json::from_value(list)?)
}

// ---------------------------------------------------------------------------
// 账户
// ---------------------------------------------------------------------------
impl UsdmClient {
    /// 账户信息（总权益、保证金、各资产、有仓位的合约）。
    pub async fn account(&self) -> Result<AccountInfo> {
        self.request_signed(Method::GET, "/fapi/v3/account", Params::new())
            .await
    }

    /// 各资产余额。
    pub async fn balances(&self) -> Result<Vec<Balance>> {
        self.request_signed(Method::GET, "/fapi/v3/balance", Params::new())
            .await
    }

    /// 持仓；`symbol` 为空时返回所有有仓位或挂单的合约。
    pub async fn positions(&self, symbol: Option<&str>) -> Result<Vec<Position>> {
        self.request_signed(
            Method::GET,
            "/fapi/v3/positionRisk",
            Params::new().with_opt("symbol", symbol),
        )
        .await
    }

    /// 手续费率（maker / taker）。
    pub async fn commission_rate(&self, symbol: &str) -> Result<CommissionRate> {
        self.request_signed(
            Method::GET,
            "/fapi/v1/commissionRate",
            Params::new().with("symbol", symbol),
        )
        .await
    }

    /// 调整杠杆倍数（1–125）。
    pub async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<LeverageChange> {
        let params = Params::new()
            .with("symbol", symbol)
            .with("leverage", leverage);
        self.request_signed(Method::POST, "/fapi/v1/leverage", params)
            .await
    }

    /// 设置逐仓 / 全仓。已经是目标模式时交易所返回 `-4046`，这里视为成功。
    pub async fn set_margin_type(&self, symbol: &str, margin_type: MarginType) -> Result<()> {
        let params = Params::new()
            .with("symbol", symbol)
            .with("marginType", margin_type);
        match self
            .request_signed::<CodeMsg>(Method::POST, "/fapi/v1/marginType", params)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.api_code() == Some(-4046) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// 是否双向持仓模式。
    pub async fn dual_side_position(&self) -> Result<bool> {
        let value: serde_json::Value = self
            .request_signed(Method::GET, "/fapi/v1/positionSide/dual", Params::new())
            .await?;
        value["dualSidePosition"]
            .as_bool()
            .ok_or_else(|| ExchangeClientError::ResponseParse("dualSidePosition 缺失".into()))
    }

    /// 切换单向 / 双向持仓模式（账户有仓位或挂单时不能切换）。已经是目标模式（`-4059`）视为成功。
    pub async fn set_dual_side_position(&self, dual: bool) -> Result<()> {
        let params = Params::new().with("dualSidePosition", dual);
        match self
            .request_signed::<CodeMsg>(Method::POST, "/fapi/v1/positionSide/dual", params)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.api_code() == Some(-4059) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// 调整逐仓保证金：`add = true` 增加，`false` 减少。
    pub async fn modify_isolated_margin(
        &self,
        symbol: &str,
        position_side: Option<PositionSide>,
        amount: Decimal,
        add: bool,
    ) -> Result<serde_json::Value> {
        let params = Params::new()
            .with("symbol", symbol)
            .with_opt("positionSide", position_side)
            .with("amount", amount.normalize())
            .with("type", if add { 1 } else { 2 });
        self.request_signed(Method::POST, "/fapi/v1/positionMargin", params)
            .await
    }

    /// 资金流水（资金费、手续费、已实现盈亏等），`income_type` 例如 `FUNDING_FEE`、`COMMISSION`、`REALIZED_PNL`。
    /// `limit` 默认 100、最多 1000。
    pub async fn income(
        &self,
        symbol: Option<&str>,
        income_type: Option<&str>,
        range: Range,
    ) -> Result<Vec<Income>> {
        let mut params = Params::new()
            .with_opt("symbol", symbol)
            .with_opt("incomeType", income_type);
        range.push_to(&mut params);
        self.request_signed(Method::GET, "/fapi/v1/income", params)
            .await
    }
}

// ---------------------------------------------------------------------------
// 用户数据流 listenKey
// ---------------------------------------------------------------------------
impl UsdmClient {
    /// 创建（或取回已有的）listenKey，有效期 60 分钟。
    pub async fn start_user_stream(&self) -> Result<String> {
        let key: ListenKey = self
            .request_with_key(Method::POST, "/fapi/v1/listenKey", Params::new())
            .await?;
        Ok(key.listen_key)
    }

    /// 续期 listenKey（延长到 60 分钟后）。listenKey 已失效时交易所返回 `-1125`。
    pub async fn keepalive_user_stream(&self) -> Result<()> {
        let _: serde_json::Value = self
            .request_with_key(Method::PUT, "/fapi/v1/listenKey", Params::new())
            .await?;
        Ok(())
    }

    /// 关闭 listenKey。
    pub async fn close_user_stream(&self) -> Result<()> {
        let _: serde_json::Value = self
            .request_with_key(Method::DELETE, "/fapi/v1/listenKey", Params::new())
            .await?;
        Ok(())
    }
}
