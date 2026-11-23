//! OKX 永续合约接口的请求和响应类型。
//!
//! OKX 的数都是字符串，没有值时是空字符串：`Decimal` 字段空串读成 0，`Option<Decimal>` 字段空串读成 `None`。
//! **数量 `sz` / `pos` / `fillSz` 的单位是"张"**，一张是多少币看交易规则里的 `ctVal`（[`Instrument::ct_val`]）。

use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;

fn dec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Decimal, D::Error> {
    Ok(opt_dec(deserializer)?.unwrap_or_default())
}

fn opt_dec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Decimal>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(text)) => Decimal::from_str(text.trim())
            .or_else(|_| Decimal::from_scientific(text.trim()))
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(serde_json::Value::Number(number)) => Decimal::from_str(&number.to_string())
            .or_else(|_| Decimal::from_scientific(&number.to_string()))
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(other) => Err(serde::de::Error::custom(format!("{other} 不是数"))),
    }
}

fn int<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        Some(serde_json::Value::String(text)) if !text.is_empty() => {
            text.parse().map_err(serde::de::Error::custom)
        }
        Some(serde_json::Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom("不是整数")),
        _ => Ok(0),
    }
}

fn flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Bool(b)) => b,
        Some(serde_json::Value::String(text)) => text == "true",
        _ => false,
    })
}

// ---------------------------------------------------------------------------
// 行情
// ---------------------------------------------------------------------------

/// 交易规则（`/api/v5/public/instruments`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Instrument {
    pub inst_id: String,
    #[serde(default)]
    pub inst_type: String,
    #[serde(default)]
    pub inst_family: String,
    #[serde(default)]
    pub uly: String,
    /// 保证金币种（U 本位是 `USDT`）。
    #[serde(default)]
    pub settle_ccy: String,
    /// `linear`（U 本位）或 `inverse`（币本位）。
    #[serde(default)]
    pub ct_type: String,
    /// 一张合约是多少 `ct_val_ccy`。
    #[serde(deserialize_with = "dec")]
    pub ct_val: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub ct_mult: Decimal,
    #[serde(default)]
    pub ct_val_ccy: String,
    /// 下单数量步长（张）。
    #[serde(deserialize_with = "dec")]
    pub lot_sz: Decimal,
    /// 最小下单数量（张）。
    #[serde(deserialize_with = "dec")]
    pub min_sz: Decimal,
    /// 价格步长。
    #[serde(deserialize_with = "dec")]
    pub tick_sz: Decimal,
    /// 市价单单笔最大数量（张）。
    #[serde(default, deserialize_with = "opt_dec")]
    pub max_mkt_sz: Option<Decimal>,
    /// 限价单单笔最大数量（张）。
    #[serde(default, deserialize_with = "opt_dec")]
    pub max_lmt_sz: Option<Decimal>,
    /// 最大杠杆。
    #[serde(default, deserialize_with = "opt_dec")]
    pub lever: Option<Decimal>,
    /// `live` 才能交易；还有 `suspend`、`preopen`、`test`。
    #[serde(default)]
    pub state: String,
    #[serde(default, deserialize_with = "int")]
    pub list_time: i64,
}

impl Instrument {
    pub fn is_live(&self) -> bool {
        self.state == "live"
    }

    /// U 本位永续（USDT 保证金、正向合约）。
    pub fn is_usdt_linear(&self) -> bool {
        self.ct_type == "linear" && self.settle_ccy == "USDT"
    }

    /// 标的币（`BTC-USDT-SWAP` → `BTC`）。
    pub fn base(&self) -> &str {
        self.inst_id.split('-').next().unwrap_or_default()
    }
}

/// 行情（`/api/v5/market/tickers`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ticker {
    pub inst_id: String,
    #[serde(default, deserialize_with = "dec")]
    pub last: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub bid_px: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub bid_sz: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub ask_px: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub ask_sz: Decimal,
    #[serde(default, deserialize_with = "int")]
    pub ts: i64,
}

// ---------------------------------------------------------------------------
// 下单
// ---------------------------------------------------------------------------

/// 新订单（`POST /api/v5/trade/order`）。`sz` 的单位是张。
///
/// 客户端订单号 `clOrdId` 只能是字母和数字，最长 32 位。
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewOrder {
    pub inst_id: String,
    /// 保证金模式：`cross` 全仓 / `isolated` 逐仓。
    pub td_mode: String,
    /// `buy` / `sell`。
    pub side: String,
    /// 双向持仓下 `long` / `short`；单向持仓不填。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_side: Option<String>,
    /// `market`、`limit`、`post_only`（只做 Maker，会立即成交时订单被撤销）等。
    pub ord_type: String,
    pub sz: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<bool>,
}

impl NewOrder {
    fn base(inst_id: &str, side: &str, ord_type: &str, sz: Decimal) -> Self {
        Self {
            inst_id: inst_id.into(),
            td_mode: "cross".into(),
            side: side.into(),
            pos_side: None,
            ord_type: ord_type.into(),
            sz: sz.normalize().to_string(),
            px: None,
            cl_ord_id: None,
            reduce_only: None,
        }
    }

    /// 市价单（全仓）。`side` 是 `buy` / `sell`。
    pub fn market(inst_id: &str, side: &str, sz: Decimal) -> Self {
        Self::base(inst_id, side, "market", sz)
    }

    /// 限价单（全仓）。
    pub fn limit(inst_id: &str, side: &str, sz: Decimal, px: Decimal) -> Self {
        let mut order = Self::base(inst_id, side, "limit", sz);
        order.px = Some(px.normalize().to_string());
        order
    }

    /// 只做 Maker 的限价单（全仓）。
    pub fn post_only(inst_id: &str, side: &str, sz: Decimal, px: Decimal) -> Self {
        let mut order = Self::base(inst_id, side, "post_only", sz);
        order.px = Some(px.normalize().to_string());
        order
    }

    pub fn pos_side(mut self, value: &str) -> Self {
        self.pos_side = Some(value.into());
        self
    }

    pub fn client_order_id(mut self, value: &str) -> Self {
        self.cl_ord_id = Some(value.into());
        self
    }

    pub fn td_mode(mut self, value: &str) -> Self {
        self.td_mode = value.into();
        self
    }

    pub fn reduce_only(mut self, value: bool) -> Self {
        self.reduce_only = Some(value);
        self
    }
}

/// 下单 / 撤单的回执。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderAck {
    #[serde(default)]
    pub ord_id: String,
    #[serde(default)]
    pub cl_ord_id: String,
    #[serde(default)]
    pub s_code: String,
    #[serde(default)]
    pub s_msg: String,
}

/// 按交易所订单号或客户端订单号查 / 撤。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderRef {
    Id(String),
    Client(String),
}

impl OrderRef {
    pub(crate) fn key(&self) -> &'static str {
        match self {
            OrderRef::Id(_) => "ordId",
            OrderRef::Client(_) => "clOrdId",
        }
    }

    pub(crate) fn value(&self) -> &str {
        match self {
            OrderRef::Id(v) | OrderRef::Client(v) => v,
        }
    }
}

/// 订单（`/api/v5/trade/order`、`/api/v5/trade/orders-pending`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub inst_id: String,
    pub ord_id: String,
    #[serde(default)]
    pub cl_ord_id: String,
    /// 委托价（市价单为 0）。
    #[serde(default, deserialize_with = "dec")]
    pub px: Decimal,
    /// 委托数量（张）。
    #[serde(default, deserialize_with = "dec")]
    pub sz: Decimal,
    #[serde(default)]
    pub ord_type: String,
    pub side: String,
    #[serde(default)]
    pub pos_side: String,
    #[serde(default)]
    pub td_mode: String,
    /// 累计成交数量（张）。
    #[serde(default, deserialize_with = "dec")]
    pub acc_fill_sz: Decimal,
    /// 成交均价；没成交时为 0。
    #[serde(default, deserialize_with = "dec")]
    pub avg_px: Decimal,
    /// `live`、`partially_filled`、`filled`、`canceled`、`mmp_canceled`。
    pub state: String,
    /// 累计手续费（负数 = 扣费，正数 = 返佣）。
    #[serde(default, deserialize_with = "dec")]
    pub fee: Decimal,
    #[serde(default)]
    pub fee_ccy: String,
    #[serde(default, deserialize_with = "flag")]
    pub reduce_only: bool,
    #[serde(default, deserialize_with = "int")]
    pub c_time: i64,
    #[serde(default, deserialize_with = "int")]
    pub u_time: i64,
}

impl Order {
    /// 已经结束（全部成交或已撤）。
    pub fn is_final(&self) -> bool {
        matches!(self.state.as_str(), "filled" | "canceled" | "mmp_canceled")
    }
}

/// 成交明细（`/api/v5/trade/fills`、`/api/v5/trade/fills-history`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub inst_id: String,
    #[serde(default)]
    pub trade_id: String,
    pub ord_id: String,
    #[serde(default)]
    pub cl_ord_id: String,
    #[serde(default)]
    pub bill_id: String,
    pub side: String,
    #[serde(default)]
    pub pos_side: String,
    #[serde(deserialize_with = "dec")]
    pub fill_px: Decimal,
    /// 成交数量（张）。
    #[serde(deserialize_with = "dec")]
    pub fill_sz: Decimal,
    /// 手续费（负数 = 扣费，正数 = 返佣）。
    #[serde(default, deserialize_with = "dec")]
    pub fee: Decimal,
    #[serde(default)]
    pub fee_ccy: String,
    /// `M` 挂单成交（Maker），`T` 吃单成交（Taker）。
    #[serde(default)]
    pub exec_type: String,
    /// 平仓盈亏（不含手续费）。
    #[serde(default, deserialize_with = "dec")]
    pub fill_pnl: Decimal,
    #[serde(default, deserialize_with = "int")]
    pub ts: i64,
}

impl Fill {
    pub fn is_maker(&self) -> bool {
        self.exec_type == "M"
    }
}

// ---------------------------------------------------------------------------
// 账户
// ---------------------------------------------------------------------------

/// 持仓（`/api/v5/account/positions`）。双向持仓下 `pos` 都是正数，方向看 `pos_side`；
/// 单向持仓（`pos_side = net`）下 `pos` 为负表示空头。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub inst_id: String,
    #[serde(default)]
    pub pos_side: String,
    /// 持仓数量（张）。
    #[serde(default, deserialize_with = "dec")]
    pub pos: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub avg_px: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub mark_px: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub upl: Decimal,
    #[serde(default, deserialize_with = "opt_dec")]
    pub liq_px: Option<Decimal>,
    #[serde(default, deserialize_with = "opt_dec")]
    pub lever: Option<Decimal>,
    #[serde(default)]
    pub mgn_mode: String,
    #[serde(default, deserialize_with = "dec")]
    pub notional_usd: Decimal,
    #[serde(default, deserialize_with = "int")]
    pub u_time: i64,
}

/// 账户余额（`/api/v5/account/balance`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    /// 账户总权益（美元）。
    #[serde(default, deserialize_with = "dec")]
    pub total_eq: Decimal,
    #[serde(default)]
    pub details: Vec<BalanceDetail>,
    #[serde(default, deserialize_with = "int")]
    pub u_time: i64,
}

impl Balance {
    pub fn detail(&self, ccy: &str) -> Option<&BalanceDetail> {
        self.details.iter().find(|d| d.ccy == ccy)
    }
}

/// 某个币种的余额。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceDetail {
    pub ccy: String,
    /// 币种权益（含未实现盈亏）。
    #[serde(default, deserialize_with = "dec")]
    pub eq: Decimal,
    #[serde(default, deserialize_with = "dec")]
    pub cash_bal: Decimal,
    /// 可用保证金（保证金模式下）。
    #[serde(default, deserialize_with = "opt_dec")]
    pub avail_eq: Option<Decimal>,
    /// 可用余额。
    #[serde(default, deserialize_with = "opt_dec")]
    pub avail_bal: Option<Decimal>,
    #[serde(default, deserialize_with = "dec")]
    pub upl: Decimal,
}

/// 账户配置（`/api/v5/account/config`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    #[serde(default)]
    pub uid: String,
    /// `long_short_mode`（双向持仓）/ `net_mode`（单向持仓）。
    #[serde(default)]
    pub pos_mode: String,
    /// 账户模式：1 现货、2 合约（单币种保证金）、3 跨币种保证金、4 组合保证金。
    #[serde(default)]
    pub acct_lv: String,
}

impl AccountConfig {
    pub fn is_long_short(&self) -> bool {
        self.pos_mode == "long_short_mode"
    }
}

/// 设置杠杆的回执（`/api/v5/account/set-leverage`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeverageInfo {
    #[serde(default)]
    pub inst_id: String,
    #[serde(default, deserialize_with = "opt_dec")]
    pub lever: Option<Decimal>,
    #[serde(default)]
    pub mgn_mode: String,
    #[serde(default)]
    pub pos_side: String,
}

/// 账单流水（`/api/v5/account/bills`、`/api/v5/account/bills-archive`）。资金费是 `type = 8`。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bill {
    pub bill_id: String,
    #[serde(default)]
    pub inst_id: String,
    #[serde(default)]
    pub ccy: String,
    /// 余额变动（收到为正）。
    #[serde(default, deserialize_with = "dec")]
    pub bal_chg: Decimal,
    #[serde(default, rename = "type")]
    pub bill_type: String,
    #[serde(default)]
    pub sub_type: String,
    #[serde(default, deserialize_with = "int")]
    pub ts: i64,
}

// ---------------------------------------------------------------------------
// 条件单
// ---------------------------------------------------------------------------

/// 止损条件单（`POST /api/v5/trade/order-algo`，`ordType = conditional`）：标记价格触发，市价成交。`sz` 的单位是张。
///
/// `algoClOrdId` 只能是字母和数字，最长 32 位。
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewStopOrder {
    pub inst_id: String,
    pub td_mode: String,
    pub side: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_side: Option<String>,
    pub ord_type: String,
    pub sz: String,
    pub sl_trigger_px: String,
    /// `-1` 表示市价。
    pub sl_ord_px: String,
    /// `last` / `index` / `mark`。
    pub sl_trigger_px_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algo_cl_ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<bool>,
}

impl NewStopOrder {
    /// 标记价格到 `trigger` 时按市价减仓（全仓）。`side` 是 `buy` / `sell`。
    pub fn stop_market(inst_id: &str, side: &str, sz: Decimal, trigger: Decimal) -> Self {
        Self {
            inst_id: inst_id.into(),
            td_mode: "cross".into(),
            side: side.into(),
            pos_side: None,
            ord_type: "conditional".into(),
            sz: sz.normalize().to_string(),
            sl_trigger_px: trigger.normalize().to_string(),
            sl_ord_px: "-1".into(),
            sl_trigger_px_type: "mark".into(),
            algo_cl_ord_id: None,
            reduce_only: Some(true),
        }
    }

    pub fn pos_side(mut self, value: &str) -> Self {
        self.pos_side = Some(value.into());
        // 双向持仓下平仓单由 posSide 决定方向，OKX 不接受 reduceOnly。
        self.reduce_only = None;
        self
    }

    pub fn client_id(mut self, value: &str) -> Self {
        self.algo_cl_ord_id = Some(value.into());
        self
    }
}

/// 条件单回执。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlgoAck {
    #[serde(default)]
    pub algo_id: String,
    #[serde(default)]
    pub algo_cl_ord_id: String,
    #[serde(default)]
    pub s_code: String,
    #[serde(default)]
    pub s_msg: String,
}

/// 未触发的条件单（`/api/v5/trade/orders-algo-pending`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlgoOrder {
    pub algo_id: String,
    #[serde(default)]
    pub algo_cl_ord_id: String,
    pub inst_id: String,
    #[serde(default)]
    pub ord_type: String,
    pub side: String,
    #[serde(default)]
    pub pos_side: String,
    /// 数量（张）。
    #[serde(default, deserialize_with = "dec")]
    pub sz: Decimal,
    #[serde(default, deserialize_with = "opt_dec")]
    pub sl_trigger_px: Option<Decimal>,
    #[serde(default)]
    pub sl_trigger_px_type: String,
    #[serde(default)]
    pub state: String,
    #[serde(default, deserialize_with = "int")]
    pub c_time: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_strings_and_flags_parse() {
        let order: Order = serde_json::from_str(
            r#"{"instId":"BTC-USDT-SWAP","ordId":"312269865356374016","clOrdId":"b1","px":"","sz":"2","ordType":"market","side":"buy","posSide":"long","tdMode":"cross","accFillSz":"0","avgPx":"","state":"live","fee":"","feeCcy":"USDT","reduceOnly":"false","cTime":"1597026383085","uTime":"1597026383085"}"#,
        )
        .unwrap();
        assert_eq!(order.px, Decimal::ZERO);
        assert_eq!(order.avg_px, Decimal::ZERO);
        assert!(!order.reduce_only);
        assert!(!order.is_final());
        assert_eq!(order.c_time, 1597026383085);
        let instrument: Instrument = serde_json::from_str(
            r#"{"instId":"PEPE-USDT-SWAP","instType":"SWAP","settleCcy":"USDT","ctType":"linear","ctVal":"10000000","ctMult":"1","ctValCcy":"PEPE","lotSz":"0.1","minSz":"0.1","tickSz":"0.0000000001","maxMktSz":"","state":"live","listTime":"1683000000000","lever":"50"}"#,
        )
        .unwrap();
        assert!(instrument.is_live() && instrument.is_usdt_linear());
        assert_eq!(instrument.base(), "PEPE");
        assert_eq!(instrument.max_mkt_sz, None);
        assert_eq!(instrument.ct_val, Decimal::new(10_000_000, 0));
    }

    #[test]
    fn orders_serialize_like_okx_expects() {
        let order = NewOrder::post_only(
            "BTC-USDT-SWAP",
            "buy",
            Decimal::new(120, 2),
            Decimal::new(6500010, 2),
        )
        .pos_side("long")
        .client_order_id("xsZ1");
        let json = serde_json::to_value(&order).unwrap();
        assert_eq!(json["ordType"], "post_only");
        assert_eq!(json["sz"], "1.2");
        assert_eq!(json["px"], "65000.1");
        assert_eq!(json["posSide"], "long");
        assert_eq!(json["tdMode"], "cross");
        assert!(json.get("reduceOnly").is_none());
        let stop = NewStopOrder::stop_market(
            "ETH-USDT-SWAP",
            "sell",
            Decimal::new(3, 0),
            Decimal::new(1000, 0),
        )
        .pos_side("long")
        .client_id("xsZslZ1");
        let json = serde_json::to_value(&stop).unwrap();
        assert_eq!(json["slOrdPx"], "-1");
        assert_eq!(json["slTriggerPxType"], "mark");
        assert!(json.get("reduceOnly").is_none());
    }
}
