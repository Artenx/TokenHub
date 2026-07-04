use crate::error::AppError;
use crate::models::*;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::Value;
use tracing::{debug, error, warn};

/// 截断上游错误响应体，防止敏感信息通过错误消息泄露给客户端
fn sanitize_error_body(body: &str) -> String {
    if body.chars().count() > 200 {
        format!("{}...(已截断)", body.chars().take(200).collect::<String>())
    } else {
        body.to_string()
    }
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

/// 重试循环的上下文
struct RetryContext {
    exposed_api: ExposedApi,
    pool: Pool,
    algorithm: ScheduleAlgorithm,
    retry_mode: RetryMode,
    max_retries: usize,
    last_error: Option<AppError>,
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
    fn record_error(&mut self, e: AppError) -> bool {
        let retryable = e.is_retryable();
        self.last_error = Some(e);
        if self.retry_mode == RetryMode::Pool {
            return true;
        }
        retryable && self.retry_mode != RetryMode::None
    }

    /// 返回最终错误
    fn into_final_error(self) -> AppError {
        if let Some(e) = &self.last_error {
            warn!("端点池所有接口均不可用，最后错误: {}", e);
        }
        AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())
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
        ApiType::OpenAI | ApiType::OpenAIResponses => {
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
    let client_ip = req.connection_info().peer_addr().unwrap_or("unknown").to_string();
    let method = req.method().to_string();
    let mut last_endpoint_id: Option<String> = None;
    let mut last_endpoint_name: Option<String> = None;

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
            if !ctx.record_error(e) {
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
                if !ctx.record_error(e) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        };

        match forward_to_endpoint(state, req, &mapped_body, &endpoint, actual_path, &ctx.exposed_api.api_type).await {
            Ok(response) => {
                result = Some(Ok(response));
                break;
            }
            Err(e) => {
                warn!("端点 {} 请求失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
            }
        }
    }

    let result = result.unwrap_or_else(|| Err(ctx.into_final_error()));

    // 只记录命中对外 API 前缀的请求，过滤扫描器流量
    if api_prefix.is_some() {
        let (status_code, status, error_message) = match &result {
            Ok(resp) => (resp.status().as_u16(), "success".to_string(), None),
            Err(e) => (e.status_code(), "error".to_string(), Some(e.to_string())),
        };
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
    let client_ip = req.connection_info().peer_addr().unwrap_or("unknown").to_string();
    let method = req.method().to_string();
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
            if !ctx.record_error(e) {
                result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                break;
            }
            continue;
        }

        let actual_path = path.strip_prefix(&ctx.exposed_api.prefix).unwrap_or(path);
        let target_path = crate::converter::convert_path(actual_path, &ctx.exposed_api.api_type, &endpoint.config.api_type);
        let target_url = build_target_url(&endpoint.config.url, &target_path);
        
        let mapped_body = match map_model_name(&body, &endpoint, &ctx.pool, state.get_ref()).await {
            Ok(b) => b,
            Err(e) => {
                warn!("端点 {} 模型名称处理失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e) {
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
        let req_start = std::time::Instant::now();
        let response = match send_request(request_builder, &endpoint.config.name, endpoint.config.timeout).await {
            Ok(r) => r,
            Err(e) => {
                state.increment_endpoint_errors(&endpoint_id);
                state.release_endpoint_request(&endpoint_id);
                if !ctx.record_error(e) {
                    result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                    break;
                }
                continue;
            }
        };

        let resp_status = response.status();
        if resp_status != 200 {
            let error_body = response.text().await.unwrap_or_default();
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
            if !ctx.record_error(e) {
                result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                break;
            }
            continue;
        }

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
                if !ctx.record_error(e) {
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
                if !ctx.record_error(e) {
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
            if !ctx.record_error(e) {
                result = Some(Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string())));
                break;
            }
            continue;
        }

        // 无错误，记录首字节延迟，将第一个 chunk 和剩余 stream 合并后转发给客户端
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

        let body_stream = if need_convert {
            // 需要格式转换
            let mut converter = crate::converter::StreamConverter::new(ep_api_type.clone(), client_api_type);
            response_builder
                .content_type("text/event-stream")
                .insert_header(("Cache-Control", "no-cache"))
                .insert_header(("Connection", "keep-alive"))
                .streaming({
                    let mut buffer = String::new();
                    let mut output_buffer = Vec::new();
                    full_stream.map(move |chunk| {
                        let chunk = chunk.map_err(std::io::Error::other);
                        if let Ok(data) = &chunk {
                            if let Ok(text) = std::str::from_utf8(data) {
                                buffer.push_str(text);
                                while let Some(line_end) = buffer.find('\n') {
                                    let line = buffer[..line_end].trim().to_string();
                                    buffer = buffer[line_end + 1..].to_string();
                                    if line.is_empty() { continue; }
                                    // token 统计（从原始格式解析）
                                    if line.starts_with("data: ") && !line.contains("[DONE]") {
                                        let json_str = &line[6..];
                                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                                            if json.get("usage").is_some() {
                                                let tokens = parse_token_usage(json_str.as_bytes(), &ep_api_type);
                                                if tokens > 0 {
                                                    state_clone.update_endpoint_tokens(&ep_id, tokens);
                                                }
                                            }
                                        }
                                    }
                                    // 格式转换
                                    let converted_lines = converter.convert_chunk(&line);
                                    for converted in converted_lines {
                                        output_buffer.push(converted);
                                    }
                                }
                            }
                        }
                        // 返回转换后的数据
                        if output_buffer.is_empty() {
                            Ok::<_, std::io::Error>(bytes::Bytes::new())
                        } else {
                            let output: String = output_buffer.drain(..).collect();
                            Ok(bytes::Bytes::from(output))
                        }
                    })
                })
        } else {
            // 同格式，直接转发
            response_builder
                .content_type("text/event-stream")
                .insert_header(("Cache-Control", "no-cache"))
                .insert_header(("Connection", "keep-alive"))
                .streaming({
                    let mut buffer = String::new();
                    full_stream.map(move |chunk| {
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
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        chunk
                    })
                })
        };

        result = Some(Ok(body_stream));
        break;
    }

    let result = result.unwrap_or_else(|| Err(ctx.into_final_error()));

    // 只记录命中对外 API 前缀的请求，过滤扫描器流量
    if api_prefix.is_some() {
        let (status_code, status, error_message) = match &result {
            Ok(resp) => (resp.status().as_u16(), "success".to_string(), None),
            Err(e) => (e.status_code(), "error".to_string(), Some(e.to_string())),
        };
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
        });
    }

    result
}

/// 转发请求到指定端点（非流式）
async fn forward_to_endpoint(
    state: &AppState,
    req: &HttpRequest,
    body: &bytes::Bytes,
    endpoint: &EndpointState,
    path: &str,
    client_api_type: &ApiType,
) -> Result<HttpResponse, AppError> {
    let target_path = crate::converter::convert_path(path, client_api_type, &endpoint.config.api_type);
    let target_url = build_target_url(&endpoint.config.url, &target_path);
    debug!("转发到: {} (客户端格式: {:?}, 端点格式: {:?})", target_url, client_api_type, endpoint.config.api_type);

    // 转换请求体
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

    if status != 200 {
        let error_body = response.text().await.unwrap_or_default();
        let duration_ms = start.elapsed().as_millis() as u64;
        state.record_latency(&endpoint.config.id, duration_ms);
        error!("端点 {} 返回错误状态 {}: {}", endpoint.config.name, status, error_body);
        let sanitized = sanitize_error_body(&error_body);
        if status.is_client_error() && status.as_u16() != 429 {
            return Err(AppError::UpstreamError(format!("上游返回状态 {}: {}", status, sanitized)));
        }
        return Err(AppError::Proxy(format!("上游返回状态 {}: {}", status, sanitized)));
    }

    let response_body = response.bytes().await.map_err(|e| AppError::Proxy(format!("读取响应失败: {}", e)))?;
    let duration_ms = start.elapsed().as_millis() as u64;
    state.record_latency(&endpoint.config.id, duration_ms);

    if let Some((error_code, error_msg)) = detect_response_error(&response_body) {
        error!("端点 {} 响应中包含错误 [{}]: {}", endpoint.config.name, error_code, error_msg);
        return Err(AppError::Proxy(format!("上游错误 [{}]: {}", error_code, error_msg)));
    }

    let tokens_used = parse_token_usage(&response_body, &endpoint.config.api_type);
    if tokens_used > 0 {
        state.update_endpoint_tokens(&endpoint.config.id, tokens_used);
        debug!("端点 {} 消耗 {} tokens", endpoint.config.name, tokens_used);
    }

    // 转换响应体（仅对 chat/completions 和 responses/messages 路径转换，/models 等特殊路径不转换）
    let is_api_request = path == "chat/completions" || path.starts_with("chat/completions?")
        || path == "responses" || path.starts_with("responses?")
        || path == "messages" || path.starts_with("messages?");
    let final_body = if is_api_request && std::mem::discriminant(client_api_type) != std::mem::discriminant(&endpoint.config.api_type) {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body) {
            let converted = crate::converter::convert_response(&json, &endpoint.config.api_type, client_api_type);
            debug!("响应体已从 {:?} 转换为 {:?}", endpoint.config.api_type, client_api_type);
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

    for (key, value) in &headers {
        let key_str = key.as_str().to_lowercase();
        // 跳过连接控制头和传输编码头（actix-web 会自动处理）
        if key_str == "connection" || key_str == "transfer-encoding" {
            continue;
        }
        if let Ok(v) = value.to_str() {
            response_builder.insert_header((key.as_str(), v));
        }
    }

    Ok(response_builder.body(final_body))
}

/// 解析响应中的token使用量
fn parse_token_usage(body: &[u8], api_type: &ApiType) -> u64 {
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let json: serde_json::Value = match serde_json::from_str(body_str) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    match api_type {
        ApiType::OpenAI | ApiType::OpenAIResponses => {
            json.get("usage")
                .and_then(|u| u.get("total_tokens"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0)
        }
        ApiType::Anthropic => {
            let input = json.get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            let output = json.get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            input.saturating_add(output)
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
}
