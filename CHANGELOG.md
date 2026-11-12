# 更新记录

## 0.2.0（2026-11-12）

- 新增 `binance::usdm`：币安 U 本位合约完整客户端
  - 下单：市价、限价、只做 Maker（GTX）、按盘口定价（priceMatch）、测试下单、批量下单
  - 改单、撤单（单个 / 批量 / 全部 / 倒计时撤单）、查单（单个 / 挂单 / 历史 / 成交明细）
  - 条件单（algoOrder）：下单、撤单、查询
  - 账户：账户信息、余额、持仓、手续费率、杠杆、保证金模式、持仓模式、资金流水
  - 行情：交易规则、深度、盘口、最新价、24 小时行情、K 线、标记价格、资金费、持仓量、归集成交
  - 行情推送和用户数据流：新版 `/public`、`/market`、`/private` 地址，自动重连，listenKey 自动续期和重建
  - 精度工具：按价格/数量步长取整、下单规则检查、只做 Maker 挂单价
  - 时间同步、限频计数、交易所错误码解析、"执行结果未知"判断
- 按交易所拆分 feature：`binance`、`okx`、`hyperliquid`（默认全部启用）
- 错误类型新增 `ExchangeClientError::Api { status, code, msg }`

## 0.1.0

- 从 quantitative-service-rust 拆出：币安 / OKX / Hyperliquid 的下单与 WebSocket 客户端
