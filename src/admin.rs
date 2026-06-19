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
pub async fn check_endpoint(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EndpointRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req)?;

    let ep = body.into_inner();
    let client = &state.http_client;

    // 构建测试请求 URL - 尝试访问根路径或 models 端点
    let base_url = ep.url.trim_end_matches('/');

    // 根据接口类型设置认证头
    let mut request_builder = client.get(base_url);
    match ep.api_type {
        crate::models::ApiType::OpenAI | crate::models::ApiType::OpenAIResponses => {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        crate::models::ApiType::Anthropic => {
            request_builder = request_builder.header("x-api-key", &ep.api_key);
            request_builder = request_builder.header("anthropic-version", "2023-06-01");
        }
    }

    // 发送测试请求，设置5秒超时
    match request_builder.timeout(std::time::Duration::from_secs(5)).send().await {
        Ok(response) => {
            let status = response.status();
            // 任何非连接错误的响应都算成功（包括401、404等，说明服务可达）
            if status.as_u16() > 0 {
                Ok(HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": format!("端点可达 (HTTP {})", status),
                    "status": status.as_u16()
                })))
            } else {
                Ok(HttpResponse::Ok().json(serde_json::json!({
                    "success": false,
                    "message": "端点无响应",
                    "status": 0
                })))
            }
        }
        Err(e) => {
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "success": false,
                "message": format!("连接失败: {}", e),
                "status": 0
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
