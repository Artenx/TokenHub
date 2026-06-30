use chrono::{DateTime, Duration, Utc};
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
    /// 随机：随机选择端点
    Random,
}

/// 重试模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetryMode {
    /// 无重试：异常直接返回
    #[default]
    None,
    /// 原地重试：异常时继续向原端点重试
    Same,
    /// 端点重试：异常时切换到池内其他端点
    Pool,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ResetPolicy {
    /// 一次性手动重置
    #[serde(rename = "manual")]
    #[default]
    Manual,
    /// 每日零点自动重置
    #[serde(rename = "daily")]
    Daily,
    /// 滚动5小时自动重置（仅统计最近5小时消耗）
    #[serde(rename = "Rolling5h", alias = "rolling5h")]
    Rolling5h,
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
    /// 请求次数限制（0 表示无上限）
    #[serde(default)]
    pub request_limit: u64,
    /// 请求次数重置方式
    #[serde(default)]
    pub request_reset_policy: ResetPolicy,
    pub enabled: bool,
    /// 所属池ID列表（支持多池）
    #[serde(default)]
    pub pool_ids: Vec<String>,
    /// 超时时间（秒）
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    /// 模型名称映射列表（用于映射模式）
    #[serde(default)]
    pub model_mappings: Vec<ModelMapping>,
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
    /// 滑动窗口历史：(时间戳, 当时的累计tokens_used)
    #[serde(default)]
    pub token_history: Vec<(DateTime<Utc>, u64)>,
    /// 已使用的请求次数（重置后归零）
    pub requests_used: u64,
    /// 请求滑动窗口历史：(时间戳, 当时的累计requests_used)
    #[serde(default)]
    pub request_history: Vec<(DateTime<Utc>, u64)>,
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
            token_history: Vec::new(),
            requests_used: 0,
            request_history: Vec::new(),
        }
    }

    /// 计算滚动5小时窗口内的有效 token 消耗量
    pub fn effective_tokens(&self) -> u64 {
        match self.config.reset_policy {
            ResetPolicy::Rolling5h => {
                let now = Utc::now();
                let window_start = now - Duration::hours(5);
                let mut tokens_before_window = 0u64;
                for (ts, cum_tokens) in &self.token_history {
                    if *ts <= window_start {
                        tokens_before_window = *cum_tokens;
                    } else {
                        break;
                    }
                }
                self.tokens_used.saturating_sub(tokens_before_window)
            }
            _ => self.tokens_used,
        }
    }

    /// 计算滚动窗口内的有效请求次数
    pub fn effective_requests(&self) -> u64 {
        match self.config.request_reset_policy {
            ResetPolicy::Rolling5h => {
                let now = Utc::now();
                let window_start = now - Duration::hours(5);
                let mut reqs_before_window = 0u64;
                for (ts, cum_reqs) in &self.request_history {
                    if *ts <= window_start {
                        reqs_before_window = *cum_reqs;
                    } else {
                        break;
                    }
                }
                self.requests_used.saturating_sub(reqs_before_window)
            }
            _ => self.requests_used,
        }
    }

    pub fn is_available(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        // 检查 token 限制
        if self.config.token_limit > 0 {
            let below_token_limit = match self.config.reset_policy {
                ResetPolicy::Rolling5h => self.effective_tokens() < self.config.token_limit,
                _ => self.tokens_used < self.config.token_limit,
            };
            if !below_token_limit {
                return false;
            }
        }

        // 检查请求次数限制
        if self.config.request_limit > 0 {
            let below_req_limit = match self.config.request_reset_policy {
                ResetPolicy::Rolling5h => self.effective_requests() < self.config.request_limit,
                _ => self.requests_used < self.config.request_limit,
            };
            if !below_req_limit {
                return false;
            }
        }

        true
    }

    pub fn tokens_remaining(&self) -> u64 {
        if self.config.token_limit == 0 {
            u64::MAX // 无上限
        } else {
            match self.config.reset_policy {
                ResetPolicy::Rolling5h => {
                    let used = self.effective_tokens();
                    self.config.token_limit.saturating_sub(used)
                }
                _ => self.config.token_limit.saturating_sub(self.tokens_used),
            }
        }
    }

    /// 计算剩余可用请求次数
    pub fn requests_remaining(&self) -> u64 {
        if self.config.request_limit == 0 {
            u64::MAX
        } else {
            match self.config.request_reset_policy {
                ResetPolicy::Rolling5h => {
                    let used = self.effective_requests();
                    self.config.request_limit.saturating_sub(used)
                }
                _ => self.config.request_limit.saturating_sub(self.requests_used),
            }
        }
    }

    pub fn add_tokens(&mut self, amount: u64) {
        self.tokens_used = self.tokens_used.saturating_add(amount);
        self.last_used = Some(Utc::now());
        self.total_requests = self.total_requests.saturating_add(1);
        self.requests_used = self.requests_used.saturating_add(1);

        // Token 滑动窗口记录
        if self.config.reset_policy == ResetPolicy::Rolling5h {
            let now = Utc::now();
            let cutoff = now - Duration::hours(6);
            self.token_history.retain(|(ts, _)| *ts > cutoff);
            let last_ts = self.token_history.last().map(|(ts, _)| *ts);
            if last_ts.map(|ts| now - ts > Duration::seconds(10)).unwrap_or(true) {
                self.token_history.push((now, self.tokens_used));
            }
        }

        // 请求次数滑动窗口记录
        if self.config.request_reset_policy == ResetPolicy::Rolling5h {
            let now = Utc::now();
            let cutoff = now - Duration::hours(6);
            self.request_history.retain(|(ts, _)| *ts > cutoff);
            let last_ts = self.request_history.last().map(|(ts, _)| *ts);
            if last_ts.map(|ts| now - ts > Duration::seconds(10)).unwrap_or(true) {
                self.request_history.push((now, self.requests_used));
            }
        }
    }

    /// 仅重置请求次数（保留token使用量）
    pub fn reset_requests(&mut self) {
        self.requests_used = 0;
        self.request_history.clear();
        self.last_reset = Utc::now();
    }

    pub fn reset(&mut self) {
        self.tokens_used = 0;
        self.last_reset = Utc::now();
        self.error_count = 0;
        self.token_history.clear();
        self.requests_used = 0;
        self.request_history.clear();
    }
}

/// 模型参数传递模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ModelMode {
    /// 透传模式：客户端直接使用端点支持的模型名称
    #[default]
    Passthrough,
    /// 映射模式：客户端使用统一名称，后端映射到端点实际模型
    Mapping,
}


/// 模型名称映射
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMapping {
    /// 客户端请求的模型名称
    pub client_model: String,
    /// 端点实际的模型名称
    pub endpoint_model: String,
}

/// 代理端点池
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 调度算法
    pub schedule_algorithm: ScheduleAlgorithm,
    /// 模型参数传递模式
    #[serde(default)]
    pub model_mode: ModelMode,
    /// 重试模式
    #[serde(default)]
    pub retry_mode: RetryMode,
    /// 重试次数
    #[serde(default = "default_retry_count")]
    pub retry_count: u32,
    /// 关联的对外API ID
    pub exposed_api_id: Option<String>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

fn default_retry_count() -> u32 {
    1
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
    /// 请求次数限制
    #[serde(default)]
    pub request_limit: u64,
    /// 请求次数重置方式
    #[serde(default)]
    pub request_reset_policy: ResetPolicy,
    pub enabled: Option<bool>,
    /// 所属池ID列表（支持多池）
    #[serde(default)]
    pub pool_ids: Vec<String>,
    pub timeout: Option<u64>,
    /// 测试时指定的模型名称（可选）
    #[serde(default)]
    pub model: Option<String>,
    /// 模型名称映射列表（用于映射模式）
    #[serde(default)]
    pub model_mappings: Vec<ModelMapping>,
}

/// 池创建/更新请求
#[derive(Debug, Deserialize)]
pub struct PoolRequest {
    pub name: String,
    pub description: Option<String>,
    pub schedule_algorithm: ScheduleAlgorithm,
    #[serde(default)]
    pub model_mode: ModelMode,
    #[serde(default)]
    pub retry_mode: RetryMode,
    #[serde(default = "default_retry_count")]
    pub retry_count: u32,
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
    /// 所属池ID列表（支持多池）
    pub pool_ids: Vec<String>,
    pub timeout: u64,
    pub reset_policy: ResetPolicy,
    pub request_limit: u64,
    pub requests_used: u64,
    pub requests_remaining: u64,
    pub request_reset_policy: ResetPolicy,
    pub model_mappings: Vec<ModelMapping>,
}

#[derive(Debug, Serialize)]
pub struct PoolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schedule_algorithm: ScheduleAlgorithm,
    pub model_mode: ModelMode,
    pub retry_mode: RetryMode,
    pub retry_count: u32,
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
