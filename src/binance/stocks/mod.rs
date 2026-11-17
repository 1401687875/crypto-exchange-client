//! Binance Stocks Trading SAPI. Prices and quantities remain decimal strings.
mod types;
pub mod websocket;
pub use types::*;
#[cfg(test)]
mod tests;

use crate::{signing::sign_hmac_sha256_hex, ExchangeClientError, Result};
use reqwest::{Client, Method};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::time::Duration;

pub const BINANCE_STOCKS_API_URL: &str = "https://api.binance.com";

#[derive(Clone)]
pub struct BinanceStocksClient {
    api_key: String,
    api_secret: String,
    http: Client,
    recv_window: u64,
    base_url: String,
}

impl BinanceStocksClient {
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            http: Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(8))
                .build()
                .expect("valid HTTP configuration"),
            recv_window: 5000,
            base_url: BINANCE_STOCKS_API_URL.into(),
        }
    }

    pub fn with_recv_window(mut self, milliseconds: u64) -> Result<Self> {
        if milliseconds == 0 || milliseconds > 60_000 {
            return invalid("recvWindow must be 1..=60000");
        }
        self.recv_window = milliseconds;
        Ok(self)
    }

    // Encode once, then sign precisely the bytes sent over the wire.
    fn request(
        &self,
        method: Method,
        path: &str,
        mut params: Vec<(String, String)>,
        signed: bool,
        timed: bool,
    ) -> Result<reqwest::Request> {
        if self.api_key.is_empty() || (signed && self.api_secret.is_empty()) {
            return invalid("API credentials are required");
        }
        if timed {
            params.push((
                "timestamp".into(),
                chrono::Utc::now().timestamp_millis().to_string(),
            ));
            params.push(("recvWindow".into(), self.recv_window.to_string()));
        }
        let mut request = self
            .http
            .request(method, format!("{}/sapi/v1/equity/{}", self.base_url, path))
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&params)
            .build()?;
        if signed {
            let signature =
                sign_hmac_sha256_hex(&self.api_secret, request.url().query().unwrap_or(""));
            request
                .url_mut()
                .query_pairs_mut()
                .append_pair("signature", &signature);
        }
        Ok(request)
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        params: Vec<(String, String)>,
        signed: bool,
        timed: bool,
    ) -> Result<T> {
        let request = self.request(method, path, params, signed, timed)?;
        // Never replay a mutation after a timeout: its outcome may be unknown.
        let response = self.http.execute(request).await.map_err(|e| {
            ExchangeClientError::Connection(format!(
                "stocks {path}: {} (mutations may have been accepted; reconcile before retrying)",
                e.without_url()
            ))
        })?;
        let status = response.status();
        let raw = response
            .text()
            .await
            .map_err(|e| ExchangeClientError::Connection(e.without_url().to_string()))?;
        if !status.is_success() {
            return Err(ExchangeClientError::Order(format!(
                "stocks {path}: HTTP {status}: {raw}"
            )));
        }
        let value: Value = if raw.trim().is_empty() && path == "market/quote" {
            Value::Null
        } else {
            serde_json::from_str(&raw)?
        };
        if value
            .get("code")
            .is_some_and(|c| c.as_i64().is_some_and(|n| n != 0))
        {
            return Err(ExchangeClientError::Order(format!(
                "stocks {path}: {value}"
            )));
        }
        Ok(serde_json::from_value(value)?)
    }

    pub async fn exchange_info(&self, symbol: Option<&str>) -> Result<ExchangeInfo> {
        self.call(
            Method::GET,
            "market/exchangeInfo",
            symbol
                .map(|s| vec![("symbol".into(), s.to_uppercase())])
                .unwrap_or_default(),
            false,
            false,
        )
        .await
    }
    pub async fn tokenized_assets(&self) -> Result<Vec<TokenizedAsset>> {
        self.call(Method::GET, "market/tokenized-assets", vec![], false, false)
            .await
    }
    pub async fn quote(&self, symbol: &str) -> Result<Option<Quote>> {
        nonempty(symbol, "symbol")?;
        self.call(
            Method::GET,
            "market/quote",
            vec![("symbol".into(), symbol.to_uppercase())],
            false,
            false,
        )
        .await
    }
    pub async fn place_order(&self, request: &PlaceOrderRequest) -> Result<OrderAcknowledgement> {
        request.validate()?;
        self.call(Method::POST, "order/place", params(request)?, true, true)
            .await
    }
    pub async fn cancel_order(&self, order_id: &str) -> Result<OrderAcknowledgement> {
        nonempty(order_id, "orderId")?;
        self.call(
            Method::POST,
            "order/cancel",
            vec![("orderId".into(), order_id.into())],
            true,
            true,
        )
        .await
    }
    /// Cancels ALL open stock orders for this account.
    pub async fn cancel_all_orders(&self) -> Result<SuccessResponse> {
        self.call(Method::POST, "order/cancel-all", vec![], true, true)
            .await
    }
    pub async fn open_orders(&self) -> Result<Vec<StockOrder>> {
        self.call(Method::GET, "order/open-orders", vec![], true, true)
            .await
    }
    pub async fn order_detail(&self, id: OrderId<'_>) -> Result<StockOrder> {
        let (key, value) = match id {
            OrderId::Exchange(v) => ("orderId", v),
            OrderId::Client(v) => ("clientOrderId", v),
        };
        nonempty(value, key)?;
        self.call(
            Method::GET,
            "order/detail",
            vec![(key.into(), value.into())],
            true,
            true,
        )
        .await
    }
    pub async fn order_history(&self, query: &OrderHistoryQuery) -> Result<Page<StockOrder>> {
        query.range.validate()?;
        if query.order_status.as_ref().is_some_and(|statuses| {
            statuses.split(',').any(|s| {
                !matches!(
                    s,
                    "FILLED" | "PARTIALLY_FILLED" | "CANCELED" | "EXPIRED" | "REJECTED"
                )
            })
        }) {
            return invalid("unsupported orderStatus history filter");
        }
        self.call(Method::GET, "order/history", params(query)?, true, true)
            .await
    }
    pub async fn trade_history(&self, query: &TradeHistoryQuery) -> Result<Page<StockTrade>> {
        query.range.validate()?;
        self.call(Method::GET, "trade/history", params(query)?, true, true)
            .await
    }
    /// Explicit acceptance only; never called automatically when submitting orders.
    pub async fn accept_disclaimer(&self) -> Result<SuccessResponse> {
        self.call(Method::POST, "account/disclaimer", vec![], true, true)
            .await
    }
    /// USER_STREAM requires API key and timestamp, but no signature.
    pub async fn create_or_renew_listen_key(&self) -> Result<ListenKeyResponse> {
        self.call(Method::POST, "listenKey", vec![], false, true)
            .await
    }
    pub async fn mint(
        &self,
        asset: &str,
        amount: &str,
        client_order_id: Option<&str>,
    ) -> Result<ConversionAcknowledgement> {
        self.convert(
            "mint",
            "underlyingAsset",
            "underlyingAssetAmount",
            asset,
            amount,
            client_order_id,
        )
        .await
    }
    pub async fn redeem(
        &self,
        asset: &str,
        amount: &str,
        client_order_id: Option<&str>,
    ) -> Result<ConversionAcknowledgement> {
        self.convert(
            "redeem",
            "tokenizedAsset",
            "tokenizedAssetAmount",
            asset,
            amount,
            client_order_id,
        )
        .await
    }
    async fn convert(
        &self,
        action: &str,
        asset_key: &str,
        amount_key: &str,
        asset: &str,
        amount: &str,
        id: Option<&str>,
    ) -> Result<ConversionAcknowledgement> {
        nonempty(asset, asset_key)?;
        positive(amount)?;
        let mut p = vec![
            (asset_key.into(), asset.into()),
            (amount_key.into(), amount.into()),
        ];
        if let Some(id) = id {
            validate_client_id(id)?;
            p.push(("clientOrderId".into(), id.into()));
        }
        self.call(Method::POST, &format!("tokenized/{action}"), p, true, true)
            .await
    }
    pub async fn conversion_status(
        &self,
        issuer_request_id: &str,
        convert_type: ConvertType,
    ) -> Result<Option<Conversion>> {
        nonempty(issuer_request_id, "issuerRequestId")?;
        let value: Value = self
            .call(
                Method::GET,
                "tokenized/convert-status",
                vec![
                    ("issuerRequestId".into(), issuer_request_id.into()),
                    (
                        "convertType".into(),
                        serde_json::to_value(convert_type)?.as_str().unwrap().into(),
                    ),
                ],
                true,
                true,
            )
            .await?;
        if value.as_object().is_some_and(|o| o.is_empty()) {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(value)?))
        }
    }
    pub async fn conversion_history(
        &self,
        query: &ConversionHistoryQuery,
    ) -> Result<ConversionPage> {
        validate_range(query.start_time, query.end_time, None, query.size)?;
        self.call(Method::GET, "tokenized/history", params(query)?, true, true)
            .await
    }
}

fn params<T: Serialize>(value: &T) -> Result<Vec<(String, String)>> {
    let Value::Object(object) = serde_json::to_value(value)? else {
        return invalid("expected parameter object");
    };
    object
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| {
            let text = match v {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return invalid("expected scalar parameter"),
            };
            Ok((k, text))
        })
        .collect()
}
fn invalid<T>(message: &str) -> Result<T> {
    Err(ExchangeClientError::Validation(message.into()))
}
fn nonempty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        invalid(&format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}
fn positive(value: &str) -> Result<()> {
    use std::str::FromStr;
    if !value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        || bigdecimal::BigDecimal::from_str(value).map_or(true, |v| v <= 0.into())
    {
        return invalid("amount/price must be a positive plain decimal");
    }
    Ok(())
}
fn validate_client_id(id: &str) -> Result<()> {
    if !(32..=36).contains(&id.len())
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return invalid("clientOrderId must contain 32..=36 ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}
fn validate_range(
    start: Option<i64>,
    end: Option<i64>,
    current: Option<u64>,
    size: Option<u64>,
) -> Result<()> {
    if start.is_some_and(|v| v < 0)
        || end.is_some_and(|v| v < 0)
        || start.zip(end).is_some_and(|(s, e)| s > e)
        || current == Some(0)
        || size.is_some_and(|s| s == 0 || s > 100)
    {
        return invalid("invalid time range or pagination (current >= 1, size 1..=100)");
    }
    Ok(())
}
