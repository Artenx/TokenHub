use crate::models::*;
use chrono::Utc;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, info, warn};

/// 应用程序共享状态
pub struct AppState {
    /// 应用配置
    pub config: RwLock<AppConfig>,
    /// 端点运行时状态
    pub endpoints: RwLock<HashMap<String, EndpointState>>,
    /// 调度器当前索引（用于轮询和轮换）
    pub scheduler_index: RwLock<HashMap<String, usize>>,
    /// 轮换模式下当前活跃端点索引
    pub failover_index: RwLock<HashMap<String, usize>>,
    /// 端点模型列表缓存 (endpoint_id -> Vec<String>)
    pub model_cache: RwLock<HashMap<String, Vec<String>>>,
    /// HTTP 客户端
    pub http_client: reqwest::Client,
    /// 配置文件管理器
    pub config_manager: crate::config::ConfigManager,
    /// 运行时状态文件路径
    pub state_path: PathBuf,
    /// 数据变更标记（用于后台持久化）
    pub dirty: AtomicBool,
    /// 持久化写锁（防止并发写入导致数据覆盖）
    pub save_state_mutex: tokio::sync::Mutex<()>,
}

impl AppState {
    pub async fn new(config_manager: crate::config::ConfigManager) -> anyhow::Result<Self> {
        let config = config_manager.load().await?;
        let mut endpoints = HashMap::new();

        for ep_config in &config.endpoints {
            let state = EndpointState::new(ep_config.clone());
            endpoints.insert(ep_config.id.clone(), state);
        }

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let state_path = config_manager.config_dir().join("state.json");

        let app = Self {
            config: RwLock::new(config),
            endpoints: RwLock::new(endpoints),
            scheduler_index: RwLock::new(HashMap::new()),
            failover_index: RwLock::new(HashMap::new()),
            model_cache: RwLock::new(HashMap::new()),
            http_client,
            config_manager,
            state_path,
            dirty: AtomicBool::new(false),
            save_state_mutex: tokio::sync::Mutex::new(()),
        };

        // 从 state.json 恢复运行时状态
        app.load_runtime_state();

        Ok(app)
    }

    /// 获取指定池中可用的端点ID列表
    pub fn available_endpoint_ids_in_pool(&self, pool_id: &str) -> Vec<String> {
        let endpoints = self.endpoints.read();
        endpoints
            .values()
            .filter(|ep| {
                ep.is_available() && ep.config.pool_ids.contains(&pool_id.to_string())
            })
            .map(|ep| ep.config.id.clone())
            .collect()
    }

    /// 获取端点状态
    pub fn get_endpoint(&self, id: &str) -> Option<EndpointState> {
        let endpoints = self.endpoints.read();
        endpoints.get(id).cloned()
    }

    /// 更新端点token使用量
    pub fn update_endpoint_tokens(&self, id: &str, tokens: u64) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            ep.add_tokens(tokens);
            tracing::debug!("端点 {} 消耗 {} tokens, 总计: {}/{}", id, tokens, ep.tokens_used, ep.config.token_limit);
        }
        self.mark_dirty();
    }

    /// 增加端点错误计数
    pub fn increment_endpoint_errors(&self, id: &str) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            ep.error_count += 1;
        }
    }

    /// 添加端点
    pub async fn add_endpoint(&self, req: EndpointRequest) -> anyhow::Result<EndpointState> {
        let id = uuid::Uuid::new_v4().to_string();
        let config = EndpointConfig {
            id: id.clone(),
            name: req.name,
            url: req.url,
            api_type: req.api_type,
            api_key: req.api_key,
            token_limit: req.token_limit,
            reset_policy: req.reset_policy,
            enabled: req.enabled.unwrap_or(true),
            pool_ids: req.pool_ids,
            timeout: req.timeout.unwrap_or(300),
            model_mappings: req.model_mappings,
        };
        let state = EndpointState::new(config.clone());

        {
            let mut endpoints = self.endpoints.write();
            endpoints.insert(id.clone(), state.clone());
        }

        // 更新并保存配置（先克隆数据，释放锁后再保存）
        let config_to_save = {
            let mut app_config = self.config.write();
            app_config.endpoints.push(config);
            app_config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已添加端点: {} ({})", state.config.name, id);
        Ok(state)
    }

    /// 更新端点
    pub async fn update_endpoint(&self, id: &str, req: EndpointRequest) -> anyhow::Result<EndpointState> {
        let state = {
            let mut endpoints = self.endpoints.write();
            let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;

            ep.config.name = req.name;
            ep.config.url = req.url;
            ep.config.api_type = req.api_type;
            ep.config.api_key = req.api_key;
            ep.config.token_limit = req.token_limit;
            ep.config.reset_policy = req.reset_policy;
            if let Some(enabled) = req.enabled {
                ep.config.enabled = enabled;
            }
            // 更新 pool_ids
            ep.config.pool_ids = req.pool_ids;
            if let Some(timeout) = req.timeout {
                ep.config.timeout = timeout;
            }
            // 更新模型映射
            if !req.model_mappings.is_empty() {
                ep.config.model_mappings = req.model_mappings;
            }
            ep.clone()
        }; // endpoints 锁在此释放

        // 更新配置（锁已释放后再保存）
        let config_to_save = {
            let mut app_config = self.config.write();
            if let Some(ep_config) = app_config.endpoints.iter_mut().find(|e| e.id == id) {
                *ep_config = state.config.clone();
            }
            app_config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已更新端点: {} ({})", state.config.name, id);
        Ok(state)
    }

    /// 删除端点
    pub async fn delete_endpoint(&self, id: &str) -> anyhow::Result<()> {
        {
            let mut endpoints = self.endpoints.write();
            endpoints.remove(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        }

        let config_to_save = {
            let mut app_config = self.config.write();
            app_config.endpoints.retain(|e| e.id != id);
            app_config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已删除端点: {}", id);
        Ok(())
    }

    /// 切换端点启用状态
    pub async fn toggle_endpoint(&self, id: &str) -> anyhow::Result<EndpointState> {
        let state = {
            let mut endpoints = self.endpoints.write();
            let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
            ep.config.enabled = !ep.config.enabled;
            ep.clone()
        }; // endpoints 锁在此释放

        // 更新配置（锁已释放后再保存）
        let config_to_save = {
            let mut app_config = self.config.write();
            if let Some(ep_config) = app_config.endpoints.iter_mut().find(|e| e.id == id) {
                ep_config.enabled = state.config.enabled;
            }
            app_config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("端点 {} 已{}", state.config.name, if state.config.enabled { "启用" } else { "禁用" });
        Ok(state)
    }

    /// 重置指定端点的token使用量
    pub async fn reset_endpoint_tokens(&self, id: &str) -> anyhow::Result<()> {
        let mut endpoints = self.endpoints.write();
        let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        ep.reset();
        self.mark_dirty();
        info!("已重置端点 {} 的token使用量", ep.config.name);
        Ok(())
    }

    /// 重置所有端点的token使用量
    pub fn reset_all_tokens(&self) {
        let mut endpoints = self.endpoints.write();
        for ep in endpoints.values_mut() {
            ep.reset();
        }
        self.mark_dirty();
        info!("已重置所有端点的token使用量");
    }

    // ========== 池管理 ==========

    /// 添加池
    pub async fn add_pool(&self, req: PoolRequest) -> anyhow::Result<Pool> {
        let id = uuid::Uuid::new_v4().to_string();
        let pool = Pool {
            id: id.clone(),
            name: req.name,
            description: req.description.unwrap_or_default(),
            schedule_algorithm: req.schedule_algorithm,
            model_mode: req.model_mode,
            retry_mode: req.retry_mode,
            retry_count: req.retry_count,
            exposed_api_id: req.exposed_api_id,
            created_at: Utc::now(),
        };

        let config_to_save = {
            let mut config = self.config.write();
            config.pools.push(pool.clone());
            config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已添加池: {} ({})", pool.name, id);
        Ok(pool)
    }

    /// 更新池
    pub async fn update_pool(&self, id: &str, req: PoolRequest) -> anyhow::Result<Pool> {
        let config_to_save = {
            let mut config = self.config.write();
            let pool = config.pools.iter_mut().find(|p| p.id == id)
                .ok_or_else(|| anyhow::anyhow!("池不存在: {}", id))?;

            pool.name = req.name;
            if let Some(desc) = req.description {
                pool.description = desc;
            }
            pool.schedule_algorithm = req.schedule_algorithm;
            pool.model_mode = req.model_mode;
            pool.retry_mode = req.retry_mode;
            pool.retry_count = req.retry_count;
            pool.exposed_api_id = req.exposed_api_id;
            config.clone()
        };
        
        let pool = config_to_save.pools.iter().find(|p| p.id == id).unwrap().clone();
        self.config_manager.save(&config_to_save).await?;
        info!("已更新池: {} ({})", pool.name, id);
        Ok(pool)
    }

    /// 删除池
    pub async fn delete_pool(&self, id: &str) -> anyhow::Result<()> {
        // 同步清理内存中端点的 pool_ids
        {
            let mut endpoints = self.endpoints.write();
            for ep in endpoints.values_mut() {
                ep.config.pool_ids.retain(|pid| pid != id);
            }
        }

        let config_to_save = {
            let mut config = self.config.write();
            config.pools.retain(|p| p.id != id);
            // 清除关联的端点中的池ID
            for ep in config.endpoints.iter_mut() {
                ep.pool_ids.retain(|pid| pid != id);
            }
            // 清除关联的对外API
            config.exposed_apis.retain(|a| a.pool_id != id);
            config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已删除池: {}", id);
        Ok(())
    }

    /// 获取池信息
    pub fn get_pool(&self, id: &str) -> Option<Pool> {
        let config = self.config.read();
        config.pools.iter().find(|p| p.id == id).cloned()
    }

    // ========== 对外API管理 ==========

    /// 添加对外API
    pub async fn add_exposed_api(&self, req: ExposedApiRequest) -> anyhow::Result<ExposedApi> {
        let id = uuid::Uuid::new_v4().to_string();
        let api = ExposedApi {
            id: id.clone(),
            name: req.name,
            prefix: req.prefix,
            api_type: req.api_type,
            api_key: req.api_key,
            enabled: req.enabled.unwrap_or(true),
            pool_id: req.pool_id,
            created_at: Utc::now(),
        };

        let config_to_save = {
            let mut config = self.config.write();
            config.exposed_apis.push(api.clone());
            config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已添加对外API: {} ({})", api.name, id);
        Ok(api)
    }

    /// 更新对外API
    pub async fn update_exposed_api(&self, id: &str, req: ExposedApiRequest) -> anyhow::Result<ExposedApi> {
        let config_to_save = {
            let mut config = self.config.write();
            let api = config.exposed_apis.iter_mut().find(|a| a.id == id)
                .ok_or_else(|| anyhow::anyhow!("对外API不存在: {}", id))?;

            api.name = req.name;
            api.prefix = req.prefix;
            api.api_type = req.api_type;
            api.api_key = req.api_key;
            if let Some(enabled) = req.enabled {
                api.enabled = enabled;
            }
            api.pool_id = req.pool_id;
            config.clone()
        };
        
        let api = config_to_save.exposed_apis.iter().find(|a| a.id == id).unwrap().clone();
        self.config_manager.save(&config_to_save).await?;
        info!("已更新对外API: {} ({})", api.name, id);
        Ok(api)
    }

    /// 删除对外API
    pub async fn delete_exposed_api(&self, id: &str) -> anyhow::Result<()> {
        let config_to_save = {
            let mut config = self.config.write();
            config.exposed_apis.retain(|a| a.id != id);
            config.clone()
        };
        self.config_manager.save(&config_to_save).await?;

        info!("已删除对外API: {}", id);
        Ok(())
    }

    /// 切换对外API启用状态
    pub async fn toggle_exposed_api(&self, id: &str) -> anyhow::Result<ExposedApi> {
        let config_to_save = {
            let mut config = self.config.write();
            let api = config.exposed_apis.iter_mut().find(|a| a.id == id)
                .ok_or_else(|| anyhow::anyhow!("对外API不存在: {}", id))?;

            api.enabled = !api.enabled;
            config.clone()
        };

        let api = config_to_save.exposed_apis.iter().find(|a| a.id == id).unwrap().clone();
        self.config_manager.save(&config_to_save).await?;
        info!("对外API {} 已{}", api.name, if api.enabled { "启用" } else { "禁用" });
        Ok(api)
    }

    /// 根据请求路径匹配对外API（取最长前缀匹配）
    pub fn match_exposed_api(&self, path: &str) -> Option<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.iter()
            .filter(|a| a.enabled && path.starts_with(&a.prefix))
            .max_by_key(|a| a.prefix.len())
            .cloned()
    }

    // ========== 统计 ==========

    /// 获取统计信息
    pub fn get_stats(&self) -> StatsInfo {
        let endpoints = self.endpoints.read();
        let config = self.config.read();

        let active_count = endpoints.values().filter(|ep| ep.is_available()).count();
        // 排除无限制（token_limit == 0 或 >= 999999999000）的端点
        let limited_endpoints: Vec<_> = endpoints.values()
            .filter(|ep| ep.config.token_limit > 0 && ep.config.token_limit < 999999999000)
            .collect();
        let total_tokens_used: u64 = limited_endpoints.iter().map(|ep| ep.tokens_used).sum();
        let total_tokens_limit: u64 = limited_endpoints.iter().map(|ep| ep.config.token_limit).sum();
        let total_requests: u64 = endpoints.values().map(|ep| ep.total_requests).sum();

        let endpoint_stats: Vec<EndpointStats> = endpoints
            .values()
            .map(|ep| EndpointStats {
                id: ep.config.id.clone(),
                name: ep.config.name.clone(),
                url: ep.config.url.clone(),
                api_type: ep.config.api_type.clone(),
                tokens_used: ep.tokens_used,
                token_limit: ep.config.token_limit,
                tokens_remaining: ep.tokens_remaining(),
                enabled: ep.config.enabled,
                last_used: ep.last_used,
                total_requests: ep.total_requests,
                error_count: ep.error_count,
                pool_ids: ep.config.pool_ids.clone(),
                timeout: ep.config.timeout,
                reset_policy: ep.config.reset_policy.clone(),
                model_mappings: ep.config.model_mappings.clone(),
            })
            .collect();

        let pool_infos: Vec<PoolInfo> = config.pools.iter().map(|pool| {
            let pool_endpoints: Vec<_> = endpoints.values()
                .filter(|ep| ep.config.pool_ids.contains(&pool.id))
                .collect();
            let active_in_pool = pool_endpoints.iter().filter(|ep| ep.is_available()).count();
            let tokens_in_pool: u64 = pool_endpoints.iter().map(|ep| ep.tokens_used).sum();
            let requests_in_pool: u64 = pool_endpoints.iter().map(|ep| ep.total_requests).sum();

            PoolInfo {
                id: pool.id.clone(),
                name: pool.name.clone(),
                description: pool.description.clone(),
                schedule_algorithm: pool.schedule_algorithm.clone(),
                model_mode: pool.model_mode.clone(),
                retry_mode: pool.retry_mode.clone(),
                retry_count: pool.retry_count,
                exposed_api_id: pool.exposed_api_id.clone(),
                endpoint_count: pool_endpoints.len(),
                active_endpoint_count: active_in_pool,
                total_tokens_used: tokens_in_pool,
                total_requests: requests_in_pool,
            }
        }).collect();

        let api_infos: Vec<ExposedApiInfo> = config.exposed_apis.iter().map(|api| {
            let pool_name = config.pools.iter()
                .find(|p| p.id == api.pool_id)
                .map(|p| p.name.clone());
            let ep_count = endpoints.values()
                .filter(|ep| ep.config.pool_ids.contains(&api.pool_id))
                .count();

            ExposedApiInfo {
                id: api.id.clone(),
                name: api.name.clone(),
                prefix: api.prefix.clone(),
                api_type: api.api_type.clone(),
                enabled: api.enabled,
                pool_id: api.pool_id.clone(),
                pool_name,
                endpoint_count: ep_count,
            }
        }).collect();

        StatsInfo {
            total_endpoints: endpoints.len(),
            active_endpoints: active_count,
            total_tokens_used,
            total_tokens_limit,
            total_requests,
            total_pools: config.pools.len(),
            total_exposed_apis: config.exposed_apis.len(),
            endpoints: endpoint_stats,
            pools: pool_infos,
            exposed_apis: api_infos,
        }
    }

    /// 标记数据为脏（需要持久化）
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// 从 state.json 加载端点的运行时状态
    pub fn load_runtime_state(&self) {
        let path = &self.state_path;
        if !path.exists() {
            return;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("读取状态文件失败: {}", e);
                return;
            }
        };
        let data: HashMap<String, serde_json::Value> = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(e) => {
                warn!("解析状态文件失败: {}", e);
                return;
            }
        };
        let count = data.len();
        let mut endpoints = self.endpoints.write();
        for (id, state_data) in &data {
            if let Some(ep) = endpoints.get_mut(id) {
                if let Some(tokens) = state_data.get("tokens_used").and_then(|v| v.as_u64()) {
                    ep.tokens_used = tokens;
                }
                if let Some(reset_str) = state_data.get("last_reset").and_then(|v| v.as_str()) {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_str) {
                        ep.last_reset = dt.with_timezone(&chrono::Utc);
                    }
                }
                if let Some(used_str) = state_data.get("last_used").and_then(|v| v.as_str()) {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(used_str) {
                        ep.last_used = Some(dt.with_timezone(&chrono::Utc));
                    }
                }
                if let Some(errors) = state_data.get("error_count").and_then(|v| v.as_u64()) {
                    ep.error_count = errors as u32;
                }
                if let Some(reqs) = state_data.get("total_requests").and_then(|v| v.as_u64()) {
                    ep.total_requests = reqs;
                }
            }
        }
        info!("已从状态文件恢复 {} 个端点的运行时状态", count);
    }

    /// 异步保存运行时状态到 state.json
    pub async fn save_runtime_state(&self) -> anyhow::Result<()> {
        let _guard = self.save_state_mutex.lock().await;
        let data: HashMap<String, serde_json::Value> = {
            let endpoints = self.endpoints.read();
            endpoints.iter().map(|(id, ep)| {
                (id.clone(), serde_json::json!({
                    "tokens_used": ep.tokens_used,
                    "last_reset": ep.last_reset,
                    "last_used": ep.last_used,
                    "error_count": ep.error_count,
                    "total_requests": ep.total_requests,
                }))
            }).collect()
        };
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| anyhow::anyhow!("序列化运行时状态失败: {}", e))?;
        if let Some(parent) = self.state_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&self.state_path, json).await
            .map_err(|e| anyhow::anyhow!("写入状态文件失败: {}", e))?;
        debug!("已保存运行时状态 ({})", data.len());
        Ok(())
    }

    /// 更新管理密码
    pub async fn change_admin_password(&self, new_password: &str) -> anyhow::Result<()> {
        let config_to_save = {
            let mut config = self.config.write();
            config.admin_password = new_password.to_string();
            config.clone()
        };
        self.config_manager.save(&config_to_save).await?;
        Ok(())
    }

    /// 执行每日重置检查
    pub async fn check_daily_reset(&self) {
        let mut endpoints = self.endpoints.write();
        let now = Utc::now();
        
        for ep in endpoints.values_mut() {
            // 每日重置模式：每天零点自动清零
            if ep.config.reset_policy == ResetPolicy::Daily && ep.config.token_limit > 0 {
                let last_reset_date = ep.last_reset.date_naive();
                let today = now.date_naive();
                if last_reset_date < today {
                    ep.tokens_used = 0;
                    ep.last_reset = now;
                    self.mark_dirty();
                    info!("端点 {} 每日自动重置", ep.config.name);
                }
            }
            
            // 手动重置模式：不做任何操作，已使用达到限额时 is_available() 自动返回 false
        }
    }

    /// 获取调度器索引
    pub fn get_scheduler_index(&self, pool_id: &str) -> usize {
        let index = self.scheduler_index.read();
        index.get(pool_id).copied().unwrap_or(0)
    }

    /// 更新调度器索引
    pub fn update_scheduler_index(&self, pool_id: &str, new_index: usize) {
        let mut index = self.scheduler_index.write();
        index.insert(pool_id.to_string(), new_index);
    }

    /// 获取轮换索引
    pub fn get_failover_index(&self, pool_id: &str) -> usize {
        let index = self.failover_index.read();
        index.get(pool_id).copied().unwrap_or(0)
    }

    /// 更新轮换索引
    pub fn update_failover_index(&self, pool_id: &str, new_index: usize) {
        let mut index = self.failover_index.write();
        index.insert(pool_id.to_string(), new_index);
    }

    // ========== 模型缓存管理 ==========

    /// 获取端点的模型列表缓存
    pub fn get_cached_models(&self, endpoint_id: &str) -> Option<Vec<String>> {
        let cache = self.model_cache.read();
        cache.get(endpoint_id).cloned()
    }

    /// 更新端点的模型列表缓存
    pub fn update_model_cache(&self, endpoint_id: &str, models: Vec<String>) {
        let mut cache = self.model_cache.write();
        cache.insert(endpoint_id.to_string(), models);
        debug!("已更新端点 {} 的模型缓存", endpoint_id);
    }

    /// 从 API 获取端点的模型列表
    pub async fn fetch_endpoint_models(&self, endpoint_id: &str) -> anyhow::Result<Vec<String>> {
        let endpoint = self.get_endpoint(endpoint_id)
            .ok_or_else(|| anyhow::anyhow!("端点不存在: {}", endpoint_id))?;

        let base_url = endpoint.config.url.trim_end_matches('/');
        let models_url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
            format!("{}/models", base_url)
        } else {
            format!("{}/v1/models", base_url)
        };

        let mut request_builder = self.http_client.get(&models_url)
            .header("Content-Type", "application/json");

        match endpoint.config.api_type {
            ApiType::OpenAI | ApiType::OpenAIResponses => {
                request_builder = request_builder.header("Authorization", format!("Bearer {}", endpoint.config.api_key));
            }
            ApiType::Anthropic => {
                request_builder = request_builder.header("x-api-key", &endpoint.config.api_key);
                request_builder = request_builder.header("anthropic-version", "2023-06-01");
            }
        }

        match request_builder.timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let text = response.text().await.unwrap_or_default();
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(data) = json["data"].as_array() {
                            let models: Vec<String> = data.iter()
                                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                                .collect();
                            // 更新缓存
                            self.update_model_cache(endpoint_id, models.clone());
                            return Ok(models);
                        }
                    }
                }
                Ok(vec![])
            }
            Err(e) => {
                warn!("获取端点 {} 模型列表失败: {}", endpoint.config.name, e);
                Ok(vec![])
            }
        }
    }

    /// 匹配模型名称（不区分大小写，后缀匹配）
    /// 返回匹配到的端点实际模型名称
    pub fn match_model_name(&self, endpoint_id: &str, client_model: &str) -> Option<String> {
        let cache = self.model_cache.read();
        let models = cache.get(endpoint_id)?;

        // 首先检查是否完全一致（包括大小写），则无需替换
        if models.iter().any(|m| m == client_model) {
            return None;
        }

        // 模糊匹配：不区分大小写，去掉组织前缀后做最长包含匹配
        // 匹配度 = min(客户端长度, 端点长度)，取匹配度最高的
        // 支持：
        //   deepseek-ai/deepseek-v4-flash 匹配 DeepSeek-V4-Flash（相等）
        //   mimo-v2.5-pro 匹配 mimo-v2.5-pro-20260606（客户端被包含）
        //   mimo-v2.5-pro 匹配 mimo-v2.5（端点被包含，但匹配度较低）
        let client_lower = client_model.to_lowercase();
        let client_suffix = client_lower.split('/').last().unwrap_or(&client_lower);
        
        let mut best_match: Option<(&String, usize)> = None;
        for m in models.iter() {
            let m_lower = m.to_lowercase();
            let m_suffix = m_lower.split('/').last().unwrap_or(&m_lower);
            
            // 计算匹配度：如果一方包含另一方，取较短方的长度
            let match_len = if m_suffix.contains(client_suffix) {
                // 端点包含客户端（如 mimo-v2.5-pro-20260606 包含 mimo-v2.5-pro）
                client_suffix.len()
            } else if client_suffix.contains(m_suffix) {
                // 客户端包含端点（如 mimo-v2.5-pro 包含 mimo-v2.5）
                m_suffix.len()
            } else {
                0
            };
            
            if match_len > 0 {
                let is_better = match best_match {
                    None => true,
                    Some((_, best_len)) => match_len > best_len,
                };
                if is_better {
                    best_match = Some((m, match_len));
                }
            }
        }
        
        let matches: Vec<&String> = best_match.map(|(m, _)| vec![m]).unwrap_or_default();

        match matches.len() {
            0 => None,
            1 => Some(matches[0].clone()),
            _ => {
                // 匹配到多个模型，返回错误信息
                warn!("模型 '{}' 匹配到多个模型: {:?}", client_model, matches);
                Some("ERROR:模型名称参数有误，匹配到多个模型，请修改后重试".to_string())
            }
        }
    }

    /// 获取端点在指定池中匹配的模型名称
    pub fn resolve_model_for_endpoint(
        &self,
        pool: &Pool,
        endpoint: &EndpointState,
        client_model: &str,
    ) -> String {
        // 映射模式：使用手动配置的映射关系（精确匹配）
        if pool.model_mode == ModelMode::Mapping {
            if let Some(mapping) = endpoint.config.model_mappings.iter().find(|m| m.client_model == client_model) {
                return mapping.endpoint_model.clone();
            }
            // 没有找到映射，返回原始名称
            return client_model.to_string();
        }

        // 透传模式：自动匹配模型名称（模糊匹配，不区分大小写）
        // 先检查是否有缓存，如果没有则尝试获取
        if self.get_cached_models(&endpoint.config.id).is_none() {
            // 缓存未命中，返回原始名称（异步获取缓存由调用方处理）
            return client_model.to_string();
        }

        // 尝试从缓存中匹配，返回端点模型列表中的实际名称
        self.match_model_name(&endpoint.config.id, client_model)
            .unwrap_or_else(|| client_model.to_string())
    }
}
