use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 接口类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ApiType {
    OpenAI,
    Anthropic,
    #[serde(rename = "openai-responses")]
    OpenAIResponses,
}

impl std::fmt::Display for ApiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiType::OpenAI => write!(f, "openai"),
            ApiType::Anthropic => write!(f, "anthropic"),
            ApiType::OpenAIResponses => write!(f, "openai-responses"),
        }
    }
}

/// 调度算法
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleAlgorithm {
    /// 轮询：依次转发，跳过耗尽端点
    RoundRobin,
    /// 轮换：用完一个再换下一个
    Failover,
    /// 随机：随机选择，失败自动重试下一个
    Random,
}

impl std::fmt::Display for ScheduleAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleAlgorithm::RoundRobin => write!(f, "round_robin"),
            ScheduleAlgorithm::Failover => write!(f, "failover"),
            ScheduleAlgorithm::Random => write!(f, "random"),
        }
    }
}

/// 限额重置方式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResetPolicy {
    /// 一次性手动重置
    Manual,
    /// 每日零点自动重置
    Daily,
}

/// 代理端点配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub api_type: ApiType,
    pub api_key: String,
    pub token_limit: u64,
    pub reset_policy: ResetPolicy,
    pub enabled: bool,
    /// 所属池ID
    pub pool_id: Option<String>,
    /// 超时时间（秒）
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    300
}

/// 端点运行时状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointState {
    pub config: EndpointConfig,
    pub tokens_used: u64,
    pub last_reset: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub error_count: u32,
    pub total_requests: u64,
}

impl EndpointState {
    pub fn new(config: EndpointConfig) -> Self {
        Self {
            config,
            tokens_used: 0,
            last_reset: Utc::now(),
            last_used: None,
            error_count: 0,
            total_requests: 0,
        }
    }

    pub fn is_available(&self) -> bool {
        self.config.enabled && (self.config.token_limit == 0 || self.tokens_used < self.config.token_limit)
    }

    pub fn tokens_remaining(&self) -> u64 {
        if self.config.token_limit == 0 {
            u64::MAX // 无上限
        } else if self.tokens_used >= self.config.token_limit {
            0
        } else {
            self.config.token_limit - self.tokens_used
        }
    }

    pub fn add_tokens(&mut self, amount: u64) {
        self.tokens_used += amount;
        self.last_used = Some(Utc::now());
        self.total_requests += 1;
    }

    pub fn reset(&mut self) {
        self.tokens_used = 0;
        self.last_reset = Utc::now();
        self.error_count = 0;
    }
}

/// 代理端点池
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 调度算法
    pub schedule_algorithm: ScheduleAlgorithm,
    /// 关联的对外API ID
    pub exposed_api_id: Option<String>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

/// 对外暴露的API接口
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposedApi {
    pub id: String,
    pub name: String,
    /// URL前缀，如 /v1, /api/gpt4
    pub prefix: String,
    /// 接口类型
    pub api_type: ApiType,
    /// 认证密钥（为空则不需要认证）
    pub api_key: Option<String>,
    /// 是否启用
    pub enabled: bool,
    /// 关联的池ID
    pub pool_id: String,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

/// 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// 监听地址
    pub listen_addr: String,
    /// 监听端口
    pub listen_port: u16,
    /// 管理后台密码
    pub admin_password: String,
    /// 代理端点列表
    pub endpoints: Vec<EndpointConfig>,
    /// 端点池列表
    pub pools: Vec<Pool>,
    /// 对外暴露的API列表
    pub exposed_apis: Vec<ExposedApi>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0".to_string(),
            listen_port: 8080,
            admin_password: "admin123".to_string(),
            endpoints: Vec::new(),
            pools: Vec::new(),
            exposed_apis: Vec::new(),
        }
    }
}

/// 端点创建/更新请求
#[derive(Debug, Deserialize)]
pub struct EndpointRequest {
    pub name: String,
    pub url: String,
    pub api_type: ApiType,
    pub api_key: String,
    pub token_limit: u64,
    pub reset_policy: ResetPolicy,
    pub enabled: Option<bool>,
    pub pool_id: Option<String>,
    pub timeout: Option<u64>,
    /// 测试时指定的模型名称（可选）
    #[serde(default)]
    pub model: Option<String>,
}

/// 池创建/更新请求
#[derive(Debug, Deserialize)]
pub struct PoolRequest {
    pub name: String,
    pub description: Option<String>,
    pub schedule_algorithm: ScheduleAlgorithm,
    pub exposed_api_id: Option<String>,
}

/// 对外API创建/更新请求
#[derive(Debug, Deserialize)]
pub struct ExposedApiRequest {
    pub name: String,
    pub prefix: String,
    pub api_type: ApiType,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
    pub pool_id: String,
}

/// 全局配置更新请求
#[derive(Debug, Deserialize)]
pub struct ConfigUpdateRequest {
    pub admin_password: Option<String>,
}

/// 统计信息
#[derive(Debug, Serialize)]
pub struct StatsInfo {
    pub total_endpoints: usize,
    pub active_endpoints: usize,
    pub total_tokens_used: u64,
    pub total_tokens_limit: u64,
    pub total_requests: u64,
    pub total_pools: usize,
    pub total_exposed_apis: usize,
    pub endpoints: Vec<EndpointStats>,
    pub pools: Vec<PoolInfo>,
    pub exposed_apis: Vec<ExposedApiInfo>,
}

#[derive(Debug, Serialize)]
pub struct EndpointStats {
    pub id: String,
    pub name: String,
    pub url: String,
    pub api_type: ApiType,
    pub tokens_used: u64,
    pub token_limit: u64,
    pub tokens_remaining: u64,
    pub enabled: bool,
    pub last_used: Option<DateTime<Utc>>,
    pub total_requests: u64,
    pub error_count: u32,
    pub pool_id: Option<String>,
    pub timeout: u64,
}

#[derive(Debug, Serialize)]
pub struct PoolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schedule_algorithm: ScheduleAlgorithm,
    pub exposed_api_id: Option<String>,
    pub endpoint_count: usize,
    pub active_endpoint_count: usize,
    pub total_tokens_used: u64,
    pub total_requests: u64,
}

#[derive(Debug, Serialize)]
pub struct ExposedApiInfo {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub api_type: ApiType,
    pub enabled: bool,
    pub pool_id: String,
    pub pool_name: Option<String>,
    pub endpoint_count: usize,
}
