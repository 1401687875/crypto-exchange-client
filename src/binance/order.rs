use crate::binance::user_data::wait_for_usdt_trade_result;
use crate::error::{ExchangeClientError, Result};
use crate::signing::sign_hmac_sha256_hex;
use crate::types::{BinaOrderResponse, CoinOrderResponse, FillInfo, SpotOrderResponse, TradeOrder};
use bigdecimal::BigDecimal;
use reqwest::Client;
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const BINANCE_USDT_FUTURES_API_URL: &str = "https://fapi.binance.com";
pub const BINANCE_USDT_FUTURES_API_TESTNET_URL: &str = "https://testnet.binancefuture.com";
pub const BINANCE_COIN_FUTURES_API_URL: &str = "https://dapi.binance.com";
pub const BINANCE_COIN_FUTURES_API_TESTNET_URL: &str = "https://testnet.binancefuture.com";
pub const BINANCE_SPOT_API_URL: &str = "https://api.binance.com";
pub const BINANCE_SPOT_API_TESTNET_URL: &str = "https://testnet.binance.vision";

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const RECV_WINDOW_MS: i64 = 5_000;

#[derive(Debug)]
enum BinanceRequestError {
    Transport {
        message: String,
        timeout: bool,
        connect: bool,
    },
    Http {
        status: u16,
        body: String,
    },
}

impl BinanceRequestError {
    fn from_reqwest(error: reqwest::Error) -> Self {
        Self::Transport {
            timeout: error.is_timeout(),
            connect: error.is_connect(),
            message: error.to_string(),
        }
    }

    /// 下单请求可能已经到达撮合引擎，此类错误不能直接重试 POST。
    fn submission_outcome_is_unknown(&self) -> bool {
        match self {
            Self::Transport { .. } => true,
            Self::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
        }
    }

    fn binance_error_code(&self) -> Option<i64> {
        let Self::Http { body, .. } = self else {
            return None;
        };
        serde_json::from_str::<serde_json::Value>(body)
            .ok()?
            .get("code")?
            .as_i64()
    }
}

impl fmt::Display for BinanceRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport {
                message,
                timeout,
                connect,
            } => write!(
                formatter,
                "transport error: timeout={}, connect={}, message={}",
                timeout, connect, message
            ),
            Self::Http { status, body } => {
                write!(formatter, "http status={}, body={}", status, body)
            }
        }
    }
}

impl From<BinanceRequestError> for ExchangeClientError {
    fn from(error: BinanceRequestError) -> Self {
        let message = error.to_string();
        match error {
            BinanceRequestError::Transport { .. } => ExchangeClientError::Connection(message),
            BinanceRequestError::Http { status, body } => {
                match serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| Some((value.get("code")?.as_i64()?, value)))
                {
                    Some((code, value)) => ExchangeClientError::Api {
                        status,
                        code,
                        msg: value
                            .get("msg")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    },
                    None => ExchangeClientError::Order(message),
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct BinanceOrderClient {
    api_key: String,
    api_secret: String,
    use_testnet: bool,
    http: Client,
    server_time_offset_ms: Arc<AtomicI64>,
}

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

impl fmt::Debug for BinanceOrderClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        // 签名请求不跟随重定向，避免将自定义账户请求头转发到另一主机。
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("valid Binance HTTP client configuration");
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            use_testnet,
            http,
            server_time_offset_ms: Arc::new(AtomicI64::new(0)),
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
        // 文档: newOrderRespType 仅支持 ACK / RESULT。MARKET + RESULT 返回最终订单状态，
        // 但当前响应结构不保证包含 avgPrice/cumQuote/fills，成交价需由私有流或查询接口补齐。
        params.insert("newOrderRespType".to_string(), "RESULT".to_string());
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("side".to_string(), side.to_string());
        params.insert("type".to_string(), "MARKET".to_string());
        params.insert("positionSide".to_string(), position_side.to_string());
        params.insert("quantity".to_string(), quantity.to_string());
        let normalized_client_order_id = client_order_id.and_then(normalize_client_order_id);
        if let Some(client_order_id) = normalized_client_order_id.as_ref() {
            params.insert("newClientOrderId".to_string(), client_order_id.clone());
        }
        self.sign_params(&mut params);

        let first_submission = self.post_form(&endpoint, &params).await;
        let submission = match first_submission {
            Err(error) if error.binance_error_code() == Some(-1021) => {
                log::warn!(
                    "[USDT] timestamp rejected, synchronizing Binance server time | error: {}",
                    error
                );
                if let Err(sync_error) = self.sync_server_time(base_url).await {
                    return Err(ExchangeClientError::Order(format!(
                        "USDT order rejected by timestamp and server time sync failed: submission_error=[{}], sync_error=[{}]",
                        error, sync_error
                    )));
                }
                self.sign_params(&mut params);
                self.post_form(&endpoint, &params).await
            }
            result => result,
        };

        let raw_text = match submission {
            Ok(raw_text) => raw_text,
            Err(err) => {
                if err.submission_outcome_is_unknown() {
                    if let Some(client_order_id) = params.get("newClientOrderId") {
                        log::warn!(
                            "[USDT] create_market_order outcome unknown, reconciling by clientOrderId={} | error: {}",
                            client_order_id,
                            err
                        );
                        return self
                            .recover_usdt_submission(symbol, client_order_id, &err)
                            .await;
                    }
                    log::warn!(
                        "[USDT] create_market_order outcome unknown without clientOrderId | error: {}",
                        err
                    );
                }
                return Err(err.into());
            }
        };
        log::info!("[USDT] create_market_order raw response: {}", raw_text);

        let json: BinaOrderResponse = serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))?;
        log::info!("[USDT] create_market_order parsed response: {:?}", json);

        self.usdt_response_to_trade_order(symbol, json, normalized_client_order_id.as_deref())
            .await
    }

    async fn usdt_response_to_trade_order(
        &self,
        symbol: &str,
        mut json: BinaOrderResponse,
        expected_client_order_id: Option<&str>,
    ) -> Result<TradeOrder> {
        if let Some(result) = Self::trade_order_from_usdt_response(&json) {
            log::info!("[USDT] create_market_order trade order: {:?}", result);
            return Ok(result);
        }

        if let Some(result) = wait_for_usdt_trade_result(
            symbol,
            Some(json.order_id),
            expected_client_order_id,
            Duration::from_millis(1_200),
        )
        .await
        {
            log::info!(
                "[USDT] create_market_order reconciled from ORDER_TRADE_UPDATE | order_id: {} | result: {:?}",
                json.order_id,
                result
            );
            return Ok(result);
        }

        // 兼容实际响应仍为 NEW/executedQty=0 的情况，轮询 GET /fapi/v1/order
        // 直到拿到 FILLED / PARTIALLY_FILLED 或超时。
        //
        // 注意：有些 U 本位合约市价单会同步返回 FILLED + executedQty>0，但不带
        // avgPrice/cumQuote。此时订单已经真实成交，不能因为补查订单价格失败就把下单判定
        // 为失败；成交价缺失时改查 userTrades 汇总均价。
        if Self::usdt_needs_order_status_poll(&json) {
            json = self
                .poll_usdt_order_until_filled(symbol, json.order_id)
                .await?;
        }

        if let Some(result) = Self::trade_order_from_usdt_response(&json) {
            log::info!("[USDT] create_market_order trade order: {:?}", result);
            return Ok(result);
        }

        // REST 轮询期间可能已经收到终态事件，先读取事件缓存再请求 userTrades。
        if let Some(result) = wait_for_usdt_trade_result(
            symbol,
            Some(json.order_id),
            expected_client_order_id,
            Duration::ZERO,
        )
        .await
        {
            return Ok(result);
        }

        if Self::usdt_executed_quantity(&json) > BigDecimal::from(0) {
            log::warn!(
                "[USDT] order filled but avg price missing, reconciling order and userTrades | order_id: {} | status: {} | executedQty: {} | avgPrice: {} | cumQuote: {}",
                json.order_id,
                json.status,
                json.executed_qty,
                json.avg_price,
                json.cum_quote
            );
            return self
                .reconcile_usdt_filled_order(symbol, json.order_id)
                .await;
        }

        Err(ExchangeClientError::Order(format!(
            "USDT order filled status could not be converted to trade result: order_id={}, status={}, executedQty={}, avgPrice={}, cumQuote={}",
            json.order_id, json.status, json.executed_qty, json.avg_price, json.cum_quote
        )))
    }

    fn usdt_needs_order_status_poll(json: &BinaOrderResponse) -> bool {
        json.status == "NEW" || Self::usdt_executed_quantity(json) <= BigDecimal::from(0)
    }

    fn usdt_executed_quantity(json: &BinaOrderResponse) -> BigDecimal {
        let executed_qty = BigDecimal::from_str(&json.executed_qty).unwrap_or_default();
        if executed_qty > BigDecimal::from(0) {
            executed_qty
        } else {
            BigDecimal::from_str(&json.cum_qty).unwrap_or_default()
        }
    }

    fn trade_order_from_usdt_response(json: &BinaOrderResponse) -> Option<TradeOrder> {
        let amount = Self::usdt_executed_quantity(json);
        if amount <= BigDecimal::from(0) {
            return None;
        }

        let mut sell_price = BigDecimal::from_str(&json.avg_price).unwrap_or_default();
        if sell_price <= BigDecimal::from(0) {
            let cum_quote = BigDecimal::from_str(&json.cum_quote).unwrap_or_default();
            if cum_quote > BigDecimal::from(0) {
                sell_price = &cum_quote / &amount;
            }
        }

        if sell_price <= BigDecimal::from(0) {
            return None;
        }

        Some(TradeOrder {
            fee: BigDecimal::from(0),
            sell_price,
            amount,
        })
    }

    /// 轮询 USDT 合约订单直到成交（或达到最大重试次数）
    ///
    /// 兼容市价单 HTTP 响应为 NEW + executedQty=0 的情况。
    /// 此函数在 1 秒内最多重试 10 次（每次间隔 100ms），拉到 FILLED / PARTIALLY_FILLED 就返回。
    async fn poll_usdt_order_until_filled(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> Result<BinaOrderResponse> {
        const MAX_RETRIES: usize = 10;
        const INTERVAL_MS: u64 = 100;

        for attempt in 1..=MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_millis(INTERVAL_MS)).await;
            match self.query_usdt_order_by_id(symbol, order_id).await {
                Ok(resp) => {
                    log::info!(
                        "[USDT] poll order attempt {}/{} | order_id: {} | status: {} | executedQty: {} | avgPrice: {}",
                        attempt,
                        MAX_RETRIES,
                        order_id,
                        resp.status,
                        resp.executed_qty,
                        resp.avg_price
                    );
                    let zero = BigDecimal::from(0);
                    let filled = BigDecimal::from_str(&resp.executed_qty)
                        .map(|q| q > zero)
                        .unwrap_or(false);
                    if resp.status == "FILLED" || resp.status == "PARTIALLY_FILLED" || filled {
                        return Ok(resp);
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[USDT] poll order query failed attempt {}/{} | order_id: {} | err: {:?}",
                        attempt,
                        MAX_RETRIES,
                        order_id,
                        e
                    );
                }
            }
        }
        log::warn!(
            "[USDT] poll order exhausted retries | order_id: {} | 返回最后一次查询结果",
            order_id
        );
        // 兜底：再查一次返回最终状态（即使是 NEW 也返回，让上层决定）
        self.query_usdt_order_by_id(symbol, order_id).await
    }

    /// 按交易所订单 ID 查询 U 本位订单状态（GET /fapi/v1/order），返回完整响应。
    pub async fn query_usdt_order_by_id(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> Result<BinaOrderResponse> {
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let endpoint = format!("{}/fapi/v1/order", base_url);

        let mut params = BTreeMap::new();
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("orderId".to_string(), order_id.to_string());

        let raw_text = self
            .get_usdt_signed(&endpoint, base_url, &mut params)
            .await?;
        let json: BinaOrderResponse = serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))?;
        Ok(json)
    }

    /// 按客户端订单号查询 U 本位订单状态（GET /fapi/v1/order），返回完整响应。
    pub async fn query_usdt_order_by_client_order_id(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<BinaOrderResponse> {
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let endpoint = format!("{}/fapi/v1/order", base_url);

        let mut params = BTreeMap::new();
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("origClientOrderId".to_string(), client_order_id.to_string());

        let raw_text = self
            .get_usdt_signed(&endpoint, base_url, &mut params)
            .await?;
        let json: BinaOrderResponse = serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))?;
        Ok(json)
    }

    /// POST 的结果不确定时，只查询同一个 clientOrderId，绝不重新提交订单。
    async fn recover_usdt_submission(
        &self,
        symbol: &str,
        client_order_id: &str,
        submission_error: &BinanceRequestError,
    ) -> Result<TradeOrder> {
        const RECOVERY_DELAYS_MS: &[u64] = &[200, 300, 500, 1_000, 2_000, 3_000, 3_000];
        const RECOVERY_BUDGET: Duration = Duration::from_secs(12);

        if let Some(result) = wait_for_usdt_trade_result(
            symbol,
            None,
            Some(client_order_id),
            Duration::from_millis(1_200),
        )
        .await
        {
            log::warn!(
                "[USDT] uncertain submission recovered from ORDER_TRADE_UPDATE | clientOrderId: {} | result: {:?}",
                client_order_id,
                result
            );
            return Ok(result);
        }

        let mut last_query_error = "not queried".to_string();
        let mut attempts = 0usize;
        let deadline = tokio::time::Instant::now() + RECOVERY_BUDGET;
        for (index, delay_ms) in RECOVERY_DELAYS_MS.iter().enumerate() {
            if !sleep_before_deadline(deadline, Duration::from_millis(*delay_ms)).await {
                last_query_error = "submission recovery deadline reached before next query".into();
                break;
            }
            attempts += 1;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let query = tokio::time::timeout(
                remaining,
                self.query_usdt_order_by_client_order_id(symbol, client_order_id),
            )
            .await;
            match query {
                Err(_) => {
                    last_query_error = "submission recovery deadline reached during query".into();
                    break;
                }
                Ok(Ok(order)) => {
                    log::warn!(
                        "[USDT] uncertain submission recovered attempt {}/{} | clientOrderId: {} | order_id: {} | status: {} | executedQty: {}",
                        index + 1,
                        RECOVERY_DELAYS_MS.len(),
                        client_order_id,
                        order.order_id,
                        order.status,
                        order.executed_qty
                    );
                    return self
                        .usdt_response_to_trade_order(symbol, order, Some(client_order_id))
                        .await;
                }
                Ok(Err(error)) => {
                    last_query_error = error.to_string();
                    log::warn!(
                        "[USDT] uncertain submission query attempt {}/{} failed | clientOrderId: {} | error: {}",
                        index + 1,
                        RECOVERY_DELAYS_MS.len(),
                        client_order_id,
                        error
                    );
                }
            }
        }

        Err(ExchangeClientError::Order(format!(
            "USDT order submission outcome remains unknown: clientOrderId={}, attempts={}, original_submission_error=[{}], last_query_error=[{}]; POST was not retried",
            client_order_id,
            attempts,
            submission_error,
            last_query_error
        )))
    }

    /// 已确认订单成交后，对账订单详情和成交明细，直到拿到真实成交均价。
    ///
    /// Binance 偶尔会先返回 FILLED + executedQty，但 avgPrice/cumQuote 和 userTrades
    /// 存在短暂的最终一致性延迟。此处绝不能重试下单，只查询同一个 orderId。
    async fn reconcile_usdt_filled_order(&self, symbol: &str, order_id: u64) -> Result<TradeOrder> {
        const ORDER_QUERY_DELAYS_MS: &[u64] = &[200, 300, 500, 1_000, 2_000, 3_000, 3_000];
        const USER_TRADE_QUERY_ATTEMPTS: &[usize] = &[2, 4, 6, 7];
        const RECONCILIATION_BUDGET: Duration = Duration::from_secs(12);

        let mut last_order_state = "not queried".to_string();
        let mut last_order_error = "none".to_string();
        let mut last_trade_count = 0usize;
        let mut last_trade_error = "none".to_string();
        let mut attempts = 0usize;
        let deadline = tokio::time::Instant::now() + RECONCILIATION_BUDGET;

        for (index, delay_ms) in ORDER_QUERY_DELAYS_MS.iter().enumerate() {
            let attempt = index + 1;
            if !sleep_before_deadline(deadline, Duration::from_millis(*delay_ms)).await {
                last_order_error = "reconciliation deadline reached before next query".into();
                break;
            }
            attempts += 1;

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, self.query_usdt_order_by_id(symbol, order_id))
                .await
            {
                Err(_) => {
                    last_order_error = "reconciliation deadline reached during order query".into();
                    break;
                }
                Ok(Ok(order)) => {
                    last_order_state = format!(
                        "status={}, executedQty={}, avgPrice={}, cumQuote={}",
                        order.status, order.executed_qty, order.avg_price, order.cum_quote
                    );
                    if let Some(result) = Self::trade_order_from_usdt_response(&order) {
                        log::info!(
                            "[USDT] filled order reconciled from order query attempt {}/{} | order_id: {} | result: {:?}",
                            attempt,
                            ORDER_QUERY_DELAYS_MS.len(),
                            order_id,
                            result
                        );
                        return Ok(result);
                    }
                }
                Ok(Err(error)) => {
                    last_order_error = error.to_string();
                    log::warn!(
                        "[USDT] reconcile order query failed attempt {}/{} | order_id: {} | err: {:?}",
                        attempt,
                        ORDER_QUERY_DELAYS_MS.len(),
                        order_id,
                        error
                    );
                }
            }

            // userTrades 权重为 5，只在少数时间点查询，避免异常订单并发时打满 IP 限频。
            if !USER_TRADE_QUERY_ATTEMPTS.contains(&attempt) {
                continue;
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                last_trade_error = "reconciliation deadline reached before userTrades query".into();
                break;
            }
            match tokio::time::timeout(
                remaining,
                self.query_usdt_user_trades_by_order_id(symbol, order_id),
            )
            .await
            {
                Err(_) => {
                    last_trade_error =
                        "reconciliation deadline reached during userTrades query".into();
                    break;
                }
                Ok(Ok(trades)) => {
                    last_trade_count = trades.len();
                    let (avg_price, total_fee, executed_qty) = calculate_trade_summary(&trades);
                    log::info!(
                        "[USDT] reconcile userTrades attempt {}/{} | order_id: {} | trades: {} | qty: {} | avgPrice: {}",
                        attempt,
                        ORDER_QUERY_DELAYS_MS.len(),
                        order_id,
                        trades.len(),
                        executed_qty,
                        avg_price
                    );
                    if executed_qty > BigDecimal::from(0) && avg_price > BigDecimal::from(0) {
                        return Ok(TradeOrder {
                            fee: total_fee,
                            sell_price: avg_price,
                            amount: executed_qty,
                        });
                    }
                }
                Ok(Err(error)) => {
                    last_trade_error = error.to_string();
                    log::warn!(
                        "[USDT] reconcile userTrades query failed attempt {}/{} | order_id: {} | err: {:?}",
                        attempt,
                        ORDER_QUERY_DELAYS_MS.len(),
                        order_id,
                        error
                    );
                }
            }
        }

        Err(ExchangeClientError::Order(format!(
            "USDT filled order reconciliation exhausted: order_id={}, attempts={}, last_order=[{}], last_order_error={}, last_user_trades_count={}, last_user_trades_error={}",
            order_id,
            attempts,
            last_order_state,
            last_order_error,
            last_trade_count,
            last_trade_error
        )))
    }

    async fn query_usdt_user_trades_by_order_id(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> Result<Vec<FillInfo>> {
        let base_url = if self.use_testnet {
            BINANCE_USDT_FUTURES_API_TESTNET_URL
        } else {
            BINANCE_USDT_FUTURES_API_URL
        };
        let endpoint = format!("{}/fapi/v1/userTrades", base_url);

        let mut params = BTreeMap::new();
        params.insert("symbol".to_string(), symbol.to_string());
        params.insert("orderId".to_string(), order_id.to_string());
        params.insert("limit".to_string(), "1000".to_string());

        let raw_text = self
            .get_usdt_signed(&endpoint, base_url, &mut params)
            .await?;
        serde_json::from_str(&raw_text)
            .map_err(|e| ExchangeClientError::ResponseParse(format!("{} - {}", e, raw_text)))
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
        params.remove("signature");
        params.insert("recvWindow".to_string(), RECV_WINDOW_MS.to_string());
        params.insert(
            "timestamp".to_string(),
            (chrono::Utc::now().timestamp_millis()
                + self.server_time_offset_ms.load(Ordering::Relaxed))
            .to_string(),
        );
        let payload = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        let signature = sign_hmac_sha256_hex(&self.api_secret, &payload);
        params.insert("signature".to_string(), signature);
    }

    async fn sync_server_time(&self, base_url: &str) -> Result<()> {
        let before = chrono::Utc::now().timestamp_millis();
        let endpoint = format!("{}/fapi/v1/time", base_url);
        let response =
            self.http.get(&endpoint).send().await.map_err(|error| {
                ExchangeClientError::from(BinanceRequestError::from_reqwest(error))
            })?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| ExchangeClientError::from(BinanceRequestError::from_reqwest(error)))?;
        if !status.is_success() {
            return Err(BinanceRequestError::Http {
                status: status.as_u16(),
                body,
            }
            .into());
        }

        let server_time = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("serverTime")?.as_i64())
            .ok_or_else(|| {
                ExchangeClientError::ResponseParse(format!(
                    "Binance server time response missing serverTime - {}",
                    body
                ))
            })?;
        let after = chrono::Utc::now().timestamp_millis();
        let local_midpoint = before + (after - before) / 2;
        let offset = server_time - local_midpoint;
        self.server_time_offset_ms.store(offset, Ordering::Relaxed);
        log::info!(
            "[USDT] synchronized Binance server time | offset_ms: {} | round_trip_ms: {}",
            offset,
            after - before
        );
        Ok(())
    }

    async fn get_usdt_signed(
        &self,
        endpoint: &str,
        base_url: &str,
        params: &mut BTreeMap<String, String>,
    ) -> Result<String> {
        self.sign_params(params);
        match self.get_form_raw(endpoint, params).await {
            Err(error) if error.binance_error_code() == Some(-1021) => {
                log::warn!(
                    "[USDT] signed GET timestamp rejected, synchronizing Binance server time | endpoint: {} | error: {}",
                    endpoint,
                    error
                );
                self.sync_server_time(base_url).await.map_err(|sync_error| {
                    ExchangeClientError::Order(format!(
                        "USDT signed GET rejected by timestamp and server time sync failed: request_error=[{}], sync_error=[{}]",
                        error, sync_error
                    ))
                })?;
                self.sign_params(params);
                self.get_form_raw(endpoint, params)
                    .await
                    .map_err(ExchangeClientError::from)
            }
            result => result.map_err(ExchangeClientError::from),
        }
    }

    async fn post_form(
        &self,
        endpoint: &str,
        params: &BTreeMap<String, String>,
    ) -> std::result::Result<String, BinanceRequestError> {
        let response = self
            .http
            .post(endpoint)
            .header("X-MBX-APIKEY", &self.api_key)
            .form(params)
            .send()
            .await
            .map_err(BinanceRequestError::from_reqwest)?;

        let status = response.status();
        let raw_text = response
            .text()
            .await
            .map_err(BinanceRequestError::from_reqwest)?;
        if !status.is_success() {
            return Err(BinanceRequestError::Http {
                status: status.as_u16(),
                body: raw_text,
            });
        }
        Ok(raw_text)
    }

    async fn get_form_raw(
        &self,
        endpoint: &str,
        params: &BTreeMap<String, String>,
    ) -> std::result::Result<String, BinanceRequestError> {
        let response = self
            .http
            .get(endpoint)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(params)
            .send()
            .await
            .map_err(BinanceRequestError::from_reqwest)?;

        let status = response.status();
        let raw_text = response
            .text()
            .await
            .map_err(BinanceRequestError::from_reqwest)?;
        if !status.is_success() {
            return Err(BinanceRequestError::Http {
                status: status.as_u16(),
                body: raw_text,
            });
        }
        Ok(raw_text)
    }
}

async fn sleep_before_deadline(deadline: tokio::time::Instant, delay: Duration) -> bool {
    if deadline.saturating_duration_since(tokio::time::Instant::now()) <= delay {
        return false;
    }
    tokio::time::sleep(delay).await;
    true
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

    fn parse_usdt_order(raw: &str) -> BinaOrderResponse {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn deterministic_http_rejection_is_not_treated_as_unknown() {
        let error = BinanceRequestError::Http {
            status: 400,
            body: r#"{"code":-1111,"msg":"Precision is over the maximum"}"#.to_string(),
        };

        assert!(!error.submission_outcome_is_unknown());
        assert_eq!(error.binance_error_code(), Some(-1111));
        assert!(error.to_string().contains("Precision is over the maximum"));
    }

    #[test]
    fn transport_and_server_errors_have_unknown_submission_outcome() {
        let transport = BinanceRequestError::Transport {
            message: "request timed out".to_string(),
            timeout: true,
            connect: false,
        };
        let server = BinanceRequestError::Http {
            status: 503,
            body: r#"{"code":-1000,"msg":"Unknown error"}"#.to_string(),
        };

        assert!(transport.submission_outcome_is_unknown());
        assert!(server.submission_outcome_is_unknown());
    }

    #[test]
    fn minimal_filled_usdt_response_allows_omitted_nonessential_fields() {
        let order = parse_usdt_order(
            r#"{
                "orderId":8389766260419795834,
                "status":"FILLED",
                "executedQty":"0.010"
            }"#,
        );

        assert_eq!(order.order_id, 8389766260419795834);
        assert_eq!(order.status, "FILLED");
        assert_eq!(order.executed_qty, "0.010");
        assert!(order.avg_price.is_empty());
    }

    #[test]
    fn signing_adds_recv_window_and_replaces_stale_signature() {
        let client = BinanceOrderClient::new("api-key", "api-secret");
        let mut params = BTreeMap::from([
            ("symbol".to_string(), "ETHUSDT".to_string()),
            ("signature".to_string(), "stale".to_string()),
        ]);

        client.sign_params(&mut params);

        assert_eq!(params.get("recvWindow").map(String::as_str), Some("5000"));
        assert_ne!(params.get("signature").map(String::as_str), Some("stale"));
        assert!(params
            .get("timestamp")
            .and_then(|value| value.parse::<i64>().ok())
            .is_some());
    }

    #[test]
    fn filled_usdt_order_with_executed_qty_does_not_need_status_poll() {
        let order = parse_usdt_order(
            r#"{
                "orderId":10526962290,
                "symbol":"HYPEUSDT",
                "status":"FILLED",
                "clientOrderId":"7480434628117794816",
                "price":"0.00000",
                "origQty":"0.58",
                "executedQty":"0.58",
                "cumQty":"0.58",
                "timeInForce":"GTC",
                "type":"MARKET",
                "reduceOnly":false,
                "closePosition":false,
                "side":"BUY",
                "positionSide":"LONG",
                "stopPrice":"0.00000",
                "workingType":"CONTRACT_PRICE",
                "priceProtect":false,
                "origType":"MARKET",
                "priceMatch":"NONE",
                "selfTradePreventionMode":"EXPIRE_MAKER",
                "goodTillDate":0,
                "updateTime":1783474595307
            }"#,
        );

        assert!(!BinanceOrderClient::usdt_needs_order_status_poll(&order));
        assert!(BinanceOrderClient::trade_order_from_usdt_response(&order).is_none());
    }

    #[test]
    fn usdt_trade_order_uses_cum_quote_when_avg_price_is_missing() {
        let order = parse_usdt_order(
            r#"{
                "orderId":1,
                "symbol":"HYPEUSDT",
                "status":"FILLED",
                "clientOrderId":"test",
                "price":"0.00000",
                "avgPrice":"",
                "origQty":"0.58",
                "executedQty":"0.58",
                "cumQty":"0.58",
                "cumQuote":"39.672",
                "timeInForce":"GTC",
                "type":"MARKET",
                "reduceOnly":false,
                "closePosition":false,
                "side":"BUY",
                "positionSide":"LONG",
                "stopPrice":"0.00000",
                "workingType":"CONTRACT_PRICE",
                "priceProtect":false,
                "origType":"MARKET",
                "priceMatch":"NONE",
                "selfTradePreventionMode":"EXPIRE_MAKER",
                "goodTillDate":0,
                "updateTime":1
            }"#,
        );

        let trade_order = BinanceOrderClient::trade_order_from_usdt_response(&order).unwrap();
        assert_eq!(trade_order.amount, BigDecimal::from_str("0.58").unwrap());
        assert_eq!(
            trade_order.sell_price.with_scale(4),
            BigDecimal::from_str("68.4000").unwrap()
        );
    }

    #[test]
    fn usdt_user_trades_are_parsed_and_aggregated() {
        let trades: Vec<FillInfo> = serde_json::from_str(
            r#"[
                {
                    "buyer": true,
                    "commission": "0.00388480",
                    "commissionAsset": "USDT",
                    "id": 1,
                    "maker": false,
                    "orderId": 8389766260419795834,
                    "price": "2428.00",
                    "qty": "0.004",
                    "quoteQty": "9.71200",
                    "realizedPnl": "0",
                    "side": "BUY",
                    "positionSide": "LONG",
                    "symbol": "ETHUSDT",
                    "time": 1787450000000
                },
                {
                    "buyer": true,
                    "commission": "0.00583200",
                    "commissionAsset": "USDT",
                    "id": 2,
                    "maker": false,
                    "orderId": 8389766260419795834,
                    "price": "2430.00",
                    "qty": "0.006",
                    "quoteQty": "14.58000",
                    "realizedPnl": "0",
                    "side": "BUY",
                    "positionSide": "LONG",
                    "symbol": "ETHUSDT",
                    "time": 1787450000001
                }
            ]"#,
        )
        .unwrap();

        let (avg_price, total_fee, executed_qty) = calculate_trade_summary(&trades);

        assert_eq!(
            avg_price.with_scale(2),
            BigDecimal::from_str("2429.20").unwrap()
        );
        assert_eq!(executed_qty, BigDecimal::from_str("0.010").unwrap());
        assert_eq!(total_fee, BigDecimal::from_str("0.00971680").unwrap());
    }

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
    fn http_rejection_with_binance_code_maps_to_api_error() {
        let error = ExchangeClientError::from(BinanceRequestError::Http {
            status: 400,
            body: r#"{"code":-5022,"msg":"Due to the order could not be executed as maker"}"#
                .into(),
        });
        assert_eq!(error.api_code(), Some(-5022));
        assert!(!error.is_unknown_execution());

        let error = ExchangeClientError::from(BinanceRequestError::Http {
            status: 502,
            body: "<html>bad gateway</html>".into(),
        });
        assert!(matches!(error, ExchangeClientError::Order(_)));
    }

    #[test]
    fn debug_output_hides_credentials() {
        let client = BinanceOrderClient::new("key-visible?", "secret-visible?");
        let output = format!("{:?}", client);
        assert!(!output.contains("key-visible"));
        assert!(!output.contains("secret-visible"));
    }
}
