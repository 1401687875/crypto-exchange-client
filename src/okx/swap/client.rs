//! OKX 永续合约 REST 客户端的底座：地址配置、签名、模拟盘请求头、统一的错误解析。

use crate::error::{ExchangeClientError, Result};
use base64::{engine::general_purpose, Engine as _};
use hmac::{Hmac, Mac};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use std::time::Duration;

pub const OKX_REST_URL: &str = "https://www.okx.com";

/// 接口地址和请求参数。实盘用 [`SwapConfig::mainnet`]，模拟盘用 [`SwapConfig::demo`]
/// （地址相同，请求带 `x-simulated-trading: 1`，要用模拟盘的 API Key）。
#[derive(Clone, Debug)]
pub struct SwapConfig {
    pub rest_url: String,
    /// 模拟盘。
    pub simulated: bool,
    /// 单个 HTTP 请求的超时。
    pub timeout: Duration,
}

impl Default for SwapConfig {
    fn default() -> Self {
        Self::mainnet()
    }
}

impl SwapConfig {
    pub fn mainnet() -> Self {
        Self {
            rest_url: OKX_REST_URL.into(),
            simulated: false,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn demo() -> Self {
        Self {
            simulated: true,
            ..Self::mainnet()
        }
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

/// 查询参数，按加入顺序编码。
#[derive(Clone, Debug, Default)]
pub struct Query(Vec<(&'static str, String)>);

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: &'static str, value: impl ToString) -> Self {
        self.0.push((key, value.to_string()));
        self
    }

    pub fn with_opt<T: ToString>(mut self, key: &'static str, value: Option<T>) -> Self {
        if let Some(value) = value {
            self.0.push((key, value.to_string()));
        }
        self
    }

    fn encode(&self) -> String {
        let mut out = String::new();
        for (index, (key, value)) in self.0.iter().enumerate() {
            if index > 0 {
                out.push('&');
            }
            out.push_str(key);
            out.push('=');
            out.push_str(&url_encode(value));
        }
        out
    }
}

fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

struct Credentials {
    api_key: String,
    secret: String,
    passphrase: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(***)")
    }
}

#[derive(Debug)]
struct Inner {
    config: SwapConfig,
    http: reqwest::Client,
    credentials: Option<Credentials>,
}

/// OKX 永续合约（U 本位 SWAP）客户端。克隆成本很低（内部是 `Arc`）。
///
/// 所有接口都检查 OKX 的业务错误码：HTTP 200 但 `code != "0"` 时返回 [`ExchangeClientError::Api`]，
/// 下单、撤单这类按条目返回结果的接口取条目里的 `sCode` / `sMsg`。
#[derive(Clone, Debug)]
pub struct SwapClient {
    inner: Arc<Inner>,
}

/// OKX 的统一响应。
#[derive(Deserialize)]
struct Envelope<T> {
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

impl SwapClient {
    pub fn new(
        config: SwapConfig,
        api_key: impl Into<String>,
        secret: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Self {
        Self::build(
            config,
            Some(Credentials {
                api_key: api_key.into(),
                secret: secret.into(),
                passphrase: passphrase.into(),
            }),
        )
    }

    /// 只调公开接口（交易规则、行情）。
    pub fn public(config: SwapConfig) -> Self {
        Self::build(config, None)
    }

    fn build(config: SwapConfig, credentials: Option<Credentials>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                config,
                http,
                credentials,
            }),
        }
    }

    pub fn config(&self) -> &SwapConfig {
        &self.inner.config
    }

    pub fn has_credentials(&self) -> bool {
        self.inner.credentials.is_some()
    }

    /// 公开接口，返回 `data` 数组。
    pub async fn get_public<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Query,
    ) -> Result<Vec<T>> {
        self.send(Method::GET, path, &query, None, false).await
    }

    /// 签名的 GET，返回 `data` 数组。
    pub async fn get_signed<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Query,
    ) -> Result<Vec<T>> {
        self.send(Method::GET, path, &query, None, true).await
    }

    /// 签名的 POST（JSON 请求体），返回 `data` 数组。
    pub async fn post_signed<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<Vec<T>> {
        let body = serde_json::to_string(body)?;
        self.send(Method::POST, path, &Query::new(), Some(body), true)
            .await
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &Query,
        body: Option<String>,
        signed: bool,
    ) -> Result<Vec<T>> {
        let query = query.encode();
        let request_path = if query.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{query}")
        };
        let url = format!(
            "{}{}",
            self.inner.config.rest_url.trim_end_matches('/'),
            request_path
        );
        let mut request = self
            .inner
            .http
            .request(method.clone(), &url)
            .header("Content-Type", "application/json");
        if self.inner.config.simulated {
            request = request.header("x-simulated-trading", "1");
        }
        if signed {
            let credentials = self.inner.credentials.as_ref().ok_or_else(|| {
                ExchangeClientError::Validation("这个接口要 API Key（客户端是公开模式）".into())
            })?;
            let timestamp = chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string();
            let signature = sign(
                &credentials.secret,
                &timestamp,
                method.as_str(),
                &request_path,
                body.as_deref().unwrap_or(""),
            );
            request = request
                .header("OK-ACCESS-KEY", &credentials.api_key)
                .header("OK-ACCESS-SIGN", signature)
                .header("OK-ACCESS-TIMESTAMP", timestamp)
                .header("OK-ACCESS-PASSPHRASE", &credentials.passphrase);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.map_err(|error| {
            ExchangeClientError::Connection(format!("{path}: {}", error.without_url()))
        })?;
        let status = response.status().as_u16();
        let text = response.text().await.map_err(|error| {
            ExchangeClientError::Connection(format!("{path}: {}", error.without_url()))
        })?;
        parse_envelope(status, path, &text)
    }
}

/// OKX 签名：Base64(HMAC-SHA256(时间戳 + 方法 + 请求路径（含查询串）+ 请求体))。
pub fn sign(secret: &str, timestamp: &str, method: &str, request_path: &str, body: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC 可以用任意长度的密钥");
    mac.update(timestamp.as_bytes());
    mac.update(method.to_uppercase().as_bytes());
    mac.update(request_path.as_bytes());
    mac.update(body.as_bytes());
    general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// 解析响应：`code == "0"` 时返回 `data`；否则返回 [`ExchangeClientError::Api`]。
/// 按条目返回结果的接口（下单、撤单等）失败时外层是 `code = "1"`（批量是 `"2"`），这时取第一条失败条目的 `sCode` / `sMsg`。
pub(crate) fn parse_envelope<T: DeserializeOwned>(
    status: u16,
    path: &str,
    text: &str,
) -> Result<Vec<T>> {
    let raw: Envelope<serde_json::Value> = match serde_json::from_str(text) {
        Ok(raw) => raw,
        Err(error) => {
            return Err(if !(200..300).contains(&status) {
                ExchangeClientError::Api {
                    status,
                    code: 0,
                    msg: truncate(text, 300).to_string(),
                }
            } else {
                ExchangeClientError::ResponseParse(format!(
                    "{path}: {error}; body: {}",
                    truncate(text, 500)
                ))
            })
        }
    };
    if raw.code != "0" {
        let item = raw.data.iter().find(|item| {
            item.get("sCode")
                .and_then(|c| c.as_str())
                .is_some_and(|c| !c.is_empty() && c != "0")
        });
        let (code, msg) = match item {
            Some(item) => (
                item["sCode"].as_str().unwrap_or_default().to_string(),
                item["sMsg"].as_str().unwrap_or_default().to_string(),
            ),
            None => (raw.code.clone(), raw.msg.clone()),
        };
        return Err(ExchangeClientError::Api {
            status,
            code: code.parse().unwrap_or(0),
            msg: format!("{path}: {msg}"),
        });
    }
    raw.data
        .into_iter()
        .map(|item| {
            serde_json::from_value(item).map_err(|error| {
                ExchangeClientError::ResponseParse(format!(
                    "{path}: {error}; body: {}",
                    truncate(text, 500)
                ))
            })
        })
        .collect()
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
    fn signature_matches_okx_doc_style() {
        // 自己算一遍：HMAC-SHA256 后 Base64。
        let sig = sign(
            "22582BD0CFF14C41EDBF1AB98506286D",
            "2020-12-08T09:08:57.715Z",
            "get",
            "/api/v5/account/balance?ccy=BTC",
            "",
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(b"22582BD0CFF14C41EDBF1AB98506286D").unwrap();
        mac.update(b"2020-12-08T09:08:57.715ZGET/api/v5/account/balance?ccy=BTC");
        assert_eq!(
            sig,
            general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        );
    }

    #[test]
    fn business_errors_are_parsed() {
        let ok: Vec<serde_json::Value> =
            parse_envelope(200, "/x", r#"{"code":"0","msg":"","data":[{"a":1}]}"#).unwrap();
        assert_eq!(ok.len(), 1);
        let err = parse_envelope::<serde_json::Value>(
            200,
            "/x",
            r#"{"code":"51000","msg":"Parameter instId error","data":[]}"#,
        )
        .unwrap_err();
        assert_eq!(err.api_code(), Some(51000));
        let err = parse_envelope::<serde_json::Value>(
            200,
            "/api/v5/trade/order",
            r#"{"code":"1","msg":"All operations failed","data":[{"ordId":"","clOrdId":"a","sCode":"51008","sMsg":"Order failed. Insufficient margin."}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.api_code(), Some(51008));
        assert!(err.to_string().contains("Insufficient"));
        let err = parse_envelope::<serde_json::Value>(503, "/x", "<html>busy</html>").unwrap_err();
        assert!(err.is_unknown_execution());
        let err = parse_envelope::<serde_json::Value>(
            200,
            "/x",
            r#"{"code":"50004","msg":"API endpoint request timeout","data":[]}"#,
        )
        .unwrap_err();
        assert!(err.is_unknown_execution());
    }

    #[test]
    fn query_is_encoded_in_order() {
        let q = Query::new()
            .with("instType", "SWAP")
            .with("instId", "BTC-USDT-SWAP")
            .with_opt::<u32>("limit", None);
        assert_eq!(q.encode(), "instType=SWAP&instId=BTC-USDT-SWAP");
    }
}
