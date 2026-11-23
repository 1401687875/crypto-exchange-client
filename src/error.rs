#[derive(Debug, thiserror::Error)]
pub enum ExchangeClientError {
    #[error("connection error: {0}")]
    Connection(String),
    #[error("signing error: {0}")]
    Signing(String),
    #[error("response parse error: {0}")]
    ResponseParse(String),
    #[error("exchange order error: {0}")]
    Order(String),
    #[error("validation error: {0}")]
    Validation(String),
    /// 交易所返回的业务错误（带交易所错误码）：币安是 HTTP 非 2xx；OKX 是响应里的 `code` / `sCode` 不为 0（HTTP 可能是 200）。
    #[error("exchange api error: http {status}, code {code}: {msg}")]
    Api { status: u16, code: i64, msg: String },
}

pub type Result<T> = std::result::Result<T, ExchangeClientError>;

impl ExchangeClientError {
    /// 下单、改单、撤单请求返回这个错误时，交易所是否已经执行**无法确定**：
    /// 必须按客户端订单 ID 查询订单后再决定下一步，不能直接重发。
    ///
    /// 包括：连接失败或超时（请求可能已到达交易所）、HTTP 5xx、
    /// 币安 `-1006`（意外响应）和 `-1007`（等待后端超时，状态未知）、
    /// OKX `50001`（服务暂时不可用）、`50004`（接口请求超时，不代表成功或失败）、`50013`（系统繁忙）、`50026`（系统错误）。
    pub fn is_unknown_execution(&self) -> bool {
        match self {
            Self::Connection(_) => true,
            Self::Api { status, code, .. } => {
                *status >= 500 || matches!(code, -1006 | -1007 | 50001 | 50004 | 50013 | 50026)
            }
            _ => false,
        }
    }

    /// 交易所错误码（只有 [`ExchangeClientError::Api`] 有）。
    pub fn api_code(&self) -> Option<i64> {
        match self {
            Self::Api { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// 是否触发了限频（HTTP 429 / 418，币安 `-1003`，OKX `50011` / `50061`）。
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::Api { status, code, .. } if *status == 429 || *status == 418 || matches!(code, -1003 | 50011 | 50061))
    }
}

impl From<reqwest::Error> for ExchangeClientError {
    fn from(value: reqwest::Error) -> Self {
        Self::Connection(value.without_url().to_string())
    }
}

impl From<serde_json::Error> for ExchangeClientError {
    fn from(value: serde_json::Error) -> Self {
        Self::ResponseParse(value.to_string())
    }
}
