use crate::error::{ExchangeClientError, Result};
use crate::signing::sign_hmac_sha256_hex;
use crate::types::{BinaOrderResponse, CoinOrderResponse, FillInfo, SpotOrderResponse, TradeOrder};
use bigdecimal::BigDecimal;
use reqwest::Client;
use std::collections::BTreeMap;
use std::str::FromStr;

pub const BINANCE_USDT_FUTURES_API_URL: &str = "https://fapi.binance.com";
pub const BINANCE_USDT_FUTURES_API_TESTNET_URL: &str = "https://testnet.binancefuture.com";
pub const BINANCE_COIN_FUTURES_API_URL: &str = "https://dapi.binance.com";
pub const BINANCE_COIN_FUTURES_API_TESTNET_URL: &str = "https://testnet.binancefuture.com";
pub const BINANCE_SPOT_API_URL: &str = "https://api.binance.com";
pub const BINANCE_SPOT_API_TESTNET_URL: &str = "https://testnet.binance.vision";

/// 按客户端订单 ID 查询的 U 本位订单快照；只保留恢复所需的明确字段。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsdtOrderSnapshot {
    pub order_id: i64,
    pub symbol: String,
    pub client_order_id: String,
    pub side: String,
    pub position_side: String,
    pub status: String,
    pub orig_qty: String,
    pub executed_qty: String,
    pub avg_price: String,
    pub cum_quote: String,
    pub orig_type: String,
    pub reduce_only: bool,
    pub close_position: bool,
}

/// U 本位账户按持仓方向返回的币数量；双向持仓模式下多头为正、空头为负。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsdtPositionSnapshot {
    pub symbol: String,
    pub position_side: String,
    pub position_amt: String,
}

#[derive(Clone)]
pub struct BinanceOrderClient {
    api_key: String,
    api_secret: String,
    use_testnet: bool,
    http: Client,
}

impl std::fmt::Debug for BinanceOrderClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 调试输出仅展示环境，不能包含账户凭据或请求头。
        f.debug_struct("BinanceOrderClient")
            .field("use_testnet", &self.use_testnet)
            .finish_non_exhaustive()
    }
}

impl BinanceOrderClient {
    /// 账户事实采集专用 GET；接口路径由库内调用方固定，异常不输出签名地址或响应正文。
    pub(super) async fn read_usdt_account<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(String, String)],
    ) -> Result<T> {
        let base = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let mut url = reqwest::Url::parse(&format!("{base}{path}"))
            .map_err(|_| ExchangeClientError::Validation("账户查询地址无效".into()))?;
        url.query_pairs_mut()
            .extend_pairs(params)
            .append_pair("recvWindow", "5000")
            .append_pair(
                "timestamp",
                &chrono::Utc::now().timestamp_millis().to_string(),
            );
        let signature = sign_hmac_sha256_hex(&self.api_secret, url.query().unwrap_or_default());
        url.query_pairs_mut().append_pair("signature", &signature);
        let response = self
            .http
            .get(url)
            .header("X-MBX-APIKEY", &self.api_key)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| ExchangeClientError::Connection("账户事实查询连接失败".into()))?;
        if !response.status().is_success() {
            return Err(ExchangeClientError::Order(format!(
                "账户事实查询失败: HTTP {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|_| ExchangeClientError::ResponseParse("账户事实响应字段不符合预期".into()))
    }

    /// 只读核对账户持仓，不发送交易、撤单或模式切换请求。
    pub async fn query_usdt_positions(&self, symbol: &str) -> Result<Vec<UsdtPositionSnapshot>> {
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let mut url = reqwest::Url::parse(&format!("{base_url}/fapi/v2/positionRisk"))
            .map_err(|_| ExchangeClientError::Validation("账户持仓查询地址无效".into()))?;
        url.query_pairs_mut()
            .append_pair("symbol", symbol)
            .append_pair(
                "timestamp",
                &chrono::Utc::now().timestamp_millis().to_string(),
            );
        let signature = sign_hmac_sha256_hex(&self.api_secret, url.query().unwrap_or_default());
        url.query_pairs_mut().append_pair("signature", &signature);
        let response = self
            .http
            .get(url)
            .header("X-MBX-APIKEY", &self.api_key)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| ExchangeClientError::Connection(error.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(ExchangeClientError::Order(format!(
                "账户持仓查询失败: HTTP {}",
                response.status()
            )));
        }
        let positions: Vec<UsdtPositionSnapshot> = response
            .json()
            .await
            .map_err(|error| ExchangeClientError::ResponseParse(error.without_url().to_string()))?;
        if positions.iter().any(|position| position.symbol != symbol) {
            return Err(ExchangeClientError::Validation(
                "账户持仓查询品种不一致".into(),
            ));
        }
        Ok(positions)
    }

    /// 只查询历史请求，不因不存在、超时或解析失败而重新下单。
    pub async fn query_usdt_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<UsdtOrderSnapshot> {
        let client_id = normalize_client_order_id(client_order_id)
            .ok_or_else(|| ExchangeClientError::Validation("客户端订单 ID 无效".into()))?;
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let mut params = BTreeMap::new();
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("origClientOrderId".to_string(), client_id.clone());
        params.insert(
            "timestamp".to_string(),
            chrono::Utc::now().timestamp_millis().to_string(),
        );
        let mut url = reqwest::Url::parse(&format!("{}/fapi/v1/order", base_url))
            .map_err(|_| ExchangeClientError::Validation("订单查询地址无效".into()))?;
        url.query_pairs_mut().extend_pairs(&params);
        // 对实际发送的编码后查询串签名，避免客户端 ID 中合法特殊字符产生签名差异。
        let signature = sign_hmac_sha256_hex(&self.api_secret, url.query().unwrap_or_default());
        url.query_pairs_mut().append_pair("signature", &signature);
        let response = self
            .http
            .get(url)
            .header("X-MBX-APIKEY", &self.api_key)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            // 查询 URL 含签名参数，错误日志不能输出完整 URL。
            .map_err(|error| ExchangeClientError::Connection(error.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(ExchangeClientError::Order(format!(
                "查询订单未成功，状态未知，不允许重发: HTTP {}",
                response.status()
            )));
        }
        let snapshot: UsdtOrderSnapshot = response
            .json()
            .await
            .map_err(|error| ExchangeClientError::ResponseParse(error.without_url().to_string()))?;
        if snapshot.client_order_id != client_id || snapshot.symbol != symbol {
            return Err(ExchangeClientError::Validation(
                "查询返回的订单身份不匹配".into(),
            ));
        }
        Ok(snapshot)
    }

    /// 仅撤销指定客户端 ID 的剩余委托；返回成功不是终态成交证明，必须继续查询。
    pub async fn cancel_usdt_order(&self, symbol: &str, client_order_id: &str) -> Result<()> {
        let client_id = normalize_client_order_id(client_order_id)
            .ok_or_else(|| ExchangeClientError::Validation("撤单客户端订单 ID 无效".into()))?;
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let mut url = reqwest::Url::parse(&format!("{base_url}/fapi/v1/order"))
            .map_err(|_| ExchangeClientError::Validation("撤单地址无效".into()))?;
        url.query_pairs_mut()
            .append_pair("symbol", symbol)
            .append_pair("origClientOrderId", &client_id)
            .append_pair(
                "timestamp",
                &chrono::Utc::now().timestamp_millis().to_string(),
            );
        let signature = sign_hmac_sha256_hex(&self.api_secret, url.query().unwrap_or_default());
        url.query_pairs_mut().append_pair("signature", &signature);
        let response = self
            .http
            .delete(url)
            .header("X-MBX-APIKEY", &self.api_key)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| ExchangeClientError::Connection(error.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(ExchangeClientError::Order(format!(
                "撤余量未确认，须重新查原订单: HTTP {}",
                response.status()
            )));
        }
        Ok(())
    }

    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self::with_testnet(api_key, api_secret, false)
    }

    pub fn testnet(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self::with_testnet(api_key, api_secret, true)
    }

    pub fn with_testnet(
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
        use_testnet: bool,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            use_testnet,
            // 签名请求不跟随重定向，避免将自定义账户请求头转发到另一主机。
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("固定 HTTP 客户端配置无效"),
        }
    }

    pub async fn create_usdt_market_order(
        &self,
        symbol: &str,
        side: &str,
        position_side: &str,
        quantity: &BigDecimal,
        client_order_id: Option<&str>,
    ) -> Result<TradeOrder> {
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let endpoint = format!("{}/fapi/v1/order", base_url);

        let mut params = BTreeMap::new();
        params.insert("newOrderRespType".to_string(), "RESULT".to_string());
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("side".to_string(), side.to_string());
        params.insert("type".to_string(), "MARKET".to_string());
        params.insert("positionSide".to_string(), position_side.to_string());
        params.insert("quantity".to_string(), quantity.to_string());
        if let Some(client_order_id) = client_order_id.and_then(normalize_client_order_id) {
            params.insert("newClientOrderId".to_string(), client_order_id);
        }
        self.sign_params(&mut params);

        let raw_text = self.post_form(&endpoint, &params).await?;
        log::info!("[USDT] create_market_order raw response: {}", raw_text);

        let json: BinaOrderResponse = serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))?;
        log::info!("[USDT] create_market_order parsed response: {:?}", json);

        // orig_qty 是委托数量，不能在缺少成交数量时作为已成交证据。
        let amount = first_positive_decimal(&[&json.cum_qty, &json.executed_qty]);
        let avg_price = resolve_avg_price(&json.avg_price, &json.cum_quote, &amount);
        let result = TradeOrder {
            fee: BigDecimal::from(0),
            sell_price: avg_price,
            amount,
        };
        log::info!("[USDT] create_market_order trade order: {:?}", result);

        Ok(result)
    }

    pub async fn create_coin_market_order(
        &self,
        symbol: &str,
        side: &str,
        position_side: &str,
        quantity: &BigDecimal,
        client_order_id: Option<&str>,
    ) -> Result<TradeOrder> {
        let endpoint = format!("{}/dapi/v1/order", self.coin_base_url());
        let mut params = BTreeMap::new();
        params.insert("newOrderRespType".to_string(), "RESULT".to_string());
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("side".to_string(), side.to_string());
        params.insert("type".to_string(), "MARKET".to_string());
        params.insert("positionSide".to_string(), position_side.to_string());
        params.insert("quantity".to_string(), quantity.to_string());
        if let Some(client_order_id) = client_order_id.and_then(normalize_client_order_id) {
            params.insert("newClientOrderId".to_string(), client_order_id);
        }
        self.sign_params(&mut params);

        let raw_text = self.post_form(&endpoint, &params).await?;
        log::info!("[COIN] create_market_order raw response: {}", raw_text);

        let json: CoinOrderResponse = serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))?;
        log::info!("[COIN] create_market_order parsed response: {:?}", json);

        let result = TradeOrder {
            fee: BigDecimal::from(0),
            sell_price: BigDecimal::from_str(&json.avg_price).unwrap_or_default(),
            amount: BigDecimal::from_str(&json.cum_base).unwrap_or_default(),
        };
        log::info!("[COIN] create_market_order trade order: {:?}", result);

        Ok(result)
    }

    pub async fn create_coin_limit_order(
        &self,
        symbol: &str,
        side: &str,
        position_side: &str,
        quantity: &BigDecimal,
        price: &BigDecimal,
        time_in_force: &str,
    ) -> Result<CoinOrderResponse> {
        let endpoint = format!("{}/dapi/v1/order", self.coin_base_url());
        let mut params = BTreeMap::new();
        params.insert("newOrderRespType".to_string(), "RESULT".to_string());
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("side".to_string(), side.to_string());
        params.insert("type".to_string(), "LIMIT".to_string());
        params.insert("positionSide".to_string(), position_side.to_string());
        params.insert("quantity".to_string(), quantity.to_string());
        params.insert("price".to_string(), price.to_string());
        params.insert("timeInForce".to_string(), time_in_force.to_string());
        self.sign_params(&mut params);

        let raw_text = self.post_form(&endpoint, &params).await?;
        log::info!("[COIN] create_limit_order raw response: {}", raw_text);

        serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))
    }

    pub async fn create_coin_stop_market_order(
        &self,
        symbol: &str,
        side: &str,
        position_side: &str,
        quantity: &BigDecimal,
        stop_price: &BigDecimal,
        order_type: &str,
    ) -> Result<CoinOrderResponse> {
        let endpoint = format!("{}/dapi/v1/order", self.coin_base_url());
        let mut params = BTreeMap::new();
        params.insert("newOrderRespType".to_string(), "RESULT".to_string());
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("side".to_string(), side.to_string());
        params.insert("type".to_string(), order_type.to_string());
        params.insert("positionSide".to_string(), position_side.to_string());
        params.insert("quantity".to_string(), quantity.to_string());
        params.insert("stopPrice".to_string(), stop_price.to_string());
        self.sign_params(&mut params);

        let raw_text = self.post_form(&endpoint, &params).await?;
        log::info!("[COIN] create_stop_market_order raw response: {}", raw_text);

        serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))
    }

    pub async fn create_spot_market_order(
        &self,
        symbol: &str,
        side: &str,
        quantity: &BigDecimal,
        client_order_id: Option<&str>,
    ) -> Result<TradeOrder> {
        let base_url = if self.use_testnet {
            BINANCE_SPOT_API_TESTNET_URL
        } else {
            BINANCE_SPOT_API_URL
        };
        let endpoint = format!("{}/api/v3/order", base_url);

        let mut params = BTreeMap::new();
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("side".to_string(), side.to_string());
        params.insert("type".to_string(), "MARKET".to_string());
        params.insert("quantity".to_string(), quantity.to_string());
        params.insert("newOrderRespType".to_string(), "FULL".to_string());
        if let Some(client_order_id) = client_order_id.and_then(normalize_client_order_id) {
            params.insert("newClientOrderId".to_string(), client_order_id);
        }
        self.sign_params(&mut params);

        let raw_text = self.post_form(&endpoint, &params).await?;
        log::info!("[SPOT] create_market_order raw response: {}", raw_text);

        let json: SpotOrderResponse = serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))?;
        let (avg_price, total_fee, executed_qty) = calculate_trade_summary(&json.fills);

        Ok(TradeOrder {
            fee: total_fee,
            sell_price: avg_price,
            amount: executed_qty,
        })
    }

    fn coin_base_url(&self) -> &'static str {
        if self.use_testnet {
            BINANCE_COIN_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_COIN_FUTURES_API_URL
        }
    }

    fn sign_params(&self, params: &mut BTreeMap<String, String>) {
        params.insert(
            "timestamp".to_string(),
            chrono::Utc::now().timestamp_millis().to_string(),
        );
        let payload = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        let signature = sign_hmac_sha256_hex(&self.api_secret, &payload);
        params.insert("signature".to_string(), signature);
    }

    async fn post_form(&self, endpoint: &str, params: &BTreeMap<String, String>) -> Result<String> {
        let response = self
            .http
            .post(endpoint)
            .header("X-MBX-APIKEY", &self.api_key)
            .form(params)
            .send()
            .await?;

        let status = response.status();
        let raw_text = response.text().await?;
        if !status.is_success() {
            return Err(ExchangeClientError::Order(format!(
                "status: {}, body: {}",
                status, raw_text
            )));
        }
        Ok(raw_text)
    }
}

pub fn parse_order_response(
    raw: &str,
) -> std::result::Result<BinaOrderResponse, serde_json::Error> {
    serde_json::from_str::<BinaOrderResponse>(raw)
}

pub fn parse_coin_order_response(
    raw: &str,
) -> std::result::Result<CoinOrderResponse, serde_json::Error> {
    serde_json::from_str::<CoinOrderResponse>(raw)
}

fn normalize_client_order_id(client_order_id: &str) -> Option<String> {
    let mut normalized = String::with_capacity(client_order_id.len().min(36));
    for ch in client_order_id.trim().chars() {
        if normalized.len() >= 36 {
            break;
        }

        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/') {
            normalized.push(ch);
        }
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn parse_decimal(value: &str) -> BigDecimal {
    BigDecimal::from_str(value).unwrap_or_default()
}

fn first_positive_decimal(values: &[&str]) -> BigDecimal {
    let zero = BigDecimal::from(0);
    values
        .iter()
        .map(|value| parse_decimal(value))
        .find(|value| value > &zero)
        .unwrap_or(zero)
}

fn resolve_avg_price(avg_price: &str, cum_quote: &str, amount: &BigDecimal) -> BigDecimal {
    let parsed_avg = parse_decimal(avg_price);
    if parsed_avg > BigDecimal::from(0) {
        return parsed_avg;
    }

    let quote = parse_decimal(cum_quote);
    if quote > BigDecimal::from(0) && amount > &BigDecimal::from(0) {
        quote / amount
    } else {
        BigDecimal::from(0)
    }
}

pub fn calculate_trade_summary(fills: &[FillInfo]) -> (BigDecimal, BigDecimal, BigDecimal) {
    let mut total_value = BigDecimal::from(0);
    let mut total_qty = BigDecimal::from(0);
    let mut total_fee = BigDecimal::from(0);

    for fill in fills {
        let price = BigDecimal::from_str(&fill.price).unwrap_or_default();
        let qty = BigDecimal::from_str(&fill.qty).unwrap_or_default();
        let fee = BigDecimal::from_str(&fill.commission).unwrap_or_default();

        total_value += &price * &qty;
        total_qty += qty;
        if !fill.commission_asset.eq("BNB") {
            total_fee += fee;
        }
    }

    let avg_price = if total_qty > BigDecimal::from(0) {
        &total_value / &total_qty
    } else {
        BigDecimal::from(0)
    };

    (avg_price, total_fee.with_scale(8), total_qty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_client_order_id_keeps_binance_safe_chars() {
        let id = "  abc-123_./:中文!@#$%^&*()extra-extra-extra-extra-extra  ";
        assert_eq!(
            normalize_client_order_id(id).unwrap(),
            "abc-123_./:extra-extra-extra-extra-e"
        );
    }

    #[test]
    fn normalize_client_order_id_returns_none_when_empty_after_filtering() {
        assert_eq!(normalize_client_order_id(" 中文!@# "), None);
    }

    #[test]
    fn first_positive_decimal_uses_first_non_zero_value() {
        assert_eq!(
            first_positive_decimal(&["0", "0.025", "0.030"]),
            BigDecimal::from_str("0.025").unwrap()
        );
    }

    #[test]
    fn resolve_avg_price_falls_back_to_quote_over_amount() {
        let amount = BigDecimal::from_str("0.5").unwrap();
        assert_eq!(
            resolve_avg_price("0", "1250", &amount),
            BigDecimal::from_str("2500").unwrap()
        );
    }
}
