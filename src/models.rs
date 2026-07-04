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
    Custom,
}

impl std::fmt::Display for ApiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiType::OpenAI => write!(f, "openai"),
            ApiType::Anthropic => write!(f, "anthropic"),
            ApiType::OpenAIResponses => write!(f, "openai-responses"),
            ApiType::Custom => write!(f, "custom"),
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
    None,
    /// 原地重试：异常时继续向原端点重试
    Same,
    /// 端点重试：异常时切换到池内其他端点
    #[default]
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
    /// 每分钟自动重置
    #[serde(rename = "minutely")]
    Minutely,
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

fn default_now() -> DateTime<Utc> {
    Utc::now()
}

/// 端点运行时状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointState {
    pub config: EndpointConfig,
    pub tokens_used: u64,
    pub last_reset: DateTime<Utc>,
    /// 请求次数最后重置时间（用于每分钟等独立重置策略）
    #[serde(default = "default_now")]
    pub request_last_reset: DateTime<Utc>,
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
        let now = Utc::now();
        Self {
            config,
            tokens_used: 0,
            last_reset: now,
            request_last_reset: now,
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

    /// 原子地预留一次请求额度。选择端点后、实际转发前调用，
    /// 防止并发请求同时通过 `is_available()` 导致限额超支。
    pub fn try_reserve_request(&mut self) -> bool {
        // 无限制时直接占用计数
        if self.config.request_limit == 0 {
            self.requests_used = self.requests_used.saturating_add(1);
            self.total_requests = self.total_requests.saturating_add(1);
            self.last_used = Some(Utc::now());
            self.record_request_history();
            return true;
        }

        let below_limit = match self.config.request_reset_policy {
            ResetPolicy::Rolling5h => self.effective_requests() < self.config.request_limit,
            _ => self.requests_used < self.config.request_limit,
        };

        if !below_limit {
            return false;
        }

        self.requests_used = self.requests_used.saturating_add(1);
        self.total_requests = self.total_requests.saturating_add(1);
        self.last_used = Some(Utc::now());
        self.record_request_history();
        true
    }

    /// 预留失败或转发失败时回滚请求计数
    /// 注意：total_requests 只增不减，用于统计实际尝试次数；仅回滚 requests_used。
    pub fn release_request(&mut self) {
        self.requests_used = self.requests_used.saturating_sub(1);
        // 同时从最近一条 history 记录中移除，保持滚动窗口一致性
        if self.config.request_reset_policy == ResetPolicy::Rolling5h {
            if let Some(last) = self.request_history.last().cloned() {
                if last.1 == self.requests_used.saturating_add(1) {
                    self.request_history.pop();
                }
            }
        }
    }

    pub fn add_tokens(&mut self, amount: u64) {
        self.tokens_used = self.tokens_used.saturating_add(amount);
        self.last_used = Some(Utc::now());

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
    }

    fn record_request_history(&mut self) {
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
        self.request_last_reset = Utc::now();
    }

    pub fn reset(&mut self) {
        self.tokens_used = 0;
        self.last_reset = Utc::now();
        self.request_last_reset = Utc::now();
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

/// 池一键测试请求
#[derive(Debug, Deserialize)]
pub struct PoolTestRequest {
    /// 手动模式：指定测试用的模型名称
    #[serde(default)]
    pub model: Option<String>,
    /// 测试模式："auto" 自动选择模型（默认），"manual" 手动指定模型
    #[serde(default = "default_test_mode")]
    pub mode: String,
}

fn default_test_mode() -> String {
    "auto".to_string()
}

/// 单个端点测试结果
#[derive(Debug, Serialize)]
pub struct EndpointTestResult {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub success: bool,
    pub message: String,
    pub model_used: String,
    pub status: u16,
    pub tested_url: String,
}

/// 池测试汇总
#[derive(Debug, Serialize)]
pub struct PoolTestSummary {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
}

/// API 调用日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCallLog {
    /// 请求时间
    pub timestamp: DateTime<Utc>,
    /// 调用方 IP
    pub client_ip: String,
    /// HTTP 方法
    pub method: String,
    /// 请求路径（含查询参数）
    pub path: String,
    /// 命中的对外 API 前缀
    pub api_prefix: Option<String>,
    /// 实际使用的端点 ID
    pub endpoint_id: Option<String>,
    /// 实际使用的端点名称
    pub endpoint_name: Option<String>,
    /// HTTP 状态码
    pub status_code: u16,
    /// 响应状态：success / error
    pub status: String,
    /// 错误信息（失败时）
    pub error_message: Option<String>,
    /// 请求耗时（毫秒）
    pub duration_ms: u64,
    /// 输入 Token 数量
    pub input_tokens: Option<u64>,
    /// 输出 Token 数量
    pub output_tokens: Option<u64>,
    /// 总 Token 数量
    pub total_tokens: Option<u64>,
}

/// 端点延迟统计
#[derive(Debug, Clone, Serialize)]
pub struct EndpointLatencyStats {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub enabled: bool,
    pub samples: usize,
    pub avg_ms: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p95_ms: u64,
    /// 总请求次数（含失败尝试）
    pub total_requests: u64,
    /// 错误次数
    pub error_count: u32,
    /// 错误率（百分比，0-100）
    pub error_rate: f64,
}
