use crate::auth::check_admin_auth;
use crate::error::AppError;
use crate::models::*;
use crate::skill_repository::{delete_skill_package, import_skill_package, list_skill_files, preview_zip_archive, repository_root, scan_local_skills};
use crate::skill_sources::{adapters_for_sources, search_sources};
use crate::state::AppState;
use crate::validator::InputValidator;
use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{redirect::Policy, Url};
use serde_json::json;
use std::io::Cursor;
use std::net::IpAddr;
use tokio::net::lookup_host;

struct GithubSkillLink {
    archive_url: Url,
    directory: String,
}

fn github_skill_link(url: &Url) -> Result<GithubSkillLink, AppError> {
    let parts: Vec<_> = url.path_segments().ok_or_else(|| AppError::BadRequest("GitHub 技能地址无效".to_string()))?.collect();
    let (owner, repository, directory, version) = match parts.as_slice() {
        [owner, repository] if !owner.is_empty() && !repository.is_empty() => {
            (*owner, *repository, String::new(), "HEAD")
        }
        [owner, repository, link_type, version, ..] if !owner.is_empty() && !repository.is_empty() && !version.is_empty() => {
            let directory_parts = match *link_type {
                "tree" => &parts[4..],
                "blob" if parts.last() == Some(&"SKILL.md") => &parts[4..parts.len().saturating_sub(1)],
                "blob" => return Err(AppError::BadRequest("GitHub 文件链接必须指向 SKILL.md".to_string())),
                _ => return Err(AppError::BadRequest("GitHub 技能链接必须使用 tree 或 blob 路径".to_string())),
            };
            (*owner, *repository, directory_parts.join("/"), *version)
        }
        _ => return Err(AppError::BadRequest("GitHub 技能链接必须包含仓库、分支或标签和技能目录".to_string())),
    };
    let archive_url = Url::parse(&format!("https://codeload.github.com/{owner}/{repository}/zip/{version}"))
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(GithubSkillLink { archive_url, directory })
}

fn isolate_github_skill_archive(archive: &[u8], skill_directory: &str) -> Result<Vec<u8>, AppError> {
    let mut source = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|error| AppError::BadRequest(format!("读取 GitHub 归档失败: {}", error)))?;
    let is_repository_root = skill_directory.is_empty();
    let root = if is_repository_root {
        "github-skill"
    } else {
        skill_directory.rsplit('/').next().filter(|name| !name.is_empty())
            .ok_or_else(|| AppError::BadRequest("GitHub 技能目录无效".to_string()))?
    };
    let prefix = format!("{}/", skill_directory.trim_matches('/'));
    let output = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(output);
    let mut has_skill_md = false;
    for index in 0..source.len() {
        let mut entry = source.by_index(index)
            .map_err(|error| AppError::BadRequest(format!("读取 GitHub 归档条目失败: {}", error)))?;
        if entry.is_dir() {
            continue;
        }
        let Some((_, repository_path)) = entry.name().split_once('/') else { continue; };
        let relative_path = if is_repository_root {
            repository_path.to_string()
        } else {
            let Some(relative_path) = repository_path.strip_prefix(&prefix).map(str::to_owned) else { continue; };
            relative_path
        };
        if relative_path.is_empty() {
            continue;
        }
        writer.start_file(format!("{}/{}", root, relative_path), zip::write::FileOptions::default())
            .map_err(|error| AppError::Internal(format!("创建技能归档失败: {}", error)))?;
        std::io::copy(&mut entry, &mut writer)
            .map_err(|error| AppError::BadRequest(format!("读取 GitHub 技能内容失败: {}", error)))?;
        has_skill_md |= relative_path == "SKILL.md";
    }
    if !has_skill_md {
        return Err(AppError::BadRequest("GitHub 归档中未找到目标技能目录的 SKILL.md".to_string()));
    }
    writer.finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| AppError::Internal(format!("完成技能归档失败: {}", error)))
}

#[derive(serde::Deserialize)]
pub struct SkillImportRequest {
    preview_id: String,
    #[serde(default)]
    replace: bool,
}

#[derive(serde::Deserialize)]
pub struct DeleteSkillRequest {
    confirmation: String,
}

#[derive(serde::Deserialize)]
pub struct SkillSourceSearchQuery {
    keyword: String,
    #[serde(default = "default_skill_search_limit")]
    limit: usize,
}

fn default_skill_search_limit() -> usize { 20 }

#[derive(serde::Deserialize)]
pub struct RemoteSkillPreviewRequest {
    source_id: String,
    archive_url: String,
    #[serde(default)]
    version: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SkillLinkPreviewRequest {
    url: String,
}

fn parse_public_skill_link(input: &str) -> Result<Url, AppError> {
    let url = Url::parse(input.trim()).map_err(|_| AppError::BadRequest("技能链接无效".to_string()))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(AppError::BadRequest("技能链接必须使用 HTTPS".to_string()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::BadRequest("技能链接不能包含访问凭据".to_string()));
    }
    if url.port_or_known_default() != Some(443) {
        return Err(AppError::BadRequest("技能链接必须使用 HTTPS 默认端口".to_string()));
    }
    Ok(url)
}

fn is_public_skill_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast())
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

async fn resolve_public_skill_host(url: &Url) -> Result<(String, Vec<std::net::SocketAddr>), AppError> {
    let host = url.host_str().ok_or_else(|| AppError::BadRequest("技能链接缺少主机".to_string()))?.to_string();
    let addresses: Vec<_> = lookup_host((host.as_str(), 443)).await
        .map_err(|error| AppError::BadRequest(format!("无法解析技能链接主机: {error}")))?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_skill_ip(address.ip())) {
        return Err(AppError::BadRequest("技能链接主机必须解析为公开单播地址".to_string()));
    }
    Ok((host, addresses))
}

async fn download_public_skill_archive(url: &Url, max_size: u64) -> Result<Vec<u8>, AppError> {
    let (host, addresses) = resolve_public_skill_host(url).await?;
    let mut builder = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(std::time::Duration::from_secs(120));
    for address in addresses {
        builder = builder.resolve(&host, address);
    }
    let client = builder.build().map_err(|error| AppError::Internal(error.to_string()))?;
    let response = client.get(url.clone()).send().await
        .map_err(|error| AppError::BadRequest(format!("下载技能包失败: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!("下载技能包失败: HTTP {}", response.status())));
    }
    if response.content_length().is_some_and(|size| size > max_size) {
        return Err(AppError::BadRequest("技能包下载内容超过总容量上限".to_string()));
    }
    let mut archive = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| AppError::BadRequest(format!("读取技能包失败: {error}")))?;
        let next_size = archive.len().saturating_add(chunk.len());
        if next_size > max_size as usize {
            return Err(AppError::BadRequest("技能包下载内容超过总容量上限".to_string()));
        }
        archive.extend_from_slice(&chunk);
    }
    Ok(archive)
}

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
    if input.targets.len() < 2 || input.cases.is_empty() {
        return Err(AppError::BadRequest("需要选择至少两个端点与模型组合和一条样本".to_string()));
    }
    let mut case_ids = std::collections::HashSet::new();
    for case in &input.cases {
        if case.id.trim().is_empty() || !case_ids.insert(case.id.clone()) {
            return Err(AppError::BadRequest("每条评测样本需要唯一且非空的标识".to_string()));
        }
    }
    let mut target_keys = std::collections::HashSet::new();
    let mut snapshots = Vec::new();
    let mut endpoint_ids = Vec::new();
    for target in &input.targets {
        if target.model.trim().is_empty() || !target_keys.insert((target.endpoint_id.clone(), target.model.clone())) {
            return Err(AppError::BadRequest("每个端点与模型组合需要唯一且包含模型名称".to_string()));
        }
        let endpoint = state.get_endpoint(&target.endpoint_id).ok_or_else(|| AppError::BadRequest(format!("端点不存在: {}", target.endpoint_id)))?;
        if !endpoint_ids.contains(&target.endpoint_id) {
            endpoint_ids.push(target.endpoint_id.clone());
            snapshots.push(endpoint.config);
        }
    }
    if state.get_endpoint(&input.judge.endpoint_id).is_none() {
        return Err(AppError::BadRequest("评审端点不存在".to_string()));
    }
    let run = ModelBenchmarkRun { id: uuid::Uuid::new_v4().to_string(), status: ModelBenchmarkStatus::Queued, created_at: Utc::now(), completed_at: None, model: String::new(), endpoint_ids, targets: input.targets, endpoint_snapshots: snapshots, cases: input.cases, judge: input.judge, attempts_per_case: 3, attempts: Vec::new(), judge_results: Vec::new() };
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

fn skill_repository_root(state: &AppState) -> Result<(std::path::PathBuf, SkillRepositoryConfig), AppError> {
    let config = state.config.read().skill_repository.clone();
    let root = repository_root(&state.config_manager.config_dir(), &config)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok((root, config))
}

fn refresh_local_skills(state: &AppState) -> Result<Vec<LocalSkill>, AppError> {
    let (root, config) = skill_repository_root(state)?;
    let mut skills = scan_local_skills(&root, &config).map_err(|error| AppError::Internal(error.to_string()))?;
    let previous = state.skill_repository_state();
    for skill in &mut skills {
        if let Some(saved) = previous.skills.iter().find(|saved| saved.directory_name == skill.directory_name) {
            skill.source = saved.source.clone();
            skill.imported_at = saved.imported_at;
        }
    }
    let mut repository = previous;
    repository.skills = skills.clone();
    state.update_skill_repository_state(repository);
    Ok(skills)
}

fn add_skill_audit(state: &AppState, operation: &str, directory_name: &str, source: Option<SkillOrigin>, status: &str, error_message: Option<String>) {
    state.add_skill_audit_entry(SkillAuditEntry {
        id: uuid::Uuid::new_v4().to_string(),
        operation: operation.to_string(),
        directory_name: directory_name.to_string(),
        source,
        created_at: Utc::now(),
        status: status.to_string(),
        error_message,
    });
}

pub async fn list_skills(state: web::Data<AppState>, req: HttpRequest) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    Ok(HttpResponse::Ok().json(refresh_local_skills(state.get_ref())?))
}

pub async fn get_skill(state: web::Data<AppState>, req: HttpRequest, path: web::Path<String>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let id = path.into_inner();
    let skill = refresh_local_skills(state.get_ref())?.into_iter()
        .find(|skill| skill.id == id)
        .ok_or_else(|| AppError::NotFound("技能包不存在".to_string()))?;
    let (root, config) = skill_repository_root(state.get_ref())?;
    let skill_md = std::fs::read_to_string(root.join(&skill.directory_name).join("SKILL.md"))
        .map_err(|error| AppError::Internal(format!("读取 SKILL.md 失败: {}", error)))?;
    let files = list_skill_files(&root, &skill.directory_name, &config)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(HttpResponse::Ok().json(json!({ "skill": skill, "skill_md": skill_md, "files": files })))
}

pub async fn preview_skill_upload(state: web::Data<AppState>, req: HttpRequest, archive: web::Bytes) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let (root, config) = skill_repository_root(state.get_ref())?;
    let source = SkillOrigin { source_type: SkillSourceType::CustomIndex, url: "local-upload".to_string(), version: None, content_digest: None };
    let (preview, package) = preview_zip_archive(&archive, source, &root, &config)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    state.store_skill_import_preview(preview.clone(), package);
    Ok(HttpResponse::Ok().json(preview))
}

async fn import_preview(state: &AppState, input: SkillImportRequest, force_replace: bool) -> Result<HttpResponse, AppError> {
    let preview = state.get_skill_import_preview(&input.preview_id)
        .ok_or_else(|| AppError::NotFound("导入预览不存在或已过期".to_string()))?;
    if !preview.valid {
        return Err(AppError::BadRequest(preview.validation_message.unwrap_or_else(|| "导入预览校验失败".to_string())));
    }
    let package = state.get_prepared_skill_package(&input.preview_id)
        .ok_or_else(|| AppError::NotFound("导入预览内容不存在或已过期".to_string()))?;
    let (root, _) = skill_repository_root(state)?;
    let replace = force_replace || input.replace;
    if preview.conflict && !replace {
        return Err(AppError::BadRequest("目标技能已存在，请明确选择替换".to_string()));
    }
    let operation = if replace { "replace" } else { "import" };
    if let Err(error) = import_skill_package(&root, &package, replace) {
        let message = error.to_string();
        add_skill_audit(state, operation, &preview.target_directory_name, Some(preview.source), "failed", Some(message.clone()));
        return Err(AppError::Internal(message));
    }
    let mut skills = refresh_local_skills(state)?;
    let imported_at = Utc::now();
    if let Some(skill) = skills.iter_mut().find(|skill| skill.directory_name == preview.target_directory_name) {
        skill.source = Some(preview.source.clone());
        skill.imported_at = Some(imported_at);
    }
    let mut repository = state.skill_repository_state();
    repository.skills = skills;
    state.update_skill_repository_state(repository);
    add_skill_audit(state, operation, &preview.target_directory_name, Some(preview.source), "success", None);
    state.remove_skill_import_preview(&input.preview_id);
    Ok(HttpResponse::Ok().json(json!({ "success": true, "directory_name": preview.target_directory_name })))
}

pub async fn import_skill(state: web::Data<AppState>, req: HttpRequest, body: web::Json<SkillImportRequest>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    import_preview(state.get_ref(), body.into_inner(), false).await
}

pub async fn replace_skill(state: web::Data<AppState>, req: HttpRequest, path: web::Path<String>, body: web::Json<SkillImportRequest>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let input = body.into_inner();
    let preview = state.get_skill_import_preview(&input.preview_id)
        .ok_or_else(|| AppError::NotFound("导入预览不存在或已过期".to_string()))?;
    if preview.target_directory_name != path.into_inner() {
        return Err(AppError::BadRequest("替换目标与导入预览不一致".to_string()));
    }
    import_preview(state.get_ref(), input, true).await
}

pub async fn delete_skill(state: web::Data<AppState>, req: HttpRequest, path: web::Path<String>, body: web::Json<DeleteSkillRequest>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let directory_name = path.into_inner();
    let (root, _) = skill_repository_root(state.get_ref())?;
    if let Err(error) = delete_skill_package(&root, &directory_name, &body.confirmation) {
        let message = error.to_string();
        add_skill_audit(state.get_ref(), "delete", &directory_name, None, "failed", Some(message.clone()));
        return Err(AppError::BadRequest(message));
    }
    refresh_local_skills(state.get_ref())?;
    add_skill_audit(state.get_ref(), "delete", &directory_name, None, "success", None);
    Ok(HttpResponse::Ok().json(json!({ "success": true })))
}

pub async fn search_skill_sources(state: web::Data<AppState>, req: HttpRequest, query: web::Query<SkillSourceSearchQuery>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let query = query.into_inner();
    if query.keyword.trim().is_empty() {
        return Err(AppError::BadRequest("搜索关键词不能为空".to_string()));
    }
    let sources = state.skill_repository_state().sources;
    let adapters = adapters_for_sources(&sources).map_err(|error| AppError::BadRequest(error.to_string()))?;
    let outcomes = search_sources(&adapters, &query.keyword, query.limit.clamp(1, 100)).await;
    let mut repository = state.skill_repository_state();
    for source in &mut repository.sources {
        if let Some(outcome) = outcomes.iter().find(|outcome| outcome.source_id == source.id) {
            source.last_checked_at = Some(Utc::now());
            source.last_status = outcome.error.clone().or_else(|| Some("available".to_string()));
        }
    }
    state.update_skill_repository_state(repository.clone());
    Ok(HttpResponse::Ok().json(json!({ "sources": repository.sources, "outcomes": outcomes })))
}

pub async fn list_skill_sources(state: web::Data<AppState>, req: HttpRequest) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    Ok(HttpResponse::Ok().json(state.skill_repository_state().sources))
}

pub async fn update_skill_sources(state: web::Data<AppState>, req: HttpRequest, body: web::Json<Vec<SkillSourceConfig>>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let sources = body.into_inner();
    let mut ids = std::collections::HashSet::new();
    for source in &sources {
        if source.id.trim().is_empty() || !ids.insert(source.id.clone()) || source.name.trim().is_empty() {
            return Err(AppError::BadRequest("来源标识和名称必须唯一且非空".to_string()));
        }
        let url = Url::parse(&source.url).map_err(|_| AppError::BadRequest(format!("来源地址无效: {}", source.name)))?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err(AppError::BadRequest(format!("来源地址必须使用 HTTPS: {}", source.name)));
        }
    }
    let mut repository = state.skill_repository_state();
    repository.sources = sources;
    state.update_skill_repository_state(repository.clone());
    Ok(HttpResponse::Ok().json(repository.sources))
}

pub async fn preview_remote_skill(state: web::Data<AppState>, req: HttpRequest, body: web::Json<RemoteSkillPreviewRequest>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let input = body.into_inner();
    let source = state.skill_repository_state().sources.into_iter()
        .find(|source| source.id == input.source_id && source.enabled)
        .ok_or_else(|| AppError::NotFound("公开来源不存在或未启用".to_string()))?;
    let archive_url = Url::parse(&input.archive_url).map_err(|_| AppError::BadRequest("技能包下载地址无效".to_string()))?;
    if archive_url.scheme() != "https" || archive_url.host_str().is_none() {
        return Err(AppError::BadRequest("技能包下载地址必须使用 HTTPS".to_string()));
    }
    let source_host = Url::parse(&source.url).ok().and_then(|url| url.host_str().map(str::to_string));
    let permitted = archive_url.host_str() == source_host.as_deref()
        || matches!(source.source_type, SkillSourceType::Github) && matches!(archive_url.host_str(), Some("github.com") | Some("codeload.github.com"));
    if !permitted {
        return Err(AppError::BadRequest("技能包下载地址不属于所选公开来源".to_string()));
    }
    let (download_url, github_skill_directory) = if matches!(source.source_type, SkillSourceType::Github) && archive_url.host_str() == Some("github.com") {
        let github_link = github_skill_link(&archive_url)?;
        (github_link.archive_url, Some(github_link.directory))
    } else {
        (archive_url.clone(), None)
    };
    let client = reqwest::Client::builder().redirect(Policy::none()).timeout(std::time::Duration::from_secs(120)).build()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let response = client.get(download_url).send().await.map_err(|error| AppError::BadRequest(format!("下载技能包失败: {}", error)))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!("下载技能包失败: HTTP {}", response.status())));
    }
    let (root, config) = skill_repository_root(state.get_ref())?;
    if response.content_length().is_some_and(|size| size > config.max_total_size_bytes) {
        return Err(AppError::BadRequest("技能包下载内容超过总容量上限".to_string()));
    }
    let archive = response.bytes().await.map_err(|error| AppError::BadRequest(format!("读取技能包失败: {}", error)))?;
    let archive = github_skill_directory
        .map(|directory| isolate_github_skill_archive(&archive, &directory))
        .transpose()?
        .unwrap_or_else(|| archive.to_vec());
    let origin = SkillOrigin { source_type: source.source_type, url: archive_url.to_string(), version: input.version, content_digest: None };
    let (preview, package) = preview_zip_archive(&archive, origin, &root, &config).map_err(|error| AppError::BadRequest(error.to_string()))?;
    state.store_skill_import_preview(preview.clone(), package);
    Ok(HttpResponse::Ok().json(preview))
}

pub async fn preview_skill_link(state: web::Data<AppState>, req: HttpRequest, body: web::Json<SkillLinkPreviewRequest>) -> Result<HttpResponse, AppError> {
    check_admin_auth(&req, state.get_ref())?;
    let input = body.into_inner();
    let source_url = parse_public_skill_link(&input.url)?;
    let (download_url, github_skill_directory, source_type) = if source_url.host_str() == Some("github.com") {
        let github_link = github_skill_link(&source_url)?;
        (github_link.archive_url, Some(github_link.directory), SkillSourceType::Github)
    } else {
        (source_url.clone(), None, SkillSourceType::CustomIndex)
    };
    let (root, config) = skill_repository_root(state.get_ref())?;
    let archive = download_public_skill_archive(&download_url, config.max_total_size_bytes).await?;
    let archive = github_skill_directory
        .map(|directory| isolate_github_skill_archive(&archive, &directory))
        .transpose()?
        .unwrap_or(archive);
    let origin = SkillOrigin { source_type, url: source_url.to_string(), version: None, content_digest: None };
    let (preview, package) = preview_zip_archive(&archive, origin, &root, &config)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    state.store_skill_import_preview(preview.clone(), package);
    Ok(HttpResponse::Ok().json(preview))
}

#[cfg(test)]
mod benchmark_tests {
    use super::*;
    use actix_web::body::to_bytes;
    use actix_web::cookie::Cookie;
    use actix_web::test::TestRequest;
    use std::io::{Cursor, Write};
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

    fn skill_archive(directory_name: &str) -> web::Bytes {
        let cursor = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        archive.start_file(format!("{}/SKILL.md", directory_name), zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"---\nname: Example\ndescription: Example skill\n---\n# Example").unwrap();
        archive.start_file(format!("{}/README.md", directory_name), zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"Details").unwrap();
        web::Bytes::from(archive.finish().unwrap().into_inner())
    }

    #[test]
    fn github_archive_preview_keeps_only_selected_skill_directory() {
        let cursor = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        archive.start_file("repo-HEAD/skills/review/SKILL.md", zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"# Review").unwrap();
        archive.start_file("repo-HEAD/skills/review/README.md", zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"Details").unwrap();
        archive.start_file("repo-HEAD/skills/other/SKILL.md", zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"# Other").unwrap();
        let isolated = isolate_github_skill_archive(&archive.finish().unwrap().into_inner(), "skills/review").unwrap();
        let isolated = zip::ZipArchive::new(Cursor::new(isolated)).unwrap();
        let mut names = isolated.file_names().collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, vec!["review/README.md", "review/SKILL.md"]);
    }

    #[test]
    fn github_skill_link_converts_branch_directory_to_archive() {
        let link = github_skill_link(&Url::parse("https://github.com/acme/skills/tree/main/packages/review").unwrap()).unwrap();
        assert_eq!(link.archive_url.as_str(), "https://codeload.github.com/acme/skills/zip/main");
        assert_eq!(link.directory, "packages/review");
    }

    #[test]
    fn github_skill_link_uses_skill_md_parent_directory() {
        let link = github_skill_link(&Url::parse("https://github.com/acme/skills/blob/v1.2.0/packages/review/SKILL.md").unwrap()).unwrap();
        assert_eq!(link.archive_url.as_str(), "https://codeload.github.com/acme/skills/zip/v1.2.0");
        assert_eq!(link.directory, "packages/review");
    }

    #[test]
    fn github_skill_link_supports_repository_root_skill_md() {
        let link = github_skill_link(&Url::parse("https://github.com/acme/skills/blob/main/SKILL.md").unwrap()).unwrap();
        assert_eq!(link.archive_url.as_str(), "https://codeload.github.com/acme/skills/zip/main");
        assert!(link.directory.is_empty());
    }

    #[test]
    fn github_archive_preview_supports_repository_root_skill() {
        let cursor = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        archive.start_file("repo-main/SKILL.md", zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"# Root skill").unwrap();
        archive.start_file("repo-main/README.md", zip::write::FileOptions::default()).unwrap();
        archive.write_all(b"Details").unwrap();
        let isolated = isolate_github_skill_archive(&archive.finish().unwrap().into_inner(), "").unwrap();
        let isolated = zip::ZipArchive::new(Cursor::new(isolated)).unwrap();
        let mut names = isolated.file_names().collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, vec!["github-skill/README.md", "github-skill/SKILL.md"]);
    }

    #[test]
    fn github_skill_link_rejects_non_skill_md_files() {
        assert!(github_skill_link(&Url::parse("https://github.com/acme/skills/blob/main/packages/review/README.md").unwrap()).is_err());
    }

    #[test]
    fn github_skill_link_accepts_bare_repository_url() {
        let link = github_skill_link(&Url::parse("https://github.com/acme/skills").unwrap()).unwrap();
        assert_eq!(link.archive_url.as_str(), "https://codeload.github.com/acme/skills/zip/HEAD");
        assert!(link.directory.is_empty());
    }

    #[test]
    fn parse_public_skill_link_accepts_https_without_credentials() {
        let url = parse_public_skill_link("https://example.com/skills/review.zip").unwrap();
        assert_eq!(url.host_str(), Some("example.com"));
    }

    #[test]
    fn parse_public_skill_link_rejects_insecure_or_credentialed_urls() {
        assert!(parse_public_skill_link("http://example.com/review.zip").is_err());
        assert!(parse_public_skill_link("https://user:secret@example.com/review.zip").is_err());
        assert!(parse_public_skill_link("https://example.com:8443/review.zip").is_err());
    }

    #[test]
    fn public_skill_ip_filter_rejects_non_public_networks() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!is_public_skill_ip(address.parse().unwrap()), "{address} should be blocked");
        }
        assert!(is_public_skill_ip("8.8.8.8".parse().unwrap()));
    }

    #[actix_rt::test]
    async fn skill_link_preview_requires_admin_session() {
        let state = state().await;
        let request = TestRequest::default().to_http_request();
        let response = preview_skill_link(state, request, web::Json(SkillLinkPreviewRequest {
            url: "https://example.com/review.zip".to_string(),
        })).await;
        assert!(response.is_err());
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
        let input = CreateModelBenchmarkRequest { targets: Vec::new(), cases: Vec::new(), judge: BenchmarkJudgeConfig { endpoint_id: String::new(), model: String::new(), rubric: String::new() } };
        assert!(create_model_benchmark(state, req, web::Json(input)).await.is_err());
    }

    #[actix_rt::test]
    async fn skill_list_requires_admin_session() {
        let state = state().await;
        assert!(list_skills(state, TestRequest::default().to_http_request()).await.is_err());
    }

    #[actix_rt::test]
    async fn skill_upload_preview_requires_confirmation_and_detects_conflicts() {
        let state = state().await;
        let response = preview_skill_upload(state.clone(), authenticated_request(state.get_ref()), skill_archive("example"))
            .await.unwrap();
        let preview: SkillImportPreview = serde_json::from_slice(&to_bytes(response.into_body()).await.unwrap()).unwrap();
        assert!(!preview.conflict);

        import_skill(state.clone(), authenticated_request(state.get_ref()), web::Json(SkillImportRequest { preview_id: preview.id, replace: false }))
            .await.unwrap();
        let skills = refresh_local_skills(state.get_ref()).unwrap();
        assert_eq!(skills.len(), 1);

        let response = preview_skill_upload(state.clone(), authenticated_request(state.get_ref()), skill_archive("example"))
            .await.unwrap();
        let conflict: SkillImportPreview = serde_json::from_slice(&to_bytes(response.into_body()).await.unwrap()).unwrap();
        assert!(conflict.conflict);
        assert!(import_skill(state.clone(), authenticated_request(state.get_ref()), web::Json(SkillImportRequest { preview_id: conflict.id, replace: false }))
            .await.is_err());
    }

    #[actix_rt::test]
    async fn skill_delete_requires_matching_confirmation() {
        let state = state().await;
        let response = preview_skill_upload(state.clone(), authenticated_request(state.get_ref()), skill_archive("example"))
            .await.unwrap();
        let preview: SkillImportPreview = serde_json::from_slice(&to_bytes(response.into_body()).await.unwrap()).unwrap();
        import_skill(state.clone(), authenticated_request(state.get_ref()), web::Json(SkillImportRequest { preview_id: preview.id, replace: false }))
            .await.unwrap();

        assert!(delete_skill(state.clone(), authenticated_request(state.get_ref()), web::Path::from("example".to_string()), web::Json(DeleteSkillRequest { confirmation: "wrong".to_string() }))
            .await.is_err());
        delete_skill(state.clone(), authenticated_request(state.get_ref()), web::Path::from("example".to_string()), web::Json(DeleteSkillRequest { confirmation: "example".to_string() }))
            .await.unwrap();
        assert!(refresh_local_skills(state.get_ref()).unwrap().is_empty());
    }

    #[actix_rt::test]
    async fn skill_source_configuration_requires_https() {
        let state = state().await;
        let sources = vec![SkillSourceConfig {
            id: "custom".to_string(), name: "Custom".to_string(), source_type: SkillSourceType::CustomIndex,
            url: "http://example.test/index.json".to_string(), enabled: true, last_status: None, last_checked_at: None,
        }];
        assert!(update_skill_sources(state.clone(), authenticated_request(state.get_ref()), web::Json(sources)).await.is_err());
    }
}
