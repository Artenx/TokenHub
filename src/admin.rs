use crate::auth::check_admin_auth;
use crate::error::AppError;
use crate::models::*;
use crate::state::AppState;
use crate::validator::InputValidator;
use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use serde_json::json;

/// 获取所有端点
pub async fn list_endpoints(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats))
}

/// 获取单个端点
pub async fn get_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let endpoint = state
        .get_endpoint(&id)
        .ok_or_else(|| AppError::NotFound(format!("端点不存在: {}", id)))?;
    Ok(HttpResponse::Ok().json(endpoint))
}

/// 创建端点
pub async fn create_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let data = body.into_inner();
    
    // 输入验证
    InputValidator::validate_name(&data.name)
        .map_err(AppError::BadRequest)?;
    InputValidator::validate_url(&data.url)
        .map_err(AppError::BadRequest)?;
    InputValidator::validate_api_key(&data.api_key)
        .map_err(AppError::BadRequest)?;
    InputValidator::validate_token_limit(data.token_limit)
        .map_err(AppError::BadRequest)?;
    InputValidator::validate_request_limit(data.request_limit)
        .map_err(AppError::BadRequest)?;
    InputValidator::validate_timeout(data.timeout.unwrap_or(300))
        .map_err(AppError::BadRequest)?;
    
    let endpoint = state
        .add_endpoint(data)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    
    // 异步更新模型缓存
    let state_clone = state.clone();
    let endpoint_id = endpoint.config.id.clone();
    tokio::spawn(async move {
        if let Err(e) = state_clone.fetch_endpoint_models(&endpoint_id).await {
            tracing::warn!("更新端点模型缓存失败: {}", e);
        }
    });
    
    Ok(HttpResponse::Created().json(endpoint))
}

/// 更新端点
pub async fn update_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let endpoint = state
        .update_endpoint(&id, body.into_inner())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    
    // 异步更新模型缓存
    let state_clone = state.clone();
    let endpoint_id = id.clone();
    tokio::spawn(async move {
        if let Err(e) = state_clone.fetch_endpoint_models(&endpoint_id).await {
            tracing::warn!("更新端点模型缓存失败: {}", e);
        }
    });
    
    Ok(HttpResponse::Ok().json(endpoint))
}

/// 删除端点
pub async fn delete_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    state
        .delete_endpoint(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "端点已删除"
    })))
}

/// 切换端点启用状态
pub async fn toggle_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let endpoint = state
        .toggle_endpoint(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(endpoint))
}

/// 重置端点token使用量
pub async fn reset_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    state
        .reset_endpoint_tokens(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "Token使用量已重置"
    })))
}

/// 重置端点请求次数
pub async fn reset_endpoint_requests(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    state
        .reset_endpoint_requests(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "请求次数已重置"
    })))
}

/// 重置所有端点token使用量
pub async fn reset_all_endpoints(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    state.reset_all_tokens();
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "所有端点Token使用量已重置"
    })))
}

/// 获取全局配置
pub async fn get_config(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let config = state.config.read();
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "listen_addr": config.listen_addr,
        "listen_port": config.listen_port,
        "admin_password_set": !config.admin_password.is_empty(),
    })))
}

/// 更新全局配置
pub async fn update_config(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<ConfigUpdateRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    if let Some(new_password) = &body.admin_password {
        state.change_admin_password(new_password).await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // 修改密码后使其他会话失效
        if let Some(cookie) = req.cookie("admin_session") {
            state.clear_other_admin_sessions(cookie.value());
        }
    }
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "配置已更新"
    })))
}

/// 获取统计信息
pub async fn get_stats(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats))
}

/// 获取端点支持的模型列表
pub async fn list_models(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;

    let ep = body.into_inner();
    let client = &state.http_client;

    let base_url = ep.url.trim_end_matches('/');

    // 构建候选 URL 列表：Custom 端点支持回退到 /v1/models
    let candidate_urls: Vec<String> = match ep.api_type {
        crate::models::ApiType::Custom => {
            let mut urls = vec![base_url.to_string()];
            if let Some(fallback) = crate::models::fallback_models_url(base_url) {
                if fallback != base_url {
                    urls.push(fallback);
                }
            }
            urls
        }
        _ => {
            vec![if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/models", base_url)
            } else {
                format!("{}/v1/models", base_url)
            }]
        }
    };

    // 依次尝试每个候选 URL，返回第一个成功的
    let mut last_tested = String::new();
    let mut last_status = 0u16;
    let mut last_message = String::new();

    for models_url in &candidate_urls {
        last_tested = models_url.clone();

        let mut request_builder = client.get(models_url)
            .header("Content-Type", "application/json");

        match ep.api_type {
            crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses | crate::models::ApiType::Custom => {
                request_builder = request_builder.header("Authorization", format!("Bearer {}", ep.api_key));
            }
            crate::models::ApiType::Anthropic => {
                request_builder = request_builder.header("x-api-key", &ep.api_key);
                request_builder = request_builder.header("anthropic-version", "2023-06-01");
            }
        }

        match request_builder.timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(response) => {
                last_status = response.status().as_u16();
                let response_text = response.text().await.unwrap_or_default();

                if last_status >= 200 && last_status < 300 {
                    let models = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
                        if let Some(data) = json["data"].as_array() {
                            data.iter()
                                .filter_map(|m| {
                                    let id = m["id"].as_str()?;
                                    let owned_by = m["owned_by"].as_str().unwrap_or("unknown");
                                    Some(serde_json::json!({
                                        "id": id,
                                        "owned_by": owned_by
                                    }))
                                })
                                .collect::<Vec<_>>()
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    };

                    return Ok(HttpResponse::Ok().json(serde_json::json!({
                        "success": true,
                        "models": models,
                        "status": last_status,
                        "tested_url": models_url
                    })));
                } else {
                    last_message = format!("获取模型列表失败 (HTTP {}): {}", last_status, &response_text.chars().take(200).collect::<String>());
                    continue;
                }
            }
            Err(e) => {
                last_message = format!("连接失败: {}", e);
                continue;
            }
        }
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": false,
        "message": last_message,
        "status": last_status,
        "tested_url": last_tested
    })))
}

/// 对话测试
pub async fn check_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;

    let ep = body.into_inner();
    let client = &state.http_client;

    let base_url = ep.url.trim_end_matches('/');

    // 获取模型名称，优先使用前端传入的，否则使用默认值
    let model_name = ep.model.unwrap_or_else(|| {
        match ep.api_type {
            crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses => "gpt-3.5-turbo".to_string(),
            crate::models::ApiType::Anthropic => "claude-3-haiku-20240307".to_string(),
            crate::models::ApiType::Custom => "default".to_string(),
        }
    });

    // 根据接口类型构建测试 URL、请求体和认证头
    let (chat_url, chat_body, request_builder) = match ep.api_type {
        crate::models::ApiType::Custom => {
            let url = base_url.to_string();
            let body = serde_json::json!({
                "model": model_name,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 10
            });
            let builder = client.post(&url)
                .header("Authorization", format!("Bearer {}", ep.api_key))
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
        crate::models::ApiType::OpenAI => {
            let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/chat/completions", base_url)
            } else {
                format!("{}/v1/chat/completions", base_url)
            };
            let body = serde_json::json!({
                "model": model_name,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 10
            });
            let builder = client.post(&url)
                .header("Authorization", format!("Bearer {}", ep.api_key))
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
        crate::models::ApiType::OpenAIResponses => {
            let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/responses", base_url)
            } else {
                format!("{}/v1/responses", base_url)
            };
            let body = serde_json::json!({
                "model": model_name,
                "input": "hi",
                "max_output_tokens": 10
            });
            let builder = client.post(&url)
                .header("Authorization", format!("Bearer {}", ep.api_key))
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
        crate::models::ApiType::Anthropic => {
            let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/messages", base_url)
            } else {
                format!("{}/v1/messages", base_url)
            };
            let body = serde_json::json!({
                "model": model_name,
                "max_tokens": 10,
                "messages": [{"role": "user", "content": "hi"}]
            });
            let builder = client.post(&url)
                .header("x-api-key", &ep.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
    };

    // 发送测试请求，设置10秒超时
    let start = std::time::Instant::now();
    let result = request_builder
        .timeout(std::time::Duration::from_secs(10))
        .body(chat_body.to_string())
        .send()
        .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    let (status_code, status_str, error_msg, response_json) = match result {
        Ok(response) => {
            let status = response.status();
            let response_text = response.text().await.unwrap_or_default();
            let sc = status.as_u16();
            
            if status.is_success() {
                let reply = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
                    json["choices"][0]["message"]["content"].as_str()
                        .or_else(|| {
                            json["content"][0]["text"].as_str()
                        })
                        .unwrap_or("无回复")
                        .to_string()
                } else {
                    "响应解析失败".to_string()
                };
                (sc, "success".to_string(), None, serde_json::json!({
                    "success": true,
                    "message": reply,
                    "status": sc,
                    "tested_url": chat_url
                }))
            } else {
                (sc, "error".to_string(), Some(format!("HTTP {}", status)), serde_json::json!({
                    "success": false,
                    "message": format!("请求失败 (HTTP {}): {}", status, &response_text.chars().take(200).collect::<String>()),
                    "status": sc,
                    "tested_url": chat_url
                }))
            }
        }
        Err(e) => {
            (0, "error".to_string(), Some(e.to_string()), serde_json::json!({
                "success": false,
                "message": format!("连接失败: {}", e),
                "status": 0,
                "tested_url": chat_url
            }))
        }
    };

    state.add_call_log(crate::models::ApiCallLog {
        timestamp: chrono::Utc::now(),
        client_ip: "admin".to_string(),
        method: "POST".to_string(),
        path: chat_url.clone(),
        api_prefix: Some(format!("[对话测试] {}", ep.name)),
        endpoint_id: None,
        endpoint_name: Some(ep.name.clone()),
        status_code,
        status: status_str,
        error_message: error_msg,
        duration_ms,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
    });

    Ok(HttpResponse::Ok().json(response_json))
}

/// 端点卡片对话测试 - 根据端点ID测试
pub async fn test_endpoint_by_id(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let endpoint_id = path.into_inner();
    let model = body.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());

    let ep_state = state.get_endpoint(&endpoint_id)
        .ok_or_else(|| AppError::NotFound("端点不存在".to_string()))?;
    let ep_cfg = &ep_state.config;
    let client = &state.http_client;

    let base_url = ep_cfg.url.trim_end_matches('/');

    let model_name = model.unwrap_or_else(|| {
        match ep_cfg.api_type {
            crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses => "gpt-3.5-turbo".to_string(),
            crate::models::ApiType::Anthropic => "claude-3-haiku-20240307".to_string(),
            crate::models::ApiType::Custom => "default".to_string(),
        }
    });

    let (chat_url, chat_body, request_builder) = match ep_cfg.api_type {
        crate::models::ApiType::Custom => {
            let url = base_url.to_string();
            let body = serde_json::json!({
                "model": model_name,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 10
            });
            let builder = client.post(&url)
                .header("Authorization", format!("Bearer {}", ep_cfg.api_key))
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
        crate::models::ApiType::OpenAI => {
            let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/chat/completions", base_url)
            } else {
                format!("{}/v1/chat/completions", base_url)
            };
            let body = serde_json::json!({
                "model": model_name,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 10
            });
            let builder = client.post(&url)
                .header("Authorization", format!("Bearer {}", ep_cfg.api_key))
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
        crate::models::ApiType::OpenAIResponses => {
            let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/responses", base_url)
            } else {
                format!("{}/v1/responses", base_url)
            };
            let body = serde_json::json!({
                "model": model_name,
                "input": "hi",
                "max_output_tokens": 10
            });
            let builder = client.post(&url)
                .header("Authorization", format!("Bearer {}", ep_cfg.api_key))
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
        crate::models::ApiType::Anthropic => {
            let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                format!("{}/messages", base_url)
            } else {
                format!("{}/v1/messages", base_url)
            };
            let body = serde_json::json!({
                "model": model_name,
                "max_tokens": 10,
                "messages": [{"role": "user", "content": "hi"}]
            });
            let builder = client.post(&url)
                .header("x-api-key", &ep_cfg.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json");
            (url, body, builder)
        }
    };

    let start = std::time::Instant::now();
    let result = request_builder
        .timeout(std::time::Duration::from_secs(10))
        .body(chat_body.to_string())
        .send()
        .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    let (status_code, status_str, error_msg, response_json) = match result {
        Ok(response) => {
            let status = response.status();
            let response_text = response.text().await.unwrap_or_default();
            let sc = status.as_u16();

            if status.is_success() {
                let reply = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
                    json["choices"][0]["message"]["content"].as_str()
                        .or_else(|| json["content"][0]["text"].as_str())
                        .unwrap_or("无回复")
                        .to_string()
                } else {
                    "响应解析失败".to_string()
                };
                (sc, "success".to_string(), None, serde_json::json!({
                    "success": true,
                    "message": reply,
                    "status": sc,
                    "tested_url": chat_url
                }))
            } else {
                (sc, "error".to_string(), Some(format!("HTTP {}", status)), serde_json::json!({
                    "success": false,
                    "message": format!("请求失败 (HTTP {}): {}", status, &response_text.chars().take(200).collect::<String>()),
                    "status": sc,
                    "tested_url": chat_url
                }))
            }
        }
        Err(e) => {
            (0, "error".to_string(), Some(e.to_string()), serde_json::json!({
                "success": false,
                "message": format!("连接失败: {}", e),
                "status": 0,
                "tested_url": chat_url
            }))
        }
    };

    state.add_call_log(crate::models::ApiCallLog {
        timestamp: chrono::Utc::now(),
        client_ip: "admin".to_string(),
        method: "POST".to_string(),
        path: chat_url.clone(),
        api_prefix: Some(format!("[对话测试] {}", ep_cfg.name)),
        endpoint_id: Some(endpoint_id),
        endpoint_name: Some(ep_cfg.name.clone()),
        status_code,
        status: status_str,
        error_message: error_msg,
        duration_ms,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
    });

    Ok(HttpResponse::Ok().json(response_json))
}

// ========== 池管理 ==========

/// 获取单个对外API
pub async fn get_exposed_api(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let api = state
        .get_exposed_api(&id)
        .ok_or_else(|| AppError::NotFound(format!("对外API不存在: {}", id)))?;
    Ok(HttpResponse::Ok().json(api))
}

/// 获取所有池
pub async fn list_pools(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats.pools))
}

/// 创建池
pub async fn create_pool(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<PoolRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let pool = state.add_pool(body.into_inner()).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Created().json(pool))
}

/// 更新池
pub async fn update_pool(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<PoolRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let pool = state.update_pool(&id, body.into_inner()).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(pool))
}

/// 删除池
pub async fn delete_pool(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    state.delete_pool(&id).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "池已删除"
    })))
}

/// 获取池内所有端点的模型列表（去重合并）
pub async fn list_pool_models(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let pool_id = path.into_inner();
    let endpoint_ids = state.endpoint_ids_in_pool(&pool_id);

    let mut all_models: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for ep_id in &endpoint_ids {
        let cached = state.get_cached_models(ep_id);
        let models = if let Some(c) = cached {
            c
        } else {
            state.fetch_endpoint_models(ep_id).await.unwrap_or_default()
        };

        for m in models {
            if seen.insert(m.clone()) {
                all_models.push(m);
            }
        }
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "models": all_models,
        "endpoint_count": endpoint_ids.len()
    })))
}

/// 池一键测试：对池内所有端点进行对话测试
pub async fn test_pool_endpoints(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<PoolTestRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let pool_id = path.into_inner();
    let body = body.into_inner();

    let endpoint_ids = state.endpoint_ids_in_pool(&pool_id);
    let client = &state.http_client;
    let pool = state.get_pool(&pool_id);
    let pool_name = pool.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "未命名池".to_string());

    let mut results: Vec<EndpointTestResult> = Vec::new();

    for ep_id in &endpoint_ids {
        let start = std::time::Instant::now();
        let ep_state = match state.get_endpoint(ep_id) {
            Some(ep) => ep,
            None => continue,
        };
        let ep_cfg = &ep_state.config;

        // 确保模型缓存已填充
        let cached_models = state.get_cached_models(ep_id);
        if cached_models.is_none() {
            let _ = state.fetch_endpoint_models(ep_id).await;
        }

        // 确定测试用的模型名称
        let model_name = if body.mode == "manual" && body.model.is_some() {
            let client_model = body.model.as_ref().unwrap();
            if let Some(ref p) = pool {
                state.resolve_model_for_endpoint(p, &ep_state, client_model)
            } else {
                client_model.clone()
            }
        } else {
            let models = state.get_cached_models(ep_id).unwrap_or_default();
            models.first().cloned().unwrap_or_else(|| {
                match ep_cfg.api_type {
                    crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses => "gpt-3.5-turbo".to_string(),
                    crate::models::ApiType::Anthropic => "claude-3-haiku-20240307".to_string(),
                    crate::models::ApiType::Custom => "default".to_string(),
                }
            })
        };

        // 构建测试请求
        let base_url = ep_cfg.url.trim_end_matches('/');
        let (chat_url, chat_body, request_builder) = match ep_cfg.api_type {
            crate::models::ApiType::Custom => {
                let url = base_url.to_string();
                let body = serde_json::json!({
                    "model": model_name,
                    "messages": [{"role": "user", "content": "hi"}],
                    "max_tokens": 10
                });
                let builder = client.post(&url)
                    .header("Authorization", format!("Bearer {}", ep_cfg.api_key))
                    .header("Content-Type", "application/json");
                (url, body, builder)
            }
            crate::models::ApiType::OpenAI => {
                let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                    format!("{}/chat/completions", base_url)
                } else {
                    format!("{}/v1/chat/completions", base_url)
                };
                let body = serde_json::json!({
                    "model": model_name,
                    "messages": [{"role": "user", "content": "hi"}],
                    "max_tokens": 10
                });
                let builder = client.post(&url)
                    .header("Authorization", format!("Bearer {}", ep_cfg.api_key))
                    .header("Content-Type", "application/json");
                (url, body, builder)
            }
            crate::models::ApiType::OpenAIResponses => {
                let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                    format!("{}/responses", base_url)
                } else {
                    format!("{}/v1/responses", base_url)
                };
                let body = serde_json::json!({
                    "model": model_name,
                    "input": "hi",
                    "max_output_tokens": 10
                });
                let builder = client.post(&url)
                    .header("Authorization", format!("Bearer {}", ep_cfg.api_key))
                    .header("Content-Type", "application/json");
                (url, body, builder)
            }
            crate::models::ApiType::Anthropic => {
                let url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
                    format!("{}/messages", base_url)
                } else {
                    format!("{}/v1/messages", base_url)
                };
                let body = serde_json::json!({
                    "model": model_name,
                    "max_tokens": 10,
                    "messages": [{"role": "user", "content": "hi"}]
                });
                let builder = client.post(&url)
                    .header("x-api-key", &ep_cfg.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("Content-Type", "application/json");
                (url, body, builder)
            }
        };

        // 发送测试请求并收集结果
        let result: EndpointTestResult = match request_builder
            .timeout(std::time::Duration::from_secs(10))
            .body(chat_body.to_string())
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let response_text = response.text().await.unwrap_or_default();

                if status.is_success() {
                    let reply = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
                        json["choices"][0]["message"]["content"].as_str()
                            .or_else(|| {
                                json["content"][0]["text"].as_str()
                            })
                            .unwrap_or("无回复")
                            .to_string()
                    } else {
                        "响应解析失败".to_string()
                    };

                    EndpointTestResult {
                        endpoint_id: ep_id.clone(),
                        endpoint_name: ep_cfg.name.clone(),
                        success: true,
                        message: reply,
                        model_used: model_name.clone(),
                        status: status.as_u16(),
                        tested_url: chat_url.clone(),
                    }
                } else {
                    EndpointTestResult {
                        endpoint_id: ep_id.clone(),
                        endpoint_name: ep_cfg.name.clone(),
                        success: false,
                        message: format!("请求失败 (HTTP {}): {}", status, &response_text.chars().take(200).collect::<String>()),
                        model_used: model_name.clone(),
                        status: status.as_u16(),
                        tested_url: chat_url.clone(),
                    }
                }
            }
            Err(e) => {
                EndpointTestResult {
                    endpoint_id: ep_id.clone(),
                    endpoint_name: ep_cfg.name.clone(),
                    success: false,
                    message: format!("连接失败: {}", e),
                    model_used: model_name.clone(),
                    status: 0,
                    tested_url: chat_url.clone(),
                }
            }
        };

        // 记录调用日志
        let log_status = if result.success { "success" } else { "error" };
        let error_msg = if result.success { None } else { Some(result.message.clone()) };
        state.add_call_log(crate::models::ApiCallLog {
            timestamp: chrono::Utc::now(),
            client_ip: "admin".to_string(),
            method: "POST".to_string(),
            path: result.tested_url.clone(),
            api_prefix: Some(format!("[一键测试] {}", pool_name)),
            endpoint_id: Some(result.endpoint_id.clone()),
            endpoint_name: Some(result.endpoint_name.clone()),
            status_code: result.status,
            status: log_status.to_string(),
            error_message: error_msg,
            duration_ms: start.elapsed().as_millis() as u64,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
        });

        results.push(result);
    }

    let summary = PoolTestSummary {
        total: results.len(),
        success: results.iter().filter(|r| r.success).count(),
        failed: results.iter().filter(|r| !r.success).count(),
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "results": results,
        "summary": summary
    })))
}

// ========== 对外API管理 ==========

/// 获取所有对外API
pub async fn list_exposed_apis(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats.exposed_apis))
}

/// 创建对外API
pub async fn create_exposed_api(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<ExposedApiRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let api = state.add_exposed_api(body.into_inner()).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Created().json(api))
}

/// 更新对外API
pub async fn update_exposed_api(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<ExposedApiRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let api = state.update_exposed_api(&id, body.into_inner()).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(api))
}

/// 删除对外API
pub async fn delete_exposed_api(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    state.delete_exposed_api(&id).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "对外API已删除"
    })))
}

/// 切换对外API启用状态
pub async fn toggle_exposed_api(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let api = state.toggle_exposed_api(&id).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(api))
}

/// 切换对外 API 的数据回放状态
pub async fn toggle_exposed_api_replay(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let api = state.toggle_exposed_api_replay(&path.into_inner()).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(api))
}

/// 获取指定对外 API 的回放记录
pub async fn list_replay_records(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    if state.get_exposed_api(&id).is_none() {
        return Err(AppError::NotFound("对外API不存在".to_string()));
    }
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "records": state.get_replay_records(&id),
    })))
}

/// 清空指定对外 API 的回放记录
pub async fn clear_replay_records(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    if state.get_exposed_api(&id).is_none() {
        return Err(AppError::NotFound("对外API不存在".to_string()));
    }
    state.clear_replay_records(&id);
    Ok(HttpResponse::Ok().json(serde_json::json!({ "success": true })))
}

/// 获取回放全局配置
pub async fn get_replay_config(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    Ok(HttpResponse::Ok().json(state.replay_config.read().clone()))
}

/// 更新回放全局配置
pub async fn update_replay_config(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<ReplayConfig>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let config = state.update_replay_config(body.into_inner()).await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(HttpResponse::Ok().json(config))
}

// ========== 调用日志管理 ==========

/// 获取最近 50 条 API 调用日志
pub async fn list_call_logs(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let logs = state.get_call_logs();
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "logs": logs
    })))
}

/// 获取端点延迟排行榜
pub async fn list_latency_leaderboard(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let stats = state.get_latency_stats();
    let mut sorted = stats;
    sorted.sort_by(|a, b| {
        // 无样本的端点排最后
        match (a.samples, b.samples) {
            (0, 0) => a.endpoint_name.cmp(&b.endpoint_name),
            (0, _) => std::cmp::Ordering::Greater,
            (_, 0) => std::cmp::Ordering::Less,
            (_, _) => a.avg_ms.cmp(&b.avg_ms),
        }
    });
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "leaderboard": sorted
    })))
}

/// 创建并异步执行模型评测任务
pub async fn create_model_benchmark(
    state: web::Data<AppState>, req: HttpRequest, body: web::Json<CreateModelBenchmarkRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let input = body.into_inner();
    if input.model.trim().is_empty() || input.endpoint_ids.len() < 2 || input.cases.is_empty() {
        return Err(AppError::BadRequest("需要选择模型、至少两个端点和一条样本".to_string()));
    }
    let mut snapshots = Vec::new();
    for endpoint_id in &input.endpoint_ids {
        let endpoint = state.get_endpoint(endpoint_id).ok_or_else(|| AppError::BadRequest(format!("端点不存在: {}", endpoint_id)))?;
        snapshots.push(endpoint.config);
    }
    if state.get_endpoint(&input.judge.endpoint_id).is_none() {
        return Err(AppError::BadRequest("评审端点不存在".to_string()));
    }
    let run = ModelBenchmarkRun { id: uuid::Uuid::new_v4().to_string(), status: ModelBenchmarkStatus::Queued, created_at: Utc::now(), completed_at: None, model: input.model, endpoint_ids: input.endpoint_ids, endpoint_snapshots: snapshots, cases: input.cases, judge: input.judge, attempts_per_case: 3, attempts: Vec::new(), judge_results: Vec::new() };
    state.add_model_benchmark(run.clone());
    let task_state = state.clone();
    let task_id = run.id.clone();
    tokio::spawn(async move { crate::benchmark::execute_benchmark_run(task_state.get_ref(), &task_id).await; });
    Ok(HttpResponse::Created().json(run))
}

pub async fn list_model_benchmarks(state: web::Data<AppState>, req: HttpRequest) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    Ok(HttpResponse::Ok().json(state.get_model_benchmarks()))
}

pub async fn get_model_benchmark(state: web::Data<AppState>, req: HttpRequest, path: web::Path<String>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let run = state.get_model_benchmark(&id).ok_or_else(|| AppError::NotFound("评测任务不存在".to_string()))?;
    Ok(HttpResponse::Ok().json(json!({"run": run, "summaries": state.model_benchmark_summaries(&id)})))
}

pub async fn cancel_model_benchmark(state: web::Data<AppState>, req: HttpRequest, path: web::Path<String>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let run = state.cancel_model_benchmark(&path.into_inner()).ok_or_else(|| AppError::NotFound("评测任务不存在".to_string()))?;
    Ok(HttpResponse::Ok().json(run))
}

pub async fn list_model_benchmark_candidates(state: web::Data<AppState>, req: HttpRequest, query: web::Query<std::collections::HashMap<String, String>>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let model = query.get("model").cloned().unwrap_or_default();
    let endpoints: Vec<_> = state.endpoints.read().values().map(|endpoint| {
        (endpoint.config.id.clone(), endpoint.config.name.clone(), endpoint.config.enabled)
    }).collect();
    let mut candidates = Vec::with_capacity(endpoints.len());

    for (id, name, enabled) in endpoints {
        let mut models = match state.get_cached_models(&id) {
            Some(models) => models,
            None => state.fetch_endpoint_models(&id).await.unwrap_or_default(),
        };
        models.sort();
        models.dedup();
        let supports_model = model.is_empty() || models.iter().any(|candidate| candidate == &model);
        candidates.push(json!({"id": id, "name": name, "enabled": enabled, "models": models, "supports_model": supports_model}));
    }
    Ok(HttpResponse::Ok().json(candidates))
}

#[cfg(test)]
mod benchmark_tests {
    use super::*;
    use actix_web::cookie::Cookie;
    use actix_web::test::TestRequest;
    use tempfile::TempDir;

    async fn state() -> web::Data<AppState> {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        let manager = crate::config::ConfigManager::new(Some(path.to_str().unwrap()));
        manager.save(&AppConfig::default()).await.unwrap();
        // Keep the temporary directory for the test process by leaking it.
        std::mem::forget(tmp);
        web::Data::new(AppState::new(manager).await.unwrap())
    }

    fn authenticated_request(state: &AppState) -> HttpRequest {
        let token = state.create_admin_session();
        TestRequest::default().cookie(Cookie::new("admin_session", token)).to_http_request()
    }

    #[actix_rt::test]
    async fn model_benchmark_list_requires_admin_session() {
        let state = state().await;
        assert!(list_model_benchmarks(state, TestRequest::default().to_http_request()).await.is_err());
    }

    #[actix_rt::test]
    async fn model_benchmark_create_validates_required_input() {
        let state = state().await;
        let req = authenticated_request(state.get_ref());
        let input = CreateModelBenchmarkRequest { model: String::new(), endpoint_ids: Vec::new(), cases: Vec::new(), judge: BenchmarkJudgeConfig { endpoint_id: String::new(), model: String::new(), rubric: String::new() } };
        assert!(create_model_benchmark(state, req, web::Json(input)).await.is_err());
    }
}
