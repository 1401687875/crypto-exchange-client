# 更新记录

## 0.3.0（2026-11-17）

合并 quantitative-service-rust mading 分支里对客户端的改动，旧接口签名保持兼容（下面注明的两处除外）。

- 新增 `binance::stocks`：币安股票交易 SAPI 客户端和行情 WebSocket
- 新增 `binance::user_data`：U 本位私有订单流（`BinanceUserDataClient`），成交回报放进进程内事件总线，
  `wait_for_usdt_trade_result` 按订单 ID / 客户端 ID 取成交结果
- 旧版 U 本位市价单 `create_usdt_market_order` 增加成交核对：结果不明时按客户端 ID 查回、不重发；
  缺成交均价时等私有流、轮询订单、汇总成交明细；-1021 时同步服务器时间后重签
- `BinanceOrderClient`：连接 3 秒 / 请求 8 秒超时、`recvWindow` 5000；新增 `query_usdt_order_by_id`、
  `query_usdt_order_by_client_order_id`（返回完整订单响应）；HTTP 错误带币安错误码时返回 `ExchangeClientError::Api`
- `types`：币安订单响应的非关键字段改为可缺省，`SpotOrderResponse` 增加 `fills`
- OKX：新增 `get_pending_orders`（未完成委托）
- **不兼容**：`create_spot_market_order` 一直是 4 个参数（含客户端 ID），mading 分支里的 3 参数版本不再保留；
  币安旧接口里客户端 ID 统一按币安规则过滤（保留字母数字和 `. : / _ -`，最长 36）


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
