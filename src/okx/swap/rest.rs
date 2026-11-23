//! OKX 永续合约 REST 接口：交易规则、行情、下单/撤单/查单、成交明细、持仓、余额、账户设置、条件单、账单流水。

use super::client::{Query, SwapClient};
use super::models::*;
use crate::error::{ExchangeClientError, Result};
use rust_decimal::Decimal;
use serde_json::json;

/// 分页查询最多翻几页（防止接口异常时死循环）。
const MAX_PAGES: usize = 50;

fn first<T>(items: Vec<T>, path: &str) -> Result<T> {
    items
        .into_iter()
        .next()
        .ok_or_else(|| ExchangeClientError::ResponseParse(format!("{path}: data 为空")))
}

impl SwapClient {
    // -----------------------------------------------------------------------
    // 公开接口
    // -----------------------------------------------------------------------

    /// 全部永续合约的交易规则（含币本位，U 本位用 [`Instrument::is_usdt_linear`] 过滤）。
    pub async fn instruments(&self) -> Result<Vec<Instrument>> {
        self.get_public(
            "/api/v5/public/instruments",
            Query::new().with("instType", "SWAP"),
        )
        .await
    }

    /// 全部永续合约的行情（买一卖一、最新价）。
    pub async fn tickers(&self) -> Result<Vec<Ticker>> {
        self.get_public(
            "/api/v5/market/tickers",
            Query::new().with("instType", "SWAP"),
        )
        .await
    }

    /// 单个合约的行情。
    pub async fn ticker(&self, inst_id: &str) -> Result<Ticker> {
        let path = "/api/v5/market/ticker";
        first(
            self.get_public(path, Query::new().with("instId", inst_id))
                .await?,
            path,
        )
    }

    // -----------------------------------------------------------------------
    // 交易
    // -----------------------------------------------------------------------

    /// 下单。成功时返回回执（含交易所订单号）；`sCode != 0` 时返回 [`ExchangeClientError::Api`]，
    /// 错误码是 `sCode`（如 51008 保证金不足）。
    pub async fn place_order(&self, order: &NewOrder) -> Result<OrderAck> {
        let path = "/api/v5/trade/order";
        let ack: OrderAck = first(self.post_signed(path, &json!(order)).await?, path)?;
        check_ack(path, &ack.s_code, &ack.s_msg)?;
        Ok(ack)
    }

    /// 查单。订单不存在时返回错误码 51603。
    pub async fn order(&self, inst_id: &str, order: &OrderRef) -> Result<Order> {
        let path = "/api/v5/trade/order";
        first(
            self.get_signed(
                path,
                Query::new()
                    .with("instId", inst_id)
                    .with(order.key(), order.value()),
            )
            .await?,
            path,
        )
    }

    /// 撤单。订单已结束时返回 51400 / 51401 / 51402 等错误码。
    pub async fn cancel_order(&self, inst_id: &str, order: &OrderRef) -> Result<OrderAck> {
        let path = "/api/v5/trade/cancel-order";
        let mut body = json!({ "instId": inst_id });
        body[order.key()] = json!(order.value());
        let ack: OrderAck = first(self.post_signed(path, &body).await?, path)?;
        check_ack(path, &ack.s_code, &ack.s_msg)?;
        Ok(ack)
    }

    /// 全部未完成的永续合约委托（自动翻页）。
    pub async fn pending_orders(&self) -> Result<Vec<Order>> {
        let mut out: Vec<Order> = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page: Vec<Order> = self
                .get_signed(
                    "/api/v5/trade/orders-pending",
                    Query::new()
                        .with("instType", "SWAP")
                        .with_opt("after", after.clone())
                        .with("limit", 100),
                )
                .await?;
            let full = page.len() >= 100;
            after = page.last().map(|o| o.ord_id.clone());
            out.extend(page);
            if !full {
                break;
            }
        }
        Ok(out)
    }

    /// 一张订单的成交明细：先查近 3 天（`fills`），查不到再查近 3 个月（`fills-history`）。
    pub async fn order_fills(&self, inst_id: &str, ord_id: &str) -> Result<Vec<Fill>> {
        let query = || {
            Query::new()
                .with("instType", "SWAP")
                .with("instId", inst_id)
                .with("ordId", ord_id)
                .with("limit", 100)
        };
        let recent: Vec<Fill> = self.get_signed("/api/v5/trade/fills", query()).await?;
        if !recent.is_empty() {
            return Ok(recent);
        }
        self.get_signed("/api/v5/trade/fills-history", query())
            .await
    }

    // -----------------------------------------------------------------------
    // 账户
    // -----------------------------------------------------------------------

    /// 永续合约持仓（只返回有仓位的）。
    pub async fn positions(&self) -> Result<Vec<Position>> {
        self.get_signed(
            "/api/v5/account/positions",
            Query::new().with("instType", "SWAP"),
        )
        .await
    }

    /// 账户余额；`ccy` 只查这些币种（如 `Some("USDT")`）。
    pub async fn balance(&self, ccy: Option<&str>) -> Result<Balance> {
        let path = "/api/v5/account/balance";
        first(
            self.get_signed(path, Query::new().with_opt("ccy", ccy))
                .await?,
            path,
        )
    }

    /// 账户配置（持仓模式、账户模式）。
    pub async fn account_config(&self) -> Result<AccountConfig> {
        let path = "/api/v5/account/config";
        first(self.get_signed(path, Query::new()).await?, path)
    }

    /// 设置持仓模式：`long_short_mode`（双向）/ `net_mode`（单向）。有仓位或挂单时 OKX 会拒绝。
    pub async fn set_position_mode(&self, pos_mode: &str) -> Result<()> {
        let _: Vec<serde_json::Value> = self
            .post_signed(
                "/api/v5/account/set-position-mode",
                &json!({ "posMode": pos_mode }),
            )
            .await?;
        Ok(())
    }

    /// 设置某个合约的杠杆。全仓（`cross`）下多空共用一个杠杆，不用填持仓方向。
    pub async fn set_leverage(
        &self,
        inst_id: &str,
        lever: u32,
        mgn_mode: &str,
    ) -> Result<LeverageInfo> {
        let path = "/api/v5/account/set-leverage";
        first(
            self.post_signed(
                path,
                &json!({ "instId": inst_id, "lever": lever.to_string(), "mgnMode": mgn_mode }),
            )
            .await?,
            path,
        )
    }

    /// 账单流水，从新到旧。`archive = false` 查近 7 天（`bills`），`true` 查近 3 个月（`bills-archive`）。
    /// `bill_type` 如 `8`（资金费）；`begin_ms` 只要这个时间（含）之后的；`after` 是上一页最后一条的 `billId`（翻页用）。
    pub async fn bills(
        &self,
        archive: bool,
        bill_type: Option<&str>,
        begin_ms: Option<i64>,
        after: Option<&str>,
    ) -> Result<Vec<Bill>> {
        let path = if archive {
            "/api/v5/account/bills-archive"
        } else {
            "/api/v5/account/bills"
        };
        self.get_signed(
            path,
            Query::new()
                .with("instType", "SWAP")
                .with_opt("type", bill_type)
                .with_opt("begin", begin_ms)
                .with_opt("after", after)
                .with("limit", 100),
        )
        .await
    }

    /// `begin_ms`（含）之后的全部账单流水（自动翻页），从新到旧。开始时间早于 7 天前时查 3 个月的归档。
    pub async fn bills_since(&self, bill_type: Option<&str>, begin_ms: i64) -> Result<Vec<Bill>> {
        let week_ago = chrono::Utc::now().timestamp_millis() - 7 * 24 * 3600 * 1000 + 60_000;
        let archive = begin_ms < week_ago;
        let mut out: Vec<Bill> = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = self
                .bills(archive, bill_type, Some(begin_ms), after.as_deref())
                .await?;
            let full = page.len() >= 100;
            after = page.last().map(|b| b.bill_id.clone());
            out.extend(page.into_iter().filter(|b| b.ts >= begin_ms));
            if !full {
                break;
            }
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // 条件单
    // -----------------------------------------------------------------------

    /// 挂止损条件单。
    pub async fn place_stop(&self, order: &NewStopOrder) -> Result<AlgoAck> {
        let path = "/api/v5/trade/order-algo";
        let ack: AlgoAck = first(self.post_signed(path, &json!(order)).await?, path)?;
        check_ack(path, &ack.s_code, &ack.s_msg)?;
        Ok(ack)
    }

    /// 全部未触发的永续合约条件单（`ordType = conditional`，自动翻页）。
    pub async fn pending_stops(&self) -> Result<Vec<AlgoOrder>> {
        let mut out: Vec<AlgoOrder> = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page: Vec<AlgoOrder> = self
                .get_signed(
                    "/api/v5/trade/orders-algo-pending",
                    Query::new()
                        .with("ordType", "conditional")
                        .with("instType", "SWAP")
                        .with_opt("after", after.clone())
                        .with("limit", 100),
                )
                .await?;
            let full = page.len() >= 100;
            after = page.last().map(|o| o.algo_id.clone());
            out.extend(page);
            if !full {
                break;
            }
        }
        Ok(out)
    }

    /// 撤条件单（按交易所的 algoId）。
    pub async fn cancel_stop(&self, inst_id: &str, algo_id: &str) -> Result<AlgoAck> {
        let path = "/api/v5/trade/cancel-algos";
        let ack: AlgoAck = first(
            self.post_signed(path, &json!([{ "instId": inst_id, "algoId": algo_id }]))
                .await?,
            path,
        )?;
        check_ack(path, &ack.s_code, &ack.s_msg)?;
        Ok(ack)
    }
}

/// 条目里的 `sCode` 不是 0：当成业务错误。
fn check_ack(path: &str, code: &str, msg: &str) -> Result<()> {
    if code.is_empty() || code == "0" {
        return Ok(());
    }
    Err(ExchangeClientError::Api {
        status: 200,
        code: code.parse().unwrap_or(0),
        msg: format!("{path}: {msg}"),
    })
}

/// 币的数量 ↔ 张数：`张 = 币 ÷ ctVal`。按数量步长 `lot_sz` 向下取整。
pub fn coins_to_contracts(coins: Decimal, instrument: &Instrument) -> Decimal {
    if instrument.ct_val.is_zero() {
        return Decimal::ZERO;
    }
    let contracts = coins / instrument.ct_val;
    if instrument.lot_sz.is_zero() {
        return contracts.normalize();
    }
    ((contracts / instrument.lot_sz).floor() * instrument.lot_sz).normalize()
}

/// 张数 → 币的数量。
pub fn contracts_to_coins(contracts: Decimal, instrument: &Instrument) -> Decimal {
    (contracts * instrument.ct_val).normalize()
}
