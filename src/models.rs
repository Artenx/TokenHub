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
    #[serde(default)]
    pub total_tokens_used: u64,
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
            total_tokens_used: 0,
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
        self.total_tokens_used = self.total_tokens_used.saturating_add(amount);
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
        self.token_history.clear();
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
    /// 是否启用数据回放
    #[serde(default)]
    pub replay_enabled: bool,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

/// 数据回放配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayConfig {
    /// 每个接口最多保留的记录条数（默认 50）
    #[serde(default = "default_max_records_per_api")]
    pub max_records_per_api: usize,
    /// 回放记录持久化文件路径（相对于配置文件目录，默认 replay_state.json）
    #[serde(default = "default_replay_state_file")]
    pub state_file_path: String,
    /// 请求体/响应体截断阈值（单位 KB，默认 1024 即 1 MB）
    #[serde(default = "default_max_body_size_kb")]
    pub max_body_size_kb: usize,
}

fn default_max_records_per_api() -> usize { 50 }
fn default_replay_state_file() -> String { "replay_state.json".to_string() }
fn default_max_body_size_kb() -> usize { 1024 }

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_records_per_api: default_max_records_per_api(),
            state_file_path: default_replay_state_file(),
            max_body_size_kb: default_max_body_size_kb(),
        }
    }
}

/// 技能仓库配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRepositoryConfig {
    /// 技能包根目录，相对于配置文件目录
    #[serde(default = "default_skill_repository_root")]
    pub root_dir: String,
    /// 单个文件允许的最大容量（字节）
    #[serde(default = "default_skill_max_file_size")]
    pub max_file_size_bytes: u64,
    /// 单个技能包允许的最大文件数量
    #[serde(default = "default_skill_max_file_count")]
    pub max_file_count: usize,
    /// 单个技能包允许的最大总容量（字节）
    #[serde(default = "default_skill_max_total_size")]
    pub max_total_size_bytes: u64,
}

fn default_skill_repository_root() -> String { "skills".to_string() }
fn default_skill_max_file_size() -> u64 { 20 * 1024 * 1024 }
fn default_skill_max_file_count() -> usize { 500 }
fn default_skill_max_total_size() -> u64 { 200 * 1024 * 1024 }

impl Default for SkillRepositoryConfig {
    fn default() -> Self {
        Self {
            root_dir: default_skill_repository_root(),
            max_file_size_bytes: default_skill_max_file_size(),
            max_file_count: default_skill_max_file_count(),
            max_total_size_bytes: default_skill_max_total_size(),
        }
    }
}

/// 公开技能来源类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceType {
    Github,
    Skillhub,
    CustomIndex,
}

/// 公开技能来源配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceConfig {
    pub id: String,
    pub name: String,
    pub source_type: SkillSourceType,
    pub url: String,
    pub enabled: bool,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_checked_at: Option<DateTime<Utc>>,
}

/// 已导入的本地技能元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSkill {
    pub id: String,
    pub directory_name: String,
    pub name: String,
    pub description: String,
    pub skill_md_summary: String,
    pub file_count: usize,
    pub validation_status: String,
    #[serde(default)]
    pub validation_message: Option<String>,
    #[serde(default)]
    pub source: Option<SkillOrigin>,
    #[serde(default)]
    pub imported_at: Option<DateTime<Utc>>,
}

/// 技能包来源信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOrigin {
    pub source_type: SkillSourceType,
    pub url: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub content_digest: Option<String>,
}

/// 公开来源标准化搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSearchResult {
    pub source_id: String,
    pub external_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub popularity: Option<u64>,
    #[serde(default)]
    pub version: Option<String>,
    pub source_url: String,
    pub download_locator: String,
}

/// 待确认导入的技能包预览
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillImportPreview {
    pub id: String,
    pub target_directory_name: String,
    pub source: SkillOrigin,
    pub files: Vec<String>,
    pub valid: bool,
    #[serde(default)]
    pub validation_message: Option<String>,
    pub conflict: bool,
    pub expires_at: DateTime<Utc>,
}

/// 技能仓库操作审计记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAuditEntry {
    pub id: String,
    pub operation: String,
    pub directory_name: String,
    #[serde(default)]
    pub source: Option<SkillOrigin>,
    pub created_at: DateTime<Utc>,
    pub status: String,
    #[serde(default)]
    pub error_message: Option<String>,
}

/// 技能仓库独立持久化状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillRepositoryState {
    #[serde(default)]
    pub sources: Vec<SkillSourceConfig>,
    #[serde(default)]
    pub skills: Vec<LocalSkill>,
    #[serde(default)]
    pub audit_entries: Vec<SkillAuditEntry>,
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
    /// 数据回放配置
    #[serde(default)]
    pub replay: ReplayConfig,
    /// 技能仓库配置
    #[serde(default)]
    pub skill_repository: SkillRepositoryConfig,
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
            replay: ReplayConfig::default(),
            skill_repository: SkillRepositoryConfig::default(),
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
    #[serde(default)]
    pub replay_enabled: Option<bool>,
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
    pub total_tokens_consumed: u64,
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
    pub total_tokens_consumed: u64,
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
    pub replay_enabled: bool,
    /// 当前已记录的回放条数
    pub replay_record_count: usize,
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

/// 单条数据回放记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiReplayRecord {
    /// 记录唯一 ID
    pub id: String,
    /// 所属 ExposedApi ID
    pub api_id: String,
    /// 发生时间
    pub timestamp: DateTime<Utc>,
    /// HTTP 方法
    pub method: String,
    /// 完整请求路径（含查询参数）
    pub path: String,
    /// HTTP 状态码
    pub status_code: u16,
    /// success / error
    pub status: String,
    /// 错误信息（失败时）
    pub error_message: Option<String>,
    /// 耗时（毫秒）
    pub duration_ms: u64,
    /// 请求体（UTF-8 原文；超过配置阈值时被截断）
    pub request_body: String,
    /// 响应体（流式响应为拼接后的完整内容；超过配置阈值时被截断）
    pub response_body: String,
    /// 请求体是否被截断
    pub request_truncated: bool,
    /// 响应体是否被截断
    pub response_truncated: bool,
}

/// 模型评测任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ModelBenchmarkStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// 模型评测样本
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBenchmarkCase {
    pub id: String,
    pub name: String,
    pub messages: serde_json::Value,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

/// 任务内自动评审配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkJudgeConfig {
    pub endpoint_id: String,
    pub model: String,
    pub rubric: String,
}

/// 一个待评测的端点与模型组合
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ModelBenchmarkTarget {
    pub endpoint_id: String,
    pub model: String,
}

/// 单次端点评测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBenchmarkAttempt {
    pub id: String,
    pub case_id: String,
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub model: String,
    pub attempt_number: u8,
    pub status: String,
    pub status_code: Option<u16>,
    pub ttft_ms: Option<u64>,
    pub duration_ms: u64,
    pub total_tokens: Option<u64>,
    pub output: String,
    pub output_truncated: bool,
    pub error_message: Option<String>,
}

/// 自动评审结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBenchmarkJudgeResult {
    pub attempt_id: String,
    pub status: String,
    pub score: Option<f64>,
    pub accuracy: Option<f64>,
    pub completeness: Option<f64>,
    pub instruction_following: Option<f64>,
    pub writing_quality: Option<f64>,
    pub reason: Option<String>,
    pub confidence: Option<f64>,
    pub raw_response: String,
    pub response_truncated: bool,
}

/// 按端点聚合的模型评测摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBenchmarkSummary {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub model: String,
    pub attempts: usize,
    pub success_rate: f64,
    pub median_ttft_ms: Option<u64>,
    pub median_duration_ms: Option<u64>,
    pub average_total_tokens: Option<u64>,
    pub average_score: Option<f64>,
}

/// 模型评测任务及其不可变配置快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBenchmarkRun {
    pub id: String,
    pub status: ModelBenchmarkStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub model: String,
    pub endpoint_ids: Vec<String>,
    #[serde(default)]
    pub targets: Vec<ModelBenchmarkTarget>,
    pub endpoint_snapshots: Vec<EndpointConfig>,
    pub cases: Vec<ModelBenchmarkCase>,
    pub judge: BenchmarkJudgeConfig,
    #[serde(default = "default_benchmark_attempts")]
    pub attempts_per_case: u8,
    #[serde(default)]
    pub attempts: Vec<ModelBenchmarkAttempt>,
    #[serde(default)]
    pub judge_results: Vec<ModelBenchmarkJudgeResult>,
}

impl ModelBenchmarkRun {
    pub fn benchmark_targets(&self) -> Vec<ModelBenchmarkTarget> {
        if self.targets.is_empty() {
            self.endpoint_ids.iter().map(|endpoint_id| ModelBenchmarkTarget {
                endpoint_id: endpoint_id.clone(),
                model: self.model.clone(),
            }).collect()
        } else {
            self.targets.clone()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateModelBenchmarkRequest {
    pub targets: Vec<ModelBenchmarkTarget>,
    pub cases: Vec<ModelBenchmarkCase>,
    pub judge: BenchmarkJudgeConfig,
}

fn default_benchmark_attempts() -> u8 { 3 }

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

/// 从 Custom 类型端点 URL 提取用于模型列表查询的回退 URL
///
/// 当用户配置了具体资源路径（如 /v1/images/generations）时，
/// 尝试回退到 /v1/models 获取模型列表。
pub fn fallback_models_url(url: &str) -> Option<String> {
    let url = url.trim_end_matches('/');
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        if let Some(first_slash) = after_scheme.find('/') {
            let authority = &after_scheme[..first_slash];
            Some(format!("{}://{}/v1/models", &url[..scheme_end], authority))
        } else {
            Some(format!("{}/v1/models", url))
        }
    } else {
        None
    }
}
