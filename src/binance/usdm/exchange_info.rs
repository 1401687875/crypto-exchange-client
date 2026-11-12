//! 交易规则（`/fapi/v1/exchangeInfo`）和下单前的精度处理。

use super::models::Side;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeInfo {
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub server_time: i64,
    #[serde(default)]
    pub rate_limits: Vec<RateLimit>,
    pub symbols: Vec<SymbolInfo>,
}

impl ExchangeInfo {
    pub fn symbol(&self, symbol: &str) -> Option<&SymbolInfo> {
        self.symbols.iter().find(|s| s.symbol == symbol)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    pub rate_limit_type: String,
    pub interval: String,
    pub interval_num: u32,
    pub limit: u32,
}

/// 单个合约的交易规则。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInfo {
    pub symbol: String,
    #[serde(default)]
    pub pair: String,
    /// `PERPETUAL`、`CURRENT_QUARTER`、`TRADIFI_PERPETUAL` 等。
    #[serde(default)]
    pub contract_type: String,
    /// `TRADING` 才能交易。
    pub status: String,
    #[serde(default)]
    pub base_asset: String,
    #[serde(default)]
    pub quote_asset: String,
    #[serde(default)]
    pub margin_asset: String,
    #[serde(default)]
    pub price_precision: u32,
    #[serde(default)]
    pub quantity_precision: u32,
    /// `COIN`（加密货币）或其他（传统资产等）。
    #[serde(default)]
    pub underlying_type: String,
    #[serde(default)]
    pub underlying_sub_type: Vec<String>,
    #[serde(default)]
    pub onboard_date: i64,
    #[serde(default)]
    pub delivery_date: i64,
    #[serde(default)]
    pub filters: Vec<SymbolFilter>,
    #[serde(default)]
    pub order_types: Vec<String>,
    #[serde(default)]
    pub time_in_force: Vec<String>,
}

/// 交易规则里的过滤器。本库不认识的过滤器归到 `Other`。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "filterType", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SymbolFilter {
    #[serde(rename_all = "camelCase")]
    PriceFilter {
        min_price: Decimal,
        max_price: Decimal,
        tick_size: Decimal,
    },
    #[serde(rename_all = "camelCase")]
    LotSize {
        min_qty: Decimal,
        max_qty: Decimal,
        step_size: Decimal,
    },
    #[serde(rename_all = "camelCase")]
    MarketLotSize {
        min_qty: Decimal,
        max_qty: Decimal,
        step_size: Decimal,
    },
    MaxNumOrders {
        limit: u32,
    },
    MaxNumAlgoOrders {
        limit: u32,
    },
    MinNotional {
        notional: Decimal,
    },
    #[serde(rename_all = "camelCase")]
    PercentPrice {
        multiplier_up: Decimal,
        multiplier_down: Decimal,
        #[serde(default)]
        multiplier_decimal: Option<serde_json::Value>,
    },
    #[serde(other)]
    Other,
}

/// 价格取整方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    Down,
    Up,
    Nearest,
}

impl SymbolInfo {
    pub fn is_trading(&self) -> bool {
        self.status == "TRADING"
    }

    /// 价格步长。
    pub fn tick_size(&self) -> Option<Decimal> {
        self.filters.iter().find_map(|f| match f {
            SymbolFilter::PriceFilter { tick_size, .. } => Some(*tick_size),
            _ => None,
        })
    }

    /// 限价单数量步长。
    pub fn step_size(&self) -> Option<Decimal> {
        self.lot_size().map(|(_, _, step)| step)
    }

    /// 限价单 (最小数量, 最大数量, 步长)。
    pub fn lot_size(&self) -> Option<(Decimal, Decimal, Decimal)> {
        self.filters.iter().find_map(|f| match f {
            SymbolFilter::LotSize {
                min_qty,
                max_qty,
                step_size,
            } => Some((*min_qty, *max_qty, *step_size)),
            _ => None,
        })
    }

    /// 市价单 (最小数量, 最大数量, 步长)。
    pub fn market_lot_size(&self) -> Option<(Decimal, Decimal, Decimal)> {
        self.filters.iter().find_map(|f| match f {
            SymbolFilter::MarketLotSize {
                min_qty,
                max_qty,
                step_size,
            } => Some((*min_qty, *max_qty, *step_size)),
            _ => None,
        })
    }

    /// 最小名义金额（USDT）。
    pub fn min_notional(&self) -> Option<Decimal> {
        self.filters.iter().find_map(|f| match f {
            SymbolFilter::MinNotional { notional } => Some(*notional),
            _ => None,
        })
    }

    /// 按价格步长取整。
    pub fn round_price(&self, price: Decimal, rounding: Rounding) -> Decimal {
        match self.tick_size() {
            Some(tick) => round_to_step(price, tick, rounding),
            None => price,
        }
    }

    /// 限价单数量按步长向下取整（不会超过想下的数量）。
    pub fn floor_qty(&self, qty: Decimal) -> Decimal {
        match self.step_size() {
            Some(step) => round_to_step(qty, step, Rounding::Down),
            None => qty,
        }
    }

    /// 市价单数量按步长向下取整。
    pub fn floor_market_qty(&self, qty: Decimal) -> Decimal {
        match self.market_lot_size().or_else(|| self.lot_size()) {
            Some((_, _, step)) => round_to_step(qty, step, Rounding::Down),
            None => qty,
        }
    }

    /// 检查数量和名义金额是否满足下单规则；不满足时返回原因。`price` 用下单价或当前价。
    pub fn check_order(&self, qty: Decimal, price: Decimal, is_market: bool) -> Result<(), String> {
        let lot = if is_market {
            self.market_lot_size().or_else(|| self.lot_size())
        } else {
            self.lot_size()
        };
        if let Some((min, max, step)) = lot {
            if qty < min {
                return Err(format!("数量 {qty} 小于最小数量 {min}"));
            }
            if qty > max {
                return Err(format!("数量 {qty} 大于最大数量 {max}"));
            }
            if step > Decimal::ZERO && (qty % step) != Decimal::ZERO {
                return Err(format!("数量 {qty} 不是步长 {step} 的整数倍"));
            }
        }
        if let Some(min_notional) = self.min_notional() {
            let notional = qty * price;
            if notional < min_notional {
                return Err(format!(
                    "名义金额 {notional} 小于最小名义金额 {min_notional}"
                ));
            }
        }
        if !is_market {
            if let Some(tick) = self.tick_size() {
                if tick > Decimal::ZERO && (price % tick) != Decimal::ZERO {
                    return Err(format!("价格 {price} 不是步长 {tick} 的整数倍"));
                }
            }
        }
        Ok(())
    }

    /// 只做 Maker 单的挂单价：买单挂买一（或往里挪 `improve_ticks` 档），卖单挂卖一；
    /// 往里挪时保证不会碰到对手价（否则 GTX 会被拒）。
    pub fn post_only_price(
        &self,
        side: &Side,
        best_bid: Decimal,
        best_ask: Decimal,
        improve_ticks: u32,
    ) -> Decimal {
        let tick = self.tick_size().unwrap_or(Decimal::ZERO);
        let shift = tick * Decimal::from(improve_ticks);
        match side {
            Side::Sell => {
                let price = self.round_price(best_ask - shift, Rounding::Up);
                if tick > Decimal::ZERO && price <= best_bid {
                    best_bid + tick
                } else {
                    price
                }
            }
            _ => {
                let price = self.round_price(best_bid + shift, Rounding::Down);
                if tick > Decimal::ZERO && price >= best_ask {
                    best_ask - tick
                } else {
                    price
                }
            }
        }
    }
}

/// 按步长取整。步长为 0 时原样返回。
pub fn round_to_step(value: Decimal, step: Decimal, rounding: Rounding) -> Decimal {
    if step <= Decimal::ZERO {
        return value;
    }
    let units = value / step;
    let units = match rounding {
        Rounding::Down => units.floor(),
        Rounding::Up => units.ceil(),
        Rounding::Nearest => {
            units.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        }
    };
    (units * step).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn btc() -> SymbolInfo {
        let raw = r#"{"symbol":"BTCUSDT","pair":"BTCUSDT","contractType":"PERPETUAL","deliveryDate":4133404800000,"onboardDate":1569398400000,
            "status":"TRADING","maintMarginPercent":"2.5000","requiredMarginPercent":"5.0000","baseAsset":"BTC","quoteAsset":"USDT","marginAsset":"USDT",
            "pricePrecision":2,"quantityPrecision":3,"baseAssetPrecision":8,"quotePrecision":8,"underlyingType":"COIN","underlyingSubType":["PoW"],
            "settlePlan":0,"triggerProtect":"0.0500","liquidationFee":"0.012500","marketTakeBound":"0.05","maxMoveOrderLimit":10000,
            "filters":[{"minPrice":"556.80","maxPrice":"4529764","filterType":"PRICE_FILTER","tickSize":"0.10"},
                       {"stepSize":"0.001","filterType":"LOT_SIZE","maxQty":"1000","minQty":"0.001"},
                       {"stepSize":"0.001","filterType":"MARKET_LOT_SIZE","maxQty":"120","minQty":"0.001"},
                       {"limit":200,"filterType":"MAX_NUM_ORDERS"},{"limit":10,"filterType":"MAX_NUM_ALGO_ORDERS"},
                       {"notional":"100","filterType":"MIN_NOTIONAL"},
                       {"multiplierDown":"0.9500","multiplierUp":"1.0500","multiplierDecimal":"4","filterType":"PERCENT_PRICE"},
                       {"filterType":"POSITION_RISK_CONTROL","positionControlSide":"NONE"}],
            "orderTypes":["LIMIT","MARKET","STOP","STOP_MARKET","TAKE_PROFIT","TAKE_PROFIT_MARKET","TRAILING_STOP_MARKET"],
            "timeInForce":["GTC","IOC","FOK","GTX","GTD"],"permissionSets":["GRID","COPY"]}"#;
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn parses_filters() {
        let info = btc();
        assert!(info.is_trading());
        assert_eq!(info.tick_size(), Some(d("0.1")));
        assert_eq!(info.step_size(), Some(d("0.001")));
        assert_eq!(info.market_lot_size().unwrap().1, d("120"));
        assert_eq!(info.min_notional(), Some(d("100")));
        assert_eq!(info.filters.last(), Some(&SymbolFilter::Other));
    }

    #[test]
    fn rounding() {
        let info = btc();
        assert_eq!(
            info.round_price(d("65000.17"), Rounding::Down),
            d("65000.1")
        );
        assert_eq!(info.round_price(d("65000.11"), Rounding::Up), d("65000.2"));
        assert_eq!(
            info.round_price(d("65000.15"), Rounding::Nearest),
            d("65000.2")
        );
        assert_eq!(info.floor_qty(d("0.0129")), d("0.012"));
        assert_eq!(
            round_to_step(d("1.234"), d("0"), Rounding::Down),
            d("1.234")
        );
    }

    #[test]
    fn order_checks() {
        let info = btc();
        assert!(info.check_order(d("0.002"), d("65000"), false).is_ok());
        assert!(info
            .check_order(d("0.001"), d("65000"), false)
            .unwrap_err()
            .contains("名义金额"));
        assert!(info
            .check_order(d("0.0015"), d("65000"), false)
            .unwrap_err()
            .contains("步长"));
        assert!(info
            .check_order(d("0.002"), d("65000.05"), false)
            .unwrap_err()
            .contains("价格"));
        assert!(info
            .check_order(d("200"), d("65000"), true)
            .unwrap_err()
            .contains("最大数量"));
    }

    #[test]
    fn post_only_price_never_crosses() {
        let info = btc();
        assert_eq!(
            info.post_only_price(&Side::Buy, d("65000.0"), d("65000.5"), 0),
            d("65000.0")
        );
        assert_eq!(
            info.post_only_price(&Side::Buy, d("65000.0"), d("65000.5"), 2),
            d("65000.2")
        );
        // 价差只有 1 档时往里挪也不能碰到卖一。
        assert_eq!(
            info.post_only_price(&Side::Buy, d("65000.0"), d("65000.1"), 3),
            d("65000.0")
        );
        assert_eq!(
            info.post_only_price(&Side::Sell, d("65000.0"), d("65000.5"), 1),
            d("65000.4")
        );
        assert_eq!(
            info.post_only_price(&Side::Sell, d("65000.0"), d("65000.1"), 5),
            d("65000.1")
        );
    }
}
