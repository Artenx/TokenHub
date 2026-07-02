use actix_web::{HttpResponse, ResponseError};
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Unauthorized,
    BadRequest(String),
    Internal(String),
    /// 可重试的代理错误（网络问题、5xx、429）
    Proxy(String),
    /// 不可重试的上游错误（4xx 除 429 外），不会触发重试
    UpstreamError(String),
}

impl AppError {
    /// 判断错误是否应该触发重试
    pub fn is_retryable(&self) -> bool {
        matches!(self, AppError::Proxy(_))
    }

    /// 获取错误对应的HTTP状态码
    pub fn status_code(&self) -> u16 {
        match self {
            AppError::Unauthorized => 401,
            AppError::NotFound(_) => 404,
            AppError::BadRequest(_) => 400,
            AppError::UpstreamError(_) => 502,
            AppError::Proxy(_) => 502,
            AppError::Internal(_) => 500,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NotFound(msg) => write!(f, "未找到: {}", msg),
            AppError::Unauthorized => write!(f, "未授权"),
            AppError::BadRequest(msg) => write!(f, "请求错误: {}", msg),
            AppError::Internal(msg) => write!(f, "内部错误: {}", msg),
            AppError::Proxy(msg) => write!(f, "代理错误: {}", msg),
            AppError::UpstreamError(msg) => write!(f, "上游错误: {}", msg),
        }
    }
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        let body = serde_json::json!({
            "error": {
                "message": self.to_string(),
                "type": match self {
                    AppError::Unauthorized => "authentication_error",
                    AppError::BadRequest(_) => "invalid_request_error",
                    AppError::NotFound(_) => "not_found_error",
                    AppError::UpstreamError(_) => "upstream_error",
                    _ => "server_error",
                }
            }
        });
        match self {
            AppError::Unauthorized => HttpResponse::Unauthorized().json(body),
            AppError::NotFound(_) => HttpResponse::NotFound().json(body),
            AppError::BadRequest(_) => HttpResponse::BadRequest().json(body),
            AppError::UpstreamError(_) => HttpResponse::BadGateway().json(body),
            _ => HttpResponse::InternalServerError().json(body),
        }
    }
}
