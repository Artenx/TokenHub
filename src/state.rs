use crate::models::*;
use chrono::Utc;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use tracing::{debug, info, warn};

/// state.json 中存储调用日志的特殊键（使用内部前缀，避免与端点 UUID 冲突）
const CALL_LOGS_STATE_KEY: &str = "__internal:call_logs";
/// state.json 中存储延迟采样的特殊键
const LATENCIES_STATE_KEY: &str = "__internal:endpoint_latencies";

/// IP 限流条目
pub(crate) struct RateLimitEntry {
    window_start: Instant,
    count: u64,
    blocked_until: Option<Instant>,
}

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
    /// 运行时状态版本号，保存前读取，保存后若版本未变才清零 dirty，
    /// 防止保存期间新变更被丢失。
    pub dirty_version: AtomicU64,
    /// 持久化写锁（防止并发写入导致数据覆盖）
    pub save_state_mutex: tokio::sync::Mutex<()>,
    /// 管理后台会话令牌集合
    pub admin_sessions: RwLock<HashSet<String>>,
    /// 最近 API 调用日志（最多保留 50 条）
    pub call_logs: RwLock<VecDeque<ApiCallLog>>,
    /// IP 请求频率限制
    pub rate_limits: RwLock<HashMap<String, RateLimitEntry>>,
    /// 端点延迟采样（每个端点最多保留 100 条，毫秒）
    pub endpoint_latencies: RwLock<HashMap<String, VecDeque<u64>>>,
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
            dirty_version: AtomicU64::new(0),
            save_state_mutex: tokio::sync::Mutex::new(()),
            admin_sessions: RwLock::new(HashSet::new()),
            call_logs: RwLock::new(VecDeque::new()),
            rate_limits: RwLock::new(HashMap::new()),
            endpoint_latencies: RwLock::new(HashMap::new()),
        };

        // 从 state.json 恢复运行时状态
        app.load_runtime_state().await;

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

    /// 获取指定池中所有端点ID列表（含禁用和耗尽的端点）
    pub fn endpoint_ids_in_pool(&self, pool_id: &str) -> Vec<String> {
        let endpoints = self.endpoints.read();
        endpoints
            .values()
            .filter(|ep| ep.config.pool_ids.contains(&pool_id.to_string()))
            .map(|ep| ep.config.id.clone())
            .collect()
    }

    /// 获取端点状态
    pub fn get_endpoint(&self, id: &str) -> Option<EndpointState> {
        let endpoints = self.endpoints.read();
        endpoints.get(id).cloned()
    }

    /// 更新端点token使用量（请求计数已在选择端点时预留）
    pub fn update_endpoint_tokens(&self, id: &str, tokens: u64) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            ep.add_tokens(tokens);
            tracing::debug!("端点 {} 消耗 {} tokens, 总计: {}/{}", id, tokens, ep.tokens_used, ep.config.token_limit);
        }
        self.mark_dirty();
    }

    /// 为指定端点预留一次请求额度，返回是否预留成功
    pub fn reserve_endpoint_request(&self, id: &str) -> bool {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            let ok = ep.try_reserve_request();
            if ok {
                self.mark_dirty();
            }
            return ok;
        }
        false
    }

    /// 释放指定端点已预留的请求额度（转发失败时回滚）
    pub fn release_endpoint_request(&self, id: &str) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            ep.release_request();
            self.mark_dirty();
        }
    }

    /// 增加端点错误计数
    pub fn increment_endpoint_errors(&self, id: &str) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.get_mut(id) {
            ep.error_count += 1;
        }
        self.mark_dirty();
    }

    /// 添加端点
    pub async fn add_endpoint(&self, req: EndpointRequest) -> anyhow::Result<EndpointState> {
        // 检查名称是否重复
        {
            let config = self.config.read();
            if config.endpoints.iter().any(|e| e.name == req.name) {
                anyhow::bail!("端点名称已存在: {}", req.name);
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let config = EndpointConfig {
            id: id.clone(),
            name: req.name,
            url: req.url,
            api_type: req.api_type,
            api_key: req.api_key,
            token_limit: req.token_limit,
            reset_policy: req.reset_policy,
            request_limit: req.request_limit,
            request_reset_policy: req.request_reset_policy,
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
        // 检查名称是否与其他端点重复
        {
            let config = self.config.read();
            if config.endpoints.iter().any(|e| e.id != id && e.name == req.name) {
                anyhow::bail!("端点名称已存在: {}", req.name);
            }
        }
        let state = {
            let mut endpoints = self.endpoints.write();
            let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;

            ep.config.name = req.name;
            ep.config.url = req.url;
            ep.config.api_type = req.api_type;
            ep.config.api_key = req.api_key;
            ep.config.token_limit = req.token_limit;
            ep.config.reset_policy = req.reset_policy;
            ep.config.request_limit = req.request_limit;
            ep.config.request_reset_policy = req.request_reset_policy;
            if let Some(enabled) = req.enabled {
                ep.config.enabled = enabled;
            }
            // 更新 pool_ids
            ep.config.pool_ids = req.pool_ids;
            if let Some(timeout) = req.timeout {
                ep.config.timeout = timeout;
            }
            // 更新模型映射（允许传空数组清空映射）
            ep.config.model_mappings = req.model_mappings;
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

        // URL / API Key / 类型变更后模型缓存可能失效，清除以强制刷新
        {
            let mut cache = self.model_cache.write();
            cache.remove(id);
        }

        info!("已更新端点: {} ({})", state.config.name, id);
        Ok(state)
    }

    /// 删除端点
    pub async fn delete_endpoint(&self, id: &str) -> anyhow::Result<()> {
        {
            let mut endpoints = self.endpoints.write();
            endpoints.remove(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        }

        // 同步清理模型缓存
        {
            let mut cache = self.model_cache.write();
            cache.remove(id);
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

    /// 重置指定端点的请求次数（保留token使用量）
    pub async fn reset_endpoint_requests(&self, id: &str) -> anyhow::Result<()> {
        let mut endpoints = self.endpoints.write();
        let ep = endpoints.get_mut(id).ok_or_else(|| anyhow::anyhow!("端点不存在: {}", id))?;
        ep.reset_requests();
        self.mark_dirty();
        info!("已重置端点 {} 的请求次数", ep.config.name);
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
        // 检查名称是否重复
        {
            let config = self.config.read();
            if config.pools.iter().any(|p| p.name == req.name) {
                anyhow::bail!("池名称已存在: {}", req.name);
            }
        }
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
        // 检查名称是否与其他池重复
        {
            let config = self.config.read();
            if config.pools.iter().any(|p| p.id != id && p.name == req.name) {
                anyhow::bail!("池名称已存在: {}", req.name);
            }
        }
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

    /// 获取对外API信息
    pub fn get_exposed_api(&self, id: &str) -> Option<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.iter().find(|a| a.id == id).cloned()
    }

    // ========== 对外API管理 ==========

    /// 添加对外API
    pub async fn add_exposed_api(&self, req: ExposedApiRequest) -> anyhow::Result<ExposedApi> {
        // 检查名称是否重复
        {
            let config = self.config.read();
            if config.exposed_apis.iter().any(|a| a.name == req.name) {
                anyhow::bail!("对外API名称已存在: {}", req.name);
            }
        }
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
        // 检查名称是否与其他对外API重复
        {
            let config = self.config.read();
            if config.exposed_apis.iter().any(|a| a.id != id && a.name == req.name) {
                anyhow::bail!("对外API名称已存在: {}", req.name);
            }
        }
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
    /// 要求前缀后面必须是 '/' 或路径结束，避免 /v1 匹配 /v1xyz
    pub fn match_exposed_api(&self, path: &str) -> Option<ExposedApi> {
        let config = self.config.read();
        config.exposed_apis.iter()
            .filter(|a| {
                if !a.enabled || !path.starts_with(&a.prefix) {
                    return false;
                }
                // 前缀必须完整匹配：前缀后必须是路径结束或 '/'
                path.len() == a.prefix.len() || path[a.prefix.len()..].starts_with('/')
            })
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
        let total_tokens_consumed: u64 = endpoints.values().map(|ep| ep.total_tokens_used).sum();
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
                total_tokens_consumed: ep.total_tokens_used,
                token_limit: ep.config.token_limit,
                tokens_remaining: ep.tokens_remaining(),
                enabled: ep.config.enabled,
                request_limit: ep.config.request_limit,
                requests_used: ep.requests_used,
                requests_remaining: ep.requests_remaining(),
                request_reset_policy: ep.config.request_reset_policy.clone(),
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
            total_tokens_consumed,
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
        self.dirty_version.fetch_add(1, Ordering::SeqCst);
    }

    /// 从 state.json 加载端点的运行时状态
    pub async fn load_runtime_state(&self) {
        let path = &self.state_path;
        if !path.exists() {
            return;
        }
        let content = match tokio::fs::read_to_string(path).await {
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
        let mut endpoints = self.endpoints.write();
        let mut call_logs = self.call_logs.write();
        let mut latencies = self.endpoint_latencies.write();
        let mut restored_endpoints = 0usize;
        for (id, state_data) in &data {
            // 内部状态键：调用日志
            if id == CALL_LOGS_STATE_KEY {
                if let Ok(logs) = serde_json::from_value::<VecDeque<ApiCallLog>>(state_data.clone()) {
                    *call_logs = logs;
                    while call_logs.len() > 50 {
                        call_logs.pop_back();
                    }
                }
                continue;
            }
            // 内部状态键：端点延迟采样
            if id == LATENCIES_STATE_KEY {
                if let Ok(samples) = serde_json::from_value::<HashMap<String, VecDeque<u64>>>(state_data.clone()) {
                    *latencies = samples.into_iter().map(|(k, mut v)| {
                        while v.len() > 100 {
                            v.pop_front();
                        }
                        (k, v)
                    }).collect();
                }
                continue;
            }
            if let Some(ep) = endpoints.get_mut(id) {
                restored_endpoints += 1;
                if let Some(tokens) = state_data.get("tokens_used").and_then(|v| v.as_u64()) {
                    ep.tokens_used = tokens;
                }
                if let Some(tokens) = state_data.get("total_tokens_used").and_then(|v| v.as_u64()) {
                    ep.total_tokens_used = tokens;
                }
                if let Some(reset_str) = state_data.get("last_reset").and_then(|v| v.as_str()) {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_str) {
                        ep.last_reset = dt.with_timezone(&chrono::Utc);
                    }
                }
                if let Some(reset_str) = state_data.get("request_last_reset").and_then(|v| v.as_str()) {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_str) {
                        ep.request_last_reset = dt.with_timezone(&chrono::Utc);
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
                if let Some(reqs_used) = state_data.get("requests_used").and_then(|v| v.as_u64()) {
                    ep.requests_used = reqs_used;
                }
                if let Some(hist) = state_data.get("token_history") {
                    if let Ok(mut v) = serde_json::from_value::<Vec<(chrono::DateTime<Utc>, u64)>>(hist.clone()) {
                        v.sort_by_key(|a| a.0);
                        ep.token_history = v;
                    }
                }
                if let Some(hist) = state_data.get("request_history") {
                    if let Ok(mut v) = serde_json::from_value::<Vec<(chrono::DateTime<Utc>, u64)>>(hist.clone()) {
                        v.sort_by_key(|a| a.0);
                        ep.request_history = v;
                    }
                }
            }
        }
        info!("已从状态文件恢复 {} 个端点的运行时状态、{} 条调用日志、{} 个端点的延迟采样",
            restored_endpoints, call_logs.len(), latencies.len());
    }

    /// 异步保存运行时状态到 state.json
    pub async fn save_runtime_state(&self) -> anyhow::Result<()> {
        let _guard = self.save_state_mutex.lock().await;
        // 保存前记录版本；若保存期间版本变化，则不能清 dirty，避免丢失更新。
        let start_version = self.dirty_version.load(Ordering::SeqCst);
        let mut data: HashMap<String, serde_json::Value> = {
            let endpoints = self.endpoints.read();
            endpoints.iter().map(|(id, ep)| {
                (id.clone(), serde_json::json!({
                    "tokens_used": ep.tokens_used,
                    "total_tokens_used": ep.total_tokens_used,
                    "last_reset": ep.last_reset,
                    "request_last_reset": ep.request_last_reset,
                    "last_used": ep.last_used,
                    "error_count": ep.error_count,
                    "total_requests": ep.total_requests,
                    "requests_used": ep.requests_used,
                    "token_history": ep.token_history,
                    "request_history": ep.request_history,
                }))
            }).collect()
        };
        {
            let call_logs = self.call_logs.read();
            if !call_logs.is_empty() {
                data.insert(CALL_LOGS_STATE_KEY.to_string(), serde_json::to_value(&*call_logs)?);
            }
        }
        {
            let latencies = self.endpoint_latencies.read();
            if !latencies.is_empty() {
                data.insert(LATENCIES_STATE_KEY.to_string(), serde_json::to_value(&*latencies)?);
            }
        }
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| anyhow::anyhow!("序列化运行时状态失败: {}", e))?;
        if let Some(parent) = self.state_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&self.state_path, json).await
            .map_err(|e| anyhow::anyhow!("写入状态文件失败: {}", e))?;
        // 只有保存期间没有新变更时，才清 dirty；否则保留 dirty，等待下次保存。
        if self.dirty_version.load(Ordering::SeqCst) == start_version {
            self.dirty.store(false, Ordering::Release);
        }
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

    /// 执行自动重置检查（每日零点和每分钟）
    pub async fn check_auto_reset(&self) {
        let mut endpoints = self.endpoints.write();
        let now = Utc::now();
        let today = now.date_naive();

        for ep in endpoints.values_mut() {
            let token_daily = ep.config.reset_policy == ResetPolicy::Daily && ep.config.token_limit > 0;
            let request_daily = ep.config.request_reset_policy == ResetPolicy::Daily && ep.config.request_limit > 0;
            let request_minutely = ep.config.request_reset_policy == ResetPolicy::Minutely && ep.config.request_limit > 0;

            let needs_daily_reset = ep.last_reset.date_naive() < today && (token_daily || request_daily);
            let current_minute = now.timestamp() / 60;
            let reset_minute = ep.request_last_reset.timestamp() / 60;
            let needs_minutely_reset = request_minutely && current_minute > reset_minute;

            if !needs_daily_reset && !needs_minutely_reset {
                continue;
            }

            if needs_daily_reset && token_daily {
                ep.tokens_used = 0;
                ep.token_history.clear();
                self.mark_dirty();
                info!("端点 {} 每日自动重置Token", ep.config.name);
            }

            if needs_daily_reset && request_daily {
                ep.requests_used = 0;
                ep.request_history.clear();
                self.mark_dirty();
                info!("端点 {} 请求次数每日自动重置", ep.config.name);
            }

            if needs_minutely_reset {
                ep.requests_used = 0;
                ep.request_history.clear();
                ep.request_last_reset = now;
                self.mark_dirty();
                info!("端点 {} 请求次数每分钟自动重置", ep.config.name);
            }

            if needs_daily_reset {
                ep.last_reset = now;
            }
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

    // ========== 管理后台会话 ==========

    /// 创建新的管理后台会话，返回会话令牌
    pub fn create_admin_session(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let mut sessions = self.admin_sessions.write();
        sessions.insert(token.clone());
        token
    }

    /// 校验管理后台会话令牌是否有效
    pub fn validate_admin_session(&self, token: &str) -> bool {
        let sessions = self.admin_sessions.read();
        sessions.contains(token)
    }

    /// 销毁管理后台会话令牌
    pub fn destroy_admin_session(&self, token: &str) {
        let mut sessions = self.admin_sessions.write();
        sessions.remove(token);
    }

    /// 清除除指定令牌外的所有管理后台会话（修改密码后使用）
    pub fn clear_other_admin_sessions(&self, except_token: &str) {
        let mut sessions = self.admin_sessions.write();
        sessions.retain(|token| token == except_token);
    }

    // ========== 调用日志管理 ==========

    /// 添加一条 API 调用日志，最多保留 50 条最近记录
    pub fn add_call_log(&self, mut log: ApiCallLog) {
        // 兜底：如果只有 endpoint_id 没有 name，从端点配置中补全名称
        if log.endpoint_id.is_some() && log.endpoint_name.is_none() {
            let endpoint_id = log.endpoint_id.as_ref().unwrap();
            let endpoints = self.endpoints.read();
            if let Some(ep) = endpoints.get(endpoint_id) {
                log.endpoint_name = Some(ep.config.name.clone());
            }
        }
        let mut logs = self.call_logs.write();
        logs.push_front(log);
        while logs.len() > 50 {
            logs.pop_back();
        }
        self.mark_dirty();
    }

    /// 获取最近 50 条 API 调用日志
    pub fn get_call_logs(&self) -> Vec<ApiCallLog> {
        self.call_logs.read().iter().cloned().collect()
    }

    // ========== IP 限流 ==========

    /// 检查 IP 请求频率，返回 true 表示允许，false 表示被限流拒绝
    pub fn check_rate_limit(&self, ip: &str) -> bool {
        let mut limits = self.rate_limits.write();
        let now = Instant::now();
        let entry = limits.entry(ip.to_string()).or_insert(RateLimitEntry {
            window_start: now,
            count: 0,
            blocked_until: None,
        });

        // 检查是否处于封禁期（连续触发限流后封禁 60 秒）
        if let Some(blocked) = entry.blocked_until {
            if now < blocked {
                return false;
            }
            // 封禁期结束，重置
            entry.blocked_until = None;
            entry.window_start = now;
            entry.count = 0;
        }

        // 1 秒时间窗口
        if now.duration_since(entry.window_start) > std::time::Duration::from_secs(1) {
            entry.window_start = now;
            entry.count = 0;
        }

        entry.count += 1;

        // 每秒超过 3 个请求则限流，累计 10 次触发则封禁 60 秒
        if entry.count > 3 {
            if entry.count >= 10 {
                entry.blocked_until = Some(now + std::time::Duration::from_secs(60));
            }
            return false;
        }

        true
    }

    /// 清理过期的限流条目
    pub fn cleanup_rate_limits(&self) {
        let mut limits = self.rate_limits.write();
        let now = Instant::now();
        limits.retain(|_, entry| {
            if let Some(blocked) = entry.blocked_until {
                now < blocked
            } else {
                // 保留最近 5 分钟内活跃的条目
                now.duration_since(entry.window_start) < std::time::Duration::from_secs(300)
            }
        });
    }

    // ========== 端点延迟统计 ==========

    /// 记录一次端点请求延迟（毫秒）
    pub fn record_latency(&self, endpoint_id: &str, duration_ms: u64) {
        let mut latencies = self.endpoint_latencies.write();
        let samples = latencies.entry(endpoint_id.to_string()).or_insert_with(VecDeque::new);
        samples.push_back(duration_ms);
        while samples.len() > 100 {
            samples.pop_front();
        }
        self.mark_dirty();
    }

    /// 计算单个端点的延迟统计
    fn compute_latency_stats(samples: &[u64]) -> Option<(u64, u64, u64, u64, u64, u64)> {
        if samples.is_empty() {
            return None;
        }
        let min = *samples.iter().min()?;
        let max = *samples.iter().max()?;
        let sum: u64 = samples.iter().sum();
        let avg = sum / samples.len() as u64;

        let mut sorted = samples.to_vec();
        sorted.sort_unstable();

        fn percentile(sorted: &[u64], p: f64) -> u64 {
            if sorted.len() == 1 {
                return sorted[0];
            }
            let idx = (p / 100.0 * (sorted.len() - 1) as f64).ceil() as usize;
            sorted[idx.min(sorted.len() - 1)]
        }

        let p50 = percentile(&sorted, 50.0);
        let p90 = percentile(&sorted, 90.0);
        let p95 = percentile(&sorted, 95.0);

        Some((avg, min, max, p50, p90, p95))
    }

    /// 获取所有端点的延迟统计
    pub fn get_latency_stats(&self) -> Vec<crate::models::EndpointLatencyStats> {
        let latencies = self.endpoint_latencies.read();
        let endpoints = self.endpoints.read();

        endpoints
            .values()
            .map(|ep| {
                let samples = latencies.get(&ep.config.id).map(|v| v.as_slices()).unwrap_or_default();
                let (samples_a, samples_b) = samples;
                let mut all_samples = Vec::with_capacity(samples_a.len() + samples_b.len());
                all_samples.extend_from_slice(samples_a);
                all_samples.extend_from_slice(samples_b);

                let stats = Self::compute_latency_stats(&all_samples);
                let total_requests = ep.total_requests.max(1); // 避免除以 0
                let error_count = ep.error_count;
                let error_rate = (error_count as f64 / total_requests as f64) * 100.0;
                crate::models::EndpointLatencyStats {
                    endpoint_id: ep.config.id.clone(),
                    endpoint_name: ep.config.name.clone(),
                    enabled: ep.config.enabled,
                    samples: all_samples.len(),
                    avg_ms: stats.map(|(avg, _, _, _, _, _)| avg).unwrap_or(0),
                    min_ms: stats.map(|(_, min, _, _, _, _)| min).unwrap_or(0),
                    max_ms: stats.map(|(_, _, max, _, _, _)| max).unwrap_or(0),
                    p50_ms: stats.map(|(_, _, _, p50, _, _)| p50).unwrap_or(0),
                    p90_ms: stats.map(|(_, _, _, _, p90, _)| p90).unwrap_or(0),
                    p95_ms: stats.map(|(_, _, _, _, _, p95)| p95).unwrap_or(0),
                    total_requests: ep.total_requests,
                    error_count,
                    error_rate,
                }
            })
            .collect()
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

        let candidate_urls: Vec<String> = match endpoint.config.api_type {
            ApiType::Custom => {
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

        for models_url in &candidate_urls {
            let mut request_builder = self.http_client.get(models_url)
                .header("Content-Type", "application/json");

            match endpoint.config.api_type {
                ApiType::OpenAI | ApiType::OpenAIResponses | ApiType::Custom => {
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
                                self.update_model_cache(endpoint_id, models.clone());
                                return Ok(models);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("获取端点 {} 模型列表失败: {}", endpoint.config.name, e);
                }
            }
        }

        Ok(vec![])

    /// 匹配模型名称（不区分大小写，后缀匹配）
    /// 返回匹配到的端点实际模型名称
    /// 如果匹配到多个模型，返回 ERROR: 开头的错误提示
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
        let client_suffix = client_lower.split('/').next_back().unwrap_or(&client_lower);

        let mut best_len = 0usize;
        let mut matches: Vec<&String> = Vec::new();

        for m in models.iter() {
            let m_lower = m.to_lowercase();
            let m_suffix = m_lower.split('/').next_back().unwrap_or(&m_lower);

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

            if match_len == 0 {
                continue;
            }

            if match_len > best_len {
                best_len = match_len;
                matches.clear();
                matches.push(m);
            } else if match_len == best_len {
                matches.push(m);
            }
        }

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
