use crate::error::AppError;
use crate::models::*;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use actix_web::{web, HttpRequest, HttpResponse};
use futures_util::StreamExt;
use serde_json::Value;
use tracing::{debug, error, warn};

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

/// 检查内容文本是否包含已知错误关键词（仅对短内容检测，避免影响正常回复）
fn check_content_error(content: &str) -> Option<(String, String)> {
    // 错误信息通常很短（<200字符），正常回复的 content 通常较长
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

/// 检查单个 JSON 对象是否为错误响应（兼容三种接口类型）
/// 返回 Some((error_code, error_message)) 如果是错误
fn check_json_error(json: &Value) -> Option<(String, String)> {
    // 跳过正常响应（有 choices 或 id 字段）
    if json.get("choices").is_some() || json.get("id").is_some() {
        // 但检查 choices 中的 content 是否包含错误信息（上游可能把错误包装成模型输出）
        if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                let content = choice
                    .get("delta").and_then(|d| d.get("content")).and_then(|c| c.as_str())
                    .or_else(|| choice.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_str()));
                if let Some(content) = content {
                    // 1. 检查内容中是否嵌入了 JSON 错误对象
                    if let Some(json_start) = content.find('{') {
                        let json_part = &content[json_start..];
                        if let Ok(err_json) = serde_json::from_str::<Value>(json_part) {
                            if err_json.get("error").is_some() {
                                let msg = err_json["error"].get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("未知错误")
                                    .to_string();
                                let code = err_json["error"].get("code")
                                    .map(|c| c.to_string())
                                    .unwrap_or_default();
                                return Some((code, msg));
                            }
                        }
                    }
                    // 2. 检查内容是否包含已知错误关键词（纯文本错误）
                    if let Some(err) = check_content_error(content) {
                        return Some(err);
                    }
                }
            }
        }
        return None;
    }

    // 1. OpenAI / ModelArts 格式: {"error": {"code": "...", "message": "..."}}
    if let Some(error_obj) = json.get("error") {
        let msg = error_obj.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误").to_string();
        let code = error_obj.get("code").map(|c| c.to_string()).unwrap_or_default();
        return Some((code, msg));
    }

    // 2. Anthropic 格式: {"type": "error", "error": {"type": "...", "message": "..."}}
    //    已被上面的 json.get("error") 覆盖

    // 3. 顶层 code+message 格式: {"code": 429, "message": "..."}
    if let (Some(code), Some(msg)) = (json.get("code"), json.get("message")) {
        if code.is_number() || code.is_string() {
            let code_str = code.to_string();
            let msg_str = msg.as_str().unwrap_or("未知错误").to_string();
            return Some((code_str, msg_str));
        }
    }

    // 4. NVIDIA 格式: {"status": 429, "title": "Too Many Requests"}
    if let (Some(status), Some(title)) = (json.get("status"), json.get("title")) {
        if status.is_number() {
            let code_str = status.to_string();
            let msg_str = title.as_str().unwrap_or("未知错误").to_string();
            return Some((code_str, msg_str));
        }
    }

    None
}

/// 检查响应体中是否包含错误（支持普通 JSON 和 SSE 格式）
/// 返回 Some((error_code, error_message)) 如果检测到错误
fn detect_response_error(body: &[u8]) -> Option<(String, String)> {
    let body_str = std::str::from_utf8(body).ok()?;

    // 尝试直接解析为 JSON（非流式响应）
    if let Ok(json) = serde_json::from_str::<Value>(body_str) {
        return check_json_error(&json);
    }

    // SSE 格式：逐行检查
    if body_str.contains("data: ") || body_str.contains("event:") {
        let mut is_error_event = false;
        for line in body_str.lines() {
            let line = line.trim();
            // Anthropic 格式: event: error
            if line == "event: error" {
                is_error_event = true;
                continue;
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(json) = serde_json::from_str::<Value>(json_str) {
                    // 如果前面有 event: error，直接解析这个 data 行
                    if is_error_event {
                        let msg = json.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("未知错误")
                            .to_string();
                        let code = json.get("error")
                            .and_then(|e| e.get("type"))
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "error".to_string());
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

/// 根据模型映射转换请求体中的模型名称
async fn map_model_name(
    body: &bytes::Bytes,
    endpoint: &EndpointState,
    pool: &Pool,
    state: &AppState,
) -> Result<bytes::Bytes, AppError> {
    // 解析请求体
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else {
        return Ok(body.clone());
    };
    
    // 获取客户端请求的模型名称
    let client_model = json.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    if client_model.is_empty() {
        return Ok(body.clone());
    }
    
    // 检查是否有缓存，如果没有则尝试获取
    if state.get_cached_models(&endpoint.config.id).is_none() {
        let _ = state.fetch_endpoint_models(&endpoint.config.id).await;
    }
    
    // 使用 state 的匹配函数
    let resolved_model = state.resolve_model_for_endpoint(pool, endpoint, &client_model);
    
    // 如果模型名称发生变化，替换请求体
    if resolved_model != client_model {
        // 检查是否是错误信息
        if let Some(error_msg) = resolved_model.strip_prefix("ERROR:") {
            return Err(AppError::BadRequest(error_msg.to_string()));
        }
        if let Some(obj) = json.as_object_mut() {
            obj.insert("model".to_string(), Value::String(resolved_model.clone()));
            debug!("模型映射: {} -> {}", client_model, resolved_model);
            // 重新序列化
            if let Ok(new_body) = serde_json::to_vec(&json) {
                return Ok(bytes::Bytes::from(new_body));
            }
        }
    }
    
    Ok(body.clone())
}

/// 处理API请求转发（基于对外API和池）
pub async fn forward_request(
    state: &AppState,
    req: &HttpRequest,
    body: bytes::Bytes,
    path: &str,
) -> Result<HttpResponse, AppError> {
    // 匹配对外API
    let exposed_api = state.match_exposed_api(path)
        .ok_or_else(|| AppError::NotFound(format!("未找到匹配的对外API: {}", path)))?;

    // 获取池信息
    let pool = state.get_pool(&exposed_api.pool_id)
        .ok_or_else(|| AppError::Internal(format!("池不存在: {}", exposed_api.pool_id)))?;

    let algorithm = pool.schedule_algorithm.clone();
    let retry_mode = pool.retry_mode.clone();
    let retry_count = pool.retry_count.max(1) as usize;

    // 计算最大重试次数
    let available_count = state.available_endpoint_ids_in_pool(&pool.id).len().max(1);
    let max_retries = match retry_mode {
        RetryMode::None => 1,
        RetryMode::Same => retry_count,
        RetryMode::Pool => retry_count * available_count,
    };

    let mut last_error = None;
    let mut tried_ids: Vec<String> = Vec::new();
    let mut first_endpoint_id: Option<String> = None;

    for attempt in 0..max_retries {
        let endpoint_id = if attempt == 0 || retry_mode == RetryMode::Same {
            // 首次或原地重试：使用调度算法选择端点
            if retry_mode == RetryMode::Same && first_endpoint_id.is_some() {
                first_endpoint_id.clone().unwrap()
            } else {
                let id = Scheduler::select_endpoint(state, &pool.id, &algorithm)
                    .ok_or_else(|| AppError::Proxy("池中没有可用的代理端点".to_string()))?;
                first_endpoint_id = Some(id.clone());
                id
            }
        } else {
            // 端点重试：排除已尝试的端点
            match Scheduler::select_next_for_retry(state, &pool.id, &tried_ids) {
                Some(id) => id,
                None => break, // 所有端点都已尝试
            }
        };

        if retry_mode != RetryMode::Same || attempt == 0 {
            tried_ids.push(endpoint_id.clone());
        }

        let endpoint = state
            .get_endpoint(&endpoint_id)
            .ok_or_else(|| AppError::Proxy(format!("端点不存在: {}", endpoint_id)))?;

        debug!(
            "尝试转发请求到端点 {} ({}) (尝试 {}/{})",
            endpoint.config.name, endpoint_id, attempt + 1, max_retries
        );

        // 计算实际路径（去掉对外API的前缀）
        let actual_path = path.strip_prefix(&exposed_api.prefix).unwrap_or(path);
        let target_path = actual_path.to_string();
        
        // 根据池的模型模式处理请求体
        let mapped_body = match map_model_name(&body, &endpoint, &pool, state).await {
            Ok(b) => b,
            Err(e) => {
                warn!("端点 {} 模型名称处理失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(e);
                if retry_mode == RetryMode::None {
                    break;
                }
                continue;
            }
        };

        match forward_to_endpoint(state, req, &mapped_body, &endpoint, &target_path, &exposed_api.api_type).await {
            Ok(response) => {
                return Ok(response);
            }
            Err(e) => {
                warn!("端点 {} 请求失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(e);

                // 无重试模式直接返回
                if retry_mode == RetryMode::None {
                    break;
                }
            }
        }
    }

    if let Some(e) = &last_error {
        warn!("端点池所有接口均不可用，最后错误: {}", e);
    }
    Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string()))
}

/// 处理流式响应转发
pub async fn forward_stream_request(
    state: web::Data<AppState>,
    req: &HttpRequest,
    body: bytes::Bytes,
    path: &str,
) -> Result<HttpResponse, AppError> {
    // 匹配对外API
    let exposed_api = state.match_exposed_api(path)
        .ok_or_else(|| AppError::NotFound(format!("未找到匹配的对外API: {}", path)))?;

    // 获取池信息
    let pool = state.get_pool(&exposed_api.pool_id)
        .ok_or_else(|| AppError::Internal(format!("池不存在: {}", exposed_api.pool_id)))?;

    let algorithm = pool.schedule_algorithm.clone();
    let retry_mode = pool.retry_mode.clone();
    let retry_count = pool.retry_count.max(1) as usize;

    // 计算最大重试次数
    let available_count = state.available_endpoint_ids_in_pool(&pool.id).len().max(1);
    let max_retries = match retry_mode {
        RetryMode::None => 1,
        RetryMode::Same => retry_count,
        RetryMode::Pool => retry_count * available_count,
    };

    let mut last_error = None;
    let mut tried_ids: Vec<String> = Vec::new();
    let mut first_endpoint_id: Option<String> = None;

    for attempt in 0..max_retries {
        let endpoint_id = if attempt == 0 || retry_mode == RetryMode::Same {
            if retry_mode == RetryMode::Same && first_endpoint_id.is_some() {
                first_endpoint_id.clone().unwrap()
            } else {
                let id = Scheduler::select_endpoint(state.get_ref(), &pool.id, &algorithm)
                    .ok_or_else(|| AppError::Proxy("池中没有可用的代理端点".to_string()))?;
                first_endpoint_id = Some(id.clone());
                id
            }
        } else {
            match Scheduler::select_next_for_retry(state.get_ref(), &pool.id, &tried_ids) {
                Some(id) => id,
                None => break,
            }
        };

        if retry_mode != RetryMode::Same || attempt == 0 {
            tried_ids.push(endpoint_id.clone());
        }

        let endpoint = state
            .get_endpoint(&endpoint_id)
            .ok_or_else(|| AppError::Proxy(format!("端点不存在: {}", endpoint_id)))?;

        // 计算实际路径
        let actual_path = path.strip_prefix(&exposed_api.prefix).unwrap_or(path);
        let target_path = actual_path.to_string();
        let target_url = build_target_url(&endpoint.config.url, &target_path, &exposed_api.api_type);
        
        // 根据池的模型模式处理请求体
        let mapped_body = match map_model_name(&body, &endpoint, &pool, state.get_ref()).await {
            Ok(b) => b,
            Err(e) => {
                warn!("端点 {} 模型名称处理失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(e);
                if retry_mode == RetryMode::None {
                    break;
                }
                continue;
            }
        };

        debug!("流式转发到: {} (尝试 {}/{})", target_url, attempt + 1, max_retries);

        let mut request_builder = state.http_client.request(
            reqwest::Method::from_bytes(req.method().as_str().as_bytes())
                .map_err(|e| AppError::Proxy(format!("无效的HTTP方法: {}", e)))?,
            &target_url,
        );

        // 复制请求头（跳过认证头，后面会使用端点的 API Key）
        for (key, value) in req.headers() {
            let key_str = key.as_str().to_lowercase();
            if key_str != "host" && key_str != "content-length" && key_str != "authorization" && key_str != "x-api-key" {
                if let Ok(v) = value.to_str() {
                    request_builder = request_builder.header(key.as_str(), v);
                }
            }
        }

        // 设置认证头
        match endpoint.config.api_type {
            ApiType::OpenAI | ApiType::OpenAIResponses => {
                request_builder = request_builder.header(
                    "Authorization",
                    format!("Bearer {}", endpoint.config.api_key),
                );
            }
            ApiType::Anthropic => {
                request_builder = request_builder.header("x-api-key", &endpoint.config.api_key);
                request_builder = request_builder.header("anthropic-version", "2023-06-01");
            }
        }

        request_builder = request_builder.body(mapped_body.to_vec());

        // 发送请求，捕获网络异常（超时、连接拒绝、DNS失败等）
        let response = match request_builder.send().await {
            Ok(resp) => resp,
            Err(e) => {
                let error_msg = if e.is_timeout() {
                    format!("连接超时: {}", e)
                } else if e.is_connect() {
                    format!("连接失败: {}", e)
                } else if e.is_request() {
                    format!("请求错误: {}", e)
                } else {
                    format!("网络异常: {}", e)
                };
                warn!("端点 {} 请求异常: {}", endpoint.config.name, error_msg);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(AppError::Proxy(error_msg));

                if retry_mode == RetryMode::None {
                    break;
                }
                continue;
            }
        };

        let resp_status = response.status();
        if resp_status != 200 {
            let error_body = response.text().await.unwrap_or_default();
            warn!("端点 {} 返回错误状态 {}: {}", endpoint.config.name, resp_status, error_body);
            state.increment_endpoint_errors(&endpoint_id);
            last_error = Some(AppError::Proxy(format!(
                "上游返回状态 {}: {}",
                resp_status, error_body
            )));

            if retry_mode == RetryMode::None {
                break;
            }
            continue;
        }

        // 流式响应：只读取第一个 chunk 检查错误，后续 chunk 直接转发
        let mut stream = response.bytes_stream();

        // 读取第一个 chunk
        let first_chunk = match stream.next().await {
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => {
                warn!("端点 {} 读取响应流失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(AppError::Proxy(format!("读取响应流失败: {}", e)));
                if retry_mode == RetryMode::None {
                    break;
                }
                continue;
            }
            None => {
                warn!("端点 {} 返回空响应", endpoint.config.name);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(AppError::Proxy("上游返回空响应".to_string()));
                if retry_mode == RetryMode::None {
                    break;
                }
                continue;
            }
        };

        // 检查第一个 chunk 中是否包含错误
        if let Some((error_code, error_msg)) = detect_response_error(&first_chunk) {
            warn!("端点 {} 响应中包含错误 [{}]: {}", endpoint.config.name, error_code, error_msg);
            state.increment_endpoint_errors(&endpoint_id);
            last_error = Some(AppError::Proxy(format!("上游错误 [{}]: {}", error_code, error_msg)));
            if retry_mode == RetryMode::None {
                break;
            }
            continue;
        }

        // 无错误，将第一个 chunk 和剩余 stream 合并后转发给客户端
        let ep_id = endpoint.config.id.clone();
        let ep_api_type = endpoint.config.api_type.clone();
        let state_clone = state.clone();

        let first_stream = futures_util::stream::once(async move { Ok::<_, reqwest::Error>(first_chunk) });
        let full_stream = first_stream.chain(stream);

        let body_stream = actix_web::HttpResponse::Ok()
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
            });

        return Ok(body_stream);
    }

    // 所有重试都失败
    if let Some(e) = &last_error {
        warn!("端点池所有接口均不可用，最后错误: {}", e);
    }
    Err(AppError::Proxy("端点池所有接口均不可用，请检查后重试。".to_string()))
}

/// 转发请求到指定端点
/// 根据 API 类型和 base_url 构建完整的目标 URL
fn build_target_url(base_url: &str, path: &str, api_type: &ApiType) -> String {
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
    
    // 如果 base_url 已经包含 /v1 等路径前缀，则直接使用
    // 否则根据 API 类型自动补全
    let full_base = if base.ends_with("/v1") || base.ends_with("/v1/") {
        base.to_string()
    } else {
        match api_type {
            ApiType::OpenAI | ApiType::OpenAIResponses => {
                format!("{}/v1", base)
            }
            ApiType::Anthropic => {
                format!("{}/v1", base)
            }
        }
    };
    
    format!("{}/{}", full_base, path)
}

async fn forward_to_endpoint(
    state: &AppState,
    req: &HttpRequest,
    body: &bytes::Bytes,
    endpoint: &EndpointState,
    path: &str,
    api_type: &ApiType,
) -> Result<HttpResponse, AppError> {
    let target_url = build_target_url(&endpoint.config.url, path, api_type);
    debug!("转发到: {}", target_url);

    let mut request_builder = state.http_client.request(
        reqwest::Method::from_bytes(req.method().as_str().as_bytes())
            .map_err(|e| AppError::Proxy(format!("无效的HTTP方法: {}", e)))?,
        &target_url,
    );

    // 复制请求头（跳过认证头，后面会使用端点的 API Key）
    for (key, value) in req.headers() {
        let key_str = key.as_str().to_lowercase();
        if key_str != "host" && key_str != "content-length" && key_str != "authorization" && key_str != "x-api-key" {
            if let Ok(v) = value.to_str() {
                request_builder = request_builder.header(key.as_str(), v);
            }
        }
    }

    // 设置认证头
    match endpoint.config.api_type {
        ApiType::OpenAI | ApiType::OpenAIResponses => {
            request_builder = request_builder.header(
                "Authorization",
                format!("Bearer {}", endpoint.config.api_key),
            );
        }
        ApiType::Anthropic => {
            request_builder = request_builder.header("x-api-key", &endpoint.config.api_key);
            request_builder = request_builder.header("anthropic-version", "2023-06-01");
        }
    }

    if req.headers().get("content-type").is_none() {
        request_builder = request_builder.header("Content-Type", "application/json");
    }

    request_builder = request_builder.body(body.to_vec());

    // 发送请求，捕获网络异常（超时、连接拒绝、DNS失败等）
    let response = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let error_msg = if e.is_timeout() {
                format!("连接超时: {}", e)
            } else if e.is_connect() {
                format!("连接失败: {}", e)
            } else if e.is_request() {
                format!("请求错误: {}", e)
            } else {
                format!("网络异常: {}", e)
            };
            error!("端点 {} 请求异常: {}", endpoint.config.name, error_msg);
            return Err(AppError::Proxy(error_msg));
        }
    };

    let status = response.status();
    let headers = response.headers().clone();

    if status != 200 {
        let error_body = response.text().await.unwrap_or_default();
        error!("端点 {} 返回错误状态 {}: {}", endpoint.config.name, status, error_body);
        return Err(AppError::Proxy(format!("上游返回状态 {}: {}", status, error_body)));
    }

    let response_body = response.bytes().await.map_err(|e| AppError::Proxy(format!("读取响应失败: {}", e)))?;

    // 检查响应体中是否包含错误（LLM API 可能返回 200 但 body 中有错误）
    if let Some((error_code, error_msg)) = detect_response_error(&response_body) {
        error!("端点 {} 响应中包含错误 [{}]: {}", endpoint.config.name, error_code, error_msg);
        return Err(AppError::Proxy(format!("上游错误 [{}]: {}", error_code, error_msg)));
    }

    // 解析token使用量
    let tokens_used = parse_token_usage(&response_body, &endpoint.config.api_type);
    if tokens_used > 0 {
        state.update_endpoint_tokens(&endpoint.config.id, tokens_used);
        debug!("端点 {} 消耗 {} tokens", endpoint.config.name, tokens_used);
    }

    let mut response_builder = HttpResponse::build(
        actix_web::http::StatusCode::from_u16(status.as_u16())
            .unwrap_or(actix_web::http::StatusCode::OK),
    );

    for (key, value) in &headers {
        if let Ok(v) = value.to_str() {
            response_builder.insert_header((key.as_str(), v));
        }
    }

    Ok(response_builder.body(response_body))
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
            input + output
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
