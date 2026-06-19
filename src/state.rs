use crate::models::*;
use chrono::Utc;
use parking_lot::RwLock;
use std::collections::HashMap;
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

        Ok(Self {
            config: RwLock::new(config),
            endpoints: RwLock::new(endpoints),
            scheduler_index: RwLock::new(HashMap::new()),
            failover_index: RwLock::new(HashMap::new()),
            model_cache: RwLock::new(HashMap::new()),
            http_client,
            config_manager,
        })
    }

    /// 获取指定池中可用的端点ID列表
    pub fn available_endpoint_ids_in_pool(&self, pool_id: &str) -> Vec<String> {
        let endpoints = self.endpoints.read();
        endpoints
            .values()
            .filter(|ep| {
                ep.is_available() && ep.config.pool_id.as_deref() == Some(pool_id)
            })
            .map(|ep| ep.config.id.clone())
            .collect()
    }

    /// 获取所有可用的端点ID列表
    pub fn available_endpoint_ids(&self) -> Vec<String> {
        let endpoints = self.endpoints.read();
        endpoints
            .values()
            .filter(|ep| ep.is_available())
            .map(|ep| ep.config.id.clone())
            .collect()
    }

    /// 获取端点状态
    pub fn get_endpoint(&self, id: &str) -> Option<EndpointState> {
        let endpoints = self.endpoints.read();
        endpoints.get(id).cloned()
    }

    /// 获取所有端点状态
    pub fn get_all_endpoints(&self) -> Vec<EndpointState> {
        let endpoints = self.endpoints.read();
        endpoints.values().cloned().collect()
    }

    /// 更新端点token使用量
    pub fn update_endpoint_tokens(&self, id: &str, tokens: u64) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            ep.add_tokens(tokens);
            tracing::debug!("端点 {} 消耗 {} tokens, 总计: {}/{}", id, tokens, ep.tokens_used, ep.config.token_limit);
        }
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
            pool_id: req.pool_id,
            timeout: req.timeout.unwrap_or(300),
            model_mappings: req.model_mappings,
        };
        let state = EndpointState::new(config.clone());

        {
            let mut endpoints = self.endpoints.write();
            endpoints.insert(id.clone(), state.clone());
        }

        // 更新并保存配置
        {
            let mut app_config = self.config.write();
            app_config.endpoints.push(config);
            self.config_manager.save(&app_config).await?;
        }

        info!("已添加端点: {} ({})", state.config.name, id);
        Ok(state)
    }

    /// 更新端点
    pub async fn update_endpoint(&self, id: &str, req: EndpointRequest) -> anyhow::Result<EndpointState> {
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
        // 更新 pool_id: Some("") 表示清除，Some(非空) 表示设置，None 表示不更新
        match req.pool_id {
            Some(ref id) if id.is_empty() => {
                ep.config.pool_id = None;
            }
            Some(_) => {
                ep.config.pool_id = req.pool_id;
            }
            None => {
                // 不更新 pool_id
            }
        }
        if let Some(timeout) = req.timeout {
            ep.config.timeout = timeout;
        }
        // 更新模型映射
        if !req.model_mappings.is_empty() {
            ep.config.model_mappings = req.model_mappings;
        }
        let state = ep.clone();

        // 更新配置
        drop(endpoints);
        let mut app_config = self.config.write();
        if let Some(ep_config) = app_config.endpoints.iter_mut().find(|e| e.id == id) {
            *ep_config = state.config.clone();
        }
        self.config_manager.save(&app_config).await?;

        info!("已更新端点: {} ({})", state.config.name, id);
        Ok(state)
    }

    /// 删除端点
    pub async fn delete_endpoint(&self, id: &str) -> anyhow::Result<()> {
        {
            let mut endpoints = self.endpoints.write();
            endpoints.remove(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        }

        let mut app_config = self.config.write();
        app_config.endpoints.retain(|e| e.id != id);
        self.config_manager.save(&app_config).await?;

        info!("已删除端点: {}", id);
        Ok(())
    }

    /// 切换端点启用状态
    pub async fn toggle_endpoint(&self, id: &str) -> anyhow::Result<EndpointState> {
        let mut endpoints = self.endpoints.write();
        let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        ep.config.enabled = !ep.config.enabled;
        let state = ep.clone();

        drop(endpoints);
        let mut app_config = self.config.write();
        if let Some(ep_config) = app_config.endpoints.iter_mut().find(|e| e.id == id) {
            ep_config.enabled = state.config.enabled;
        }
        self.config_manager.save(&app_config).await?;

        info!("端点 {} 已{}", state.config.name, if state.config.enabled { "启用" } else { "禁用" });
        Ok(state)
    }

    /// 重置指定端点的token使用量
    pub async fn reset_endpoint_tokens(&self, id: &str) -> anyhow::Result<()> {
        let mut endpoints = self.endpoints.write();
        let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        ep.reset();
        info!("已重置端点 {} 的token使用量", ep.config.name);
        Ok(())
    }

    /// 重置所有端点的token使用量
    pub fn reset_all_tokens(&self) {
        let mut endpoints = self.endpoints.write();
        for ep in endpoints.values_mut() {
            ep.reset();
        }
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
            exposed_api_id: req.exposed_api_id,
            created_at: Utc::now(),
        };

        {
            let mut config = self.config.write();
            config.pools.push(pool.clone());
            self.config_manager.save(&config).await?;
        }

        info!("已添加池: {} ({})", pool.name, id);
        Ok(pool)
    }

    /// 更新池
    pub async fn update_pool(&self, id: &str, req: PoolRequest) -> anyhow::Result<Pool> {
        let mut config = self.config.write();
        let pool = config.pools.iter_mut().find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("池不存在: {}", id))?;

        pool.name = req.name;
        if let Some(desc) = req.description {
            pool.description = desc;
        }
        pool.schedule_algorithm = req.schedule_algorithm;
        pool.model_mode = req.model_mode;
        pool.exposed_api_id = req.exposed_api_id;
        let pool_clone = pool.clone();

        self.config_manager.save(&config).await?;
        info!("已更新池: {} ({})", pool_clone.name, id);
        Ok(pool_clone)
    }

    /// 删除池
    pub async fn delete_pool(&self, id: &str) -> anyhow::Result<()> {
        let mut config = self.config.write();
        config.pools.retain(|p| p.id != id);
        // 清除关联的端点
        for ep in config.endpoints.iter_mut() {
            if ep.pool_id.as_deref() == Some(id) {
                ep.pool_id = None;
            }
        }
        // 清除关联的对外API
        config.exposed_apis.retain(|a| a.pool_id != id);
        self.config_manager.save(&config).await?;

        info!("已删除池: {}", id);
        Ok(())
    }

    /// 获取池信息
    pub fn get_pool(&self, id: &str) -> Option<Pool> {
        let config = self.config.read();
        config.pools.iter().find(|p| p.id == id).cloned()
    }

    /// 获取所有池
    pub fn get_all_pools(&self) -> Vec<Pool> {
        let config = self.config.read();
        config.pools.clone()
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

        {
            let mut config = self.config.write();
            config.exposed_apis.push(api.clone());
            self.config_manager.save(&config).await?;
        }

        info!("已添加对外API: {} ({})", api.name, id);
        Ok(api)
    }

    /// 更新对外API
    pub async fn update_exposed_api(&self, id: &str, req: ExposedApiRequest) -> anyhow::Result<ExposedApi> {
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
        let api_clone = api.clone();

        self.config_manager.save(&config).await?;
        info!("已更新对外API: {} ({})", api_clone.name, id);
        Ok(api_clone)
    }

    /// 删除对外API
    pub async fn delete_exposed_api(&self, id: &str) -> anyhow::Result<()> {
        let mut config = self.config.write();
        config.exposed_apis.retain(|a| a.id != id);
        self.config_manager.save(&config).await?;

        info!("已删除对外API: {}", id);
        Ok(())
    }

    /// 切换对外API启用状态
    pub async fn toggle_exposed_api(&self, id: &str) -> anyhow::Result<ExposedApi> {
        let mut config = self.config.write();
        let api = config.exposed_apis.iter_mut().find(|a| a.id == id)
            .ok_or_else(|| anyhow::anyhow!("对外API不存在: {}", id))?;

        api.enabled = !api.enabled;
        let api_clone = api.clone();

        self.config_manager.save(&config).await?;
        info!("对外API {} 已{}", api_clone.name, if api_clone.enabled { "启用" } else { "禁用" });
        Ok(api_clone)
    }

    /// 获取对外API
    pub fn get_exposed_api(&self, id: &str) -> Option<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.iter().find(|a| a.id == id).cloned()
    }

    /// 根据前缀获取对外API
    pub fn get_exposed_api_by_prefix(&self, prefix: &str) -> Option<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.iter().find(|a| a.prefix == prefix && a.enabled).cloned()
    }

    /// 获取所有对外API
    pub fn get_all_exposed_apis(&self) -> Vec<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.clone()
    }

    /// 根据请求路径匹配对外API
    pub fn match_exposed_api(&self, path: &str) -> Option<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.iter()
            .filter(|a| a.enabled)
            .find(|a| path.starts_with(&a.prefix))
            .cloned()
    }

    // ========== 统计 ==========

    /// 获取统计信息
    pub fn get_stats(&self) -> StatsInfo {
        let endpoints = self.endpoints.read();
        let config = self.config.read();

        let active_count = endpoints.values().filter(|ep| ep.is_available()).count();
        // 排除无限制（999999999999）的端点
        let limited_endpoints: Vec<_> = endpoints.values()
            .filter(|ep| ep.config.token_limit != 999999999999)
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
                pool_id: ep.config.pool_id.clone(),
                timeout: ep.config.timeout,
                model_mappings: ep.config.model_mappings.clone(),
            })
            .collect();

        let pool_infos: Vec<PoolInfo> = config.pools.iter().map(|pool| {
            let pool_endpoints: Vec<_> = endpoints.values()
                .filter(|ep| ep.config.pool_id.as_deref() == Some(&pool.id))
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
                .filter(|ep| ep.config.pool_id.as_deref() == Some(&api.pool_id))
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

    /// 更新管理密码
    pub async fn change_admin_password(&self, new_password: &str) -> anyhow::Result<()> {
        let mut config = self.config.write();
        config.admin_password = new_password.to_string();
        self.config_manager.save(&config).await?;
        Ok(())
    }

    /// 执行每日重置检查
    pub async fn check_daily_reset(&self) {
        let mut endpoints = self.endpoints.write();
        let now = Utc::now();
        for ep in endpoints.values_mut() {
            if ep.config.reset_policy == ResetPolicy::Daily {
                let last_reset_date = ep.last_reset.date_naive();
                let today = now.date_naive();
                if last_reset_date < today {
                    ep.reset();
                    info!("已自动重置端点 {} 的每日token额度", ep.config.name);
                }
            }
        }
    }

    /// 获取池的调度算法
    pub fn get_pool_schedule_algorithm(&self, pool_id: &str) -> ScheduleAlgorithm {
        let config = self.config.read();
        config.pools.iter()
            .find(|p| p.id == pool_id)
            .map(|p| p.schedule_algorithm.clone())
            .unwrap_or(ScheduleAlgorithm::RoundRobin)
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

    /// 清除端点的模型列表缓存
    pub fn clear_model_cache(&self, endpoint_id: &str) {
        let mut cache = self.model_cache.write();
        cache.remove(endpoint_id);
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

        // 首先检查是否完全一致（包括大小写）
        if models.iter().any(|m| m == client_model) {
            return Some(client_model.to_string());
        }

        // 不区分大小写的后缀匹配
        let client_lower = client_model.to_lowercase();
        for model in models {
            let model_lower = model.to_lowercase();
            // 后缀匹配：模型名称以客户端名称结尾，或客户端名称以模型名称结尾
            if model_lower.ends_with(&client_lower) || client_lower.ends_with(&model_lower) {
                return Some(model.clone());
            }
        }

        None
    }

    /// 获取端点在指定池中匹配的模型名称
    pub fn resolve_model_for_endpoint(
        &self,
        pool: &Pool,
        endpoint: &EndpointState,
        client_model: &str,
    ) -> String {
        // 透传模式：直接返回客户端的模型名称
        if pool.model_mode == ModelMode::Passthrough {
            return client_model.to_string();
        }

        // 映射模式：先检查手动配置的映射
        if let Some(mapping) = endpoint.config.model_mappings.iter().find(|m| m.client_model == client_model) {
            return mapping.endpoint_model.clone();
        }

        // 尝试从缓存中匹配
        if let Some(matched) = self.match_model_name(&endpoint.config.id, client_model) {
            return matched;
        }

        // 如果没有匹配到，返回原始名称
        client_model.to_string()
    }
}
