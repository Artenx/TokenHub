# 接口数据回放（API Data Replay）

Feature Name: api-data-replay
Updated: 2026-07-18

## Description

为 TokenHub 的每个对外接口（ExposedApi）增加可选的数据回放能力。管理员在接口管理页面开启某个接口的「数据回放」开关后，代理转发链路会同步捕获该接口后续每次调用的完整请求体与响应体，按接口 ID 分组保存（每接口条数上限可通过配置调整，默认 50 条，超出后淘汰最旧记录），并持久化到独立的 JSON 文件（默认 `replay_state.json`，路径可配置）。管理员可在接口卡片上查看回放记录列表，展开单条记录后以 JSON 美化格式查看请求体与响应体。

## Architecture

```mermaid
flowchart LR
    Client["调用方 Client"] -->|"HTTP /v1/chat/completions"| Proxy["api_proxy 入口"]
    Proxy --> Matcher["match_exposed_api"]
    Matcher -->|ExposedApi (含 replay_enabled)| Forward["forward_request / forward_stream_request"]
    Forward --> Endpoint["上游端点"]
    Endpoint --> Forward
    Forward -->|原始响应| Client
    Forward -->|"replay_enabled = true"| Capture["ReplayCapture"]
    Capture --> Truncate["按 max_body_size_kb 截断"]
    Truncate --> Store["AppState.replay_records (HashMap)"]
    Store --> Persist["replay_state.json 持久化"]
    AdminUI["管理后台 /admin"] -->|"GET /admin/api/exposed-apis/:id/replay-records"| Store
    AdminUI -->|"POST /admin/api/exposed-apis/:id/replay-toggle"| Matcher
    AdminUI -->|"DELETE /admin/api/exposed-apis/:id/replay-records"| Store
```

回放捕获与既有的调用日志（`ApiCallLog`）完全解耦：调用日志只记录元数据；回放记录在开启开关时额外保存 body。未开启开关时捕获路径零开销（仅一次布尔判断）。

## Components and Interfaces

### 1. 数据模型（`src/models.rs`）

#### `AppConfig` 新增配置节

```rust
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

pub struct AppConfig {
    // ... 既有字段
    /// 数据回放配置
    #[serde(default)]
    pub replay: ReplayConfig,
}
```

#### `ExposedApi` 新增字段

```rust
pub struct ExposedApi {
    // ... 既有字段
    /// 是否启用数据回放（默认 false，兼容旧配置）
    #[serde(default)]
    pub replay_enabled: bool,
}
```

#### `ExposedApiRequest` 新增字段

```rust
pub struct ExposedApiRequest {
    // ... 既有字段
    #[serde(default)]
    pub replay_enabled: Option<bool>,
}
```

#### `ExposedApiInfo` 新增字段

```rust
pub struct ExposedApiInfo {
    // ... 既有字段
    pub replay_enabled: bool,
    /// 当前已记录的回放条数（用于列表展示徽标）
    pub replay_record_count: usize,
}
```

#### 新增 `ApiReplayRecord`

```rust
/// 单条数据回放记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiReplayRecord {
    /// 记录唯一 ID（UUIDv4）
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
```

### 2. 状态存储（`src/state.rs`）

```rust
pub struct AppState {
    // ... 既有字段
    /// 按 ExposedApi ID 分组的回放记录
    pub replay_records: RwLock<HashMap<String, VecDeque<ApiReplayRecord>>>,
    /// 回放记录持久化文件路径（由配置解析）
    pub replay_state_path: PathBuf,
    /// 回放配置（从 AppConfig 提取，避免频繁读锁）
    pub replay_config: RwLock<ReplayConfig>,
}
```

新增方法：

| 方法 | 行为 |
|---|---|
| `add_replay_record(record)` | 追加到对应 `api_id` 的队列尾；超过 `replay_config.max_records_per_api` 时弹出队首；标记 dirty 触发持久化调度 |
| `get_replay_records(api_id)` | 返回该接口全部记录的克隆（按时间正序存储，前端再倒序渲染） |
| `clear_replay_records(api_id)` | 清空指定接口的记录 |
| `remove_replay_records_for_api(api_id)` | 删除接口时联动清理 |
| `save_replay_state()` | 将 `replay_records` 序列化到 `replay_state_path`；仅保留每接口最新 `max_records_per_api` 条 |
| `load_replay_state()` | 启动时从 `replay_state_path` 恢复；恢复后再次校验每接口条数 ≤ `max_records_per_api` |

持久化策略：
- 回放记录独立持久化到 `replay_state_path`，与 `state.json` 分离，避免大体积 body 拖慢主状态文件读写。
- `save_runtime_state()` 中在保存 `state.json` 后，异步调用 `save_replay_state()`。
- 持久化前裁剪：仅保留每接口最新 `max_records_per_api` 条。

### 3. 捕获逻辑（`src/proxy.rs`）

#### 新增辅助函数

```rust
/// 将字节切片转换为 UTF-8 字符串，并按配置的阈值截断
fn capture_body(body: &[u8], max_size_kb: usize) -> (String, bool);

/// 构造一条 ApiReplayRecord 并写入 AppState
fn record_replay(
    state: &AppState,
    api_id: &str,
    method: &str,
    path: &str,
    status_code: u16,
    status: &str,
    error_message: Option<String>,
    duration_ms: u64,
    request_body: &[u8],
    response_body: &[u8],
);
```

#### 非流式路径（`forward_request`）

- 在 `forward_to_endpoint` 返回值中额外携带最终响应体字节（`final_body.clone()`），用于回放。
- 在现有写入 `ApiCallLog` 的位置（`src/proxy.rs:514` 附近）后追加：
  - 若 `ctx.exposed_api.replay_enabled == true`，调用 `record_replay(...)`，`request_body = body`（客户端原始请求体，未经模型名映射/格式转换），`response_body = final_body`。
  - 错误分支（`Err(e)`）时 `response_body` 记录上游原始错误内容（`last_raw_error`）或 `e.to_string()`。

#### 流式路径（`forward_stream_request`）

- 改造 `StreamLogWriter`：新增 `body_buffer: Arc<Mutex<Vec<u8>>>` 字段，在 `poll_next` 中每次返回 chunk 前把 bytes 追加到 buffer（追加前检查容量，达到 `max_body_size_kb * 1024` 字节后停止追加并标记截断）。
- 在 `on_complete` 回调中，除既有 `add_call_log` 外：若 `ctx.exposed_api.replay_enabled == true`，调用 `record_replay(...)`，`request_body = body`，`response_body = body_buffer.lock()` 内容。
- 错误路径（未进入流式阶段）同样记录，响应体为错误文本。

### 4. 管理 API（`src/admin.rs` + `src/main.rs`）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/admin/api/exposed-apis/{id}/replay-records` | 返回该接口回放记录列表（JSON 数组） |
| DELETE | `/admin/api/exposed-apis/{id}/replay-records` | 清空该接口回放记录 |
| POST | `/admin/api/exposed-apis/{id}/replay-toggle` | 切换 `replay_enabled`，返回最新状态 |
| GET | `/admin/api/replay-config` | 返回当前回放配置（`max_records_per_api`、`state_file_path`、`max_body_size_kb`） |
| PUT | `/admin/api/replay-config` | 更新回放配置，保存到 `config.toml` 并立即生效 |

所有路由复用既有的管理员会话认证中间件，未认证返回 401。

`replay-toggle` 行为：仅切换 `replay_enabled`，不清空已有记录。

`PUT /admin/api/replay-config` 更新逻辑：
- 校验 `max_records_per_api` ≥ 1 且 ≤ 1000；`max_body_size_kb` ≥ 1 且 ≤ 10240。
- 更新 `AppConfig.replay` 并调用 `config_manager.save()`。
- 同步更新 `AppState.replay_config` 与 `AppState.replay_state_path`。
- 若 `state_file_path` 变更，将现有记录迁移到新路径（先保存到新文件，成功后删除旧文件）。

### 5. 前端（`static/app.js` + `static/index.html` + `static/style.css`）

#### 接口卡片改动

在 `renderApisList()` 中：
- 在状态徽章区域追加「回放中」标识（当 `api.replay_enabled == true`，使用醒目颜色，例如橙色 badge）。
- 在 `endpoint-actions` 区域新增三个按钮：
  - **回放开关**：文本为「开启回放 / 关闭回放」，调用 `POST .../replay-toggle`，成功后刷新列表。
  - **回放记录**：右侧附带 `replay_record_count` 徽标，点击打开回放弹窗。
  - 既有「编辑 / 禁用 / 删除」按钮保持不变。

#### 回放记录弹窗

- 新增模态框 `replayModal`，包含：接口名称、记录条数、「刷新」「清空回放」按钮、记录列表。
- 列表项按时间倒序展示：时间、方法、路径、状态码（成功绿色 / 失败红色）、耗时、截断标记。
- 点击列表项展开详情，分两个区块：「请求体」「响应体」。
  - 内容为合法 JSON 时：`JSON.parse` 后以 `JSON.stringify(obj, null, 2)` 美化渲染，并使用既有的 `<pre><code>` 等宽字体样式。
  - 内容非 JSON 时：原样以 `<pre>` 展示。
  - 截断标记为 true 时在对应区块顶部显示黄色提示条：「内容超过 1 MB，仅显示前 1 MB」。
- 顶部固定提示条：「回放内容可能包含 API Key 等敏感信息，请妥善保管」。

#### 数据加载

- `loadApisPage()` 拉取 `/stats` 时已包含 `replay_enabled` 与 `replay_record_count`。
- 打开弹窗时调用 `GET .../replay-records`；点击「刷新」重新拉取；「清空回放」需 `confirm()` 二次确认后调用 `DELETE`。

## Data Models

见「Components and Interfaces → 1. 数据模型」。

`config.toml` 中新增配置节示例：

```toml
[replay]
max_records_per_api = 50
state_file_path = "replay_state.json"
max_body_size_kb = 1024
```

`replay_state.json` 中数据示例：

```json
{
  "api-uuid-1": [
    {
      "id": "rec-uuid",
      "api_id": "api-uuid-1",
      "timestamp": "2026-07-18T14:30:00Z",
      "method": "POST",
      "path": "/v1/chat/completions",
      "status_code": 200,
      "status": "success",
      "error_message": null,
      "duration_ms": 1234,
      "request_body": "{\"model\":\"gpt-4\",...}",
      "response_body": "{\"id\":\"chatcmpl-...\",...}",
      "request_truncated": false,
      "response_truncated": false
    }
  ]
}
```

## Correctness Properties

| 属性 | 描述 |
|---|---|
| CP-1 | 任何时刻 `replay_records[api_id].len() <= replay_config.max_records_per_api` |
| CP-2 | 关闭 `replay_enabled` 后，新请求不再产生 `ApiReplayRecord`，已有记录保持不变 |
| CP-3 | 未开启 `replay_enabled` 的接口，转发链路的额外开销仅为一次布尔读取 |
| CP-4 | `request_body` 始终是客户端发送到 TokenHub 的原始字节（未做格式转换/模型名映射） |
| CP-5 | 流式响应的 `response_body` 按 chunk 到达顺序拼接，与客户端实际接收内容一致 |
| CP-6 | 记录写入失败（如锁竞争）仅记录 warn 日志，不影响主转发流程返回 |
| CP-7 | 接口被删除后，`replay_records` 中对应键同步移除 |
| CP-8 | 重启后 `replay_records` 与重启前一致（同配置上限裁剪后） |
| CP-9 | 修改 `max_records_per_api` 配置后，新写入的记录立即遵循新上限；已有记录在新写入触发淘汰时逐步收敛 |
| CP-10 | 修改 `state_file_path` 后，后续回放记录写入新路径；旧路径文件在迁移成功后移除 |

## Error Handling

| 场景 | 处理策略 |
|---|---|
| 请求体为非 UTF-8 字节 | `String::from_utf8_lossy` 转换，保留替换字符 |
| 流式响应中客户端中途断开 | `StreamLogWriter::drop` 仍触发 `on_complete`，以当前 buffer 内容记录，状态码记 200，并在 `error_message` 中标注 `client disconnected` |
| 持久化序列化失败 | 记录 `error!` 日志，不影响内存中的记录 |
| 恢复 `state.json` 时某条记录反序列化失败 | 丢弃该条记录，继续恢复其余记录；记录 `warn!` 日志 |
| 请求/响应体超过配置阈值 | 截断 + 置 `*_truncated = true`，不影响主流程 |
| 管理 API 操作不存在的 api_id | 返回 404 |
| 回放配置文件格式非法 | 使用默认值并记录 `warn!` 日志，不中断启动 |

## Test Strategy

### 单元测试

- `capture_body`：UTF-8 / 非 UTF-8 / 空 / 边界长度（1 MB - 1、1 MB、1 MB + 1）。
- `add_replay_record` 的容量淘汰：连续写入 60 条，断言剩余 50 条且最旧 10 条被淘汰。
- `ExposedApi` 反序列化：缺失 `replay_enabled` 字段时默认为 `false`。
- `StreamLogWriter` 缓冲：模拟多 chunk + 提前 drop，断言拼接顺序与截断标记。

### 集成测试

- 开启回放的接口收到一次非流式调用后，`GET /replay-records` 返回 1 条记录，body 与上游 mock 响应一致。
- 开启回放的接口收到一次 SSE 流式调用后，回放记录的 `response_body` 等于所有 chunk 拼接结果。
- 关闭回放后再次调用，记录数不增加。
- 未认证请求 `GET /replay-records` 返回 401。
- 删除接口后，对应回放记录同步清空。
- 写入多条记录后调用 `save_state` + `load_state`，重启后记录保持一致。

### 手工验证

- 在管理后台对某接口开启回放 → 通过 `curl` 调用 → 在「回放记录」弹窗中确认请求体/响应体美化展示。
- 通过 `curl` 触发一次流式调用 → 确认拼接内容完整。

## References

[^1]: (Filename#L392) - [`ExposedApi` 定义](src/models.rs#L392)
[^2]: (Filename#L411) - [非流式转发入口 `forward_request`](src/proxy.rs#L411)
[^3]: (Filename#L536) - [流式转发入口 `forward_stream_request`](src/proxy.rs#L536)
[^4]: (Filename#L1191) - [`StreamLogWriter` 结构](src/proxy.rs#L1191)
[^5]: (Filename#L605) - [既有调用日志 `ApiCallLog`](src/models.rs#L605)
[^6]: (Filename#L927) - [`AppState::add_call_log`](src/state.rs#L927)
[^7]: (Filename#L222) - [管理路由注册](src/main.rs#L222)
