use crate::auth::check_admin_auth;
use crate::error::AppError;
use crate::models::*;
use crate::state::AppState;
use crate::validator::InputValidator;
use actix_web::{web, HttpRequest, HttpResponse};

/// 获取所有端点
pub async fn list_endpoints(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats))
}

/// 获取单个端点
pub async fn get_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
    let data = body.into_inner();
    
    // 输入验证
    InputValidator::validate_name(&data.name)
        .map_err(|e| AppError::BadRequest(e))?;
    InputValidator::validate_url(&data.url)
        .map_err(|e| AppError::BadRequest(e))?;
    InputValidator::validate_api_key(&data.api_key)
        .map_err(|e| AppError::BadRequest(e))?;
    InputValidator::validate_token_limit(data.token_limit)
        .map_err(|e| AppError::BadRequest(e))?;
    InputValidator::validate_timeout(data.timeout.unwrap_or(300))
        .map_err(|e| AppError::BadRequest(e))?;
    
    let endpoint = state
        .add_endpoint(data)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Created().json(endpoint))
}

/// 更新端点
pub async fn update_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
    let id = path.into_inner();
    let endpoint = state
        .update_endpoint(&id, body.into_inner())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(endpoint))
}

/// 删除端点
pub async fn delete_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
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

/// 重置所有端点token使用量
pub async fn reset_all_endpoints(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
    if let Some(new_password) = &body.admin_password {
        state.change_admin_password(new_password).await
            .map_err(|e| AppError::Internal(e.to_string()))?;
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
    check_admin_auth(&req)?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats))
}

/// 检查端点是否可用
/// 浏览模型列表
pub async fn list_models(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;

    let ep = body.into_inner();
    let client = &state.http_client;

    let base_url = ep.url.trim_end_matches('/');

    // 根据接口类型构建认证头
    let models_url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
        format!("{}/models", base_url)
    } else {
        format!("{}/v1/models", base_url)
    };

    let mut request_builder = client.get(&models_url)
        .header("Content-Type", "application/json");

    match ep.api_type {
        crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses => {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        crate::models::ApiType::Anthropic => {
            request_builder = request_builder.header("x-api-key", &ep.api_key);
            request_builder = request_builder.header("anthropic-version", "2023-06-01");
        }
    }

    match request_builder.timeout(std::time::Duration::from_secs(10)).send().await {
        Ok(response) => {
            let status = response.status();
            let response_text = response.text().await.unwrap_or_default();
            
            if status.is_success() {
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
                
                Ok(HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "models": models,
                    "status": status.as_u16(),
                    "tested_url": models_url
                })))
            } else {
                Ok(HttpResponse::Ok().json(serde_json::json!({
                    "success": false,
                    "message": format!("获取模型列表失败 (HTTP {}): {}", status, &response_text[..response_text.len().min(200)]),
                    "status": status.as_u16(),
                    "tested_url": models_url
                })))
            }
        }
        Err(e) => {
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "success": false,
                "message": format!("连接失败: {}", e),
                "status": 0,
                "tested_url": models_url
            })))
        }
    }
}

/// 对话测试
pub async fn check_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;

    let ep = body.into_inner();
    let client = &state.http_client;

    let base_url = ep.url.trim_end_matches('/');

    // 获取模型名称，优先使用前端传入的，否则使用默认值
    let model_name = ep.model.unwrap_or_else(|| {
        match ep.api_type {
            crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses => "gpt-3.5-turbo".to_string(),
            crate::models::ApiType::Anthropic => "claude-3-haiku-20240307".to_string(),
        }
    });

    // 根据接口类型构建测试 URL、请求体和认证头
    let (chat_url, chat_body, mut request_builder) = match ep.api_type {
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
    match request_builder
        .timeout(std::time::Duration::from_secs(10))
        .body(chat_body.to_string())
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let response_text = response.text().await.unwrap_or_default();
            
            if status.is_success() {
                // 解析响应获取模型回复
                let reply = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
                    // OpenAI 格式
                    json["choices"][0]["message"]["content"].as_str()
                        .or_else(|| {
                            // Anthropic 格式
                            json["content"][0]["text"].as_str()
                        })
                        .unwrap_or("无回复")
                        .to_string()
                } else {
                    "响应解析失败".to_string()
                };
                
                Ok(HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": reply,
                    "status": status.as_u16(),
                    "tested_url": chat_url
                })))
            } else {
                Ok(HttpResponse::Ok().json(serde_json::json!({
                    "success": false,
                    "message": format!("请求失败 (HTTP {}): {}", status, &response_text[..response_text.len().min(200)]),
                    "status": status.as_u16(),
                    "tested_url": chat_url
                })))
            }
        }
        Err(e) => {
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "success": false,
                "message": format!("连接失败: {}", e),
                "status": 0,
                "tested_url": chat_url
            })))
        }
    }
}

// ========== 池管理 ==========

/// 获取所有池
pub async fn list_pools(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats.pools))
}

/// 创建池
pub async fn create_pool(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<PoolRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
    let id = path.into_inner();
    state.delete_pool(&id).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "池已删除"
    })))
}

// ========== 对外API管理 ==========

/// 获取所有对外API
pub async fn list_exposed_apis(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
    let stats = state.get_stats();
    Ok(HttpResponse::Ok().json(stats.exposed_apis))
}

/// 创建对外API
pub async fn create_exposed_api(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<ExposedApiRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
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
    check_admin_auth(&req)?;
    let id = path.into_inner();
    let api = state.toggle_exposed_api(&id).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(HttpResponse::Ok().json(api))
}
