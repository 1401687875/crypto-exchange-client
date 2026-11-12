//! WebSocket 推送的数据结构：用户数据流事件和行情事件。
//!
//! 推送字段名是交易所的单字母缩写，这里逐个映射成可读的字段名；
//! 不认识的事件类型原样放在 `Other(serde_json::Value)` 里。

use super::models::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// 用户数据流
// ---------------------------------------------------------------------------

/// 用户数据流事件。
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum UserDataEvent {
    /// 订单变化：新建、成交、撤销、过期、改单。挂单是否成交以它为准。
    OrderTradeUpdate(OrderTradeUpdate),
    /// 余额和持仓变化。
    AccountUpdate(AccountUpdate),
    /// 精简的成交推送（比 `OrderTradeUpdate` 更快，字段更少）。
    TradeLite(TradeLite),
    /// 追加保证金通知。
    MarginCall(Value),
    /// 杠杆倍数或联合保证金模式变化。
    AccountConfigUpdate(Value),
    /// 条件单变化。
    AlgoUpdate(Value),
    /// 条件单触发后被拒。
    ConditionalOrderTriggerReject(Value),
    /// listenKey 已过期，需要重新创建（[`super::user_stream`] 会自动处理）。
    ListenKeyExpired { event_time: i64 },
    /// 本库不认识的事件。
    Other(Value),
}

impl UserDataEvent {
    /// 解析一条推送（兼容 `{"stream":..,"data":..}` 包装）。
    pub fn parse(text: &str) -> serde_json::Result<Self> {
        let value: Value = serde_json::from_str(text)?;
        Self::from_value(unwrap_combined(value))
    }

    pub fn from_value(value: Value) -> serde_json::Result<Self> {
        let kind = value
            .get("e")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(match kind.as_str() {
            "ORDER_TRADE_UPDATE" => UserDataEvent::OrderTradeUpdate(serde_json::from_value(value)?),
            "ACCOUNT_UPDATE" => UserDataEvent::AccountUpdate(serde_json::from_value(value)?),
            "TRADE_LITE" => UserDataEvent::TradeLite(serde_json::from_value(value)?),
            "MARGIN_CALL" => UserDataEvent::MarginCall(value),
            "ACCOUNT_CONFIG_UPDATE" => UserDataEvent::AccountConfigUpdate(value),
            "ALGO_UPDATE" => UserDataEvent::AlgoUpdate(value),
            "CONDITIONAL_ORDER_TRIGGER_REJECT" => {
                UserDataEvent::ConditionalOrderTriggerReject(value)
            }
            "listenKeyExpired" => UserDataEvent::ListenKeyExpired {
                event_time: value.get("E").and_then(Value::as_i64).unwrap_or_default(),
            },
            _ => UserDataEvent::Other(value),
        })
    }
}

/// `ORDER_TRADE_UPDATE` 事件。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OrderTradeUpdate {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "T", default)]
    pub transaction_time: i64,
    #[serde(rename = "o")]
    pub order: OrderUpdate,
}

/// 订单推送的内容。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OrderUpdate {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "c")]
    pub client_order_id: String,
    #[serde(rename = "S")]
    pub side: Side,
    #[serde(rename = "o")]
    pub order_type: OrderType,
    #[serde(rename = "f")]
    pub time_in_force: TimeInForce,
    /// 委托数量。
    #[serde(rename = "q")]
    pub orig_qty: Decimal,
    /// 委托价格。
    #[serde(rename = "p")]
    pub price: Decimal,
    /// 成交均价。
    #[serde(rename = "ap")]
    pub avg_price: Decimal,
    #[serde(rename = "sp", default)]
    pub stop_price: Decimal,
    /// 本次事件的执行类型。
    #[serde(rename = "x")]
    pub execution_type: ExecutionType,
    /// 订单当前状态。
    #[serde(rename = "X")]
    pub status: OrderStatus,
    #[serde(rename = "i")]
    pub order_id: i64,
    /// 本次成交数量。
    #[serde(rename = "l")]
    pub last_filled_qty: Decimal,
    /// 累计成交数量。
    #[serde(rename = "z")]
    pub cum_filled_qty: Decimal,
    /// 本次成交价格。
    #[serde(rename = "L")]
    pub last_filled_price: Decimal,
    /// 手续费资产（没有成交时可能缺失）。
    #[serde(rename = "N", default)]
    pub commission_asset: Option<String>,
    /// 本次成交的手续费。
    #[serde(rename = "n", default, deserialize_with = "lenient_decimal")]
    pub commission: Option<Decimal>,
    #[serde(rename = "T")]
    pub trade_time: i64,
    #[serde(rename = "t", default)]
    pub trade_id: i64,
    #[serde(rename = "b", default)]
    pub bids_notional: Decimal,
    #[serde(rename = "a", default)]
    pub asks_notional: Decimal,
    /// 本次成交是否挂单方（Maker）。
    #[serde(rename = "m", default)]
    pub is_maker: bool,
    #[serde(rename = "R", default)]
    pub reduce_only: bool,
    #[serde(rename = "wt", default)]
    pub working_type: Option<WorkingType>,
    #[serde(rename = "ot", default)]
    pub orig_type: Option<OrderType>,
    #[serde(rename = "ps", default)]
    pub position_side: Option<PositionSide>,
    #[serde(rename = "cp", default)]
    pub close_position: bool,
    #[serde(rename = "AP", default, deserialize_with = "lenient_decimal")]
    pub activation_price: Option<Decimal>,
    #[serde(rename = "cr", default, deserialize_with = "lenient_decimal")]
    pub callback_rate: Option<Decimal>,
    #[serde(rename = "pP", default)]
    pub price_protect: bool,
    /// 这笔成交的已实现盈亏。
    #[serde(rename = "rp", default)]
    pub realized_profit: Decimal,
    #[serde(rename = "V", default)]
    pub self_trade_prevention: Option<SelfTradePrevention>,
    #[serde(rename = "pm", default)]
    pub price_match: Option<PriceMatch>,
    #[serde(rename = "gtd", default)]
    pub good_till_date: i64,
}

impl OrderUpdate {
    /// 这条推送是否带一笔新的成交。
    pub fn is_fill(&self) -> bool {
        self.execution_type == ExecutionType::Trade && self.last_filled_qty > Decimal::ZERO
    }

    pub fn remaining_qty(&self) -> Decimal {
        (self.orig_qty - self.cum_filled_qty).max(Decimal::ZERO)
    }
}

/// `ACCOUNT_UPDATE` 事件。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AccountUpdate {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "T", default)]
    pub transaction_time: i64,
    #[serde(rename = "a")]
    pub data: AccountUpdateData,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AccountUpdateData {
    /// 变化原因：`ORDER`、`FUNDING_FEE`、`DEPOSIT`、`WITHDRAW` 等。
    #[serde(rename = "m")]
    pub reason: String,
    #[serde(rename = "B", default)]
    pub balances: Vec<BalanceUpdate>,
    #[serde(rename = "P", default)]
    pub positions: Vec<PositionUpdate>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BalanceUpdate {
    #[serde(rename = "a")]
    pub asset: String,
    #[serde(rename = "wb")]
    pub wallet_balance: Decimal,
    #[serde(rename = "cw", default)]
    pub cross_wallet_balance: Decimal,
    /// 除盈亏和手续费外的余额变化。
    #[serde(rename = "bc", default)]
    pub balance_change: Decimal,
}

/// 持仓变化。只推送有变化的持仓。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PositionUpdate {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "pa")]
    pub position_amt: Decimal,
    #[serde(rename = "ep")]
    pub entry_price: Decimal,
    #[serde(rename = "bep", default)]
    pub break_even_price: Decimal,
    #[serde(rename = "cr", default)]
    pub accumulated_realized: Decimal,
    #[serde(rename = "up", default)]
    pub unrealized_pnl: Decimal,
    /// `isolated` / `cross`。
    #[serde(rename = "mt", default)]
    pub margin_type: String,
    #[serde(rename = "iw", default)]
    pub isolated_wallet: Decimal,
    #[serde(rename = "ps")]
    pub position_side: PositionSide,
}

/// `TRADE_LITE` 事件。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TradeLite {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "T")]
    pub transaction_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "q")]
    pub orig_qty: Decimal,
    #[serde(rename = "p")]
    pub price: Decimal,
    #[serde(rename = "m")]
    pub is_maker: bool,
    #[serde(rename = "c")]
    pub client_order_id: String,
    #[serde(rename = "S")]
    pub side: Side,
    #[serde(rename = "L")]
    pub last_filled_price: Decimal,
    #[serde(rename = "l")]
    pub last_filled_qty: Decimal,
    #[serde(rename = "t")]
    pub trade_id: i64,
    #[serde(rename = "i")]
    pub order_id: i64,
}

// ---------------------------------------------------------------------------
// 行情推送
// ---------------------------------------------------------------------------

/// 行情推送事件。
#[derive(Clone, Debug, PartialEq)]
pub enum MarketEvent {
    BookTicker(BookTickerEvent),
    /// 增量深度和有限档深度都是这个结构。
    Depth(DepthEvent),
    AggTrade(AggTradeEvent),
    MarkPrice(MarkPriceEvent),
    /// `!markPrice@arr`：全部合约的标记价格。
    MarkPrices(Vec<MarkPriceEvent>),
    Kline(KlineEvent),
    Other(Value),
}

impl MarketEvent {
    pub fn from_value(value: Value) -> serde_json::Result<Self> {
        if let Value::Array(items) = &value {
            if items
                .iter()
                .all(|item| item.get("e").and_then(Value::as_str) == Some("markPriceUpdate"))
                && !items.is_empty()
            {
                return Ok(MarketEvent::MarkPrices(serde_json::from_value(value)?));
            }
            return Ok(MarketEvent::Other(value));
        }
        let kind = value
            .get("e")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(match kind.as_str() {
            "bookTicker" => MarketEvent::BookTicker(serde_json::from_value(value)?),
            "depthUpdate" => MarketEvent::Depth(serde_json::from_value(value)?),
            "aggTrade" => MarketEvent::AggTrade(serde_json::from_value(value)?),
            "markPriceUpdate" => MarketEvent::MarkPrice(serde_json::from_value(value)?),
            "kline" => MarketEvent::Kline(serde_json::from_value(value)?),
            _ => MarketEvent::Other(value),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BookTickerEvent {
    #[serde(rename = "u")]
    pub update_id: i64,
    #[serde(rename = "E", default)]
    pub event_time: i64,
    #[serde(rename = "T", default)]
    pub transaction_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub bid_price: Decimal,
    #[serde(rename = "B")]
    pub bid_qty: Decimal,
    #[serde(rename = "a")]
    pub ask_price: Decimal,
    #[serde(rename = "A")]
    pub ask_qty: Decimal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DepthEvent {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "T", default)]
    pub transaction_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: i64,
    #[serde(rename = "u")]
    pub final_update_id: i64,
    /// 上一条推送的 `final_update_id`，用于检查是否漏了推送。
    #[serde(rename = "pu", default)]
    pub prev_final_update_id: i64,
    #[serde(rename = "b")]
    pub bids: Vec<Level>,
    #[serde(rename = "a")]
    pub asks: Vec<Level>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AggTradeEvent {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
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
    pub trade_time: i64,
    /// true = 买方是挂单方，即主动卖出。
    #[serde(rename = "m")]
    pub buyer_is_maker: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct MarkPriceEvent {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub mark_price: Decimal,
    #[serde(rename = "i", default)]
    pub index_price: Decimal,
    #[serde(rename = "P", default)]
    pub estimated_settle_price: Decimal,
    #[serde(rename = "r", default, deserialize_with = "lenient_decimal")]
    pub funding_rate: Option<Decimal>,
    #[serde(rename = "T", default)]
    pub next_funding_time: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KlineEvent {
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: KlineData,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KlineData {
    #[serde(rename = "t")]
    pub open_time: i64,
    #[serde(rename = "T")]
    pub close_time: i64,
    #[serde(rename = "i")]
    pub interval: String,
    #[serde(rename = "o")]
    pub open: Decimal,
    #[serde(rename = "c")]
    pub close: Decimal,
    #[serde(rename = "h")]
    pub high: Decimal,
    #[serde(rename = "l")]
    pub low: Decimal,
    #[serde(rename = "v")]
    pub volume: Decimal,
    #[serde(rename = "n")]
    pub trades: i64,
    /// 这根 K 线是否已收盘。
    #[serde(rename = "x")]
    pub is_closed: bool,
    #[serde(rename = "q")]
    pub quote_volume: Decimal,
    #[serde(rename = "V")]
    pub taker_buy_volume: Decimal,
    #[serde(rename = "Q")]
    pub taker_buy_quote_volume: Decimal,
}

/// 去掉组合流的 `{"stream":..,"data":..}` 外层。
pub(crate) fn unwrap_combined(value: Value) -> Value {
    match value {
        Value::Object(mut map) if map.contains_key("stream") && map.contains_key("data") => {
            map.remove("data").unwrap_or(Value::Null)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn parses_order_trade_update() {
        let raw = r#"{"e":"ORDER_TRADE_UPDATE","E":1568879465651,"T":1568879465650,"o":{"s":"BTCUSDT","c":"TEST","S":"SELL",
            "o":"LIMIT","f":"GTX","q":"0.001","p":"7103.04","ap":"7103.04","sp":"0","x":"TRADE","X":"FILLED","i":8886774,
            "l":"0.001","z":"0.001","L":"7103.04","N":"USDT","n":"0.00142","T":1568879465650,"t":12345,"b":"0","a":"9.91",
            "m":true,"R":false,"wt":"CONTRACT_PRICE","ot":"LIMIT","ps":"BOTH","cp":false,"rp":"0.5","pP":false,"si":0,"ss":0,
            "V":"EXPIRE_TAKER","pm":"NONE","gtd":0}}"#;
        let UserDataEvent::OrderTradeUpdate(event) = UserDataEvent::parse(raw).unwrap() else {
            panic!()
        };
        let order = event.order;
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.time_in_force, TimeInForce::Gtx);
        assert!(order.is_fill());
        assert!(order.is_maker);
        assert_eq!(order.commission, Some(d("0.00142")));
        assert_eq!(order.remaining_qty(), Decimal::ZERO);
        assert_eq!(order.realized_profit, d("0.5"));
    }

    #[test]
    fn parses_new_order_without_commission() {
        let raw = r#"{"stream":"x","data":{"e":"ORDER_TRADE_UPDATE","E":1,"T":1,"o":{"s":"ETHUSDT","c":"a","S":"BUY","o":"LIMIT",
            "f":"GTC","q":"1","p":"2000","ap":"0","sp":"0","x":"NEW","X":"NEW","i":1,"l":"0","z":"0","L":"0","T":1,"t":0,
            "b":"2000","a":"0","m":false,"R":false,"wt":"CONTRACT_PRICE","ot":"LIMIT","ps":"LONG","cp":false,"rp":"0"}}}"#;
        let UserDataEvent::OrderTradeUpdate(event) = UserDataEvent::parse(raw).unwrap() else {
            panic!()
        };
        assert_eq!(event.order.commission, None);
        assert!(!event.order.is_fill());
        assert_eq!(event.order.position_side, Some(PositionSide::Long));
    }

    #[test]
    fn parses_account_update_and_others() {
        let raw = r#"{"e":"ACCOUNT_UPDATE","E":1564745798939,"T":1564745798938,"a":{"m":"ORDER",
            "B":[{"a":"USDT","wb":"122624.12345678","cw":"100.12345678","bc":"50.12345678"}],
            "P":[{"s":"BTCUSDT","pa":"-20","ep":"6563.66500","bep":"6563.6","cr":"0","up":"2850.21200","mt":"isolated","iw":"13200.70726908","ps":"BOTH"}]}}"#;
        let UserDataEvent::AccountUpdate(event) = UserDataEvent::parse(raw).unwrap() else {
            panic!()
        };
        assert_eq!(event.data.reason, "ORDER");
        assert_eq!(event.data.positions[0].position_amt, d("-20"));
        assert_eq!(event.data.balances[0].wallet_balance, d("122624.12345678"));
        assert!(matches!(
            UserDataEvent::parse(r#"{"e":"listenKeyExpired","E":1576653824250,"listenKey":"x"}"#)
                .unwrap(),
            UserDataEvent::ListenKeyExpired {
                event_time: 1576653824250
            }
        ));
        assert!(matches!(
            UserDataEvent::parse(r#"{"e":"SOMETHING_NEW","E":1}"#).unwrap(),
            UserDataEvent::Other(_)
        ));
        let lite = r#"{"e":"TRADE_LITE","E":1721895408092,"T":1721895408214,"s":"BTCUSDT","q":"0.001","p":"0","m":false,
            "c":"z8hcUoOsqEdKMeKPSABslD","S":"BUY","L":"64089.20","l":"0.040","t":109100866,"i":8886774}"#;
        assert!(
            matches!(UserDataEvent::parse(lite).unwrap(), UserDataEvent::TradeLite(t) if t.last_filled_qty == d("0.04"))
        );
    }

    #[test]
    fn parses_market_events() {
        let book = r#"{"e":"bookTicker","u":400900217,"E":1568014460893,"T":1568014460891,"s":"BNBUSDT","b":"25.35190000","B":"31.21000000","a":"25.36520000","A":"40.66000000"}"#;
        assert!(matches!(
            MarketEvent::from_value(serde_json::from_str(book).unwrap()).unwrap(),
            MarketEvent::BookTicker(b) if b.ask_price == d("25.3652")
        ));
        let depth = r#"{"e":"depthUpdate","E":123456789,"T":123456788,"s":"BTCUSDT","U":157,"u":160,"pu":149,"b":[["0.0024","10"]],"a":[["0.0026","100"]]}"#;
        assert!(matches!(
            MarketEvent::from_value(serde_json::from_str(depth).unwrap()).unwrap(),
            MarketEvent::Depth(e) if e.prev_final_update_id == 149 && e.bids[0].qty() == d("10")
        ));
        let marks = r#"[{"e":"markPriceUpdate","E":1562305380000,"s":"BTCUSDT","p":"11185.87786614","i":"11784.62659091","P":"11784.25641265","r":"0.00030000","T":1562306400000}]"#;
        assert!(matches!(
            MarketEvent::from_value(serde_json::from_str(marks).unwrap()).unwrap(),
            MarketEvent::MarkPrices(v) if v[0].funding_rate == Some(d("0.0003"))
        ));
        let kline = r#"{"e":"kline","E":1638747660000,"s":"BTCUSDT","k":{"t":1638747660000,"T":1638747719999,"s":"BTCUSDT","i":"1m",
            "f":100,"L":200,"o":"0.0010","c":"0.0020","h":"0.0025","l":"0.0015","v":"1000","n":100,"x":false,"q":"1.0000","V":"500","Q":"0.500","B":"123456"}}"#;
        assert!(matches!(
            MarketEvent::from_value(serde_json::from_str(kline).unwrap()).unwrap(),
            MarketEvent::Kline(k) if !k.kline.is_closed && k.kline.trades == 100
        ));
    }
}
