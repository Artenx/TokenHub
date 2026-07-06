use crate::error::AppError;
use crate::state::AppState;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web::cookie::SameSite;
use serde::Deserialize;

/// 登录请求
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

/// 修改密码请求
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// 管理员认证中间件 - 检查 session 中的登录状态
pub fn check_admin_auth(req: &HttpRequest, state: &AppState) -> Result<(), AppError> {
    // 从 cookie 中获取会话令牌并校验有效性
    let token = req
        .cookie("admin_session")
        .map(|c| c.value().to_string())
        .unwrap_or_default();

    if !token.is_empty() && state.validate_admin_session(&token) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// API密钥认证（基于对外API配置）
pub fn check_api_auth(
    state: &AppState,
    req: &HttpRequest,
) -> Result<(), AppError> {
    let path = req.uri().path();
    
    // 匹配对外API
    let exposed_api = match state.match_exposed_api(path) {
        Some(api) => api,
        None => return Err(AppError::NotFound("未找到匹配的对外API".to_string())),
    };

    // 如果没有配置API密钥，不需要认证
    let expected_key = match &exposed_api.api_key {
        Some(key) if !key.is_empty() => key.clone(),
        _ => return Ok(()),
    };

    // 从 Authorization 头或 x-api-key 头获取密钥
    let provided_key = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            req.headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string())
        })
        .ok_or(AppError::Unauthorized)?;

    if provided_key == expected_key {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// 管理后台登录
pub async fn admin_login(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<LoginRequest>,
) -> Result<HttpResponse, AppError> {
    let admin_password = {
        let config = state.config.read();
        config.admin_password.clone()
    };

    if body.password == admin_password {
        // 创建服务端会话令牌，避免客户端伪造登录状态
        let session_token = state.create_admin_session();

        let mut response = HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": "登录成功"
        }));

        // 设置登录 cookie 为随机会话令牌
        let is_secure = req.uri().scheme_str() == Some("https");
        response.add_cookie(
            &actix_web::cookie::Cookie::build("admin_session", session_token)
                .path("/")
                .http_only(true)
                .secure(is_secure)
                .same_site(SameSite::Lax)
                .max_age(actix_web::cookie::time::Duration::hours(24))
                .finish(),
        ).map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(response)
    } else {
        Err(AppError::Unauthorized)
    }
}

/// 管理后台登出
pub async fn admin_logout(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> HttpResponse {
    // 销毁服务端会话
    if let Some(cookie) = req.cookie("admin_session") {
        state.destroy_admin_session(cookie.value());
    }

    let mut response = HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "已登出"
    }));

    response.add_cookie(
        &actix_web::cookie::Cookie::build("admin_session", "")
            .path("/")
            .max_age(actix_web::cookie::time::Duration::ZERO)
            .finish(),
    ).unwrap();

    response
}

/// 修改管理密码
pub async fn change_admin_password(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<ChangePasswordRequest>,
) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;

    let admin_password = {
        let config = state.config.read();
        config.admin_password.clone()
    };

    if body.old_password != admin_password {
        return Err(AppError::BadRequest("原密码错误".to_string()));
    }

    if body.new_password.len() < 6 {
        return Err(AppError::BadRequest("新密码长度不能少于6位".to_string()));
    }

    state.change_admin_password(&body.new_password).await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // 修改密码后，使其他会话失效，只保留当前会话
    if let Some(cookie) = req.cookie("admin_session") {
        state.clear_other_admin_sessions(cookie.value());
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "密码修改成功"
    })))
}

/// 检查登录状态
pub async fn check_auth_status(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> HttpResponse {
    let authenticated = req
        .cookie("admin_session")
        .map(|c| state.validate_admin_session(c.value()))
        .unwrap_or(false);

    HttpResponse::Ok().json(serde_json::json!({
        "authenticated": authenticated
    }))
}
