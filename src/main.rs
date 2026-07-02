mod admin;
mod auth;
mod config;
mod converter;
mod error;
mod models;
mod proxy;
mod scheduler;
mod state;
mod validator;

use actix_cors::Cors;
use actix_files as fs;
use actix_web::{web, App, HttpServer, HttpRequest, HttpResponse, middleware};
use actix_web::web::PayloadConfig;
use state::AppState;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

/// API代理入口 - 处理所有 /v1/ 和 /api/ 路径的请求
async fn api_proxy(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Bytes,
) -> Result<HttpResponse, error::AppError> {
    use crate::models::ApiCallLog;
    use chrono::Utc;

    let start = std::time::Instant::now();
    let conn_info = req.connection_info();
    let client_ip = conn_info.peer_addr().unwrap_or("unknown").to_string();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let full_path = format!("{}{}", path, query);

    // 提前获取命中的 API 前缀，用于日志记录
    let api_prefix = state.match_exposed_api(&path).map(|a| a.prefix);

    // API密钥认证
    let auth_result = auth::check_api_auth(&state, &req);

    let result = if let Err(e) = auth_result {
        Err(e)
    } else {
        // 检查是否是流式请求（通过解析 JSON 的 stream 字段，避免子串误判）
        let is_stream = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
            .unwrap_or(false);

        if is_stream {
            proxy::forward_stream_request(state.clone(), &req, body, &full_path).await
        } else {
            proxy::forward_request(state.get_ref(), &req, body, &full_path).await
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let (status_code, status, error_message) = match &result {
        Ok(resp) => (resp.status().as_u16(), "success".to_string(), None),
        Err(e) => (e.status_code(), "error".to_string(), Some(e.to_string())),
    };

    state.add_call_log(ApiCallLog {
        timestamp: Utc::now(),
        client_ip,
        method,
        path: full_path,
        api_prefix,
        endpoint_id: None,
        endpoint_name: None,
        status_code,
        status,
        error_message,
        duration_ms,
    });

    result
}

/// 健康检查
async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "service": "tokenhub"
    }))
}

/// 首页重定向到管理后台
async fn index_redirect() -> HttpResponse {
    HttpResponse::Found()
        .append_header(("Location", "/admin/"))
        .finish()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 初始化日志
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tokenhub=debug")),
        )
        .init();

    info!("正在启动 TokenHub...");

    // 加载配置
    let config_path = std::env::var("CONFIG_PATH").ok();
    let config_manager = config::ConfigManager::new(config_path.as_deref());
    let app_state = AppState::new(config_manager)
        .await
        .expect("初始化应用状态失败");

    let listen_addr = app_state.config.read().listen_addr.clone();
    let listen_port = app_state.config.read().listen_port;

    info!("监听地址: {}:{}", listen_addr, listen_port);

    // 启动自动重置任务（每日零点和每分钟检查请求次数）
    let reset_state = web::Data::new(app_state);
    let reset_clone = reset_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60)); // 每分钟检查一次
        loop {
            interval.tick().await;
            reset_clone.check_auto_reset().await;
        }
    });

    // 启动运行时状态持久化任务（每10秒保存一次）
    let save_state = reset_state.clone();
    tokio::spawn(async move {
        use std::sync::atomic::Ordering;
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            if save_state.dirty.load(Ordering::Acquire) {
                if let Err(e) = save_state.save_runtime_state().await {
                    tracing::warn!("保存运行时状态失败: {}", e);
                }
                save_state.dirty.store(false, Ordering::Release);
            }
        }
    });

    // 启动模型缓存更新任务（每小时更新一次）
    let cache_state = reset_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // 每小时
        loop {
            interval.tick().await;
            let endpoints: Vec<String> = {
                let ep_map = cache_state.endpoints.read();
                ep_map.keys().cloned().collect()
            };
            for endpoint_id in endpoints {
                if let Err(e) = cache_state.fetch_endpoint_models(&endpoint_id).await {
                    tracing::warn!("定时更新端点 {} 模型缓存失败: {}", endpoint_id, e);
                }
            }
        }
    });

    // 启动HTTP服务器
    let state_data = reset_state;

    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(cors)
            .wrap(middleware::Logger::default())
            .app_data(PayloadConfig::new(50 * 1024 * 1024)) // 50MB 请求体限制
            .app_data(state_data.clone())
            // 健康检查
            .route("/health", web::get().to(health_check))
            // 首页重定向
            .route("/", web::get().to(index_redirect))
            // 认证相关
            .route("/admin/api/login", web::post().to(auth::admin_login))
            .route("/admin/api/logout", web::post().to(auth::admin_logout))
            .route("/admin/api/auth/status", web::get().to(auth::check_auth_status))
            .route("/admin/api/password", web::post().to(auth::change_admin_password))
            // 管理后台API
            .route("/admin/api/endpoints", web::get().to(admin::list_endpoints))
            .route("/admin/api/endpoints", web::post().to(admin::create_endpoint))
            .route("/admin/api/endpoints/check", web::post().to(admin::check_endpoint))
            .route("/admin/api/endpoints/models", web::post().to(admin::list_models))
            .route("/admin/api/endpoints/{id}", web::get().to(admin::get_endpoint))
            .route("/admin/api/endpoints/{id}", web::put().to(admin::update_endpoint))
            .route("/admin/api/endpoints/{id}", web::delete().to(admin::delete_endpoint))
            .route("/admin/api/endpoints/{id}/toggle", web::post().to(admin::toggle_endpoint))
            .route("/admin/api/endpoints/{id}/reset", web::post().to(admin::reset_endpoint))
            .route("/admin/api/endpoints/{id}/reset-requests", web::post().to(admin::reset_endpoint_requests))
            .route("/admin/api/endpoints/reset-all", web::post().to(admin::reset_all_endpoints))
            // 池管理
            .route("/admin/api/pools", web::get().to(admin::list_pools))
            .route("/admin/api/pools", web::post().to(admin::create_pool))
            .route("/admin/api/pools/{id}", web::put().to(admin::update_pool))
            .route("/admin/api/pools/{id}", web::delete().to(admin::delete_pool))
            .route("/admin/api/pools/{id}/models", web::get().to(admin::list_pool_models))
            .route("/admin/api/pools/{id}/test-all", web::post().to(admin::test_pool_endpoints))
            // 对外API管理
            .route("/admin/api/exposed-apis", web::get().to(admin::list_exposed_apis))
            .route("/admin/api/exposed-apis", web::post().to(admin::create_exposed_api))
            .route("/admin/api/exposed-apis/{id}", web::get().to(admin::get_exposed_api))
            .route("/admin/api/exposed-apis/{id}", web::put().to(admin::update_exposed_api))
            .route("/admin/api/exposed-apis/{id}", web::delete().to(admin::delete_exposed_api))
            .route("/admin/api/exposed-apis/{id}/toggle", web::post().to(admin::toggle_exposed_api))
            // 配置
            .route("/admin/api/config", web::get().to(admin::get_config))
            .route("/admin/api/config", web::put().to(admin::update_config))
            .route("/admin/api/stats", web::get().to(admin::get_stats))
            .route("/admin/api/logs", web::get().to(admin::list_call_logs))
            // 静态文件（管理后台前端）
            .service(fs::Files::new("/admin", "static").index_file("index.html"))
            // API代理（必须放在最后，捕获所有其他路径）
            .default_service(web::route().to(api_proxy))
    });

    info!("HTTP服务启动，如需HTTPS请使用nginx反向代理");
    server.bind(format!("{}:{}", listen_addr, listen_port))?.run().await
}
