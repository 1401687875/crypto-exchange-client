use crate::types::{
    CancelOrderRequest, CancelOrderResponseData, InstrumentData, OkxKlineData, OkxResponse,
    OkxTickerData, OrderData, PlaceOrderRequest, PlaceOrderResponseData,
};
use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use std::collections::HashMap;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct OkxApiClient {
    base_url: String,
    api_key: String,
    secret_key: String,
    passphrase: String,
    client: Client,
}

impl OkxApiClient {
    pub fn new_with_credentials(api_key: String, secret_key: String, passphrase: String) -> Self {
        Self {
            base_url: "https://www.okx.com".to_string(),
            api_key,
            secret_key,
            passphrase,
            client: Client::new(),
        }
    }

    pub fn with_base_url(
        base_url: impl Into<String>,
        api_key: String,
        secret_key: String,
        passphrase: String,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            secret_key,
            passphrase,
            client: Client::new(),
        }
    }

    fn sign(&self, method: &str, request_path: &str, body: &str) -> Result<(String, String)> {
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let pre_hash = format!(
            "{}{}{}{}",
            timestamp,
            method.to_uppercase(),
            request_path,
            body
        );

        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| anyhow::anyhow!("HMAC key error: {}", e))?;
        mac.update(pre_hash.as_bytes());
        let signature = general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        Ok((signature, timestamp))
    }

    fn build_headers(
        &self,
        method: &str,
        request_path: &str,
        body: &str,
    ) -> Result<HashMap<String, String>> {
        let (signature, timestamp) = self.sign(method, request_path, body)?;
        let mut headers = HashMap::new();
        headers.insert("OK-ACCESS-KEY".to_string(), self.api_key.clone());
        headers.insert("OK-ACCESS-SIGN".to_string(), signature);
        headers.insert("OK-ACCESS-TIMESTAMP".to_string(), timestamp);
        headers.insert("OK-ACCESS-PASSPHRASE".to_string(), self.passphrase.clone());
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Ok(headers)
    }

    pub async fn place_order(
        &self,
        request: PlaceOrderRequest,
    ) -> Result<OkxResponse<PlaceOrderResponseData>> {
        let path = "/api/v5/trade/order";
        let body = serde_json::to_string(&request)?;
        let headers = self.build_headers("POST", path, &body)?;

        let mut req_builder = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .body(body);

        for (key, value) in headers {
            req_builder = req_builder.header(&key, &value);
        }

        let response_text = req_builder.send().await?.text().await?;
        log::info!("OKX place order response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn get_instruments(
        &self,
        inst_id: &str,
        inst_type: &str,
    ) -> Result<OkxResponse<InstrumentData>> {
        let path = format!(
            "/api/v5/public/instruments?instType={}&instId={}",
            inst_type, inst_id
        );
        let url = format!("{}{}", self.base_url, path);
        log::info!("[OKX API] request URL: {}", url);

        let response_text = self.client.get(&url).send().await?.text().await?;
        log::info!("OKX instruments response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn get_order(&self, inst_id: &str, ord_id: &str) -> Result<OkxResponse<OrderData>> {
        let path = format!("/api/v5/trade/order?instId={}&ordId={}", inst_id, ord_id);
        let headers = self.build_headers("GET", &path, "")?;

        let mut req_builder = self.client.get(format!("{}{}", self.base_url, path));
        for (key, value) in headers {
            req_builder = req_builder.header(&key, &value);
        }

        let response_text = req_builder.send().await?.text().await?;
        log::info!("OKX get order response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn get_order_by_client_order_id(
        &self,
        inst_id: &str,
        cl_ord_id: &str,
    ) -> Result<OkxResponse<OrderData>> {
        let path = format!(
            "/api/v5/trade/order?instId={}&clOrdId={}",
            inst_id, cl_ord_id
        );
        let headers = self.build_headers("GET", &path, "")?;

        let mut req_builder = self.client.get(format!("{}{}", self.base_url, path));
        for (key, value) in headers {
            req_builder = req_builder.header(&key, &value);
        }

        let response_text = req_builder.send().await?.text().await?;
        log::info!(
            "OKX get order by clOrdId response (clOrdId={}): {}",
            cl_ord_id,
            response_text
        );

        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn cancel_order(
        &self,
        request: CancelOrderRequest,
    ) -> Result<OkxResponse<CancelOrderResponseData>> {
        let path = "/api/v5/trade/cancel-order";
        let body = serde_json::to_string(&request)?;
        let headers = self.build_headers("POST", path, &body)?;

        let mut req_builder = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .body(body);

        for (key, value) in headers {
            req_builder = req_builder.header(&key, &value);
        }

        let response_text = req_builder.send().await?.text().await?;
        log::info!("OKX cancel order response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }

    /// 当前未完成的委托（GET /api/v5/trade/orders-pending），`inst_type` 如 `SWAP`、`SPOT`。
    /// 只返回第一页（最多 100 条），用于启动时清理本程序遗留的挂单。
    pub async fn get_pending_orders(&self, inst_type: &str) -> Result<OkxResponse<OrderData>> {
        let path = format!("/api/v5/trade/orders-pending?instType={}", inst_type);
        let headers = self.build_headers("GET", &path, "")?;

        let mut req_builder = self.client.get(format!("{}{}", self.base_url, path));
        for (key, value) in headers {
            req_builder = req_builder.header(&key, &value);
        }

        let response_text = req_builder.send().await?.text().await?;
        log::debug!("OKX pending orders response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn get_ticker(&self, inst_id: &str) -> Result<OkxResponse<OkxTickerData>> {
        let path = format!("/api/v5/market/ticker?instId={}", inst_id);
        let response_text = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await?
            .text()
            .await?;
        log::debug!("OKX ticker response: {}", response_text);
        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn get_klines(
        &self,
        inst_id: &str,
        bar: &str,
        after: Option<i64>,
        before: Option<i64>,
        limit: Option<u32>,
    ) -> Result<OkxResponse<OkxKlineData>> {
        let mut path = format!("/api/v5/market/candles?instId={}&bar={}", inst_id, bar);

        if let Some(after_ts) = after {
            path.push_str(&format!("&after={}", after_ts));
        }
        if let Some(before_ts) = before {
            path.push_str(&format!("&before={}", before_ts));
        }
        if let Some(lmt) = limit {
            path.push_str(&format!("&limit={}", lmt.min(100)));
        }

        let response_text = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await?
            .text()
            .await?;
        log::debug!("OKX klines response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }

    pub async fn get_history_klines(
        &self,
        inst_id: &str,
        bar: &str,
        after: Option<i64>,
        before: Option<i64>,
        limit: Option<u32>,
    ) -> Result<OkxResponse<OkxKlineData>> {
        let mut path = format!(
            "/api/v5/market/history-candles?instId={}&bar={}",
            inst_id, bar
        );

        if let Some(after_ts) = after {
            path.push_str(&format!("&after={}", after_ts));
        }
        if let Some(before_ts) = before {
            path.push_str(&format!("&before={}", before_ts));
        }
        if let Some(lmt) = limit {
            path.push_str(&format!("&limit={}", lmt.min(100)));
        }

        let response_text = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await?
            .text()
            .await?;
        log::debug!("OKX history klines response: {}", response_text);

        Ok(serde_json::from_str(&response_text)?)
    }
}
