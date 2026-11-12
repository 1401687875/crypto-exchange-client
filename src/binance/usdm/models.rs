//! U 本位合约的请求和响应类型。
//!
//! 金额、价格、数量一律用 [`rust_decimal::Decimal`]（交易所返回的是字符串，按十进制解析，不损失精度）。
//! 枚举遇到本库还不认识的取值时落到 `Other(String)`，交易所新增取值不会让解析失败。
//! 响应结构里交易所可能省略的字段都是 `Option` 或带默认值。

use super::client::Params;
use crate::error::{ExchangeClientError, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};

macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
            /// 本库还不认识的取值，原样保留。
            Other(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $text,)+
                    Self::Other(text) => text,
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = std::convert::Infallible;
            fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
                Ok(match text {
                    $($text => Self::$variant,)+
                    other => Self::Other(other.to_string()),
                })
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Ok(text.parse().unwrap())
            }
        }
    };
}

wire_enum!(
    /// 买卖方向。
    Side { Buy => "BUY", Sell => "SELL" }
);

impl Side {
    pub fn opposite(&self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
            Side::Other(text) => Side::Other(text.clone()),
        }
    }
}

wire_enum!(
    /// 持仓方向：单向持仓模式用 `Both`，双向持仓模式用 `Long` / `Short`。
    PositionSide { Both => "BOTH", Long => "LONG", Short => "SHORT" }
);

wire_enum!(
    /// 订单类型。条件单（`Stop*` / `TakeProfit*` / `TrailingStopMarket`）要用 [`NewAlgoOrder`] 下。
    OrderType {
        Limit => "LIMIT",
        Market => "MARKET",
        Stop => "STOP",
        StopMarket => "STOP_MARKET",
        TakeProfit => "TAKE_PROFIT",
        TakeProfitMarket => "TAKE_PROFIT_MARKET",
        TrailingStopMarket => "TRAILING_STOP_MARKET",
        Liquidation => "LIQUIDATION",
    }
);

wire_enum!(
    /// 有效方式。`Gtx` = 只做 Maker（Post Only）：会立即成交的订单直接被拒，保证按挂单费率成交。
    TimeInForce { Gtc => "GTC", Ioc => "IOC", Fok => "FOK", Gtx => "GTX", Gtd => "GTD" }
);

wire_enum!(
    /// 订单状态。
    OrderStatus {
        New => "NEW",
        PartiallyFilled => "PARTIALLY_FILLED",
        Filled => "FILLED",
        Canceled => "CANCELED",
        Rejected => "REJECTED",
        Expired => "EXPIRED",
        /// 被自成交保护撤掉。
        ExpiredInMatch => "EXPIRED_IN_MATCH",
    }
);

impl OrderStatus {
    /// 还在盘口上（未成交或部分成交）。
    pub fn is_open(&self) -> bool {
        matches!(self, Self::New | Self::PartiallyFilled)
    }

    /// 终态：不会再有成交。
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Canceled | Self::Rejected | Self::Expired | Self::ExpiredInMatch
        )
    }
}

wire_enum!(
    /// 条件单的触发价格类型。
    WorkingType { MarkPrice => "MARK_PRICE", ContractPrice => "CONTRACT_PRICE" }
);

wire_enum!(
    /// 按盘口自动定价（与 `price` 二选一）。`Queue` = 己方一档（排队挂单），`Opponent` = 对手一档。
    PriceMatch {
        None => "NONE",
        Opponent => "OPPONENT",
        Opponent5 => "OPPONENT_5",
        Opponent10 => "OPPONENT_10",
        Opponent20 => "OPPONENT_20",
        Queue => "QUEUE",
        Queue5 => "QUEUE_5",
        Queue10 => "QUEUE_10",
        Queue20 => "QUEUE_20",
    }
);

wire_enum!(
    /// 自成交保护。
    SelfTradePrevention {
        None => "NONE",
        ExpireTaker => "EXPIRE_TAKER",
        ExpireMaker => "EXPIRE_MAKER",
        ExpireBoth => "EXPIRE_BOTH",
    }
);

wire_enum!(
    /// 下单接口的响应内容：`Ack` 只返回受理信息，`Result` 返回成交结果。
    ResponseType { Ack => "ACK", Result => "RESULT" }
);

wire_enum!(
    /// 保证金模式（设置用）。
    MarginType { Isolated => "ISOLATED", Crossed => "CROSSED" }
);

wire_enum!(
    /// 条件单状态。
    AlgoStatus {
        New => "NEW",
        Canceled => "CANCELED",
        Triggering => "TRIGGERING",
        Triggered => "TRIGGERED",
        Finished => "FINISHED",
        Rejected => "REJECTED",
        Expired => "EXPIRED",
    }
);

wire_enum!(
    /// 用户数据流里订单事件的执行类型。
    ExecutionType {
        New => "NEW",
        Canceled => "CANCELED",
        Calculated => "CALCULATED",
        Expired => "EXPIRED",
        Trade => "TRADE",
        Amendment => "AMENDMENT",
    }
);

wire_enum!(
    /// K 线周期。
    Interval {
        M1 => "1m", M3 => "3m", M5 => "5m", M15 => "15m", M30 => "30m",
        H1 => "1h", H2 => "2h", H4 => "4h", H6 => "6h", H8 => "8h", H12 => "12h",
        D1 => "1d", D3 => "3d", W1 => "1w", Mo1 => "1M",
    }
);

// ---------------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------------

/// 普通订单（市价、限价、只做 Maker）。条件单用 [`NewAlgoOrder`]。
///
/// ```
/// use crypto_exchange_client::binance::usdm::{NewOrder, Side, PositionSide};
/// let order = NewOrder::post_only("BTCUSDT", Side::Buy, "0.002".parse().unwrap(), "65000.1".parse().unwrap())
///     .position_side(PositionSide::Long)
///     .client_order_id("xs-20261112-001");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct NewOrder {
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub position_side: Option<PositionSide>,
    pub time_in_force: Option<TimeInForce>,
    pub quantity: Option<Decimal>,
    pub price: Option<Decimal>,
    pub reduce_only: Option<bool>,
    pub client_order_id: Option<String>,
    pub price_match: Option<PriceMatch>,
    pub self_trade_prevention: Option<SelfTradePrevention>,
    /// `TimeInForce::Gtd` 时的自动撤单时间（毫秒时间戳）。
    pub good_till_date: Option<i64>,
    pub response_type: Option<ResponseType>,
}

impl NewOrder {
    fn base(symbol: impl Into<String>, side: Side, order_type: OrderType) -> Self {
        Self {
            symbol: symbol.into(),
            side,
            order_type,
            position_side: None,
            time_in_force: None,
            quantity: None,
            price: None,
            reduce_only: None,
            client_order_id: None,
            price_match: None,
            self_trade_prevention: None,
            good_till_date: None,
            response_type: None,
        }
    }

    /// 市价单。
    pub fn market(symbol: impl Into<String>, side: Side, quantity: Decimal) -> Self {
        let mut order = Self::base(symbol, side, OrderType::Market);
        order.quantity = Some(quantity);
        order
    }

    /// 限价单（GTC）。
    pub fn limit(symbol: impl Into<String>, side: Side, quantity: Decimal, price: Decimal) -> Self {
        let mut order = Self::base(symbol, side, OrderType::Limit);
        order.quantity = Some(quantity);
        order.price = Some(price);
        order.time_in_force = Some(TimeInForce::Gtc);
        order
    }

    /// 只做 Maker 的限价单（`timeInForce=GTX`）：如果下单时会立即成交，交易所拒单（错误码 `-5022`），
    /// 保证成交的部分都按挂单费率。
    pub fn post_only(
        symbol: impl Into<String>,
        side: Side,
        quantity: Decimal,
        price: Decimal,
    ) -> Self {
        Self::limit(symbol, side, quantity, price).time_in_force(TimeInForce::Gtx)
    }

    /// 按盘口自动定价的只做 Maker 单，例如 `PriceMatch::Queue` 挂在己方一档。
    pub fn post_only_matched(
        symbol: impl Into<String>,
        side: Side,
        quantity: Decimal,
        price_match: PriceMatch,
    ) -> Self {
        let mut order = Self::base(symbol, side, OrderType::Limit);
        order.quantity = Some(quantity);
        order.price_match = Some(price_match);
        order.time_in_force = Some(TimeInForce::Gtx);
        order
    }

    pub fn position_side(mut self, value: PositionSide) -> Self {
        self.position_side = Some(value);
        self
    }

    pub fn time_in_force(mut self, value: TimeInForce) -> Self {
        self.time_in_force = Some(value);
        self
    }

    /// 只减仓（单向持仓模式使用；双向持仓模式下不能带这个参数）。
    pub fn reduce_only(mut self, value: bool) -> Self {
        self.reduce_only = Some(value);
        self
    }

    /// 客户端订单 ID：1–36 位，只能用字母、数字和 `.:/_-`。用来在超时后查询订单、避免重复下单。
    pub fn client_order_id(mut self, value: impl Into<String>) -> Self {
        self.client_order_id = Some(value.into());
        self
    }

    pub fn price_match(mut self, value: PriceMatch) -> Self {
        self.price_match = Some(value);
        self.price = None;
        self
    }

    pub fn self_trade_prevention(mut self, value: SelfTradePrevention) -> Self {
        self.self_trade_prevention = Some(value);
        self
    }

    pub fn good_till_date(mut self, ms: i64) -> Self {
        self.good_till_date = Some(ms);
        self.time_in_force = Some(TimeInForce::Gtd);
        self
    }

    pub fn response_type(mut self, value: ResponseType) -> Self {
        self.response_type = Some(value);
        self
    }

    /// 本地检查明显错误，避免浪费请求权重。
    pub fn validate(&self) -> Result<()> {
        if self.symbol.is_empty() {
            return Err(invalid("symbol 不能为空"));
        }
        match self.order_type {
            OrderType::Market | OrderType::Limit => {}
            ref other => {
                return Err(invalid(&format!(
                    "{other} 是条件单，币安已把条件单迁到 algoOrder 接口，请用 NewAlgoOrder"
                )))
            }
        }
        if !matches!(self.quantity, Some(q) if q > Decimal::ZERO) {
            return Err(invalid("quantity 必须为正"));
        }
        if self.order_type == OrderType::Limit {
            if self.price.is_none() && self.price_match.is_none() {
                return Err(invalid("限价单需要 price 或 price_match"));
            }
            if self.price.is_some() && self.price_match.is_some() {
                return Err(invalid("price 和 price_match 不能同时设置"));
            }
            if matches!(self.price, Some(p) if p <= Decimal::ZERO) {
                return Err(invalid("price 必须为正"));
            }
        }
        if let Some(id) = &self.client_order_id {
            validate_client_id(id)?;
        }
        if self.time_in_force == Some(TimeInForce::Gtd) && self.good_till_date.is_none() {
            return Err(invalid("GTD 订单需要 good_till_date"));
        }
        Ok(())
    }

    pub(crate) fn to_params(&self) -> Params {
        let mut params = Params::new();
        params
            .push("symbol", &self.symbol)
            .push("side", &self.side)
            .push("type", &self.order_type)
            .opt("positionSide", self.position_side.as_ref())
            .opt("timeInForce", self.time_in_force.as_ref())
            .opt("quantity", self.quantity.map(|q| q.normalize()))
            .opt("price", self.price.map(|p| p.normalize()))
            .opt("reduceOnly", self.reduce_only)
            .opt("newClientOrderId", self.client_order_id.as_ref())
            .opt("priceMatch", self.price_match.as_ref())
            .opt(
                "selfTradePreventionMode",
                self.self_trade_prevention.as_ref(),
            )
            .opt("goodTillDate", self.good_till_date)
            .opt("newOrderRespType", self.response_type.as_ref());
        params
    }

    /// 批量下单接口要的 JSON 对象（字段与单个下单的参数相同）。
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let map: serde_json::Map<String, serde_json::Value> = self
            .to_params()
            .0
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
            .collect();
        serde_json::Value::Object(map)
    }
}

/// 改单（只支持限价单）：改价格或数量，订单 ID 不变；改单后在盘口的排队位置会重置。
#[derive(Clone, Debug, PartialEq)]
pub struct ModifyOrder {
    pub symbol: String,
    pub id: OrderId,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Option<Decimal>,
    pub price_match: Option<PriceMatch>,
}

impl ModifyOrder {
    pub fn new(
        symbol: impl Into<String>,
        id: OrderId,
        side: Side,
        quantity: Decimal,
        price: Decimal,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            id,
            side,
            quantity,
            price: Some(price),
            price_match: None,
        }
    }

    pub(crate) fn to_params(&self) -> Params {
        let mut params = Params::new();
        params.push("symbol", &self.symbol);
        self.id.push_to(&mut params);
        params
            .push("side", &self.side)
            .push("quantity", self.quantity.normalize())
            .opt("price", self.price.map(|p| p.normalize()))
            .opt("priceMatch", self.price_match.as_ref());
        params
    }
}

/// 订单标识：交易所订单 ID 或客户端订单 ID。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderId {
    Exchange(i64),
    Client(String),
}

impl OrderId {
    pub(crate) fn push_to(&self, params: &mut Params) {
        match self {
            OrderId::Exchange(id) => params.push("orderId", id),
            OrderId::Client(id) => params.push("origClientOrderId", id),
        };
    }
}

impl From<i64> for OrderId {
    fn from(value: i64) -> Self {
        OrderId::Exchange(value)
    }
}

impl From<&str> for OrderId {
    fn from(value: &str) -> Self {
        OrderId::Client(value.to_string())
    }
}

impl From<String> for OrderId {
    fn from(value: String) -> Self {
        OrderId::Client(value)
    }
}

/// 条件单（止损、止盈、跟踪止损）。币安已把这类订单从 `/fapi/v1/order` 迁到 `/fapi/v1/algoOrder`。
#[derive(Clone, Debug, PartialEq)]
pub struct NewAlgoOrder {
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub position_side: Option<PositionSide>,
    pub quantity: Option<Decimal>,
    pub price: Option<Decimal>,
    pub trigger_price: Option<Decimal>,
    pub working_type: Option<WorkingType>,
    pub close_position: Option<bool>,
    pub reduce_only: Option<bool>,
    pub price_protect: Option<bool>,
    pub activate_price: Option<Decimal>,
    /// 跟踪止损回调比例，0.1–10（1 表示 1%）。
    pub callback_rate: Option<Decimal>,
    pub time_in_force: Option<TimeInForce>,
    pub client_algo_id: Option<String>,
    pub self_trade_prevention: Option<SelfTradePrevention>,
    pub good_till_date: Option<i64>,
}

impl NewAlgoOrder {
    fn base(symbol: impl Into<String>, side: Side, order_type: OrderType) -> Self {
        Self {
            symbol: symbol.into(),
            side,
            order_type,
            position_side: None,
            quantity: None,
            price: None,
            trigger_price: None,
            working_type: None,
            close_position: None,
            reduce_only: None,
            price_protect: None,
            activate_price: None,
            callback_rate: None,
            time_in_force: None,
            client_algo_id: None,
            self_trade_prevention: None,
            good_till_date: None,
        }
    }

    /// 市价止损：价格触及 `trigger_price` 后按市价成交。还要设置 `quantity(..)` 或 `close_position()`。
    pub fn stop_market(symbol: impl Into<String>, side: Side, trigger_price: Decimal) -> Self {
        let mut order = Self::base(symbol, side, OrderType::StopMarket);
        order.trigger_price = Some(trigger_price);
        order
    }

    /// 市价止盈。
    pub fn take_profit_market(
        symbol: impl Into<String>,
        side: Side,
        trigger_price: Decimal,
    ) -> Self {
        let mut order = Self::base(symbol, side, OrderType::TakeProfitMarket);
        order.trigger_price = Some(trigger_price);
        order
    }

    /// 限价止损：触发后挂 `price` 的限价单。
    pub fn stop_limit(
        symbol: impl Into<String>,
        side: Side,
        quantity: Decimal,
        trigger_price: Decimal,
        price: Decimal,
    ) -> Self {
        let mut order = Self::base(symbol, side, OrderType::Stop);
        order.quantity = Some(quantity);
        order.trigger_price = Some(trigger_price);
        order.price = Some(price);
        order.time_in_force = Some(TimeInForce::Gtc);
        order
    }

    /// 限价止盈。
    pub fn take_profit_limit(
        symbol: impl Into<String>,
        side: Side,
        quantity: Decimal,
        trigger_price: Decimal,
        price: Decimal,
    ) -> Self {
        let mut order = Self::stop_limit(symbol, side, quantity, trigger_price, price);
        order.order_type = OrderType::TakeProfit;
        order
    }

    /// 跟踪止损。
    pub fn trailing_stop(
        symbol: impl Into<String>,
        side: Side,
        quantity: Decimal,
        callback_rate: Decimal,
    ) -> Self {
        let mut order = Self::base(symbol, side, OrderType::TrailingStopMarket);
        order.quantity = Some(quantity);
        order.callback_rate = Some(callback_rate);
        order
    }

    pub fn quantity(mut self, value: Decimal) -> Self {
        self.quantity = Some(value);
        self
    }

    /// 触发后平掉这个方向的全部仓位（不能同时设置 quantity / reduce_only）。
    pub fn close_position(mut self) -> Self {
        self.close_position = Some(true);
        self
    }

    pub fn position_side(mut self, value: PositionSide) -> Self {
        self.position_side = Some(value);
        self
    }

    pub fn working_type(mut self, value: WorkingType) -> Self {
        self.working_type = Some(value);
        self
    }

    pub fn reduce_only(mut self, value: bool) -> Self {
        self.reduce_only = Some(value);
        self
    }

    pub fn price_protect(mut self, value: bool) -> Self {
        self.price_protect = Some(value);
        self
    }

    pub fn activate_price(mut self, value: Decimal) -> Self {
        self.activate_price = Some(value);
        self
    }

    pub fn client_algo_id(mut self, value: impl Into<String>) -> Self {
        self.client_algo_id = Some(value.into());
        self
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(
            self.order_type,
            OrderType::Stop
                | OrderType::StopMarket
                | OrderType::TakeProfit
                | OrderType::TakeProfitMarket
                | OrderType::TrailingStopMarket
        ) {
            return Err(invalid("条件单类型只能是 STOP / STOP_MARKET / TAKE_PROFIT / TAKE_PROFIT_MARKET / TRAILING_STOP_MARKET"));
        }
        let close = self.close_position == Some(true);
        if close && (self.quantity.is_some() || self.reduce_only.is_some()) {
            return Err(invalid(
                "close_position 不能和 quantity / reduce_only 同时设置",
            ));
        }
        if !close && self.quantity.is_none() {
            return Err(invalid("条件单需要 quantity 或 close_position"));
        }
        if self.order_type != OrderType::TrailingStopMarket && self.trigger_price.is_none() {
            return Err(invalid("条件单需要 trigger_price"));
        }
        if matches!(self.order_type, OrderType::Stop | OrderType::TakeProfit)
            && self.price.is_none()
        {
            return Err(invalid("限价条件单需要 price"));
        }
        if let Some(id) = &self.client_algo_id {
            validate_client_id(id)?;
        }
        Ok(())
    }

    pub(crate) fn to_params(&self) -> Params {
        let mut params = Params::new();
        params
            .push("algoType", "CONDITIONAL")
            .push("symbol", &self.symbol)
            .push("side", &self.side)
            .push("type", &self.order_type)
            .opt("positionSide", self.position_side.as_ref())
            .opt("timeInForce", self.time_in_force.as_ref())
            .opt("quantity", self.quantity.map(|v| v.normalize()))
            .opt("price", self.price.map(|v| v.normalize()))
            .opt("triggerPrice", self.trigger_price.map(|v| v.normalize()))
            .opt("workingType", self.working_type.as_ref())
            .opt("closePosition", self.close_position)
            .opt(
                "priceProtect",
                self.price_protect.map(|v| if v { "TRUE" } else { "FALSE" }),
            )
            .opt("reduceOnly", self.reduce_only)
            .opt("activatePrice", self.activate_price.map(|v| v.normalize()))
            .opt("callbackRate", self.callback_rate.map(|v| v.normalize()))
            .opt("clientAlgoId", self.client_algo_id.as_ref())
            .opt(
                "selfTradePreventionMode",
                self.self_trade_prevention.as_ref(),
            )
            .opt("goodTillDate", self.good_till_date);
        params
    }
}

fn invalid(message: &str) -> ExchangeClientError {
    ExchangeClientError::Validation(message.to_string())
}

pub(crate) fn validate_client_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 36
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '/' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(invalid(
            "客户端订单 ID 需为 1–36 位，只能包含字母、数字和 .:/_-",
        ))
    }
}

// ---------------------------------------------------------------------------
// 响应：订单与成交
// ---------------------------------------------------------------------------

/// 订单（下单、查单、改单、撤单的响应都是这个结构）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub order_id: i64,
    pub symbol: String,
    pub status: OrderStatus,
    pub client_order_id: String,
    #[serde(default)]
    pub price: Decimal,
    #[serde(default)]
    pub avg_price: Decimal,
    #[serde(default)]
    pub orig_qty: Decimal,
    #[serde(default)]
    pub executed_qty: Decimal,
    #[serde(default)]
    pub cum_quote: Decimal,
    #[serde(default)]
    pub time_in_force: Option<TimeInForce>,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub close_position: bool,
    pub side: Side,
    #[serde(default)]
    pub position_side: Option<PositionSide>,
    #[serde(default)]
    pub stop_price: Option<Decimal>,
    #[serde(default)]
    pub working_type: Option<WorkingType>,
    #[serde(default)]
    pub price_protect: bool,
    #[serde(default)]
    pub orig_type: Option<OrderType>,
    #[serde(default)]
    pub price_match: Option<PriceMatch>,
    #[serde(default)]
    pub self_trade_prevention_mode: Option<SelfTradePrevention>,
    #[serde(default)]
    pub good_till_date: i64,
    /// 下单时间（查单接口才有）。
    #[serde(default)]
    pub time: Option<i64>,
    #[serde(default)]
    pub update_time: i64,
}

impl Order {
    /// 未成交数量。
    pub fn remaining_qty(&self) -> Decimal {
        (self.orig_qty - self.executed_qty).max(Decimal::ZERO)
    }

    /// 成交均价；`avgPrice` 为 0 时用成交额 / 成交量推算。
    pub fn fill_price(&self) -> Option<Decimal> {
        if self.avg_price > Decimal::ZERO {
            Some(self.avg_price)
        } else if self.executed_qty > Decimal::ZERO && self.cum_quote > Decimal::ZERO {
            Some(self.cum_quote / self.executed_qty)
        } else {
            None
        }
    }
}

/// 批量下单 / 批量撤单里单个订单的结果：成功是订单，失败是交易所错误。
#[derive(Clone, Debug, PartialEq)]
pub enum BatchItem {
    Ok(Box<Order>),
    Err { code: i64, msg: String },
}

impl<'de> Deserialize<'de> for BatchItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("orderId").is_none() {
            if let Some(code) = value.get("code").and_then(|c| c.as_i64()) {
                let msg = value
                    .get("msg")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default()
                    .to_string();
                return Ok(BatchItem::Err { code, msg });
            }
        }
        serde_json::from_value::<Order>(value)
            .map(|order| BatchItem::Ok(Box::new(order)))
            .map_err(serde::de::Error::custom)
    }
}

/// 账户成交明细（`/fapi/v1/userTrades`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserTrade {
    pub id: i64,
    pub order_id: i64,
    pub symbol: String,
    pub side: Side,
    #[serde(default)]
    pub position_side: Option<PositionSide>,
    pub price: Decimal,
    pub qty: Decimal,
    pub quote_qty: Decimal,
    pub commission: Decimal,
    pub commission_asset: String,
    #[serde(default)]
    pub realized_pnl: Decimal,
    /// 是否挂单方（Maker）成交。
    pub maker: bool,
    #[serde(default)]
    pub buyer: bool,
    pub time: i64,
}

/// 条件单。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlgoOrder {
    pub algo_id: i64,
    #[serde(default)]
    pub client_algo_id: String,
    #[serde(default)]
    pub algo_type: Option<String>,
    pub order_type: OrderType,
    pub symbol: String,
    pub side: Side,
    #[serde(default)]
    pub position_side: Option<PositionSide>,
    #[serde(default)]
    pub time_in_force: Option<TimeInForce>,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub quantity: Option<Decimal>,
    #[serde(default)]
    pub algo_status: Option<AlgoStatus>,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub trigger_price: Option<Decimal>,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub price: Option<Decimal>,
    #[serde(default)]
    pub working_type: Option<WorkingType>,
    #[serde(default)]
    pub price_match: Option<PriceMatch>,
    #[serde(default)]
    pub close_position: bool,
    #[serde(default)]
    pub price_protect: bool,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub activate_price: Option<Decimal>,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub callback_rate: Option<Decimal>,
    #[serde(default)]
    pub create_time: i64,
    #[serde(default)]
    pub update_time: i64,
    #[serde(default)]
    pub trigger_time: i64,
    #[serde(default)]
    pub good_till_date: i64,
    /// 触发后在撮合引擎生成的订单 ID。
    #[serde(default)]
    pub actual_order_id: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub executed_qty: Option<Decimal>,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub avg_price: Option<Decimal>,
}

/// 只有 code / msg 的响应（如撤全部挂单、调整保证金模式）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CodeMsg {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub msg: String,
}

/// 倒计时撤单的响应。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CountdownCancel {
    pub symbol: String,
    pub countdown_time: String,
}

// ---------------------------------------------------------------------------
// 响应：账户
// ---------------------------------------------------------------------------

/// 持仓（`/fapi/v3/positionRisk`）。单向持仓模式下 `position_amt` 为负表示空头。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub symbol: String,
    #[serde(default)]
    pub position_side: Option<PositionSide>,
    pub position_amt: Decimal,
    #[serde(default)]
    pub entry_price: Decimal,
    #[serde(default)]
    pub break_even_price: Option<Decimal>,
    #[serde(default)]
    pub mark_price: Decimal,
    #[serde(default, rename = "unRealizedProfit")]
    pub unrealized_profit: Decimal,
    #[serde(default)]
    pub liquidation_price: Decimal,
    #[serde(default)]
    pub notional: Decimal,
    #[serde(default)]
    pub margin_asset: Option<String>,
    #[serde(default)]
    pub isolated_margin: Option<Decimal>,
    #[serde(default)]
    pub isolated_wallet: Option<Decimal>,
    #[serde(default)]
    pub initial_margin: Option<Decimal>,
    #[serde(default)]
    pub maint_margin: Option<Decimal>,
    #[serde(default)]
    pub position_initial_margin: Option<Decimal>,
    #[serde(default)]
    pub open_order_initial_margin: Option<Decimal>,
    #[serde(default)]
    pub adl: Option<i64>,
    #[serde(default)]
    pub bid_notional: Option<Decimal>,
    #[serde(default)]
    pub ask_notional: Option<Decimal>,
    /// v2 接口才有：杠杆倍数（字符串）。
    #[serde(default)]
    pub leverage: Option<String>,
    /// v2 接口才有：`cross` / `isolated`。
    #[serde(default)]
    pub margin_type: Option<String>,
    #[serde(default)]
    pub update_time: i64,
}

/// 账户余额（`/fapi/v3/balance`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    #[serde(default)]
    pub account_alias: String,
    pub asset: String,
    pub balance: Decimal,
    #[serde(default)]
    pub cross_wallet_balance: Decimal,
    #[serde(default, rename = "crossUnPnl")]
    pub cross_unrealized_pnl: Decimal,
    #[serde(default)]
    pub available_balance: Decimal,
    #[serde(default)]
    pub max_withdraw_amount: Decimal,
    #[serde(default)]
    pub margin_available: bool,
    #[serde(default)]
    pub update_time: i64,
}

/// 账户信息（`/fapi/v3/account`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(default)]
    pub total_initial_margin: Decimal,
    #[serde(default)]
    pub total_maint_margin: Decimal,
    #[serde(default)]
    pub total_wallet_balance: Decimal,
    #[serde(default)]
    pub total_unrealized_profit: Decimal,
    #[serde(default)]
    pub total_margin_balance: Decimal,
    #[serde(default)]
    pub total_position_initial_margin: Decimal,
    #[serde(default)]
    pub total_open_order_initial_margin: Decimal,
    #[serde(default)]
    pub total_cross_wallet_balance: Decimal,
    #[serde(default, rename = "totalCrossUnPnl")]
    pub total_cross_unrealized_pnl: Decimal,
    #[serde(default)]
    pub available_balance: Decimal,
    #[serde(default)]
    pub max_withdraw_amount: Decimal,
    #[serde(default)]
    pub assets: Vec<AccountAsset>,
    #[serde(default)]
    pub positions: Vec<AccountPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountAsset {
    pub asset: String,
    #[serde(default)]
    pub wallet_balance: Decimal,
    #[serde(default)]
    pub unrealized_profit: Decimal,
    #[serde(default)]
    pub margin_balance: Decimal,
    #[serde(default)]
    pub maint_margin: Decimal,
    #[serde(default)]
    pub initial_margin: Decimal,
    #[serde(default)]
    pub position_initial_margin: Decimal,
    #[serde(default)]
    pub open_order_initial_margin: Decimal,
    #[serde(default)]
    pub cross_wallet_balance: Decimal,
    #[serde(default, rename = "crossUnPnl")]
    pub cross_unrealized_pnl: Decimal,
    #[serde(default)]
    pub available_balance: Decimal,
    #[serde(default)]
    pub max_withdraw_amount: Decimal,
    #[serde(default)]
    pub update_time: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountPosition {
    pub symbol: String,
    #[serde(default)]
    pub position_side: Option<PositionSide>,
    #[serde(default)]
    pub position_amt: Decimal,
    #[serde(default)]
    pub unrealized_profit: Decimal,
    #[serde(default)]
    pub isolated_margin: Decimal,
    #[serde(default)]
    pub notional: Decimal,
    #[serde(default)]
    pub isolated_wallet: Decimal,
    #[serde(default)]
    pub initial_margin: Decimal,
    #[serde(default)]
    pub maint_margin: Decimal,
    #[serde(default)]
    pub update_time: i64,
}

/// 手续费率（`/fapi/v1/commissionRate`），例如 0.0002 = 2bp。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommissionRate {
    pub symbol: String,
    pub maker_commission_rate: Decimal,
    pub taker_commission_rate: Decimal,
}

/// 调整杠杆的响应。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeverageChange {
    pub symbol: String,
    pub leverage: u32,
    #[serde(default)]
    pub max_notional_value: Option<String>,
}

/// 资金流水（`/fapi/v1/income`）：资金费、手续费、已实现盈亏、划转等。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Income {
    #[serde(default)]
    pub symbol: String,
    pub income_type: String,
    pub income: Decimal,
    pub asset: String,
    #[serde(default)]
    pub info: String,
    pub time: i64,
    pub tran_id: i64,
    #[serde(default)]
    pub trade_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListenKey {
    #[serde(rename = "listenKey")]
    pub listen_key: String,
}

// ---------------------------------------------------------------------------
// 响应：行情
// ---------------------------------------------------------------------------

/// 最优买卖价（`/fapi/v1/ticker/bookTicker`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookTicker {
    pub symbol: String,
    pub bid_price: Decimal,
    pub bid_qty: Decimal,
    pub ask_price: Decimal,
    pub ask_qty: Decimal,
    #[serde(default)]
    pub time: i64,
    #[serde(default)]
    pub last_update_id: Option<i64>,
}

/// 一档价格和数量。交易所返回 `["价格","数量"]`。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct Level(pub Decimal, pub Decimal);

impl Level {
    pub fn price(&self) -> Decimal {
        self.0
    }
    pub fn qty(&self) -> Decimal {
        self.1
    }
}

/// 深度快照（`/fapi/v1/depth`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OrderBook {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: i64,
    /// 消息时间。
    #[serde(rename = "E", default)]
    pub event_time: i64,
    /// 撮合时间。
    #[serde(rename = "T", default)]
    pub transaction_time: i64,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

/// 最新价（`/fapi/v2/ticker/price`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PriceTicker {
    pub symbol: String,
    pub price: Decimal,
    #[serde(default)]
    pub time: i64,
}

/// 24 小时行情（`/fapi/v1/ticker/24hr`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ticker24h {
    pub symbol: String,
    pub price_change: Decimal,
    pub price_change_percent: Decimal,
    pub weighted_avg_price: Decimal,
    pub last_price: Decimal,
    #[serde(default)]
    pub last_qty: Decimal,
    pub open_price: Decimal,
    pub high_price: Decimal,
    pub low_price: Decimal,
    pub volume: Decimal,
    pub quote_volume: Decimal,
    pub open_time: i64,
    pub close_time: i64,
    #[serde(default)]
    pub first_id: i64,
    #[serde(default)]
    pub last_id: i64,
    #[serde(default)]
    pub count: i64,
}

/// K 线（`/fapi/v1/klines`）。交易所返回数组，这里按位置解析。
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Kline {
    pub open_time: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub close_time: i64,
    pub quote_volume: Decimal,
    pub trades: i64,
    pub taker_buy_volume: Decimal,
    pub taker_buy_quote_volume: Decimal,
}

impl<'de> Deserialize<'de> for Kline {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw(
            i64,
            Decimal,
            Decimal,
            Decimal,
            Decimal,
            Decimal,
            i64,
            Decimal,
            i64,
            Decimal,
            Decimal,
            #[serde(default)] serde::de::IgnoredAny,
        );
        let raw = Raw::deserialize(deserializer)?;
        Ok(Kline {
            open_time: raw.0,
            open: raw.1,
            high: raw.2,
            low: raw.3,
            close: raw.4,
            volume: raw.5,
            close_time: raw.6,
            quote_volume: raw.7,
            trades: raw.8,
            taker_buy_volume: raw.9,
            taker_buy_quote_volume: raw.10,
        })
    }
}

/// 标记价格和资金费（`/fapi/v1/premiumIndex`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PremiumIndex {
    pub symbol: String,
    pub mark_price: Decimal,
    pub index_price: Decimal,
    #[serde(default)]
    pub estimated_settle_price: Decimal,
    /// 当期资金费率（下次结算预估）。
    pub last_funding_rate: Decimal,
    #[serde(default)]
    pub interest_rate: Decimal,
    pub next_funding_time: i64,
    pub time: i64,
}

/// 资金费历史（`/fapi/v1/fundingRate`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FundingRate {
    pub symbol: String,
    pub funding_rate: Decimal,
    pub funding_time: i64,
    #[serde(default, deserialize_with = "lenient_decimal")]
    pub mark_price: Option<Decimal>,
}

/// 被调整过资金费上下限或结算间隔的合约（`/fapi/v1/fundingInfo`）。不在列表里的合约是默认 8 小时。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FundingInfo {
    pub symbol: String,
    pub adjusted_funding_rate_cap: Decimal,
    pub adjusted_funding_rate_floor: Decimal,
    pub funding_interval_hours: u32,
    #[serde(default)]
    pub disclaimer: bool,
}

/// 持仓量（`/fapi/v1/openInterest`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenInterest {
    pub symbol: String,
    pub open_interest: Decimal,
    pub time: i64,
}

/// 归集成交（`/fapi/v1/aggTrades`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AggTrade {
    #[serde(rename = "a")]
    pub id: i64,
    #[serde(rename = "p")]
    pub price: Decimal,
    #[serde(rename = "q")]
    pub qty: Decimal,
    #[serde(rename = "f")]
    pub first_trade_id: i64,
    #[serde(rename = "l")]
    pub last_trade_id: i64,
    #[serde(rename = "T")]
    pub time: i64,
    /// 买方是否挂单方（true = 主动卖出）。
    #[serde(rename = "m")]
    pub buyer_is_maker: bool,
}

/// 解析可能是空字符串、"null" 或缺失的十进制字段。
pub(crate) fn lenient_decimal<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Decimal>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(text)) => text.parse().ok(),
        Some(serde_json::Value::Number(number)) => number.to_string().parse().ok(),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn enums_round_trip_and_keep_unknown_values() {
        assert_eq!("GTX".parse::<TimeInForce>().unwrap(), TimeInForce::Gtx);
        let unknown: OrderStatus = serde_json::from_str("\"NEW_STATUS\"").unwrap();
        assert_eq!(unknown, OrderStatus::Other("NEW_STATUS".into()));
        assert_eq!(
            serde_json::to_string(&PriceMatch::Queue5).unwrap(),
            "\"QUEUE_5\""
        );
        assert!(OrderStatus::PartiallyFilled.is_open());
        assert!(OrderStatus::ExpiredInMatch.is_final());
    }

    #[test]
    fn post_only_order_params() {
        let order = NewOrder::post_only("BTCUSDT", Side::Buy, d("0.0020"), d("65000.10"))
            .position_side(PositionSide::Long)
            .client_order_id("abc-1");
        order.validate().unwrap();
        let params = order.to_params();
        assert_eq!(params.get("type"), Some("LIMIT"));
        assert_eq!(params.get("timeInForce"), Some("GTX"));
        assert_eq!(params.get("quantity"), Some("0.002"));
        assert_eq!(params.get("price"), Some("65000.1"));
        assert_eq!(params.get("positionSide"), Some("LONG"));
        assert_eq!(params.get("newClientOrderId"), Some("abc-1"));
        assert_eq!(params.get("reduceOnly"), None);
    }

    #[test]
    fn order_validation() {
        assert!(NewOrder::market("BTCUSDT", Side::Sell, d("0"))
            .validate()
            .is_err());
        assert!(NewOrder::limit("BTCUSDT", Side::Sell, d("1"), d("1"))
            .client_order_id("中文")
            .validate()
            .is_err());
        let mut stop = NewOrder::market("BTCUSDT", Side::Sell, d("1"));
        stop.order_type = OrderType::StopMarket;
        assert!(stop.validate().is_err());
        let matched = NewOrder::post_only_matched("BTCUSDT", Side::Buy, d("1"), PriceMatch::Queue);
        matched.validate().unwrap();
        assert_eq!(matched.to_params().get("priceMatch"), Some("QUEUE"));
        assert_eq!(matched.to_params().get("price"), None);
    }

    #[test]
    fn algo_order_params_and_validation() {
        let stop = NewAlgoOrder::stop_market("ETHUSDT", Side::Sell, d("2500.5"))
            .close_position()
            .working_type(WorkingType::MarkPrice)
            .client_algo_id("guard-eth");
        stop.validate().unwrap();
        let params = stop.to_params();
        assert_eq!(params.get("algoType"), Some("CONDITIONAL"));
        assert_eq!(params.get("type"), Some("STOP_MARKET"));
        assert_eq!(params.get("triggerPrice"), Some("2500.5"));
        assert_eq!(params.get("closePosition"), Some("true"));
        assert!(NewAlgoOrder::stop_market("ETHUSDT", Side::Sell, d("1"))
            .validate()
            .is_err());
        assert!(NewAlgoOrder::stop_market("ETHUSDT", Side::Sell, d("1"))
            .close_position()
            .quantity(d("1"))
            .validate()
            .is_err());
    }

    #[test]
    fn parses_order_response() {
        let raw = r#"{"clientOrderId":"testOrder","cumQty":"0","cumQuote":"0","executedQty":"0","orderId":22542179,
            "avgPrice":"0.00000","origQty":"10","price":"0","reduceOnly":false,"side":"BUY","positionSide":"SHORT",
            "status":"NEW","stopPrice":"9300","closePosition":false,"symbol":"BTCUSDT","timeInForce":"GTD",
            "type":"TRAILING_STOP_MARKET","origType":"TRAILING_STOP_MARKET","activatePrice":"9020","priceRate":"0.3",
            "updateTime":1566818724722,"workingType":"CONTRACT_PRICE","priceProtect":false,"priceMatch":"NONE",
            "selfTradePreventionMode":"NONE","goodTillDate":1693207680000}"#;
        let order: Order = serde_json::from_str(raw).unwrap();
        assert_eq!(order.order_id, 22542179);
        assert_eq!(order.status, OrderStatus::New);
        assert_eq!(order.remaining_qty(), d("10"));
        assert_eq!(order.fill_price(), None);
        assert_eq!(order.time_in_force, Some(TimeInForce::Gtd));
    }

    #[test]
    fn parses_batch_results() {
        let raw = r#"[{"orderId":1,"symbol":"BTCUSDT","status":"NEW","clientOrderId":"a","type":"LIMIT","side":"BUY","origQty":"1","price":"1"},
                      {"code":-2022,"msg":"ReduceOnly Order is rejected."}]"#;
        let items: Vec<BatchItem> = serde_json::from_str(raw).unwrap();
        assert!(matches!(&items[0], BatchItem::Ok(order) if order.order_id == 1));
        assert_eq!(
            items[1],
            BatchItem::Err {
                code: -2022,
                msg: "ReduceOnly Order is rejected.".into()
            }
        );
    }

    #[test]
    fn parses_market_data() {
        let kline: Vec<Kline> = serde_json::from_str(
            r#"[[1499040000000,"0.01634790","0.80000000","0.01575800","0.01577100","148976.11427815",1499644799999,"2434.19055334",308,"1756.87402397","28.46694368","17928899.62484339"]]"#,
        )
        .unwrap();
        assert_eq!(kline[0].trades, 308);
        assert_eq!(kline[0].close, d("0.01577100"));
        let book: OrderBook = serde_json::from_str(
            r#"{"lastUpdateId":1027024,"E":1589436922972,"T":1589436922959,"bids":[["4.00000000","431.00000000"]],"asks":[["4.00000200","12.00000000"]]}"#,
        )
        .unwrap();
        assert_eq!(book.bids[0].price(), d("4"));
        assert_eq!(book.asks[0].qty(), d("12"));
        let premium: PremiumIndex = serde_json::from_str(
            r#"{"symbol":"BTCUSDT","markPrice":"11793.63104562","indexPrice":"11781.80495970","estimatedSettlePrice":"11781.16138815","lastFundingRate":"0.00038246","interestRate":"0.00010000","nextFundingTime":1597392000000,"time":1597370495002}"#,
        )
        .unwrap();
        assert_eq!(premium.last_funding_rate, d("0.00038246"));
    }

    #[test]
    fn parses_algo_order() {
        let raw = r#"{"algoId":3000000000003505,"clientAlgoId":"0Xkl1p621E4EryvufmYre1","algoType":"CONDITIONAL","orderType":"TAKE_PROFIT",
            "symbol":"BTCUSDT","side":"SELL","positionSide":"SHORT","timeInForce":"GTC","quantity":"1.000","algoStatus":"NEW",
            "triggerPrice":"120000.00","price":"160000.00","icebergQuantity":"null","selfTradePreventionMode":"EXPIRE_MAKER",
            "workingType":"CONTRACT_PRICE","priceMatch":"NONE","closePosition":false,"priceProtect":false,"reduceOnly":false,
            "createTime":1762507264142,"updateTime":1762507264143,"triggerTime":0,"goodTillDate":0}"#;
        let order: AlgoOrder = serde_json::from_str(raw).unwrap();
        assert_eq!(order.algo_status, Some(AlgoStatus::New));
        assert_eq!(order.trigger_price, Some(d("120000")));
        assert_eq!(order.order_type, OrderType::TakeProfit);
    }
}
