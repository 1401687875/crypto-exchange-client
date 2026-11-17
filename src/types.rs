use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOrder {
    pub fee: BigDecimal,
    pub sell_price: BigDecimal,
    pub amount: BigDecimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaOrderResponse {
    pub order_id: u64,
    #[serde(default)]
    pub symbol: String,
    pub status: String,
    #[serde(default)]
    pub client_order_id: String,
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub avg_price: String,
    #[serde(default)]
    pub orig_qty: String,
    pub executed_qty: String,
    #[serde(default)]
    pub cum_qty: String,
    #[serde(default)]
    pub cum_quote: String,
    #[serde(default)]
    pub time_in_force: String,
    #[serde(rename = "type", default)]
    pub order_type: String,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub close_position: bool,
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub position_side: String,
    #[serde(default)]
    pub stop_price: String,
    #[serde(default)]
    pub working_type: String,
    #[serde(default)]
    pub price_protect: bool,
    #[serde(default)]
    pub orig_type: String,
    #[serde(default)]
    pub price_match: String,
    #[serde(default)]
    pub self_trade_prevention_mode: String,
    #[serde(default)]
    pub good_till_date: u64,
    #[serde(default)]
    pub update_time: u64,
    pub activate_price: Option<String>,
    pub price_rate: Option<String>,
    #[serde(default)]
    pub fills: Vec<FillInfo>,
}

impl BinaOrderResponse {
    pub fn from_json_str(s: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str::<Self>(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoinOrderResponse {
    #[serde(default)]
    pub client_order_id: String,
    #[serde(default)]
    pub cum_base: String,
    #[serde(default)]
    pub cum_qty: String,
    #[serde(default)]
    pub executed_qty: String,
    pub order_id: i64,
    #[serde(default)]
    pub avg_price: String,
    #[serde(default)]
    pub orig_qty: String,
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub close_position: bool,
    pub side: String,
    #[serde(default)]
    pub position_side: String,
    pub status: String,
    #[serde(default)]
    pub stop_price: String,
    pub symbol: String,
    #[serde(default)]
    pub pair: String,
    #[serde(default)]
    pub time_in_force: String,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(default)]
    pub orig_type: String,
    #[serde(default)]
    pub activate_price: Option<String>,
    #[serde(default)]
    pub price_rate: Option<String>,
    #[serde(default)]
    pub update_time: i64,
    #[serde(default)]
    pub working_type: String,
    #[serde(default)]
    pub price_protect: bool,
    #[serde(default)]
    pub price_match: Option<String>,
    #[serde(default)]
    pub self_trade_prevention_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotOrderResponse {
    pub symbol: String,
    pub order_id: i64,
    #[serde(default)]
    pub client_order_id: String,
    pub transact_time: i64,
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub orig_qty: String,
    #[serde(default)]
    pub executed_qty: String,
    #[serde(default)]
    pub cummulative_quote_qty: String,
    pub status: String,
    #[serde(default)]
    pub time_in_force: String,
    #[serde(rename = "type", default)]
    pub order_type: String,
    pub side: String,
    #[serde(default)]
    pub fills: Vec<FillInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillInfo {
    pub price: String,
    pub qty: String,
    pub commission: String,
    pub commission_asset: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OkxResponse<T> {
    pub code: String,
    pub msg: String,
    pub data: Vec<T>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlaceOrderResponseData {
    #[serde(rename = "clOrdId")]
    pub cl_ord_id: String,
    #[serde(rename = "ordId")]
    pub ord_id: String,
    #[serde(rename = "sCode")]
    pub s_code: String,
    #[serde(rename = "sMsg")]
    pub s_msg: String,
    pub tag: String,
    pub ts: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderRequest {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
    pub ord_id: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderResponseData {
    #[serde(rename = "clOrdId")]
    pub cl_ord_id: String,
    #[serde(rename = "ordId")]
    pub ord_id: String,
    #[serde(rename = "sCode")]
    pub s_code: String,
    #[serde(rename = "sMsg")]
    pub s_msg: String,
    pub ts: String,
}

fn deserialize_empty_string_as_none<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    Ok(s.filter(|s| !s.is_empty()))
}

fn deserialize_empty_string_as_none_decimal<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<BigDecimal>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(s) if !s.is_empty() => s
            .parse::<BigDecimal>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        _ => Ok(None),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstrumentData {
    #[serde(rename = "instType")]
    pub inst_type: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub uly: Option<String>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub category: Option<String>,
    #[serde(rename = "baseCcy", default)]
    pub base_ccy: String,
    #[serde(rename = "quoteCcy", default)]
    pub quote_ccy: String,
    #[serde(
        rename = "settleCcy",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub settle_ccy: Option<String>,
    #[serde(
        rename = "ctVal",
        default,
        deserialize_with = "deserialize_empty_string_as_none_decimal"
    )]
    pub ct_val: Option<BigDecimal>,
    #[serde(
        rename = "ctMult",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub ct_mult: Option<String>,
    #[serde(
        rename = "ctValCcy",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub ct_val_ccy: Option<String>,
    #[serde(
        rename = "optType",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub opt_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub stk: Option<String>,
    #[serde(rename = "listTime", default)]
    pub list_time: String,
    #[serde(
        rename = "expTime",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub exp_time: Option<String>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub lever: Option<String>,
    #[serde(rename = "tickSz")]
    pub tick_sz: String,
    #[serde(rename = "lotSz")]
    pub lot_sz: String,
    #[serde(rename = "minSz")]
    pub min_sz: String,
    #[serde(
        rename = "ctType",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub ct_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub alias: Option<String>,
    pub state: String,
    #[serde(
        rename = "maxLmtSz",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub max_lmt_sz: Option<String>,
    #[serde(
        rename = "maxMktSz",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub max_mkt_sz: Option<String>,
    #[serde(
        rename = "maxTwapSz",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub max_twap_sz: Option<String>,
    #[serde(
        rename = "maxIcebergSz",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub max_iceberg_sz: Option<String>,
    #[serde(
        rename = "maxTriggerSz",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub max_trigger_sz: Option<String>,
    #[serde(
        rename = "maxStopSz",
        default,
        deserialize_with = "deserialize_empty_string_as_none"
    )]
    pub max_stop_sz: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderData {
    #[serde(rename = "instType")]
    pub inst_type: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "ccy")]
    pub ccy: Option<String>,
    #[serde(rename = "ordId")]
    pub ord_id: String,
    #[serde(rename = "clOrdId")]
    pub cl_ord_id: Option<String>,
    pub tag: Option<String>,
    pub px: String,
    pub sz: String,
    pub pnl: String,
    #[serde(rename = "ordType")]
    pub ord_type: String,
    pub side: String,
    #[serde(rename = "posSide")]
    pub pos_side: Option<String>,
    #[serde(rename = "tdMode")]
    pub td_mode: String,
    #[serde(rename = "accFillSz")]
    pub acc_fill_sz: String,
    #[serde(rename = "fillPx")]
    pub fill_px: String,
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    #[serde(rename = "fillSz")]
    pub fill_sz: String,
    #[serde(rename = "fillTime")]
    pub fill_time: String,
    pub state: String,
    #[serde(rename = "avgPx")]
    pub avg_px: String,
    pub lever: String,
    #[serde(rename = "tpTriggerPx")]
    pub tp_trigger_px: Option<String>,
    #[serde(rename = "tpOrdPx")]
    pub tp_ord_px: Option<String>,
    #[serde(rename = "slTriggerPx")]
    pub sl_trigger_px: Option<String>,
    #[serde(rename = "slOrdPx")]
    pub sl_ord_px: Option<String>,
    #[serde(rename = "feeCcy")]
    pub fee_ccy: String,
    pub fee: String,
    #[serde(rename = "rebateCcy")]
    pub rebate_ccy: String,
    pub rebate: String,
    #[serde(rename = "tgtCcy")]
    pub tgt_ccy: Option<String>,
    pub source: Option<String>,
    pub category: Option<String>,
    #[serde(rename = "uTime")]
    pub u_time: String,
    #[serde(rename = "cTime")]
    pub c_time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaceOrderRequest {
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "tdMode")]
    pub td_mode: String,
    pub side: String,
    #[serde(rename = "ordType")]
    pub ord_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<String>,
    pub sz: String,
    #[serde(rename = "posSide", skip_serializing_if = "String::is_empty")]
    pub pos_side: String,
    #[serde(rename = "tgtCcy", skip_serializing_if = "Option::is_none")]
    pub tgt_ccy: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OkxTickerData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "last")]
    pub last: String,
    #[serde(rename = "askPx")]
    pub ask_px: String,
    #[serde(rename = "bidPx")]
    pub bid_px: String,
    #[serde(rename = "ts")]
    pub ts: String,
}

#[derive(Debug, Deserialize)]
pub struct OkxKlineData(
    pub String,
    pub String,
    pub String,
    pub String,
    pub String,
    pub String,
    pub String,
    pub String,
    pub String,
);

impl OkxKlineData {
    pub fn open_time(&self) -> std::result::Result<i64, std::num::ParseIntError> {
        self.0.parse::<i64>()
    }

    pub fn open_price(&self) -> &str {
        &self.1
    }

    pub fn high_price(&self) -> &str {
        &self.2
    }

    pub fn low_price(&self) -> &str {
        &self.3
    }

    pub fn close_price(&self) -> &str {
        &self.4
    }

    pub fn volume(&self) -> &str {
        &self.6
    }

    pub fn quote_volume(&self) -> &str {
        &self.7
    }

    pub fn is_closed(&self) -> bool {
        self.8 == "1"
    }
}
