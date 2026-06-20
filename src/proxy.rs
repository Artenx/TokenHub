use crate::error::AppError;
use crate::models::*;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use actix_web::{web, HttpRequest, HttpResponse};
use futures_util::StreamExt;
use serde_json::Value;
use tracing::{debug, error, warn};

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
        if resolved_model.starts_with("ERROR:") {
            let error_msg = &resolved_model[6..];
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

    // 计算最大重试次数
    let available_count = state.available_endpoint_ids_in_pool(&pool.id).len();
    let max_retries = match algorithm {
        ScheduleAlgorithm::Random => available_count,
        _ => 1,
    };

    let mut last_error = None;
    let mut tried_ids: Vec<String> = Vec::new();

    for attempt in 0..max_retries {
        let endpoint_id = if attempt == 0 {
            Scheduler::select_endpoint(state, &pool.id, &algorithm)
                .ok_or_else(|| AppError::Proxy("池中没有可用的代理端点".to_string()))?
        } else {
            let last_id = tried_ids.last().unwrap();
            Scheduler::select_next_for_retry(state, &pool.id, last_id)
                .ok_or_else(|| AppError::Proxy("所有代理端点均不可用".to_string()))?
        };

        tried_ids.push(endpoint_id.clone());

        let endpoint = state
            .get_endpoint(&endpoint_id)
            .ok_or_else(|| AppError::Proxy(format!("端点不存在: {}", endpoint_id)))?;

        debug!(
            "尝试转发请求到端点 {} ({}) (尝试 {}/{})",
            endpoint.config.name, endpoint_id, attempt + 1, max_retries
        );

        // 计算实际路径（去掉对外API的前缀）
        let actual_path = path.strip_prefix(&exposed_api.prefix).unwrap_or(path);
        // build_target_url 会自动补全 /v1，这里不再重复添加
        let target_path = actual_path.to_string();
        
        // 根据池的模型模式处理请求体
        let mapped_body = map_model_name(&body, &endpoint, &pool, state).await?;

        match forward_to_endpoint(state, req, &mapped_body, &endpoint, &target_path, &exposed_api.api_type).await {
            Ok(response) => {
                return Ok(response);
            }
            Err(e) => {
                warn!("端点 {} 请求失败: {}", endpoint.config.name, e);
                state.increment_endpoint_errors(&endpoint_id);
                last_error = Some(e);

                if algorithm != ScheduleAlgorithm::Random {
                    break;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| AppError::Proxy("转发请求失败".to_string())))
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

    // 计算最大重试次数
    let available_count = state.available_endpoint_ids_in_pool(&pool.id).len();
    let max_retries = match algorithm {
        ScheduleAlgorithm::Random => available_count,
        _ => 1,
    };

    let mut last_error = None;
    let mut tried_ids: Vec<String> = Vec::new();

    for attempt in 0..max_retries {
        let endpoint_id = if attempt == 0 {
            Scheduler::select_endpoint(state.get_ref(), &pool.id, &algorithm)
                .ok_or_else(|| AppError::Proxy("池中没有可用的代理端点".to_string()))?
        } else {
            let last_id = tried_ids.last().unwrap();
            Scheduler::select_next_for_retry(state.get_ref(), &pool.id, last_id)
                .ok_or_else(|| AppError::Proxy("所有代理端点均不可用".to_string()))?
        };

        tried_ids.push(endpoint_id.clone());

        let endpoint = state
            .get_endpoint(&endpoint_id)
            .ok_or_else(|| AppError::Proxy(format!("端点不存在: {}", endpoint_id)))?;

        // 计算实际路径
        let actual_path = path.strip_prefix(&exposed_api.prefix).unwrap_or(path);
        // build_target_url 会自动补全 /v1，这里不再重复添加
        let target_path = actual_path.to_string();
        let target_url = build_target_url(&endpoint.config.url, &target_path, &exposed_api.api_type);
        
        // 根据池的模型模式处理请求体
        let mapped_body = map_model_name(&body, &endpoint, &pool, state.get_ref()).await?;

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

            if algorithm != ScheduleAlgorithm::Random {
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

        if algorithm != ScheduleAlgorithm::Random {
            break;
        }
        continue;
    }

    // 流式转发 - 同时收集token使用量
    let stream = response.bytes_stream();
    let ep_id = endpoint.config.id.clone();
    let ep_api_type = endpoint.config.api_type.clone();
    let state_clone = state.clone();

    let body_stream = actix_web::HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("Connection", "keep-alive"))
        .streaming({
            let mut buffer = String::new();
            stream.map(move |chunk| {
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
                                            tracing::debug!("流式响应 token 使用量: {}", tokens);
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
    Err(last_error.unwrap_or_else(|| AppError::Proxy("转发请求失败".to_string())))
}

/// 转发请求到指定端点
/// 根据 API 类型和 base_url 构建完整的目标 URL
fn build_target_url(base_url: &str, path: &str, api_type: &ApiType) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    
    // 如果路径已经包含 /v1 前缀，则直接拼接 base 和 path
    if path.starts_with("v1/") || path.starts_with("v1") {
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
