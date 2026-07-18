use crate::error::AppError;
use crate::get_client_ip;
use crate::models::*;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tracing::{debug, error, warn};

/// 截断上游错误响应体，防止敏感信息通过错误消息泄露给客户端
fn sanitize_error_body(body: &str) -> String {
    if body.chars().count() > 200 {
        format!("{}...(已截断)", body.chars().take(200).collect::<String>())
    } else {
        body.to_string()
    }
}

/// 将字节切片转换为 UTF-8 字符串，并按配置的阈值截断
/// 返回 (字符串, 是否截断)
fn capture_body(body: &[u8], max_size_kb: usize) -> (String, bool) {
    let max_bytes = max_size_kb.saturating_mul(1024);
    if max_bytes == 0 {
        return (String::new(), !body.is_empty());
    }
    if body.len() <= max_bytes {
        (String::from_utf8_lossy(body).into_owned(), false)
    } else {
        let slice = &body[..max_bytes];
        (String::from_utf8_lossy(slice).into_owned(), true)
    }
}

/// 构造一条 ApiReplayRecord 并写入 AppState
#[allow(clippy::too_many_arguments)]
fn record_replay(
    state: &AppState,
    api_id: &str,
    method: &str,
    path: &str,
    status_code: u16,
    status: &str,
    error_message: Option<String>,
    duration_ms: u64,
    request_body: &[u8],
    response_body: &[u8],
) {
    let max_kb = state.replay_config.read().max_body_size_kb;
    let (req_body, req_trunc) = capture_body(request_body, max_kb);
    let (resp_body, resp_trunc) = capture_body(response_body, max_kb);
    state.add_replay_record(ApiReplayRecord {
        id: uuid::Uuid::new_v4().to_string(),
        api_id: api_id.to_string(),
        timestamp: Utc::now(),
        method: method.to_string(),
        path: path.to_string(),
        status_code,
        status: status.to_string(),
        error_message,
        duration_ms,
        request_body: req_body,
        response_body: resp_body,
        request_truncated: req_trunc,
        response_truncated: resp_trunc,
    });
}

/// 流式响应的有界缓冲区，保留客户端实际接收的前 N 个字节。
struct ReplayBuffer {
    body: Vec<u8>,
    max_bytes: usize,
    truncated: bool,
}

impl ReplayBuffer {
    fn new(max_size_kb: usize) -> Self {
        Self {
            body: Vec::new(),
            max_bytes: max_size_kb.saturating_mul(1024),
            truncated: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        if self.truncated {
            return;
        }
        let available = self.max_bytes.saturating_sub(self.body.len());
        if chunk.len() > available {
            self.body.extend_from_slice(&chunk[..available]);
            self.truncated = true;
        } else {
            self.body.extend_from_slice(chunk);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_stream_replay(
    state: &AppState,
    api_id: &str,
    method: &str,
    path: &str,
    duration_ms: u64,
    request_body: &[u8],
    response: ReplayBuffer,
) {
    let max_kb = state.replay_config.read().max_body_size_kb;
    let (request_body, request_truncated) = capture_body(request_body, max_kb);
    state.add_replay_record(ApiReplayRecord {
        id: uuid::Uuid::new_v4().to_string(),
        api_id: api_id.to_string(),
        timestamp: Utc::now(),
        method: method.to_string(),
        path: path.to_string(),
        status_code: 200,
        status: "success".to_string(),
        error_message: None,
        duration_ms,
        request_body,
        response_body: String::from_utf8_lossy(&response.body).into_owned(),
        request_truncated,
        response_truncated: response.truncated,
    });
}

/// 已知的错误关键词（用于检测纯文本错误内容）
const ERROR_KEYWORDS: &[&str] = &[
    "请求负载过高",
    "请稍后再试",
    "rate limit",
    "too many requests",
    "quota exceeded",
    "insufficient_quota",
    "overloaded",
    "capacity exceeded",
    "额度不足",
    "额度已用",
    "额度已不足",
    "额度已用完",
    "额度已用尽",
    "token已用完",
    "token已不足",
    "token额度",
];

// ========== 错误检测 ==========

/// 检查内容文本是否包含已知错误关键词（仅对短内容检测，避免影响正常回复）
fn check_content_error(content: &str) -> Option<(String, String)> {
    if content.len() > 200 {
        return None;
    }
    let content_lower = content.to_lowercase();
    for keyword in ERROR_KEYWORDS {
        if content_lower.contains(&keyword.to_lowercase()) {
            return Some(("CONTENT_ERROR".to_string(), content.to_string()));
        }
    }
    None
}

/// 检查单个 JSON 对象是否为错误响应（兼容多种接口类型）
fn check_json_error(json: &Value) -> Option<(String, String)> {
    // Anthropic 错误: {"type": "error", "error": {"type": "...", "message": "..."}}
    if json.get("type").and_then(|v| v.as_str()) == Some("error") {
        if let Some(error_obj) = json.get("error") {
            let msg = error_obj.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误").to_string();
            let code = error_obj.get("type").and_then(|c| c.as_str()).unwrap_or("error").to_string();
            return Some((code, msg));
        }
    }

    // OpenAI 格式: {"error": {"code": "...", "message": "..."}}
    if let Some(error_obj) = json.get("error") {
        let msg = error_obj.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误").to_string();
        let code = error_obj.get("code").map(|c| c.to_string()).unwrap_or_default();
        return Some((code, msg));
    }

    // 顶层 code+message 格式: {"code": 429, "message": "..."}
    if let (Some(code), Some(msg)) = (json.get("code"), json.get("message")) {
        if code.is_number() || code.is_string() {
            return Some((code.to_string(), msg.as_str().unwrap_or("未知错误").to_string()));
        }
    }

    // NVIDIA 格式: {"status": 429, "title": "Too Many Requests"}
    if let (Some(status), Some(title)) = (json.get("status"), json.get("title")) {
        if status.is_number() {
            return Some((status.to_string(), title.as_str().unwrap_or("未知错误").to_string()));
        }
    }

    // 正常响应，但检查 choices content 中是否嵌入了错误 JSON
    if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
        for choice in choices {
            let content = choice
                .get("delta").and_then(|d| d.get("content")).and_then(|c| c.as_str())
                .or_else(|| choice.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_str()));
            if let Some(content) = content {
                if let Some(json_start) = content.find('{') {
                    let json_part = &content[json_start..];
                    if let Ok(err_json) = serde_json::from_str::<Value>(json_part) {
                        if err_json.get("error").is_some() {
                            let msg = err_json["error"].get("message")
                                .and_then(|m| m.as_str()).unwrap_or("未知错误").to_string();
                            let code = err_json["error"].get("code")
                                .map(|c| c.to_string()).unwrap_or_default();
                            return Some((code, msg));
                        }
                    }
                }
                if let Some(err) = check_content_error(content) {
                    return Some(err);
                }
            }
        }
    }

    None
}

/// 检查响应体中是否包含错误（支持普通 JSON 和 SSE 格式）
fn detect_response_error(body: &[u8]) -> Option<(String, String)> {
    let body_str = std::str::from_utf8(body).ok()?;

    if let Ok(json) = serde_json::from_str::<Value>(body_str) {
        return check_json_error(&json);
    }

    // SSE 格式：逐行检查
    if body_str.contains("data: ") || body_str.contains("event:") {
        let mut is_error_event = false;
        for line in body_str.lines() {
            let line = line.trim();
            if line == "event: error" {
                is_error_event = true;
                continue;
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(json) = serde_json::from_str::<Value>(json_str) {
                    if is_error_event {
                        let msg = json.get("error").and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str()).unwrap_or("未知错误").to_string();
                        let code = json.get("error").and_then(|e| e.get("type"))
                            .map(|c| c.to_string()).unwrap_or_else(|| "error".to_string());
                        return Some((code, msg));
                    }
                    if let Some(err) = check_json_error(&json) {
                        return Some(err);
                    }
                }
                is_error_event = false;
            }
        }
    }

    None
}

// ========== 模型映射 ==========

/// 根据模型映射转换请求体中的模型名称
async fn map_model_name(
    body: &bytes::Bytes,
    endpoint: &EndpointState,
    pool: &Pool,
    state: &AppState,
) -> Result<bytes::Bytes, AppError> {
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else {
        return Ok(body.clone());
    };
    
    let client_model = json.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    if client_model.is_empty() {
        return Ok(body.clone());
    }
    
    if state.get_cached_models(&endpoint.config.id).is_none() {
        let _ = state.fetch_endpoint_models(&endpoint.config.id).await;
    }
    
    let resolved_model = state.resolve_model_for_endpoint(pool, endpoint, &client_model);
    
    if resolved_model != client_model {
        if let Some(error_msg) = resolved_model.strip_prefix("ERROR:") {
            return Err(AppError::BadRequest(error_msg.to_string()));
        }
        if let Some(obj) = json.as_object_mut() {
            obj.insert("model".to_string(), Value::String(resolved_model.clone()));
            debug!("模型映射: {} -> {}", client_model, resolved_model);
            if let Ok(new_body) = serde_json::to_vec(&json) {
                return Ok(bytes::Bytes::from(new_body));
            }
        }
    }
    
    Ok(body.clone())
}

// ========== URL 构建 ==========

/// 根据 base_url 构建完整的目标 URL
fn build_target_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    
    // 如果 base_url 已经包含 /v1 前缀，且 path 也以 v1/ 开头，则去掉 path 中的 v1/
    if (base.ends_with("/v1") || base.ends_with("/v1/")) && (path.starts_with("v1/") || path == "v1") {
        let stripped = path.strip_prefix("v1/").or_else(|| path.strip_prefix("v1")).unwrap_or("");
        return format!("{}/{}", base, stripped);
    }
    
    // 如果 path 已经包含 v1/ 前缀，直接拼接
    if path.starts_with("v1/") || path == "v1" {
        return format!("{}/{}", base, path);
    }
    
    // 检查 base_url 路径中是否已包含版本前缀（如 /v1, /v6, /v2 等）
    // 通过检查路径中是否有 /v数字 的模式
    let has_version = base.split('/').any(|seg| {
        seg.len() >= 2 && seg.starts_with('v') && seg[1..].chars().all(|c| c.is_ascii_digit())
    });
    
    if has_version {
        // 已有版本前缀，直接拼接
        format!("{}/{}", base, path)
    } else {
        // 没有版本前缀，添加 /v1
        format!("{}/v1/{}", base, path)
    }
}

// ========== 公共重试逻辑 ==========

/// 端点调用失败的原始错误信息，用于调用日志展示
struct EndpointCallError {
    error: AppError,
    /// 端点直接返回的原始报错内容（如响应体中的 error.message）
    raw_message: Option<String>,
}

impl From<AppError> for EndpointCallError {
    fn from(error: AppError) -> Self {
        Self { error, raw_message: None }
    }
}

/// 重试循环的上下文
struct RetryContext {
    exposed_api: ExposedApi,
    pool: Pool,
    algorithm: ScheduleAlgorithm,
    retry_mode: RetryMode,
    max_retries: usize,
    last_error: Option<AppError>,
    /// 最后一个端点的原始报错内容
    last_raw_error: Option<String>,
    tried_ids: Vec<String>,
    first_endpoint_id: Option<String>,
}

impl RetryContext {
    fn new(state: &AppState, path: &str) -> Result<Self, AppError> {
        let exposed_api = state.match_exposed_api(path)
            .ok_or_else(|| AppError::NotFound(format!("未找到匹配的对外API: {}", path)))?;
        let pool = state.get_pool(&exposed_api.pool_id)
            .ok_or_else(|| AppError::Internal(format!("池不存在: {}", exposed_api.pool_id)))?;

        let algorithm = pool.schedule_algorithm.clone();
        let retry_mode = pool.retry_mode.clone();
        let retry_count = pool.retry_count.max(1) as usize;
        let available_count = state.available_endpoint_ids_in_pool(&pool.id).len().max(1);
        let max_retries = match retry_mode {
            RetryMode::None => 1,
            RetryMode::Same => retry_count,
            RetryMode::Pool => available_count,
        };

        Ok(Self {
            exposed_api,
            pool,
            algorithm,
            retry_mode,
            max_retries,
            last_error: None,
            last_raw_error: None,
            tried_ids: Vec::new(),
            first_endpoint_id: None,
        })
    }

    /// 选择当前尝试的端点
    fn select_endpoint(&mut self, state: &AppState, attempt: usize) -> Option<String> {
        let endpoint_id = if attempt == 0 || self.retry_mode == RetryMode::Same {
            if self.retry_mode == RetryMode::Same {
                if let Some(cached_id) = self.first_endpoint_id.as_ref() {
                    // Same 模式：检查缓存的端点是否仍然可用
                    if state.get_endpoint(cached_id).as_ref().is_some_and(|ep| ep.is_available()) {
                        return Some(cached_id.clone());
                    }
                    // 端点不可用（如 token 耗尽），重新调度
                    warn!("Same 模式缓存端点不可用，重新选择端点: {}", cached_id);
                    self.first_endpoint_id = None;
                }
            }
            let id = Scheduler::select_endpoint(state, &self.pool.id, &self.algorithm)?;
            self.first_endpoint_id = Some(id.clone());
            id
        } else {
            Scheduler::select_next_for_retry(state, &self.pool.id, &self.tried_ids)?
        };

        if self.retry_mode != RetryMode::Same || attempt == 0 {
            self.tried_ids.push(endpoint_id.clone());
        }

        Some(endpoint_id)
    }

    /// 记录错误并判断是否继续重试
    /// raw_message 为端点直接返回的原始报错内容，供调用日志展示
    fn record_error(&mut self, e: AppError, raw_message: Option<String>) -> bool {
        let retryable = e.is_retryable();
        self.last_error = Some(e);
        if raw_message.is_some() {
            self.last_raw_error = raw_message;
        }
        if self.retry_mode == RetryMode::Pool {
            return true;
        }
        retryable && self.retry_mode != RetryMode::None
    }

    /// 返回最终错误
    fn into_final_error(self) -> AppError {
        let detail = self.last_error
            .as_ref()
            .map(|e| format!("，最后错误: {}", e))
            .unwrap_or_default();
        warn!("端点池所有接口均不可用{}", detail);
        AppError::Proxy(format!("端点池所有接口均不可用{}，请检查后重试。", detail))
    }
}

/// 构建上游请求
fn build_upstream_request(
    state: &AppState,
    req: &HttpRequest,
    endpoint: &EndpointState,
    target_url: &str,
    body: &[u8],
) -> Result<reqwest::RequestBuilder, AppError> {
    let mut builder = state.http_client.request(
        reqwest::Method::from_bytes(req.method().as_str().as_bytes())
            .map_err(|e| AppError::Proxy(format!("无效的HTTP方法: {}", e)))?,
        target_url,
    );

    // 复制请求头（跳过认证头和连接控制头）
    for (key, value) in req.headers() {
        let key_str = key.as_str().to_lowercase();
        if key_str != "host" && key_str != "content-length" && key_str != "authorization" && key_str != "x-api-key" && key_str != "connection" {
            if let Ok(v) = value.to_str() {
                builder = builder.header(key.as_str(), v);
            }
        }
    }

    // 设置认证头
    match endpoint.config.api_type {
        ApiType::OpenAI | ApiType::OpenAIResponses | ApiType::Custom => {
            builder = builder.header("Authorization", format!("Bearer {}", endpoint.config.api_key));
        }
        ApiType::Anthropic => {
            builder = builder.header("x-api-key", &endpoint.config.api_key);
            builder = builder.header("anthropic-version", "2023-06-01");
        }
    }

    if req.headers().get("content-type").is_none() {
        builder = builder.header("Content-Type", "application/json");
    }

    Ok(builder.body(body.to_vec()))
}

/// 发送请求并检查网络错误
async fn send_request(
    builder: reqwest::RequestBuilder,
    endpoint_name: &str,
    timeout_secs: u64,
) -> Result<reqwest::Response, AppError> {
    let builder = builder.timeout(std::time::Duration::from_secs(timeout_secs));
    builder.send().await.map_err(|e| {
        let error_msg = if e.is_timeout() {
            format!("连接超时: {}", e)
        } else if e.is_connect() {
            format!("连接失败: {}", e)
        } else if e.is_request() {
            format!("请求错误: {}", e)
        } else {
            format!("网络异常: {}", e)
        };
        error!("端点 {} 请求异常: {}", endpoint_name, error_msg);
        AppError::Proxy(error_msg)
    })
}

// ========== API 转发入口 ==========

/// 处理API请求转发（非流式）
pub async fn forward_request(
    state: &AppState,
    req: &HttpRequest,
    body: bytes::Bytes,
    path: &str,
    api_prefix: Option<String>,
) -> Result<HttpResponse, AppError> {
    let start = std::time::Instant::now();
    let client_ip = get_client_ip(req);
    let method = req.method().to_string();
    let mut last_endpoint_id: Option<String> = None;
    let mut last_endpoint_name: Option<String> = None;
    let mut last_token_usage: Option<TokenUsage> = None;
    let mut last_replay_error: Option<(u16, String)> = None;

    let mut ctx = RetryContext::new(state, path)?;
    let mut result: Option<Result<HttpResponse, AppError>> = None;

    for attempt in 0..ctx.max_retries {
        let endpoint_id = match ctx.select_endpoint(state, attempt) {
            Some(id) => id,
            None => {
                result = Some(Err(AppError::Proxy("池中没有可用的代理端点".to_string())));
                break;
            }
        };

        let endpoint = match state.get_endpoint(&endpoint_id) {
            Some(ep) => ep,
            None => {
                result = Some(Err(AppError::Proxy(format!("端点不存在: {}", endpoint_id))));
                break;
            }
        };

        last_endpoint_id = Some(endpoint_id.clone());
        last_endpoint_name = Some(endpoint.config.name.clone());

        // 原子预留请求额度，防止并发超支
        if !state.reserve_endpoint_request(&endpoint_id) {
            // 端点刚好被耗尽，当作可重试错误继续选择其他端点
            let e = AppError::Proxy(format!("端点 {} 额度已耗尽", endpoint.config.name));
            state.increment_endpoint_errors(&endpoint_id);
            if !ctx.record_error(e, None) {
                result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                break;
            }
            continue;
        }

        debug!("尝试转发请求到端点 {} ({}) (尝试 {}/{})", endpoint.config.name, endpoint_id, attempt + 1, ctx.max_retries);

        let actual_path = path.strip_prefix(&ctx.exposed_api.prefix).unwrap_or(path);
        let mapped_body = match map_model_name(&body, &endpoint, &ctx.pool, state).await {
            Ok(b) => b,
            Err(e) => {
                warn!("端点 {} 模型名称处理失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e, None) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        };

        match forward_to_endpoint(state, req, &mapped_body, &endpoint, actual_path, &ctx.exposed_api.api_type).await {
            Ok((response, token_usage, response_body_bytes)) => {
                // 若该接口开启了数据回放，记录完整请求/响应体
                if ctx.exposed_api.replay_enabled {
                    record_replay(
                        state,
                        &ctx.exposed_api.id,
                        &method,
                        path,
                        response.status().as_u16(),
                        "success",
                        None,
                        start.elapsed().as_millis() as u64,
                        &body,
                        &response_body_bytes,
                    );
                }
                result = Some(Ok(response));
                last_token_usage = Some(token_usage);
                break;
            }
            Err(EndpointCallError { error, raw_message }) => {
                warn!("端点 {} 请求失败: {}", endpoint.config.name, error);
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                let status_code = error.status_code();
                let error_body = raw_message.clone().unwrap_or_else(|| error.to_string());
                last_replay_error = Some((status_code, error_body));
                if !ctx.record_error(error, raw_message) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
            }
        }
    }

    // 保留最后一个端点直接返回的错误，用于调用日志展示
    let last_raw_error = ctx.last_raw_error.clone();
    let replay_api_id = ctx.exposed_api.id.clone();
    let replay_enabled = ctx.exposed_api.replay_enabled;
    let result = match result {
        Some(result) => result,
        None => Err(ctx.into_final_error()),
    };

    // 重试链路只保留最终结果，避免为每次失败尝试写入冗余回放。
    if replay_enabled {
        if let Err(error) = &result {
            let (status_code, response_body) = last_replay_error
                .unwrap_or_else(|| (error.status_code(), error.to_string()));
            record_replay(
                state,
                &replay_api_id,
                &method,
                path,
                status_code,
                "error",
                Some(response_body.clone()),
                start.elapsed().as_millis() as u64,
                &body,
                response_body.as_bytes(),
            );
        }
    }

    // 只记录命中对外 API 前缀的请求，过滤扫描器流量
    if api_prefix.is_some() {
        let (status_code, status, error_message) = match &result {
            Ok(resp) => (resp.status().as_u16(), "success".to_string(), None),
            Err(e) => {
                // 优先显示端点直接返回的报错，而非对外接口包装后的统一错误
                let err_msg = last_raw_error
                    .clone()
                    .unwrap_or_else(|| e.to_string());
                (e.status_code(), "error".to_string(), Some(err_msg))
            }
        };
        let (input_tokens, output_tokens, total_tokens) = last_token_usage
            .map(|u| (u.input, u.output, u.total))
            .unwrap_or((None, None, None));
        state.add_call_log(ApiCallLog {
            timestamp: Utc::now(),
            client_ip,
            method,
            path: path.to_string(),
            api_prefix,
            endpoint_id: last_endpoint_id,
            endpoint_name: last_endpoint_name,
            status_code,
            status,
            error_message,
            duration_ms: start.elapsed().as_millis() as u64,
            input_tokens,
            output_tokens,
            total_tokens,
        });
    }

    result
}

/// 处理流式响应转发
pub async fn forward_stream_request(
    state: web::Data<AppState>,
    req: &HttpRequest,
    body: bytes::Bytes,
    path: &str,
    api_prefix: Option<String>,
) -> Result<HttpResponse, AppError> {
    let start = std::time::Instant::now();
    let client_ip = get_client_ip(req);
    let method = req.method().to_string();
    let usage_tracker = Arc::new(Mutex::new(None::<TokenUsage>));
    let mut last_endpoint_id: Option<String> = None;
    let mut last_endpoint_name: Option<String> = None;

    let mut ctx = RetryContext::new(state.get_ref(), path)?;
    let mut result: Option<Result<HttpResponse, AppError>> = None;

    for attempt in 0..ctx.max_retries {
        let endpoint_id = match ctx.select_endpoint(state.get_ref(), attempt) {
            Some(id) => id,
            None => {
                result = Some(Err(AppError::Proxy("池中没有可用的代理端点".to_string())));
                break;
            }
        };

        let endpoint = match state.get_endpoint(&endpoint_id) {
            Some(ep) => ep,
            None => {
                result = Some(Err(AppError::Proxy(format!("端点不存在: {}", endpoint_id))));
                break;
            }
        };

        last_endpoint_id = Some(endpoint_id.clone());
        last_endpoint_name = Some(endpoint.config.name.clone());

        // 原子预留请求额度，防止并发超支
        if !state.reserve_endpoint_request(&endpoint_id) {
            let e = AppError::Proxy(format!("端点 {} 额度已耗尽", endpoint.config.name));
            state.increment_endpoint_errors(&endpoint_id);
            if !ctx.record_error(e, None) {
                result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                break;
            }
            continue;
        }

        let actual_path = path.strip_prefix(&ctx.exposed_api.prefix).unwrap_or(path);
        let target_path = crate::converter::convert_path(actual_path, &ctx.exposed_api.api_type, &endpoint.config.api_type);
        let target_url = if endpoint.config.api_type == crate::models::ApiType::Custom {
            endpoint.config.url.clone()
        } else {
            build_target_url(&endpoint.config.url, &target_path)
        };

        let mapped_body = match map_model_name(&body, &endpoint, &ctx.pool, state.get_ref()).await {
            Ok(b) => b,
            Err(e) => {
                warn!("端点 {} 模型名称处理失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e, None) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        };

        // 转换请求体
        let converted_body = if std::mem::discriminant(&ctx.exposed_api.api_type) != std::mem::discriminant(&endpoint.config.api_type) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&mapped_body) {
                let converted = crate::converter::convert_request(&json, &ctx.exposed_api.api_type, &endpoint.config.api_type);
                debug!("流式请求体已从 {:?} 转换为 {:?}", ctx.exposed_api.api_type, endpoint.config.api_type);
                bytes::Bytes::from(serde_json::to_vec(&converted).unwrap_or(mapped_body.to_vec()))
            } else {
                mapped_body
            }
        } else {
            mapped_body
        };

        debug!("流式转发到: {} (尝试 {}/{})", target_url, attempt + 1, ctx.max_retries);

        let request_builder = build_upstream_request(state.get_ref(), req, &endpoint, &target_url, &converted_body)?;
        let mut req_start = std::time::Instant::now();
        let mut response: Option<reqwest::Response> = Some(match send_request(request_builder, &endpoint.config.name, endpoint.config.timeout).await {
            Ok(r) => r,
            Err(e) => {
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e, None) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        });

        let mut resp_status = response.as_ref().unwrap().status();
        if resp_status != 200 {
            let error_body = response.take().unwrap().text().await.unwrap_or_default();

            // 如果是 400 + 参数不支持，自动剥离参数后重试同一端点
            if resp_status == 400 {
                if let Some(stripped) = strip_unsupported_params(&error_body, &converted_body) {
                    let retry_builder = build_upstream_request(state.get_ref(), req, &endpoint, &target_url, &stripped);
                    if let Ok(builder) = retry_builder {
                        match send_request(builder, &endpoint.config.name, endpoint.config.timeout).await {
                            Ok(retry) => {
                                let retry_status = retry.status();
                                if retry_status == 200 {
                                    debug!("端点 {} 流式剥离参数后重试成功", endpoint.config.name);
                                    response = Some(retry);
                                    resp_status = retry_status;
                                    req_start = std::time::Instant::now();
                                }
                            }
                            Err(e) => {
                                warn!("端点 {} 流式剥离参数重试连接失败: {}", endpoint.config.name, e);
                            }
                        }
                    }
                }
            }

            if resp_status != 200 {
                let duration_ms = req_start.elapsed().as_millis() as u64;
                state.record_latency(&endpoint_id, duration_ms);
                warn!("端点 {} 返回错误状态 {}: {}", endpoint.config.name, resp_status, error_body);
                state.increment_endpoint_errors(&endpoint_id);
                let sanitized = sanitize_error_body(&error_body);
                let e = if resp_status.is_client_error() && resp_status.as_u16() != 429 {
                    AppError::UpstreamError(format!("上游返回状态 {}: {}", resp_status, sanitized))
                } else {
                    AppError::Proxy(format!("上游返回状态 {}: {}", resp_status, sanitized))
                };
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e, Some(sanitized)) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        }

        let response = response.unwrap();

        // 保存上游响应头，后续透传给客户端
        let upstream_headers = response.headers().clone();
        let mut stream = response.bytes_stream();

        let first_chunk = match stream.next().await {
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => {
                let duration_ms = req_start.elapsed().as_millis() as u64;
                state.record_latency(&endpoint_id, duration_ms);
                warn!("端点 {} 读取响应流失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                let e = AppError::Proxy(format!("读取响应流失败: {}", e));
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e, None) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
            None => {
                let duration_ms = req_start.elapsed().as_millis() as u64;
                state.record_latency(&endpoint_id, duration_ms);
                warn!("端点 {} 返回空响应", endpoint.config.name);
                state.increment_endpoint_errors(&endpoint_id);
                let e = AppError::Proxy("上游返回空响应".to_string());
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e, None) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        };

        if let Some((error_code, error_msg)) = detect_response_error(&first_chunk) {
            let duration_ms = req_start.elapsed().as_millis() as u64;
            state.record_latency(&endpoint_id, duration_ms);
            warn!("端点 {} 响应中包含错误 [{}]: {}", endpoint.config.name, error_code, error_msg);
            state.increment_endpoint_errors(&endpoint_id);
            let e = AppError::Proxy(format!("上游错误 [{}]: {}", error_code, error_msg));
            state.release_endpoint_request(&endpoint_id);
            if !ctx.record_error(e, Some(error_msg)) {
                result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                break;
            }
            continue;
        }

        // 无错误，记录首字节延迟
        let duration_ms = req_start.elapsed().as_millis() as u64;
        state.record_latency(&endpoint_id, duration_ms);
        let ep_id = endpoint.config.id.clone();
        let ep_api_type = endpoint.config.api_type.clone();
        let client_api_type = ctx.exposed_api.api_type.clone();
        let need_convert = std::mem::discriminant(&client_api_type) != std::mem::discriminant(&ep_api_type);
        let state_clone = state.clone();

        let first_stream = futures_util::stream::once(async move { Ok::<_, reqwest::Error>(first_chunk) });
        let full_stream = first_stream.chain(stream);

        let mut response_builder = actix_web::HttpResponse::Ok();
        // 透传上游响应头（白名单），保留 SSE 必需头
        for (key, value) in &upstream_headers {
            let key_str = key.as_str().to_lowercase();
            if key_str.starts_with("x-") || key_str == "cache-control" {
                if let Ok(v) = value.to_str() {
                    response_builder.insert_header((key.as_str(), v));
                }
            }
        }

        // 构建原始流（不含调用日志写入）
        let raw_stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> = if need_convert {
            let mut converter = crate::converter::StreamConverter::new(ep_api_type.clone(), client_api_type);
            let tracker = usage_tracker.clone();
            let mut buffer = String::new();
            let mut output_buffer = Vec::new();
            Box::pin(full_stream.map(move |chunk| {
                let chunk = chunk.map_err(std::io::Error::other);
                if let Ok(data) = &chunk {
                    if let Ok(text) = std::str::from_utf8(data) {
                        buffer.push_str(text);
                        while let Some(line_end) = buffer.find('\n') {
                            let line = buffer[..line_end].trim().to_string();
                            buffer = buffer[line_end + 1..].to_string();
                            if line.is_empty() { continue; }
                            if line.starts_with("data: ") && !line.contains("[DONE]") {
                                let json_str = &line[6..];
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                                    if json.get("usage").is_some() {
                                        let tokens = parse_token_usage(json_str.as_bytes(), &ep_api_type);
                                        if tokens > 0 {
                                            state_clone.update_endpoint_tokens(&ep_id, tokens);
                                        }
                                        let detail = parse_token_usage_detail(json_str.as_bytes(), &ep_api_type);
                                        if let Ok(mut t) = tracker.lock() {
                                            *t = Some(detail);
                                        }
                                    }
                                }
                            }
                            let converted_lines = converter.convert_chunk(&line);
                            for converted in converted_lines {
                                output_buffer.push(converted);
                            }
                        }
                        if output_buffer.is_empty() {
                            return Ok(bytes::Bytes::new());
                        } else {
                            let output: String = output_buffer.drain(..).collect();
                            return Ok(bytes::Bytes::from(output));
                        }
                    }
                }
                Ok(bytes::Bytes::new())
            }))
        } else {
            let tracker = usage_tracker.clone();
            let mut buffer = String::new();
            Box::pin(full_stream.map(move |chunk| {
                let chunk = chunk.map_err(std::io::Error::other);
                if let Ok(data) = &chunk {
                    if let Ok(text) = std::str::from_utf8(data) {
                        buffer.push_str(text);
                        while let Some(line_end) = buffer.find('\n') {
                            let line = buffer[..line_end].trim().to_string();
                            buffer = buffer[line_end + 1..].to_string();
                            if line.starts_with("data: ") && !line.contains("[DONE]") {
                                let json_str = &line[6..];
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                                    if json.get("usage").is_some() {
                                        let tokens = parse_token_usage(json_str.as_bytes(), &ep_api_type);
                                        if tokens > 0 {
                                            state_clone.update_endpoint_tokens(&ep_id, tokens);
                                        }
                                        let detail = parse_token_usage_detail(json_str.as_bytes(), &ep_api_type);
                                        if let Ok(mut t) = tracker.lock() {
                                            *t = Some(detail);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                chunk
            }))
        };

        // 构建调用日志所需上下文
        let log_client_ip = client_ip.clone();
        let log_method = method.clone();
        let log_path = path.to_string();
        let log_api_prefix = api_prefix.clone();
        let log_endpoint_id = last_endpoint_id.clone();
        let log_endpoint_name = last_endpoint_name.clone();
        let log_start = start;
        let state_for_log = state.clone();
        let tracker_for_log = usage_tracker.clone();
        let replay_enabled = ctx.exposed_api.replay_enabled;
        let replay_api_id = ctx.exposed_api.id.clone();
        let replay_request_body = body.clone();
        let replay_max_size_kb = state.replay_config.read().max_body_size_kb;

        // 包裹流，在流结束后写入调用日志（含 token 用量）
        let wrapped_stream = StreamLogWriter {
            inner: raw_stream,
            replay_buffer: replay_enabled.then(|| ReplayBuffer::new(replay_max_size_kb)),
            on_complete: Some(Box::new(move |replay_buffer| {
                let usage = tracker_for_log.lock().unwrap().take();
                state_for_log.add_call_log(ApiCallLog {
                    timestamp: Utc::now(),
                    client_ip: log_client_ip,
                    method: log_method.clone(),
                    path: log_path.clone(),
                    api_prefix: log_api_prefix,
                    endpoint_id: log_endpoint_id,
                    endpoint_name: log_endpoint_name,
                    status_code: 200,
                    status: "success".to_string(),
                    error_message: None,
                    duration_ms: log_start.elapsed().as_millis() as u64,
                    input_tokens: usage.as_ref().and_then(|u| u.input),
                    output_tokens: usage.as_ref().and_then(|u| u.output),
                    total_tokens: usage.as_ref().and_then(|u| u.total),
                });
                if let Some(response) = replay_buffer {
                    record_stream_replay(
                        state_for_log.get_ref(),
                        &replay_api_id,
                        &log_method,
                        &log_path,
                        log_start.elapsed().as_millis() as u64,
                        &replay_request_body,
                        response,
                    );
                }
            })),
        };

        let body_stream = response_builder
            .content_type("text/event-stream")
            .insert_header(("Cache-Control", "no-cache"))
            .insert_header(("Connection", "keep-alive"))
            .streaming(wrapped_stream);

        result = Some(Ok(body_stream));
        break;
    }

    // 错误路径：请求未达到流式响应阶段，直接写入调用日志（无 token 数据）
    // 成功路径：返回流式响应，调用日志已在流结束回调中写入
    let replay_api_id = ctx.exposed_api.id.clone();
    let replay_enabled = ctx.exposed_api.replay_enabled;
    match result {
        Some(Ok(body_stream)) => Ok(body_stream),
        Some(Err(e)) => {
            let last_raw_error = ctx.last_raw_error.clone();
            let err_msg = last_raw_error.unwrap_or_else(|| e.to_string());
            if replay_enabled {
                record_replay(
                    state.get_ref(),
                    &replay_api_id,
                    &method,
                    path,
                    e.status_code(),
                    "error",
                    Some(err_msg.clone()),
                    start.elapsed().as_millis() as u64,
                    &body,
                    err_msg.as_bytes(),
                );
            }
            if api_prefix.is_some() {
                state.add_call_log(ApiCallLog {
                    timestamp: Utc::now(),
                    client_ip,
                    method: method.clone(),
                    path: path.to_string(),
                    api_prefix,
                    endpoint_id: last_endpoint_id,
                    endpoint_name: last_endpoint_name,
                    status_code: e.status_code(),
                    status: "error".to_string(),
                    error_message: Some(err_msg),
                    duration_ms: start.elapsed().as_millis() as u64,
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                });
            }
            Err(e)
        }
        None => {
            let last_raw_error = ctx.last_raw_error.clone();
            let e = ctx.into_final_error();
            if replay_enabled {
                let err_msg = last_raw_error.unwrap_or_else(|| e.to_string());
                record_replay(
                    state.get_ref(),
                    &replay_api_id,
                    &method,
                    path,
                    e.status_code(),
                    "error",
                    Some(err_msg.clone()),
                    start.elapsed().as_millis() as u64,
                    &body,
                    err_msg.as_bytes(),
                );
            }
            Err(e)
        }
    }
}

/// 从上游错误消息中提取不支持的参数名（用于自动剥离后重试）
fn extract_unsupported_params(error_body: &str) -> Vec<String> {
    let mut params = Vec::new();
    let body_lower = error_body.to_lowercase();

    // Pattern: "Unsupported parameter(s): `param1`, `param2`" (nvidia/OpenAI style)
    if body_lower.contains("unsupported parameter") {
        for part in error_body.split('`') {
            let p = part.trim();
            if !p.is_empty() && p.len() > 1 && p.len() < 100
                && p.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && !p.contains(' ') && !p.contains(':') {
                params.push(p.to_string());
            }
        }
    }

    // Pattern: "field FooBar invalid, should be one of:" (商汤 style) → convert CamelCase to snake_case
    if body_lower.contains("should be one of") {
        if let Some(pos) = body_lower.find("field ") {
            let after = &body_lower[pos + 6..];
            if let Some(end) = after.find(" invalid") {
                let field = after[..end].trim();
                let snake: String = {
                    let mut s = String::new();
                    for (i, c) in field.chars().enumerate() {
                        if c.is_uppercase() && i > 0 { s.push('_'); }
                        s.push(c.to_ascii_lowercase());
                    }
                    s
                };
                if !snake.is_empty() && snake.len() < 100
                    && snake.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    params.push(snake);
                }
            }
        }
    }

    params
}

/// 从请求体中移除不支持的参数，返回剥离后的 body（如果无参数可移除则返回 None）
fn strip_unsupported_params(error_body: &str, body_bytes: &bytes::Bytes) -> Option<bytes::Bytes> {
    let params = extract_unsupported_params(error_body);
    let body_lower = error_body.to_lowercase();
    let has_tool_issue = body_lower.contains("tool_call");

    if params.is_empty() && !has_tool_issue { return None; }
    if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
        let mut changed = false;
        if let Some(obj) = json.as_object_mut() {
            for p in &params {
                if obj.remove(p.as_str()).is_some() { changed = true; }
            }
            // 移除 tool 相关字段（当上游端点报告 tool_call_id 错误时）
            if has_tool_issue {
                if let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
                    for msg in messages.iter_mut() {
                        if let Some(msg_obj) = msg.as_object_mut() {
                            if msg_obj.remove("tool_call_id").is_some() { changed = true; }
                            if msg_obj.remove("tool_calls").is_some() { changed = true; }
                        }
                    }
                }
            }
        }
        if !changed { return None; }
        Some(bytes::Bytes::from(serde_json::to_vec(&json).unwrap_or(body_bytes.to_vec())))
    } else {
        None
    }
}

/// 处理上游成功响应（提取公共逻辑，供正常路径和参数剥离重试路径复用）
async fn process_upstream_success(
    state: &AppState,
    endpoint: &EndpointState,
    response: reqwest::Response,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    start: std::time::Instant,
    path: &str,
    client_api_type: &ApiType,
) -> Result<(HttpResponse, TokenUsage, bytes::Bytes), EndpointCallError> {
    let response_body = response.bytes().await
        .map_err(|e| AppError::Proxy(format!("读取响应失败: {}", e)))?;
    let duration_ms = start.elapsed().as_millis() as u64;
    state.record_latency(&endpoint.config.id, duration_ms);

    if let Some((error_code, error_msg)) = detect_response_error(&response_body) {
        error!("端点 {} 响应中包含错误 [{}]: {}", endpoint.config.name, error_code, error_msg);
        return Err(EndpointCallError {
            error: AppError::Proxy(format!("上游错误 [{}]: {}", error_code, error_msg)),
            raw_message: Some(error_msg),
        });
    }

    let token_usage = parse_token_usage_detail(&response_body, &endpoint.config.api_type);
    if let Some(total) = token_usage.total {
        if total > 0 {
            state.update_endpoint_tokens(&endpoint.config.id, total);
        }
    }

    let is_api_request = path == "chat/completions" || path.starts_with("chat/completions?")
        || path == "responses" || path.starts_with("responses?")
        || path == "messages" || path.starts_with("messages?");
    let final_body = if is_api_request && std::mem::discriminant(client_api_type) != std::mem::discriminant(&endpoint.config.api_type) {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body) {
            let converted = crate::converter::convert_response(&json, &endpoint.config.api_type, client_api_type);
            bytes::Bytes::from(serde_json::to_vec(&converted).unwrap_or(response_body.to_vec()))
        } else {
            response_body
        }
    } else {
        response_body
    };

    let mut response_builder = HttpResponse::build(
        actix_web::http::StatusCode::from_u16(status.as_u16())
            .unwrap_or(actix_web::http::StatusCode::OK),
    );

    for (key, value) in headers {
        let key_str = key.as_str().to_lowercase();
        if key_str == "connection" || key_str == "transfer-encoding" {
            continue;
        }
        if let Ok(v) = value.to_str() {
            response_builder.insert_header((key.as_str(), v));
        }
    }

    Ok((response_builder.body(final_body.clone()), token_usage, final_body))
}

/// 转发请求到指定端点（非流式）
async fn forward_to_endpoint(
    state: &AppState,
    req: &HttpRequest,
    body: &bytes::Bytes,
    endpoint: &EndpointState,
    path: &str,
    client_api_type: &ApiType,
) -> Result<(HttpResponse, TokenUsage, bytes::Bytes), EndpointCallError> {
    let target_path = crate::converter::convert_path(path, client_api_type, &endpoint.config.api_type);
    let target_url = if endpoint.config.api_type == crate::models::ApiType::Custom {
        endpoint.config.url.clone()
    } else {
        build_target_url(&endpoint.config.url, &target_path)
    };
    debug!("转发到: {} (客户端格式: {:?}, 端点格式: {:?})", target_url, client_api_type, endpoint.config.api_type);

    let converted_body = if std::mem::discriminant(client_api_type) != std::mem::discriminant(&endpoint.config.api_type) {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
            let converted = crate::converter::convert_request(&json, client_api_type, &endpoint.config.api_type);
            debug!("请求体已从 {:?} 转换为 {:?}", client_api_type, endpoint.config.api_type);
            bytes::Bytes::from(serde_json::to_vec(&converted).unwrap_or(body.to_vec()))
        } else {
            body.clone()
        }
    } else {
        body.clone()
    };

    let request_builder = build_upstream_request(state, req, endpoint, &target_url, &converted_body)?;
    let start = std::time::Instant::now();
    let response = send_request(request_builder, &endpoint.config.name, endpoint.config.timeout).await?;
    let status = response.status();
    let headers = response.headers().clone();

    if status == 200 {
        return process_upstream_success(state, endpoint, response, status, &headers, start, path, client_api_type).await;
    }

    let error_body = response.text().await.unwrap_or_default();
    let duration_ms = start.elapsed().as_millis() as u64;
    state.record_latency(&endpoint.config.id, duration_ms);

    // 如果是 400 + 参数不支持错误，自动剥离参数后重试同一端点
    if status == 400 {
        if let Some(stripped_body) = strip_unsupported_params(&error_body, &converted_body) {
            debug!("端点 {} 不支持部分参数，剥离后重试", endpoint.config.name);
            let retry_builder = build_upstream_request(state, req, endpoint, &target_url, &stripped_body)?;
            match send_request(retry_builder, &endpoint.config.name, endpoint.config.timeout).await {
                Ok(retry) => {
                    let retry_status = retry.status();
                    if retry_status == 200 {
                        let retry_headers = retry.headers().clone();
                        return process_upstream_success(state, endpoint, retry, retry_status, &retry_headers, start, path, client_api_type).await;
                    }
                    let retry_error_body = retry.text().await.unwrap_or_default();
                    let retry_duration = start.elapsed().as_millis() as u64;
                    state.record_latency(&endpoint.config.id, retry_duration);
                    error!("端点 {} 剥离参数后仍然失败 {}: {}", endpoint.config.name, retry_status, retry_error_body);
                    let sanitized = sanitize_error_body(&retry_error_body);
                    return Err(EndpointCallError {
                        error: AppError::UpstreamError(format!("上游返回状态 {}: {}", retry_status, sanitized)),
                        raw_message: Some(sanitized),
                    });
                }
                Err(e) => {
                    error!("端点 {} 剥离参数重试时连接失败: {}", endpoint.config.name, e);
                    let msg = format!("剥离参数重试失败: {}", e);
                    return Err(EndpointCallError { error: AppError::Proxy(msg), raw_message: None });
                }
            }
        }
    }

    error!("端点 {} 返回错误状态 {}: {}", endpoint.config.name, status, error_body);
    let sanitized = sanitize_error_body(&error_body);
    let error = if status.is_client_error() && status.as_u16() != 429 {
        AppError::UpstreamError(format!("上游返回状态 {}: {}", status, sanitized))
    } else {
        AppError::Proxy(format!("上游返回状态 {}: {}", status, sanitized))
    };
    Err(EndpointCallError {
        error,
        raw_message: Some(sanitized),
    })
}

/// 详细的 Token 使用量
#[derive(Debug, Clone, Copy, Default)]
struct TokenUsage {
    input: Option<u64>,
    output: Option<u64>,
    total: Option<u64>,
}

/// 解析响应中的token使用量（详细）
fn parse_token_usage_detail(body: &[u8], api_type: &ApiType) -> TokenUsage {
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return TokenUsage::default(),
    };

    let json: serde_json::Value = match serde_json::from_str(body_str) {
        Ok(v) => v,
        Err(_) => return TokenUsage::default(),
    };

    match api_type {
        ApiType::OpenAI | ApiType::OpenAIResponses | ApiType::Custom => {
            let usage = json.get("usage");
            let input = usage
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|t| t.as_u64());
            let output = usage
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|t| t.as_u64());
            let total = usage
                .and_then(|u| u.get("total_tokens"))
                .and_then(|t| t.as_u64());
            TokenUsage { input, output, total }
        }
        ApiType::Anthropic => {
            let usage = json.get("usage");
            let input = usage
                .and_then(|u| u.get("input_tokens"))
                .and_then(|t| t.as_u64());
            let output = usage
                .and_then(|u| u.get("output_tokens"))
                .and_then(|t| t.as_u64());
            let total = match (input, output) {
                (Some(i), Some(o)) => Some(i.saturating_add(o)),
                _ => None,
            };
            TokenUsage { input, output, total }
        }
    }
}

/// 解析响应中的token使用量（兼容旧接口，返回总数）
fn parse_token_usage(body: &[u8], api_type: &ApiType) -> u64 {
    parse_token_usage_detail(body, api_type).total.unwrap_or(0)
}

/// 流式响应的 Stream 包装器，在流结束后写入调用日志（含 token 用量）
struct StreamLogWriter {
    inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>>,
    replay_buffer: Option<ReplayBuffer>,
    on_complete: Option<Box<dyn FnOnce(Option<ReplayBuffer>) + Send>>,
}

impl Stream for StreamLogWriter {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let poll = this.inner.as_mut().poll_next(cx);
        if let Poll::Ready(Some(Ok(chunk))) = &poll {
            if let Some(buffer) = this.replay_buffer.as_mut() {
                buffer.append(chunk);
            }
        }
        if matches!(&poll, Poll::Ready(None)) {
            if let Some(cb) = this.on_complete.take() {
                (cb)(this.replay_buffer.take());
            }
        }
        poll
    }
}

impl Drop for StreamLogWriter {
    fn drop(&mut self) {
        if let Some(cb) = self.on_complete.take() {
            (cb)(self.replay_buffer.take());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_openai_token_usage() {
        let body = r#"{"usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}}"#;
        let tokens = parse_token_usage(body.as_bytes(), &ApiType::OpenAI);
        assert_eq!(tokens, 30);
    }

    #[test]
    fn test_parse_anthropic_token_usage() {
        let body = r#"{"usage": {"input_tokens": 15, "output_tokens": 25}}"#;
        let tokens = parse_token_usage(body.as_bytes(), &ApiType::Anthropic);
        assert_eq!(tokens, 40);
    }

    #[test]
    fn test_parse_empty_usage() {
        let body = r#"{"id": "chatcmpl-123"}"#;
        let tokens = parse_token_usage(body.as_bytes(), &ApiType::OpenAI);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_parse_invalid_json() {
        let body = "not json";
        let tokens = parse_token_usage(body.as_bytes(), &ApiType::OpenAI);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_capture_body_handles_utf8_and_truncation() {
        let (body, truncated) = capture_body(b"hello", 1);
        assert_eq!(body, "hello");
        assert!(!truncated);

        let input = vec![b'x'; 1025];
        let (body, truncated) = capture_body(&input, 1);
        assert_eq!(body.len(), 1024);
        assert!(truncated);
    }

    #[test]
    fn test_capture_body_handles_non_utf8() {
        let (body, truncated) = capture_body(&[0xff, b'a'], 1);
        assert!(body.contains('\u{fffd}'));
        assert!(!truncated);
    }

    #[test]
    fn test_replay_buffer_preserves_order_and_marks_truncation() {
        let mut buffer = ReplayBuffer::new(1);
        buffer.append(b"first-");
        buffer.append(b"second");
        assert_eq!(buffer.body, b"first-second");
        assert!(!buffer.truncated);

        buffer.append(&vec![b'x'; 1024]);
        assert_eq!(buffer.body.len(), 1024);
        assert!(buffer.truncated);
    }

    #[actix_rt::test]
    async fn test_stream_log_writer_collects_chunks_in_order() {
        let completed = Arc::new(Mutex::new(None));
        let callback_result = completed.clone();
        let source = futures_util::stream::iter(vec![
            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"first")),
            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"-second")),
        ]);
        let mut stream = StreamLogWriter {
            inner: Box::pin(source),
            replay_buffer: Some(ReplayBuffer::new(1)),
            on_complete: Some(Box::new(move |buffer| {
                *callback_result.lock().unwrap() = buffer.map(|b| b.body);
            })),
        };

        while stream.next().await.is_some() {}
        assert_eq!(completed.lock().unwrap().as_deref(), Some(b"first-second".as_slice()));
    }
}
