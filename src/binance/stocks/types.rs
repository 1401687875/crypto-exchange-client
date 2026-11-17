use super::{invalid, nonempty, positive, validate_client_id, validate_range};
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Side {
    Buy,
    Sell,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderType {
    Market,
    Limit,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimeInForce {
    Day,
    Gtc,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradingSession {
    #[serde(rename = "RTH")]
    Rth,
    #[serde(rename = "EXTENDED")]
    Extended,
    #[serde(rename = "24H")]
    Hours24,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WalletType {
    Card,
    Main,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConvertType {
    Mint,
    Redeem,
}
pub enum OrderId<'a> {
    Exchange(&'a str),
    Client(&'a str),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrderRequest {
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quote_asset: Option<String>,
    pub price: Option<String>,
    pub quantity: Option<String>,
    pub notional: Option<String>,
    pub time_in_force: Option<TimeInForce>,
    pub trading_session: Option<TradingSession>,
    pub wallet_type: Option<WalletType>,
    pub client_order_id: Option<String>,
    pub tokenize: Option<bool>,
}
impl PlaceOrderRequest {
    pub fn market_buy(symbol: impl Into<String>, notional: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quote_asset: None,
            price: None,
            quantity: None,
            notional: Some(notional.into()),
            time_in_force: None,
            trading_session: None,
            wallet_type: None,
            client_order_id: None,
            tokenize: None,
        }
    }
    pub fn market_sell(symbol: impl Into<String>, quantity: impl Into<String>) -> Self {
        Self {
            side: Side::Sell,
            quantity: Some(quantity.into()),
            notional: None,
            ..Self::market_buy(symbol, "")
        }
    }
    pub fn limit(
        symbol: impl Into<String>,
        side: Side,
        quantity: impl Into<String>,
        price: impl Into<String>,
        session: TradingSession,
    ) -> Self {
        Self {
            side,
            order_type: OrderType::Limit,
            quantity: Some(quantity.into()),
            price: Some(price.into()),
            notional: None,
            trading_session: Some(session),
            ..Self::market_buy(symbol, "")
        }
    }
    pub fn validate(&self) -> Result<()> {
        nonempty(&self.symbol, "symbol")?;
        if self.symbol != self.symbol.to_ascii_uppercase() {
            return invalid("symbol must be uppercase");
        }
        if let Some(id) = &self.client_order_id {
            validate_client_id(id)?;
        }
        for value in [&self.price, &self.quantity, &self.notional]
            .into_iter()
            .flatten()
        {
            positive(value)?;
        }
        if self.price.as_ref().is_some_and(|p| {
            p.split_once('.')
                .is_some_and(|(_, fraction)| fraction.len() > 2)
        }) {
            return invalid("price supports at most two decimal places");
        }
        match (self.side, self.order_type) {
            (_, OrderType::Limit)
                if self.price.is_none()
                    || self.quantity.is_none()
                    || self.trading_session.is_none()
                    || self.notional.is_some() =>
            {
                return invalid(
                    "LIMIT requires price, quantity, tradingSession and forbids notional",
                )
            }
            (Side::Buy, OrderType::Market)
                if self.notional.is_none()
                    || self.quantity.is_some()
                    || self.price.is_some()
                    || self.trading_session.is_some() =>
            {
                return invalid(
                    "BUY MARKET requires notional and forbids quantity, price, tradingSession",
                )
            }
            (Side::Sell, OrderType::Market)
                if self.quantity.is_none()
                    || self.notional.is_some()
                    || self.price.is_some()
                    || self.trading_session.is_some() =>
            {
                return invalid(
                    "SELL MARKET requires quantity and forbids notional, price, tradingSession",
                )
            }
            _ => {}
        }
        if self.time_in_force == Some(TimeInForce::Gtc) {
            if self.order_type != OrderType::Limit {
                return invalid("GTC requires LIMIT");
            }
            let fractional = self.quantity.as_ref().is_some_and(|q| {
                q.split_once('.')
                    .is_some_and(|(_, f)| f.bytes().any(|b| b != b'0'))
            });
            if fractional && self.trading_session == Some(TradingSession::Rth) {
                return invalid("fractional GTC requires EXTENDED or 24H");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRange {
    pub start_time: i64,
    pub end_time: i64,
    pub current: Option<u64>,
    pub size: Option<u64>,
}
impl HistoryRange {
    pub fn validate(&self) -> Result<()> {
        validate_range(
            Some(self.start_time),
            Some(self.end_time),
            self.current,
            self.size,
        )
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderHistoryQuery {
    #[serde(flatten)]
    pub range: HistoryRange,
    pub symbol: Option<String>,
    pub order_type: Option<OrderType>,
    pub side: Option<Side>,
    pub order_status: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeHistoryQuery {
    #[serde(flatten)]
    pub range: HistoryRange,
    pub symbol: Option<String>,
    pub side: Option<Side>,
    pub order_id: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionHistoryQuery {
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub last_id: Option<i64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExchangeInfo {
    pub timezone: String,
    pub symbols: Vec<StockSymbol>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSymbol {
    pub symbol: String,
    pub tradability: String,
    pub tradability_update_time: i64,
    pub overnight_supported: bool,
    pub fractionable: bool,
    pub fractionable_eh: bool,
    pub extended_session: bool,
    pub max_num_orders: u64,
    pub step_size: String,
    pub multiplier_up: String,
    pub multiplier_down: String,
    pub min_qty: String,
    pub max_qty: String,
    pub min_notional: String,
    pub max_notional: String,
    pub listing_time: i64,
    pub delisting_time: Option<i64>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenizedAsset {
    pub asset_code: String,
    pub asset_name: String,
    pub underlying_equity_symbol: String,
    pub multiplier: String,
    pub multiplier_valid: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub symbol: String,
    pub bid_price: String,
    pub ask_price: String,
    pub bid_size: u64,
    pub ask_size: u64,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderAcknowledgement {
    /// S = accepted, F = failed. This is NOT an execution status.
    pub status: String,
    pub order_id: String,
    pub client_order_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenKeyResponse {
    pub listen_key: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockOrder {
    pub order_id: String,
    pub client_order_id: Option<String>,
    pub symbol: String,
    pub quote: String,
    pub side: String,
    pub order_type: String,
    pub limit_price: Option<String>,
    pub avg_filled_price: Option<String>,
    pub qty: Option<String>,
    pub notional: Option<String>,
    pub filled_qty: String,
    pub filled_total: Option<String>,
    pub fee: String,
    pub session: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub trades: Vec<OrderExecution>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderExecution {
    pub execution_id: String,
    pub execution_at: i64,
    pub price: String,
    pub qty: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockTrade {
    pub execution_id: String,
    pub order_id: String,
    pub symbol: String,
    pub quote: String,
    pub side: String,
    pub order_type: String,
    pub price: String,
    pub qty: String,
    pub total: String,
    pub execution_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Page<T> {
    pub total: u64,
    pub page: u64,
    pub size: u64,
    pub rows: Vec<T>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionAcknowledgement {
    pub issuer_request_id: String,
    pub status: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversion {
    pub underlying_asset: String,
    pub underlying_asset_amount: String,
    pub tokenized_asset: String,
    pub tokenized_asset_amount: String,
    pub issuer_request_id: String,
    pub convert_type: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionPage {
    pub rows: Vec<Conversion>,
    pub has_more: bool,
    pub next_last_id: Option<i64>,
}
