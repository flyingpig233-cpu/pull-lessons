use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// 与上游（学校系统）或网络交互时产生的错误。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("会话已失效，请重新登录")]
    NotAuthenticated,
    #[error("网络请求失败: {0}")]
    Network(#[from] reqwest::Error),
    #[error("{0}")]
    Upstream(String),
    #[error("页面解析失败: {0}")]
    Parse(String),
}

/// 对外 HTTP 接口错误。返回体统一为 { code, msg, data }。
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: i64,
    pub message: String,
}

impl AppError {
    pub fn new(status: StatusCode, code: i64, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, -2, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, 401, "会话已失效，请重新登录")
    }

    /// 业务失败：HTTP 200，但 code < 0，前端直接展示 msg。
    pub fn business(message: impl Into<String>) -> Self {
        Self::new(StatusCode::OK, -1, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, 500, message)
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, 502, message)
    }
}

impl From<ClientError> for AppError {
    fn from(err: ClientError) -> Self {
        match err {
            ClientError::NotAuthenticated => Self::unauthorized(),
            ClientError::Network(e) => Self::upstream(format!("访问学校服务器失败: {e}")),
            ClientError::Upstream(msg) => Self::business(msg),
            ClientError::Parse(msg) => Self::internal(msg),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = json!({
            "code": self.code,
            "msg": self.message,
            "data": null,
        });
        (self.status, Json(body)).into_response()
    }
}

/// 统一成功响应。
pub fn ok<T: serde::Serialize>(data: T) -> Json<serde_json::Value> {
    Json(json!({
        "code": 0,
        "msg": "ok",
        "data": data,
    }))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
