//! Binance U 本位只读账户事实接口；分页完整性由采集服务核验。

use super::BinanceOrderClient;
use crate::error::{ExchangeClientError, Result};
use serde::{Deserialize, Serialize};

/// 金额使用交易所原始十进制字符串，不将不同资产隐式折算为 USDT。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountTrade {
    pub symbol: String,
    pub id: i64,
    pub order_id: i64,
    pub side: String,
    pub position_side: String,
    pub price: String,
    pub qty: String,
    pub quote_qty: String,
    pub commission: String,
    pub commission_asset: String,
    pub realized_pnl: String,
    pub time: i64,
}

/// 同一账户内按收入类型及流水 ID 去重；成交 ID 在非成交收入中可为空。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountIncome {
    pub symbol: String,
    pub income_type: String,
    pub income: String,
    pub asset: String,
    pub time: i64,
    pub tran_id: i64,
    pub trade_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountAsset {
    pub asset: String,
    pub wallet_balance: String,
    pub unrealized_profit: String,
    pub margin_balance: String,
    pub update_time: i64,
}

/// 当前账户快照仅记录采集时刻，不充当历史请求窗口的期初或期末权益。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub assets: Vec<AccountAsset>,
    pub total_wallet_balance: String,
    pub total_unrealized_profit: String,
    pub total_margin_balance: String,
}

fn window(start_ms: i64, end_ms: i64) -> Result<Vec<(String, String)>> {
    if start_ms < 0 || end_ms <= start_ms || end_ms - start_ms > 7 * 86_400_000 {
        return Err(ExchangeClientError::Validation(
            "账户事实窗口必须为正且不超过七日".into(),
        ));
    }
    // 服务统一使用左闭右开窗口，交易所 endTime 为包含边界。
    Ok(vec![
        ("startTime".into(), start_ms.to_string()),
        ("endTime".into(), (end_ms - 1).to_string()),
        ("limit".into(), "1000".into()),
    ])
}

impl BinanceOrderClient {
    /// 不限定收入类型，包含资金费、佣金、已实现盈亏、转账及其他账户流入流出。
    pub async fn account_income_page(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<AccountIncome>> {
        self.read_usdt_account("/fapi/v1/income", &window(start_ms, end_ms)?)
            .await
    }

    /// 不混用 fromId 与时间范围，满页由上层拆分时间窗后重新读取。
    pub async fn account_trade_page(
        &self,
        symbol: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<AccountTrade>> {
        if symbol.is_empty()
            || symbol.len() > 40
            || !symbol
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(ExchangeClientError::Validation("成交查询品种无效".into()));
        }
        let mut params = window(start_ms, end_ms)?;
        params.push(("symbol".into(), symbol.into()));
        self.read_usdt_account("/fapi/v1/userTrades", &params).await
    }

    pub async fn account_snapshot(&self) -> Result<AccountSnapshot> {
        self.read_usdt_account("/fapi/v3/account", &[]).await
    }
}
