use crate::error::{ExchangeClientError, Result};
use hyperliquid_rust_sdk::{BaseUrl, InfoClient};
use serde::Deserialize;
use serde_json::json;

const HYPERLIQUID_MAINNET_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const HYPERLIQUID_TESTNET_INFO_URL: &str = "https://api.hyperliquid-testnet.xyz/info";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HyperliquidPrecisionInfo {
    pub symbol: String,
    pub inst_id: String,
    pub inst_type: String,
    pub price_precision: u32,
    pub quantity_precision: u32,
    pub tick_sz: Option<String>,
    pub step_size: Option<String>,
    pub min_qty: Option<String>,
    pub contract_type: String,
}

pub fn is_spot_contract(contract_type: Option<&str>) -> bool {
    matches!(
        contract_type
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "usdt".to_string())
            .as_str(),
        "spot" | "s"
    )
}

pub fn normalize_perp_symbol(symbol: &str) -> String {
    let trimmed = symbol.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // HIP-3 builder perp: 规范为 `dex:ASSET`（dex 保持小写，asset 大写）
    if let Some((dex, rest)) = trimmed.split_once(':') {
        let dex_name = dex.trim().to_lowercase();
        let asset = normalize_perp_symbol_without_dex(rest);
        if !dex_name.is_empty() && !asset.is_empty() {
            return format!("{}:{}", dex_name, asset);
        }
    }

    normalize_perp_symbol_without_dex(trimmed)
}

fn normalize_perp_symbol_without_dex(symbol: &str) -> String {
    let upper = symbol.trim().to_uppercase();
    if upper.contains('/') {
        return upper.split('/').next().unwrap_or(&upper).to_string();
    }
    if upper.ends_with("-USDT-SWAP") {
        return upper.trim_end_matches("-USDT-SWAP").to_string();
    }
    if upper.ends_with("-USD-SWAP") {
        return upper.trim_end_matches("-USD-SWAP").to_string();
    }
    if upper.ends_with("USD_PERP") {
        return upper.trim_end_matches("USD_PERP").to_string();
    }
    if upper.ends_with("_PERP") {
        return upper.trim_end_matches("_PERP").to_string();
    }
    // 兼容用户输入带稳定币后缀标识:
    // - XYZ100-USDC
    // - XYZ100_USDH
    // - XYZ100-USDT0
    if let Some((base, quote)) = upper.rsplit_once('-') {
        if !base.is_empty() && quote.starts_with("USD") {
            return base.to_string();
        }
    }
    if let Some((base, quote)) = upper.rsplit_once('_') {
        if !base.is_empty() && quote.starts_with("USD") {
            return base.to_string();
        }
    }
    if upper.ends_with("USDT") {
        return upper.trim_end_matches("USDT").to_string();
    }
    if upper.ends_with("USDC") {
        return upper.trim_end_matches("USDC").to_string();
    }
    if upper.ends_with("USDH") {
        return upper.trim_end_matches("USDH").to_string();
    }
    if upper.ends_with("USDE") {
        return upper.trim_end_matches("USDE").to_string();
    }
    if upper.ends_with("USDT0") {
        return upper.trim_end_matches("USDT0").to_string();
    }
    if upper.ends_with("USD") && !upper.ends_with("USDT") {
        return upper.trim_end_matches("USD").to_string();
    }
    upper
}

pub fn normalize_spot_symbol(symbol: &str) -> String {
    let upper = symbol.trim().to_uppercase();
    if upper.contains('/') {
        return upper;
    }
    if let Some((base, quote)) = upper.rsplit_once('-') {
        if !base.is_empty() && quote.starts_with("USD") {
            return format!("{}/{}", base, quote);
        }
    }
    if let Some((base, quote)) = upper.rsplit_once('_') {
        if !base.is_empty() && quote.starts_with("USD") {
            return format!("{}/{}", base, quote);
        }
    }
    format!("{}/USDC", normalize_perp_symbol(symbol))
}

pub async fn fetch_precision(
    symbol: &str,
    contract_type: Option<&str>,
    use_testnet: bool,
) -> Result<HyperliquidPrecisionInfo> {
    let base_url = if use_testnet {
        BaseUrl::Testnet
    } else {
        BaseUrl::Mainnet
    };

    let info_client = InfoClient::new(None, Some(base_url))
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;

    if is_spot_contract(contract_type) {
        fetch_spot_precision(&info_client, symbol)
            .await
            .map(|mut info| {
                info.contract_type = "spot".to_string();
                info
            })
    } else {
        fetch_perp_precision(&info_client, symbol)
            .await
            .map(|mut info| {
                info.contract_type = contract_type.unwrap_or("usdt").to_string();
                info
            })
    }
}

async fn fetch_perp_precision(
    info_client: &InfoClient,
    symbol: &str,
) -> Result<HyperliquidPrecisionInfo> {
    let normalized = normalize_perp_symbol(symbol);
    let meta = info_client
        .meta()
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;

    let found = meta
        .universe
        .iter()
        .enumerate()
        .find(|(_, asset)| asset.name.eq_ignore_ascii_case(&normalized));

    let (asset_index, asset_meta) = match found {
        Some((index, meta)) => (index, meta),
        None => {
            let builder_matches =
                find_symbol_in_builder_perp_dexes(&normalized, info_client).await?;
            if !builder_matches.is_empty() {
                let selected = &builder_matches[0];
                log::info!(
                    "[HYPERLIQUID] fallback to builder perp meta | input={} | normalized={} | matched_asset={} | dex={} | perp_dex_index={} | local_index={}",
                    symbol,
                    normalized,
                    selected.asset_name,
                    selected.dex_name,
                    selected.perp_dex_index,
                    selected.index_in_meta
                );
                return Ok(build_builder_perp_precision(&normalized, selected));
            }
            return Err(ExchangeClientError::Validation(format!(
                "hyperliquid perp symbol not found: {}",
                normalized
            )));
        }
    };

    let quantity_precision = asset_meta.sz_decimals;
    let max_decimals: u32 = if asset_index < 10000 { 6 } else { 8 };
    let price_precision = max_decimals.saturating_sub(quantity_precision);
    let step_size = decimal_step(quantity_precision);
    let tick_size = decimal_step(price_precision);

    Ok(HyperliquidPrecisionInfo {
        symbol: normalized.clone(),
        inst_id: normalized,
        inst_type: "SWAP".to_string(),
        price_precision,
        quantity_precision,
        tick_sz: Some(tick_size),
        step_size: Some(step_size.clone()),
        min_qty: Some(step_size),
        contract_type: "usdt".to_string(),
    })
}

fn build_builder_perp_precision(
    normalized_symbol: &str,
    matched: &BuilderPerpMatch,
) -> HyperliquidPrecisionInfo {
    // Perp 合约价格精度上限按 6 位处理（与默认 perp 市场一致）
    let quantity_precision = matched.sz_decimals;
    let price_precision = 6_u32.saturating_sub(quantity_precision);
    let step_size = decimal_step(quantity_precision);
    let tick_size = decimal_step(price_precision);

    HyperliquidPrecisionInfo {
        symbol: normalized_symbol.to_string(),
        inst_id: matched.asset_name.clone(),
        inst_type: "SWAP".to_string(),
        price_precision,
        quantity_precision,
        tick_sz: Some(tick_size),
        step_size: Some(step_size.clone()),
        min_qty: Some(step_size),
        contract_type: "usdt".to_string(),
    }
}

#[derive(Debug, Clone)]
struct BuilderPerpMatch {
    dex_name: String,
    perp_dex_index: usize,
    index_in_meta: usize,
    asset_name: String,
    sz_decimals: u32,
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
    sz_decimals: u32,
}

fn info_url(use_testnet: bool) -> &'static str {
    if use_testnet {
        HYPERLIQUID_TESTNET_INFO_URL
    } else {
        HYPERLIQUID_MAINNET_INFO_URL
    }
}

async fn find_symbol_in_builder_perp_dexes(
    normalized_symbol: &str,
    info_client: &InfoClient,
) -> Result<Vec<BuilderPerpMatch>> {
    let base_url = &info_client.http_client.base_url;
    let use_testnet = !base_url.contains("api.hyperliquid.xyz");
    let url = info_url(use_testnet);
    let client = reqwest::Client::new();

    let perp_dex_resp = client
        .post(url)
        .json(&json!({ "type": "perpDexs" }))
        .send()
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;

    if !perp_dex_resp.status().is_success() {
        return Ok(Vec::new());
    }

    let perp_dexes: Vec<Option<PerpDexInfo>> = perp_dex_resp
        .json()
        .await
        .map_err(|e| ExchangeClientError::ResponseParse(e.to_string()))?;

    let mut matches: Vec<BuilderPerpMatch> = Vec::new();

    for (perp_dex_index, dex_opt) in perp_dexes.into_iter().enumerate() {
        let Some(dex) = dex_opt else {
            continue;
        };
        let dex_name = dex.name;
        let meta_resp = client
            .post(url)
            .json(&json!({ "type": "meta", "dex": dex_name.clone() }))
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

        for (index_in_meta, asset) in meta.universe.into_iter().enumerate() {
            let is_match = asset.name.eq_ignore_ascii_case(normalized_symbol)
                || asset
                    .name
                    .rsplit(':')
                    .next()
                    .map(|tail| tail.eq_ignore_ascii_case(normalized_symbol))
                    .unwrap_or(false);
            if is_match {
                matches.push(BuilderPerpMatch {
                    dex_name: dex_name.clone(),
                    perp_dex_index,
                    index_in_meta,
                    asset_name: asset.name,
                    sz_decimals: asset.sz_decimals,
                });
            }
        }
    }

    matches.sort_by(|a, b| a.asset_name.cmp(&b.asset_name));
    matches.dedup_by(|a, b| {
        a.asset_name == b.asset_name
            && a.dex_name == b.dex_name
            && a.index_in_meta == b.index_in_meta
    });
    Ok(matches)
}

async fn fetch_spot_precision(
    info_client: &InfoClient,
    symbol: &str,
) -> Result<HyperliquidPrecisionInfo> {
    let normalized = normalize_spot_symbol(symbol);
    let spot_meta = info_client
        .spot_meta()
        .await
        .map_err(|e| ExchangeClientError::Connection(e.to_string()))?;

    let mut found_pair: Option<(String, u32, usize)> = None;
    for asset in &spot_meta.universe {
        let base_token = spot_meta
            .tokens
            .iter()
            .find(|token| token.index == asset.tokens[0]);
        let quote_token = spot_meta
            .tokens
            .iter()
            .find(|token| token.index == asset.tokens[1]);

        if let (Some(base), Some(quote)) = (base_token, quote_token) {
            let pair = format!("{}/{}", base.name.to_uppercase(), quote.name.to_uppercase());
            if pair.eq_ignore_ascii_case(&normalized)
                || asset.name.eq_ignore_ascii_case(&normalized)
            {
                found_pair = Some((pair, u32::from(base.sz_decimals), 10000 + asset.index));
                break;
            }
        }
    }

    let (pair, quantity_precision, asset_index) = found_pair.ok_or_else(|| {
        ExchangeClientError::Validation(format!(
            "hyperliquid spot symbol not found: {}",
            normalized
        ))
    })?;

    let max_decimals: u32 = if asset_index < 10000 { 6 } else { 8 };
    let price_precision = max_decimals.saturating_sub(quantity_precision);
    let step_size = decimal_step(quantity_precision);
    let tick_size = decimal_step(price_precision);

    Ok(HyperliquidPrecisionInfo {
        symbol: pair.clone(),
        inst_id: pair,
        inst_type: "SPOT".to_string(),
        price_precision,
        quantity_precision,
        tick_sz: Some(tick_size),
        step_size: Some(step_size.clone()),
        min_qty: Some(step_size),
        contract_type: "spot".to_string(),
    })
}
fn decimal_step(decimals: u32) -> String {
    if decimals == 0 {
        return "1".to_string();
    }
    let mut out = String::from("0.");
    for _ in 1..decimals {
        out.push('0');
    }
    out.push('1');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_perp_symbol() {
        assert_eq!(normalize_perp_symbol("BTC"), "BTC");
        assert_eq!(normalize_perp_symbol("btcusdt"), "BTC");
        assert_eq!(normalize_perp_symbol("BTC-USDT-SWAP"), "BTC");
        assert_eq!(normalize_perp_symbol("ETH/USD"), "ETH");
        assert_eq!(normalize_perp_symbol("XYZ100-USDC"), "XYZ100");
        assert_eq!(normalize_perp_symbol("XYZ100_USDH"), "XYZ100");
        assert_eq!(normalize_perp_symbol("XYZ100-USDT0"), "XYZ100");
        assert_eq!(normalize_perp_symbol("xyz:sp500"), "xyz:SP500");
        assert_eq!(normalize_perp_symbol("XYZ:SP500-USDC"), "xyz:SP500");
    }

    #[test]
    fn test_normalize_spot_symbol() {
        assert_eq!(normalize_spot_symbol("BTC"), "BTC/USDC");
        assert_eq!(normalize_spot_symbol("eth/usdc"), "ETH/USDC");
        assert_eq!(normalize_spot_symbol("xyz100-usdh"), "XYZ100/USDH");
        assert_eq!(normalize_spot_symbol("xyz100_usdt0"), "XYZ100/USDT0");
    }

    #[test]
    fn test_decimal_step() {
        assert_eq!(decimal_step(0), "1");
        assert_eq!(decimal_step(1), "0.1");
        assert_eq!(decimal_step(4), "0.0001");
    }

    #[tokio::test]
    #[ignore = "requires live Hyperliquid network"]
    async fn test_fetch_precision_perp_live_with_quote_suffix() {
        // 使用你当前前端传入风格（带 -USDC 后缀）验证真实精度拉取。
        let info = fetch_precision("BTC-USDC", Some("usdt"), false)
            .await
            .expect("fetch hyperliquid perp precision failed");
        println!("获取Hyperliquid币种信息");
        println!("{:?}", info);
    }
}
