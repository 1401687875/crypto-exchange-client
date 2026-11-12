use crate::error::{ExchangeClientError, Result};
use crate::hyperliquid::precision::{
    fetch_precision, is_spot_contract, normalize_perp_symbol, normalize_spot_symbol,
};
use crate::types::TradeOrder;
use bigdecimal::{BigDecimal, Zero};
use ethers::{signers::LocalWallet, types::H160};
use hyperliquid_rust_sdk::{
    BaseUrl, ClientLimit, ClientOrder, ClientOrderRequest, ExchangeClient, ExchangeDataStatus,
    ExchangeResponseStatus, InfoClient, MarketOrderParams,
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, str::FromStr, time::Duration};
use uuid::Uuid;

const LIMIT_OFFSET_BPS: f64 = 1.0;
const LIMIT_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const ORDER_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(300);
const LIMIT_TIF_POST_ONLY: &str = "Alo";
const MIN_ORDER_NOTIONAL_USD: f64 = 10.0;
const NEW_CLIENT_RETRY_TIMES: usize = 3;
const NEW_CLIENT_RETRY_DELAY_MS: u64 = 400;
const BUILDER_ASSET_ID_OFFSET_BASE: u32 = 100_000;
const BUILDER_ASSET_ID_DEX_STRIDE: u32 = 10_000;

#[derive(Clone, Copy, Debug)]
pub struct HyperliquidMakerConfig {
    pub offset_bps: f64,
    pub timeout_ms: u64,
    pub poll_ms: u64,
    pub fallback_market_enabled: bool,
    pub fallback_slippage: Option<f64>,
}

impl Default for HyperliquidMakerConfig {
    fn default() -> Self {
        Self {
            offset_bps: LIMIT_OFFSET_BPS,
            timeout_ms: LIMIT_WAIT_TIMEOUT.as_millis() as u64,
            poll_ms: ORDER_STATUS_POLL_INTERVAL.as_millis() as u64,
            fallback_market_enabled: false,
            fallback_slippage: Some(0.01),
        }
    }
}

pub struct HyperliquidOrderClient {
    exchange_client: ExchangeClient,
    use_testnet: bool,
}

impl HyperliquidOrderClient {
    pub async fn new(
        private_key: &str,
        use_testnet: bool,
        vault_address: Option<&str>,
    ) -> Result<Self> {
        let base_url = if use_testnet {
            BaseUrl::Testnet
        } else {
            BaseUrl::Mainnet
        };

        let wallet_key = private_key.trim().trim_start_matches("0x");
        let wallet: LocalWallet = wallet_key
            .parse()
            .map_err(|e| ExchangeClientError::Signing(format!("invalid private key: {}", e)))?;

        let parsed_vault = parse_vault_address(vault_address)?;

        let mut last_err: Option<String> = None;
        let mut exchange_client_opt: Option<ExchangeClient> = None;
        for attempt in 1..=NEW_CLIENT_RETRY_TIMES {
            match ExchangeClient::new(None, wallet.clone(), Some(base_url), None, parsed_vault)
                .await
            {
                Ok(client) => {
                    exchange_client_opt = Some(client);
                    break;
                }
                Err(err) => {
                    let msg = err.to_string();
                    last_err = Some(msg.clone());
                    log::warn!(
                        "⚠️ [Hyperliquid] 初始化 ExchangeClient 失败，准备重试 | attempt: {}/{} | use_testnet: {} | reason: {}",
                        attempt,
                        NEW_CLIENT_RETRY_TIMES,
                        use_testnet,
                        msg
                    );
                    if attempt < NEW_CLIENT_RETRY_TIMES {
                        tokio::time::sleep(Duration::from_millis(
                            NEW_CLIENT_RETRY_DELAY_MS * attempt as u64,
                        ))
                        .await;
                    }
                }
            }
        }

        let exchange_client = exchange_client_opt.ok_or_else(|| {
            ExchangeClientError::Connection(
                last_err.unwrap_or_else(|| "failed to initialize exchange client".to_string()),
            )
        })?;
        let mut exchange_client = exchange_client;
        if let Err(err) = enrich_builder_assets_map(&mut exchange_client).await {
            log::warn!(
                "⚠️ [Hyperliquid] 初始化 builder 资产映射失败（不影响主市场）: {}",
                err
            );
        }

        Ok(Self {
            exchange_client,
            use_testnet,
        })
    }

    pub async fn place_post_only_order(
        &self,
        symbol: &str,
        is_buy: bool,
        quantity: &BigDecimal,
        contract_type: Option<&str>,
        client_order_id: Option<&str>,
        config: HyperliquidMakerConfig,
    ) -> Result<TradeOrder> {
        let (asset, normalized_sz, price_precision, quantity_precision) = self
            .normalize_order_params(symbol, is_buy, quantity, contract_type)
            .await?;
        let cloid = cloid_from_client_order_id(client_order_id);

        let limit_px = self
            .build_limit_price(&asset, is_buy, price_precision, config.offset_bps)
            .await?;
        let normalized_sz = ensure_min_notional_size(
            &asset,
            normalized_sz,
            limit_px,
            quantity_precision,
            MIN_ORDER_NOTIONAL_USD,
        )?;

        let placement = self
            .place_limit_order(
                &asset,
                is_buy,
                normalized_sz,
                limit_px,
                price_precision,
                cloid,
            )
            .await?;

        match placement {
            LimitPlacement::Filled(order) => Ok(order),
            LimitPlacement::Resting(oid) => {
                log::info!(
                    "🟪 [Hyperliquid] 挂单已进入订单簿，立即返回 pending | asset: {} | oid: {} | is_buy: {} | sz: {} | offset_bps: {}",
                    asset,
                    oid,
                    is_buy,
                    normalized_sz,
                    config.offset_bps
                );
                Ok(TradeOrder {
                    fee: BigDecimal::from(0),
                    sell_price: BigDecimal::from(0),
                    amount: BigDecimal::from(0),
                })
            }
        }
    }

    pub async fn place_market_order(
        &self,
        symbol: &str,
        is_buy: bool,
        quantity: &BigDecimal,
        contract_type: Option<&str>,
        slippage: Option<f64>,
        client_order_id: Option<&str>,
    ) -> Result<TradeOrder> {
        let (asset, normalized_sz, _price_precision, _quantity_precision) = self
            .normalize_order_params(symbol, is_buy, quantity, contract_type)
            .await?;
        let response = self
            .exchange_client
            .market_open(MarketOrderParams {
                asset: asset.as_str(),
                is_buy,
                sz: normalized_sz,
                px: None,
                slippage,
                cloid: cloid_from_client_order_id(client_order_id),
                wallet: None,
            })
            .await
            .map_err(|e| ExchangeClientError::Order(e.to_string()))?;
        parse_market_order_response(response)
    }

    async fn normalize_order_params(
        &self,
        symbol: &str,
        _is_buy: bool,
        quantity: &BigDecimal,
        contract_type: Option<&str>,
    ) -> Result<(String, f64, u32, u32)> {
        let precision = fetch_precision(symbol, contract_type, self.use_testnet).await?;
        // 使用 precision 返回的 inst_id 作为真实下单资产标识。
        // 对于 HIP-3 builder perp，这里可能是 `xyz:XYZ100` 这类带 dex 前缀的标识。
        let asset = if precision.inst_id.is_empty() {
            if is_spot_contract(contract_type) {
                normalize_spot_symbol(symbol)
            } else {
                normalize_perp_symbol(symbol)
            }
        } else {
            precision.inst_id.clone()
        };
        // 先用 Decimal 精度截断，避免 f64 浮点误差导致 0.00013 -> 0.00012
        let normalized_quantity_bd = quantity.with_scale(precision.quantity_precision as i64);
        if normalized_quantity_bd <= BigDecimal::zero() {
            return Err(ExchangeClientError::Validation(
                "quantity becomes zero after precision normalization".to_string(),
            ));
        }
        let normalized_sz = normalized_quantity_bd
            .to_string()
            .parse::<f64>()
            .map_err(|e| ExchangeClientError::Validation(format!("invalid quantity: {}", e)))?;

        Ok((
            asset,
            normalized_sz,
            precision.price_precision,
            precision.quantity_precision,
        ))
    }

    async fn build_limit_price(
        &self,
        asset: &str,
        is_buy: bool,
        price_precision: u32,
        offset_bps: f64,
    ) -> Result<f64> {
        let info_client = self.new_info_client().await?;
        let mid = resolve_mid_price(&info_client, asset).await?;

        let offset = offset_bps.max(0.0) / 10_000.0;
        let raw_px = if is_buy {
            mid * (1.0 - offset)
        } else {
            mid * (1.0 + offset)
        };

        // Hyperliquid 价格精度规则：
        // 1) 最多 5 位有效数字
        // 2) perp 最大小数位 = 6 - szDecimals（这里 price_precision 已由 meta 推导）
        let limit_px = round_to_significant_and_decimal(raw_px.max(0.0), 5, price_precision);
        let limit_px = round_to_decimals(limit_px, 8);
        if limit_px <= 0.0 {
            return Err(ExchangeClientError::Validation(format!(
                "invalid limit price computed for {}: {}",
                asset, limit_px
            )));
        }

        log::info!(
            "🟪 [Hyperliquid] 价格构建 | asset: {} | is_buy: {} | mid: {} | offset_bps: {} | raw_px: {} | price_precision: {} | limit_px: {} | limit_px_wire: {}",
            asset,
            is_buy,
            mid,
            offset_bps,
            raw_px,
            price_precision,
            limit_px,
            float_to_wire_string(limit_px),
        );

        Ok(limit_px)
    }

    async fn place_limit_order(
        &self,
        asset: &str,
        is_buy: bool,
        sz: f64,
        limit_px: f64,
        price_precision: u32,
        cloid: Option<Uuid>,
    ) -> Result<LimitPlacement> {
        let asset_id = self.exchange_client.coin_to_asset.get(asset).copied();
        log::info!(
            "🟪 [Hyperliquid] 资产映射检查 | asset: {} | asset_id: {:?}",
            asset,
            asset_id
        );

        let mut candidates = vec![round_to_decimals(limit_px, 8)];
        candidates.extend(build_tick_retry_prices(limit_px, is_buy, price_precision));
        dedup_prices_by_wire(&mut candidates);
        let mut last_tick_error: Option<String> = None;

        for (idx, price) in candidates.iter().enumerate() {
            let price = *price;
            log::info!(
                "🟪 [Hyperliquid] 提交挂单 | asset: {} | is_buy: {} | sz: {} | price: {} | price_wire: {} | attempt: {}/{}",
                asset,
                is_buy,
                sz,
                price,
                float_to_wire_string(price),
                idx + 1,
                candidates.len(),
            );

            let response = self
                .exchange_client
                .order(
                    ClientOrderRequest {
                        asset: asset.to_string(),
                        is_buy,
                        reduce_only: false,
                        limit_px: price,
                        sz,
                        cloid,
                        order_type: ClientOrder::Limit(ClientLimit {
                            tif: LIMIT_TIF_POST_ONLY.to_string(),
                        }),
                    },
                    None,
                )
                .await
                .map_err(|e| ExchangeClientError::Order(e.to_string()))?;

            match parse_limit_order_response(response) {
                Ok(placement) => return Ok(placement),
                Err(ExchangeClientError::Order(error_msg)) => {
                    if is_tick_size_error(&error_msg) && idx + 1 < candidates.len() {
                        log::warn!(
                            "⚠️ [Hyperliquid] 挂单价格被 tick-size 拒绝，尝试下一个价格 | asset: {} | is_buy: {} | current_price: {} | current_wire: {} | reason: {}",
                            asset,
                            is_buy,
                            price,
                            float_to_wire_string(price),
                            error_msg,
                        );
                        last_tick_error = Some(error_msg);
                        continue;
                    }
                    return Err(ExchangeClientError::Order(error_msg));
                }
                Err(other) => return Err(other),
            }
        }

        Err(ExchangeClientError::Order(last_tick_error.unwrap_or_else(
            || "order rejected by tick size on all retry prices".to_string(),
        )))
    }

    async fn new_info_client(&self) -> Result<InfoClient> {
        InfoClient::new(None, Some(self.base_url()))
            .await
            .map_err(|e| ExchangeClientError::Connection(e.to_string()))
    }

    fn base_url(&self) -> BaseUrl {
        if self.use_testnet {
            BaseUrl::Testnet
        } else {
            BaseUrl::Mainnet
        }
    }
}

fn parse_vault_address(vault_address: Option<&str>) -> Result<Option<H160>> {
    match vault_address.map(str::trim) {
        Some(value) if !value.is_empty() => value
            .parse::<H160>()
            .map(Some)
            .map_err(|e| ExchangeClientError::Validation(format!("invalid vault address: {}", e))),
        _ => Ok(None),
    }
}

fn cloid_from_client_order_id(client_order_id: Option<&str>) -> Option<Uuid> {
    let id = client_order_id.map(str::trim).unwrap_or_default();
    if id.is_empty() {
        return None;
    }

    Uuid::parse_str(id)
        .ok()
        .or_else(|| Some(Uuid::new_v5(&Uuid::NAMESPACE_OID, id.as_bytes())))
}

fn parse_market_order_response(response: ExchangeResponseStatus) -> Result<TradeOrder> {
    let response = match response {
        ExchangeResponseStatus::Ok(data) => data,
        ExchangeResponseStatus::Err(error) => {
            return Err(ExchangeClientError::Order(error));
        }
    };

    let status = response
        .data
        .and_then(|d| d.statuses.into_iter().next())
        .ok_or_else(|| ExchangeClientError::Order("empty order status".to_string()))?;

    match status {
        ExchangeDataStatus::Filled(filled) => Ok(TradeOrder {
            fee: BigDecimal::from(0),
            sell_price: BigDecimal::from_str(&filled.avg_px).unwrap_or_default(),
            amount: BigDecimal::from_str(&filled.total_sz).unwrap_or_default(),
        }),
        ExchangeDataStatus::Resting(order) => Err(ExchangeClientError::Order(format!(
            "order still resting: oid={}",
            order.oid
        ))),
        ExchangeDataStatus::Error(error) => Err(ExchangeClientError::Order(error)),
        ExchangeDataStatus::WaitingForFill => Err(ExchangeClientError::Order(
            "order waiting for fill".to_string(),
        )),
        ExchangeDataStatus::WaitingForTrigger => Err(ExchangeClientError::Order(
            "order waiting for trigger".to_string(),
        )),
        ExchangeDataStatus::Success => Err(ExchangeClientError::Order(
            "unexpected order status: success".to_string(),
        )),
    }
}

async fn resolve_mid_price(info_client: &InfoClient, asset: &str) -> Result<f64> {
    let mids = info_client
        .all_mids()
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;

    if let Some((matched_key, mid_raw)) = find_mid_in_map(&mids, asset) {
        let mid = parse_mid(mid_raw).ok_or_else(|| {
            ExchangeClientError::ResponseParse(format!(
                "invalid mid price from allMids | asset={} | key={} | value={}",
                asset, matched_key, mid_raw
            ))
        })?;
        log::info!(
            "🟪 [Hyperliquid] 使用 allMids 价格 | asset: {} | matched_key: {} | mid: {}",
            asset,
            matched_key,
            mid
        );
        return Ok(mid);
    }

    if let Some((dex, _)) = asset.split_once(':') {
        match fetch_all_mids_by_dex(info_client, dex).await {
            Ok(dex_mids) => {
                if let Some((matched_key, mid_raw)) = find_mid_in_map(&dex_mids, asset) {
                    let mid = parse_mid(mid_raw).ok_or_else(|| {
                        ExchangeClientError::ResponseParse(format!(
                            "invalid mid price from allMids(dex) | asset={} | key={} | value={}",
                            asset, matched_key, mid_raw
                        ))
                    })?;
                    log::info!(
                        "🟪 [Hyperliquid] 使用 allMids(dex) 价格 | asset: {} | dex: {} | matched_key: {} | mid: {}",
                        asset,
                        dex,
                        matched_key,
                        mid
                    );
                    return Ok(mid);
                }

                log::warn!(
                    "⚠️ [Hyperliquid] allMids(dex) 未命中资产 | asset: {} | dex: {} | keys_count: {}",
                    asset,
                    dex,
                    dex_mids.len()
                );
            }
            Err(err) => {
                log::warn!(
                    "⚠️ [Hyperliquid] allMids(dex) 获取失败，尝试 l2Book 兜底 | asset: {} | dex: {} | reason: {}",
                    asset,
                    dex,
                    err
                );
            }
        }
    }

    if let Some(mid) = resolve_mid_from_l2_snapshot(info_client, asset).await? {
        log::info!(
            "🟪 [Hyperliquid] 使用 l2Book 中间价 | asset: {} | mid: {}",
            asset,
            mid
        );
        return Ok(mid);
    }

    Err(ExchangeClientError::Validation(format!(
        "mid price not found: {}",
        asset
    )))
}

fn find_mid_in_map<'a>(
    mids: &'a HashMap<String, String>,
    asset: &str,
) -> Option<(String, &'a str)> {
    if let Some(v) = mids.get(asset) {
        return Some((asset.to_string(), v.as_str()));
    }

    if let Some((k, v)) = mids
        .iter()
        .find(|(k, _)| k.as_str().eq_ignore_ascii_case(asset))
    {
        return Some((k.to_string(), v.as_str()));
    }

    let asset_tail = asset.rsplit(':').next().unwrap_or(asset);
    if let Some((k, v)) = mids.iter().find(|(k, _)| {
        let key_tail = k.rsplit(':').next().unwrap_or(k.as_str());
        key_tail.eq_ignore_ascii_case(asset_tail)
    }) {
        return Some((k.to_string(), v.as_str()));
    }

    None
}

async fn fetch_all_mids_by_dex(
    info_client: &InfoClient,
    dex: &str,
) -> Result<HashMap<String, String>> {
    let url = format!(
        "{}/info",
        info_client.http_client.base_url.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(&url)
        .json(&json!({
            "type": "allMids",
            "dex": dex.to_lowercase(),
        }))
        .send()
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;

    if !response.status().is_success() {
        return Err(ExchangeClientError::Connection(format!(
            "allMids(dex) http error: status={}",
            response.status()
        )));
    }

    response
        .json::<HashMap<String, String>>()
        .await
        .map_err(|e| ExchangeClientError::ResponseParse(e.to_string()))
}

async fn resolve_mid_from_l2_snapshot(
    info_client: &InfoClient,
    asset: &str,
) -> Result<Option<f64>> {
    for coin in build_l2_coin_candidates(asset) {
        let snapshot = match info_client.l2_snapshot(coin.clone()).await {
            Ok(s) => s,
            Err(err) => {
                log::warn!(
                    "⚠️ [Hyperliquid] l2Book 查询失败 | asset: {} | coin_candidate: {} | reason: {}",
                    asset,
                    coin,
                    err
                );
                continue;
            }
        };

        let best_bid = snapshot
            .levels
            .first()
            .and_then(|levels| levels.first())
            .and_then(|lvl| parse_mid(&lvl.px));
        let best_ask = snapshot
            .levels
            .get(1)
            .and_then(|levels| levels.first())
            .and_then(|lvl| parse_mid(&lvl.px));

        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) if bid > 0.0 && ask > 0.0 => {
                return Ok(Some((bid + ask) / 2.0));
            }
            (Some(bid), None) if bid > 0.0 => return Ok(Some(bid)),
            (None, Some(ask)) if ask > 0.0 => return Ok(Some(ask)),
            _ => {}
        }
    }

    Ok(None)
}

fn build_l2_coin_candidates(asset: &str) -> Vec<String> {
    use std::collections::HashSet;

    let mut candidates = vec![asset.trim().to_string()];
    if let Some((dex, coin)) = asset.split_once(':') {
        let dex_lower = dex.trim().to_lowercase();
        let coin_upper = coin.trim().to_uppercase();
        candidates.push(format!("{}:{}", dex_lower, coin_upper));
        candidates.push(coin_upper.clone());
        candidates.push(coin.trim().to_string());
    } else {
        candidates.push(asset.to_uppercase());
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

fn parse_mid(raw: &str) -> Option<f64> {
    raw.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn ensure_min_notional_size(
    asset: &str,
    sz: f64,
    price: f64,
    quantity_precision: u32,
    min_notional: f64,
) -> Result<f64> {
    if !sz.is_finite() || !price.is_finite() || sz <= 0.0 || price <= 0.0 {
        return Err(ExchangeClientError::Validation(format!(
            "invalid size/price for min-notional check | asset: {} | sz: {} | price: {}",
            asset, sz, price
        )));
    }

    let current_notional = sz * price;
    if current_notional + 1e-12 >= min_notional {
        return Ok(sz);
    }

    let required_sz = min_notional / price;
    let adjusted_sz = ceil_to_decimals(required_sz, quantity_precision);
    if adjusted_sz <= 0.0 || !adjusted_sz.is_finite() {
        return Err(ExchangeClientError::Validation(format!(
            "unable to compute min-notional size | asset: {} | required_sz: {} | adjusted_sz: {}",
            asset, required_sz, adjusted_sz
        )));
    }

    log::warn!(
        "⚠️ [Hyperliquid] 数量低于最小名义金额，自动上调 | asset: {} | old_sz: {} | price: {} | old_notional: {} | min_notional: {} | new_sz: {} | quantity_precision: {}",
        asset,
        sz,
        price,
        current_notional,
        min_notional,
        adjusted_sz,
        quantity_precision
    );

    Ok(adjusted_sz)
}

fn parse_limit_order_response(response: ExchangeResponseStatus) -> Result<LimitPlacement> {
    match response {
        ExchangeResponseStatus::Ok(data) => {
            let status = data
                .data
                .and_then(|d| d.statuses.into_iter().next())
                .ok_or_else(|| ExchangeClientError::Order("empty order status".to_string()))?;

            match status {
                ExchangeDataStatus::Filled(filled) => Ok(LimitPlacement::Filled(TradeOrder {
                    fee: BigDecimal::from(0),
                    sell_price: BigDecimal::from_str(&filled.avg_px).unwrap_or_default(),
                    amount: BigDecimal::from_str(&filled.total_sz).unwrap_or_default(),
                })),
                ExchangeDataStatus::Resting(resting) => Ok(LimitPlacement::Resting(resting.oid)),
                ExchangeDataStatus::Error(error) => Err(ExchangeClientError::Order(error)),
                ExchangeDataStatus::WaitingForFill => Err(ExchangeClientError::Order(
                    "unexpected order status: waiting for fill".to_string(),
                )),
                ExchangeDataStatus::WaitingForTrigger => Err(ExchangeClientError::Order(
                    "unexpected order status: waiting for trigger".to_string(),
                )),
                ExchangeDataStatus::Success => Err(ExchangeClientError::Order(
                    "unexpected order status: success".to_string(),
                )),
            }
        }
        ExchangeResponseStatus::Err(error) => Err(ExchangeClientError::Order(error)),
    }
}

fn floor_to_decimals(value: f64, decimals: u32) -> f64 {
    if decimals == 0 {
        return value.floor();
    }
    let factor = 10f64.powi(decimals as i32);
    (value * factor).floor() / factor
}

fn ceil_to_decimals(value: f64, decimals: u32) -> f64 {
    if decimals == 0 {
        return value.ceil();
    }
    let factor = 10f64.powi(decimals as i32);
    (value * factor).ceil() / factor
}

fn round_to_decimals(value: f64, decimals: u32) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

fn round_to_significant_and_decimal(value: f64, sig_figs: u32, max_decimals: u32) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    let abs_value = value.abs();
    let magnitude = abs_value.log10().floor() as i32;
    let scale = 10f64.powi(sig_figs as i32 - magnitude - 1);
    let rounded = (abs_value * scale).round() / scale;
    round_to_decimals(rounded.copysign(value), max_decimals)
}

fn build_tick_retry_prices(limit_px: f64, is_buy: bool, price_precision: u32) -> Vec<f64> {
    let mut prices = Vec::new();
    let aligned_by_precision = if is_buy {
        floor_to_decimals(limit_px, price_precision)
    } else {
        ceil_to_decimals(limit_px, price_precision)
    };
    prices.push(round_to_decimals(aligned_by_precision, 8));

    if price_precision > 0 {
        let stricter = if is_buy {
            floor_to_decimals(limit_px, price_precision - 1)
        } else {
            ceil_to_decimals(limit_px, price_precision - 1)
        };
        prices.push(round_to_decimals(stricter, 8));
    }

    let integer_px = if is_buy {
        limit_px.floor()
    } else {
        limit_px.ceil()
    };
    prices.push(round_to_decimals(integer_px, 8));

    let shifted_integer = if is_buy {
        (integer_px - 1.0).max(0.0)
    } else {
        integer_px + 1.0
    };
    prices.push(round_to_decimals(shifted_integer, 8));

    prices.retain(|p| p.is_finite() && *p > 0.0);
    prices
}

fn dedup_prices_by_wire(prices: &mut Vec<f64>) {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    prices.retain(|p| seen.insert(float_to_wire_string(*p)));
}

fn is_tick_size_error(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("tick size") || msg.contains("ticksize") || msg.contains("price must be divisible")
}

fn float_to_wire_string(x: f64) -> String {
    let mut s = format!("{:.8}", x);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

#[derive(Debug, Deserialize)]
struct PerpDexInfo {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerpMetaResponse {
    universe: Vec<PerpAssetMeta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerpAssetMeta {
    name: String,
}

async fn enrich_builder_assets_map(exchange_client: &mut ExchangeClient) -> Result<()> {
    let url = format!(
        "{}/info",
        exchange_client.http_client.base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::new();

    let perp_dex_resp = client
        .post(&url)
        .json(&json!({ "type": "perpDexs" }))
        .send()
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;
    if !perp_dex_resp.status().is_success() {
        return Err(ExchangeClientError::Connection(format!(
            "perpDexs request failed: status={}",
            perp_dex_resp.status()
        )));
    }

    let perp_dexes: Vec<Option<PerpDexInfo>> = perp_dex_resp
        .json()
        .await
        .map_err(|e| ExchangeClientError::ResponseParse(e.to_string()))?;

    let mut inserted = 0usize;
    for (perp_dex_index, dex_opt) in perp_dexes.into_iter().enumerate() {
        // 官方映射里 index=0 是默认市场，不按 builder 公式处理
        if perp_dex_index == 0 {
            continue;
        }
        let Some(dex) = dex_opt else {
            continue;
        };
        if dex.name.trim().is_empty() {
            continue;
        }

        let meta_resp = client
            .post(&url)
            .json(&json!({
                "type": "meta",
                "dex": dex.name,
            }))
            .send()
            .await
            .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;
        if !meta_resp.status().is_success() {
            continue;
        }

        let meta: PerpMetaResponse = match meta_resp.json().await {
            Ok(v) => v,
            Err(_) => continue,
        };

        let offset =
            BUILDER_ASSET_ID_OFFSET_BASE + (perp_dex_index as u32) * BUILDER_ASSET_ID_DEX_STRIDE;
        for (index_in_meta, asset) in meta.universe.into_iter().enumerate() {
            let asset_id = offset + index_in_meta as u32;
            // 优先保留已有映射，避免覆盖默认市场键
            if !exchange_client.coin_to_asset.contains_key(&asset.name) {
                exchange_client
                    .coin_to_asset
                    .insert(asset.name.clone(), asset_id);
                inserted += 1;
            }
        }
    }

    if inserted > 0 {
        log::info!(
            "🟪 [Hyperliquid] builder 资产映射已加载 | count: {}",
            inserted
        );
    }

    Ok(())
}

enum LimitPlacement {
    Filled(TradeOrder),
    Resting(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_float_to_wire_string() {
        assert_eq!(float_to_wire_string(93906.0), "93906");
        assert_eq!(float_to_wire_string(0.1000000001), "0.1");
        assert_eq!(float_to_wire_string(-0.0), "0");
        assert_eq!(float_to_wire_string(12.345678901), "12.3456789");
    }

    #[test]
    fn test_build_tick_retry_prices_buy() {
        let mut prices = build_tick_retry_prices(93906.234567, true, 1);
        dedup_prices_by_wire(&mut prices);
        assert!(!prices.is_empty());
        assert!(prices.iter().all(|p| *p > 0.0));
        assert!(prices.iter().any(|p| (*p - 93906.2).abs() < f64::EPSILON));
        assert!(prices.iter().any(|p| (*p - 93906.0).abs() < f64::EPSILON));
    }

    #[test]
    fn test_build_tick_retry_prices_sell() {
        let mut prices = build_tick_retry_prices(93906.234567, false, 1);
        dedup_prices_by_wire(&mut prices);
        assert!(!prices.is_empty());
        assert!(prices.iter().all(|p| *p > 0.0));
        assert!(prices.iter().any(|p| (*p - 93906.3).abs() < f64::EPSILON));
        assert!(prices.iter().any(|p| (*p - 93907.0).abs() < f64::EPSILON));
    }

    #[test]
    fn test_is_tick_size_error() {
        assert!(is_tick_size_error(
            "Price must be divisible by tick size. asset=0"
        ));
        assert!(is_tick_size_error("tickSize invalid"));
        assert!(!is_tick_size_error("order still resting"));
    }

    #[test]
    fn test_ensure_min_notional_size_raise() {
        let adjusted = ensure_min_notional_size("BTC", 0.00013, 76051.0, 5, 10.0).unwrap();
        assert!(adjusted >= 0.00014);
    }

    #[test]
    fn test_ensure_min_notional_size_keep() {
        let adjusted = ensure_min_notional_size("BTC", 0.00020, 76051.0, 5, 10.0).unwrap();
        assert!((adjusted - 0.00020).abs() < f64::EPSILON);
    }

    #[test]
    fn test_find_mid_in_map_builder_tail_match() {
        let mut mids = HashMap::new();
        mids.insert("XYZ:SP500".to_string(), "5234.5".to_string());
        let got = find_mid_in_map(&mids, "xyz:SP500");
        assert!(got.is_some());
        let (_, v) = got.unwrap();
        assert_eq!(v, "5234.5");
    }

    #[test]
    fn test_build_l2_coin_candidates_for_builder() {
        let cands = build_l2_coin_candidates("xyz:sp500");
        assert!(cands.contains(&"xyz:sp500".to_string()));
        assert!(cands.contains(&"xyz:SP500".to_string()));
        assert!(cands.contains(&"SP500".to_string()));
    }
}
