//! U 本位合约 REST 客户端的底座：地址配置、签名、时间同步、错误解析、限频计数。

use crate::error::{ExchangeClientError, Result};
use crate::signing::sign_hmac_sha256_hex;
use reqwest::Method;
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const USDM_REST_URL: &str = "https://fapi.binance.com";
pub const USDM_REST_TESTNET_URL: &str = "https://testnet.binancefuture.com";
/// 高频公开行情（盘口、深度）。
pub const USDM_WS_PUBLIC_URL: &str = "wss://fstream.binance.com/public";
/// 常规行情（K 线、标记价格、成交、强平等）。
pub const USDM_WS_MARKET_URL: &str = "wss://fstream.binance.com/market";
/// 用户数据流。
pub const USDM_WS_PRIVATE_URL: &str = "wss://fstream.binance.com/private";
/// 测试网不分路由，三类数据都走这一个地址。
pub const USDM_WS_TESTNET_URL: &str = "wss://fstream.binancefuture.com";

/// 接口地址和请求参数。默认主网；测试网用 [`UsdmConfig::testnet`]。
#[derive(Clone, Debug)]
pub struct UsdmConfig {
    pub rest_url: String,
    pub ws_public_url: String,
    pub ws_market_url: String,
    pub ws_private_url: String,
    /// 签名请求的 `recvWindow`（毫秒），币安上限 60000。
    pub recv_window_ms: u64,
    /// 单个 HTTP 请求的超时。
    pub timeout: Duration,
}

impl Default for UsdmConfig {
    fn default() -> Self {
        Self::mainnet()
    }
}

impl UsdmConfig {
    pub fn mainnet() -> Self {
        Self {
            rest_url: USDM_REST_URL.into(),
            ws_public_url: USDM_WS_PUBLIC_URL.into(),
            ws_market_url: USDM_WS_MARKET_URL.into(),
            ws_private_url: USDM_WS_PRIVATE_URL.into(),
            recv_window_ms: 5000,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn testnet() -> Self {
        Self {
            rest_url: USDM_REST_TESTNET_URL.into(),
            ws_public_url: USDM_WS_TESTNET_URL.into(),
            ws_market_url: USDM_WS_TESTNET_URL.into(),
            ws_private_url: USDM_WS_TESTNET_URL.into(),
            ..Self::mainnet()
        }
    }

    pub fn recv_window_ms(mut self, value: u64) -> Self {
        self.recv_window_ms = value.min(60_000);
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

/// 请求参数，按加入顺序编码。
#[derive(Clone, Debug, Default)]
pub struct Params(pub(crate) Vec<(&'static str, String)>);

impl Params {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, key: &'static str, value: impl ToString) -> &mut Self {
        self.0.push((key, value.to_string()));
        self
    }

    pub fn opt<T: ToString>(&mut self, key: &'static str, value: Option<T>) -> &mut Self {
        if let Some(value) = value {
            self.0.push((key, value.to_string()));
        }
        self
    }

    pub fn with(mut self, key: &'static str, value: impl ToString) -> Self {
        self.push(key, value);
        self
    }

    pub fn with_opt<T: ToString>(mut self, key: &'static str, value: Option<T>) -> Self {
        self.opt(key, value);
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }
}

struct Credentials {
    api_key: String,
    api_secret: String,
}

struct Inner {
    config: UsdmConfig,
    credentials: Option<Credentials>,
    http: reqwest::Client,
    time_offset_ms: AtomicI64,
    used_weight_1m: AtomicU64,
    order_count_10s: AtomicU64,
    order_count_1m: AtomicU64,
}

/// 币安 U 本位合约客户端。克隆开销很小（内部是 `Arc`），可以在多个任务间共享。
///
/// - 公开行情：[`UsdmClient::public`] 创建，不需要 API Key
/// - 交易和账户：[`UsdmClient::new`] 传入 API Key / Secret
#[derive(Clone)]
pub struct UsdmClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for UsdmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不输出任何凭据。
        f.debug_struct("UsdmClient")
            .field("rest_url", &self.inner.config.rest_url)
            .field("has_credentials", &self.inner.credentials.is_some())
            .finish_non_exhaustive()
    }
}

impl UsdmClient {
    /// 带凭据的客户端，可以调用全部接口。
    pub fn new(
        config: UsdmConfig,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Self {
        Self::build(
            config,
            Some(Credentials {
                api_key: api_key.into(),
                api_secret: api_secret.into(),
            }),
        )
    }

    /// 只能调用公开行情接口的客户端。
    pub fn public(config: UsdmConfig) -> Self {
        Self::build(config, None)
    }

    fn build(config: UsdmConfig, credentials: Option<Credentials>) -> Self {
        let http = reqwest::Client::builder()
            // 签名请求不跟随重定向，避免把 API Key 请求头转发到别的主机。
            .redirect(reqwest::redirect::Policy::none())
            .timeout(config.timeout)
            .build()
            .expect("固定 HTTP 客户端配置无效");
        Self {
            inner: Arc::new(Inner {
                config,
                credentials,
                http,
                time_offset_ms: AtomicI64::new(0),
                used_weight_1m: AtomicU64::new(0),
                order_count_10s: AtomicU64::new(0),
                order_count_1m: AtomicU64::new(0),
            }),
        }
    }

    pub fn config(&self) -> &UsdmConfig {
        &self.inner.config
    }

    pub fn has_credentials(&self) -> bool {
        self.inner.credentials.is_some()
    }

    /// 最近一次响应头里的 1 分钟请求权重（`X-MBX-USED-WEIGHT-1M`），主网上限通常是 2400。
    pub fn used_weight_1m(&self) -> u64 {
        self.inner.used_weight_1m.load(Ordering::Relaxed)
    }

    /// 最近一次下单响应头里的 10 秒 / 1 分钟下单计数（`X-MBX-ORDER-COUNT-10S` / `-1M`）。
    pub fn order_count(&self) -> (u64, u64) {
        (
            self.inner.order_count_10s.load(Ordering::Relaxed),
            self.inner.order_count_1m.load(Ordering::Relaxed),
        )
    }

    /// 本地时钟相对交易所的偏差（毫秒，交易所 − 本地），签名请求的时间戳会加上它。
    pub fn time_offset_ms(&self) -> i64 {
        self.inner.time_offset_ms.load(Ordering::Relaxed)
    }

    /// 用 `/fapi/v1/time` 校准本地时钟，返回偏差（毫秒）。本地时钟不准时出现 `-1021` 错误，先调用它。
    pub async fn sync_time(&self) -> Result<i64> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ServerTime {
            server_time: i64,
        }
        let before = chrono::Utc::now().timestamp_millis();
        let time: ServerTime = self.get_public("/fapi/v1/time", Params::new()).await?;
        let after = chrono::Utc::now().timestamp_millis();
        let offset = time.server_time - (before + after) / 2;
        self.inner.time_offset_ms.store(offset, Ordering::Relaxed);
        Ok(offset)
    }

    pub(crate) fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis() + self.time_offset_ms()
    }

    fn credentials(&self) -> Result<&Credentials> {
        self.inner.credentials.as_ref().ok_or_else(|| {
            ExchangeClientError::Validation(
                "该接口需要 API Key，请用 UsdmClient::new 创建客户端".into(),
            )
        })
    }

    pub(crate) fn api_key(&self) -> Result<&str> {
        Ok(&self.credentials()?.api_key)
    }

    fn url(&self, path: &str) -> Result<reqwest::Url> {
        reqwest::Url::parse(&format!(
            "{}{}",
            self.inner.config.rest_url.trim_end_matches('/'),
            path
        ))
        .map_err(|_| ExchangeClientError::Validation(format!("接口地址无效: {path}")))
    }

    /// 不需要签名、不需要 API Key 的 GET。
    pub async fn get_public<T: DeserializeOwned>(&self, path: &str, params: Params) -> Result<T> {
        let mut url = self.url(path)?;
        if !params.0.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(params.0.iter().map(|(k, v)| (*k, v.as_str())));
        }
        self.send(Method::GET, url, false).await
    }

    /// 只需要 API Key 请求头、不需要签名的请求（如 listenKey）。
    pub async fn request_with_key<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        params: Params,
    ) -> Result<T> {
        self.credentials()?;
        let mut url = self.url(path)?;
        if !params.0.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(params.0.iter().map(|(k, v)| (*k, v.as_str())));
        }
        self.send(method, url, true).await
    }

    /// 签名请求。参数全部放在查询串里，对实际发出的编码后查询串签名。
    pub async fn request_signed<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        params: Params,
    ) -> Result<T> {
        let url = self.signed_url(path, params)?;
        self.send(method, url, true).await
    }

    pub(crate) fn signed_url(&self, path: &str, params: Params) -> Result<reqwest::Url> {
        let secret = &self.credentials()?.api_secret;
        let mut url = self.url(path)?;
        {
            let mut query = url.query_pairs_mut();
            query.extend_pairs(params.0.iter().map(|(k, v)| (*k, v.as_str())));
            query.append_pair("recvWindow", &self.inner.config.recv_window_ms.to_string());
            query.append_pair("timestamp", &self.now_ms().to_string());
        }
        let signature = sign_hmac_sha256_hex(secret, url.query().unwrap_or_default());
        url.query_pairs_mut().append_pair("signature", &signature);
        Ok(url)
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        url: reqwest::Url,
        with_key: bool,
    ) -> Result<T> {
        let path = url.path().to_string();
        let mut request = self.inner.http.request(method, url);
        if with_key {
            request = request.header("X-MBX-APIKEY", self.api_key()?);
        }
        // 错误信息里不能带完整 URL：查询串里有签名。
        let response = request.send().await.map_err(|error| {
            ExchangeClientError::Connection(format!("{path}: {}", error.without_url()))
        })?;
        self.record_limits(response.headers());
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            ExchangeClientError::Connection(format!("{path}: {}", error.without_url()))
        })?;
        if !status.is_success() {
            return Err(parse_api_error(status.as_u16(), &body));
        }
        serde_json::from_str(&body).map_err(|error| {
            ExchangeClientError::ResponseParse(format!(
                "{path}: {error}; body: {}",
                truncate(&body, 500)
            ))
        })
    }

    fn record_limits(&self, headers: &reqwest::header::HeaderMap) {
        let read = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
        };
        if let Some(v) = read("x-mbx-used-weight-1m") {
            self.inner.used_weight_1m.store(v, Ordering::Relaxed);
        }
        if let Some(v) = read("x-mbx-order-count-10s") {
            self.inner.order_count_10s.store(v, Ordering::Relaxed);
        }
        if let Some(v) = read("x-mbx-order-count-1m") {
            self.inner.order_count_1m.store(v, Ordering::Relaxed);
        }
    }
}

pub(crate) fn parse_api_error(status: u16, body: &str) -> ExchangeClientError {
    #[derive(serde::Deserialize)]
    struct ApiError {
        code: i64,
        msg: String,
    }
    match serde_json::from_str::<ApiError>(body) {
        Ok(error) => ExchangeClientError::Api {
            status,
            code: error.code,
            msg: error.msg,
        },
        Err(_) => ExchangeClientError::Api {
            status,
            code: 0,
            msg: truncate(body, 300).to_string(),
        },
    }
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_binance_doc_example() {
        // 币安官方文档里的签名示例。
        let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";
        let query = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        assert_eq!(
            sign_hmac_sha256_hex(secret, query),
            "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
        );
    }

    #[test]
    fn signed_url_signs_the_encoded_query() {
        let client = UsdmClient::new(UsdmConfig::mainnet(), "key", "secret");
        let params = Params::new()
            .with("symbol", "BTCUSDT")
            .with("newClientOrderId", "a:b/c");
        let url = client.signed_url("/fapi/v1/order", params).unwrap();
        let query = url.query().unwrap();
        let (unsigned, signature) = query.rsplit_once("&signature=").unwrap();
        assert!(unsigned
            .starts_with("symbol=BTCUSDT&newClientOrderId=a%3Ab%2Fc&recvWindow=5000&timestamp="));
        assert_eq!(signature, sign_hmac_sha256_hex("secret", unsigned));
    }

    #[test]
    fn api_error_is_parsed() {
        let error = parse_api_error(
            400,
            r#"{"code":-2010,"msg":"Order would immediately match and take."}"#,
        );
        assert_eq!(error.api_code(), Some(-2010));
        assert!(!error.is_unknown_execution());
        assert!(parse_api_error(503, "Service Unavailable").is_unknown_execution());
        assert!(parse_api_error(400, r#"{"code":-1007,"msg":"Timeout"}"#).is_unknown_execution());
        assert!(parse_api_error(429, r#"{"code":-1003,"msg":"Too many"}"#).is_rate_limited());
    }

    #[test]
    fn public_client_rejects_signed_requests() {
        let client = UsdmClient::public(UsdmConfig::testnet());
        assert!(client.signed_url("/fapi/v1/order", Params::new()).is_err());
        assert!(format!("{client:?}").contains("has_credentials: false"));
    }
}
