use crate::error::AppError;
use crate::state::AppState;
use actix_web::{web, HttpRequest, HttpResponse};
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

/// 管理员认证中间件 - 检查session中的登录状态
pub fn check_admin_auth(req: &HttpRequest) -> Result<(), AppError> {
    // 从cookie中检查登录状态
    let logged_in = req
        .cookie("admin_logged_in")
        .map(|c| c.value() == "true")
        .unwrap_or(false);

    if logged_in {
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

    // 从 Authorization 头获取密钥
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    // 支持 "Bearer sk-xxx" 格式
    let provided_key = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);

    if provided_key == expected_key {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// 管理后台登录
pub async fn admin_login(
    state: web::Data<AppState>,
    body: web::Json<LoginRequest>,
) -> Result<HttpResponse, AppError> {
    let admin_password = {
        let config = state.config.read();
        config.admin_password.clone()
    };

    if body.password == admin_password {
        let mut response = HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": "登录成功"
        }));

        // 设置登录cookie
        response.add_cookie(
            &actix_web::cookie::Cookie::build("admin_logged_in", "true")
                .path("/")
                .http_only(true)
                .secure(false) // 在生产环境中应设置为true
                .max_age(actix_web::cookie::time::Duration::hours(24))
                .finish(),
        ).map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(response)
    } else {
        Err(AppError::Unauthorized)
    }
}

/// 管理后台登出
pub async fn admin_logout() -> HttpResponse {
    let mut response = HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "已登出"
    }));

    response.add_cookie(
        &actix_web::cookie::Cookie::build("admin_logged_in", "")
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
    check_admin_auth(&req)?;

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

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "密码修改成功"
    })))
}

/// 检查登录状态
pub async fn check_auth_status(req: HttpRequest) -> HttpResponse {
    let logged_in = req
        .cookie("admin_logged_in")
        .map(|c| c.value() == "true")
        .unwrap_or(false);

    HttpResponse::Ok().json(serde_json::json!({
        "authenticated": logged_in
    }))
}
