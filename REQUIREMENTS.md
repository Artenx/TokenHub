# TokenHub - LLM API Token 池管理系统

## 需求说明文档

---

## 一、项目概述

TokenHub 是一个基于 Rust 开发的 Web 服务项目，用于代理和管理多个大模型（LLM）API 端点，形成一个统一的 Token 池管理系统。系统对外暴露统一的 API 接口，内部将请求智能分发到不同的后端端点，并实时统计 Token 使用情况。

---

## 二、核心功能需求

### 2.1 代理端点管理

**功能描述：** 支持配置多个大模型 API 代理端点，每个端点可独立配置。

**端点配置项：**
- **名称** - 端点的显示名称
- **Base URL** - API 域名地址（如 `https://api.openai.com`），系统自动补全完整路径
- **接口类型** - 支持三种类型：
  - `openai` - OpenAI 兼容格式（路径：`/v1/chat/completions`）
  - `anthropic` - Anthropic 兼容格式（路径：`/v1/messages`）
  - `openai-responses` - OpenAI Responses 格式（路径：`/v1/responses`）
- **API Key** - 该端点的认证密钥
- **Token 限额** - 每个端点的 Token 使用上限，置空则无上限
- **限额重置方式**：
  - `manual` - 一次性手动重置
  - `daily` - 每日零点自动重置
  - `Rolling5h` - 5小时滚动重置（滑动窗口，仅统计最近5小时消耗）
- **请求次数限制** - 每个端点的请求次数上限，置空则无上限
- **请求重置方式**：
  - `manual` - 手动重置
  - `daily` - 每日零点自动重置
  - `Rolling5h` - 5小时滚动重置（滑动窗口，仅统计最近5小时请求）
- **超时时间** - 请求超时时间（秒），默认 300 秒
- **启用/禁用** - 可动态切换端点状态
- **所属池** - 端点可归属于多个端点池（`pool_ids` 数组）

**URL 自动补全：**
- 用户只需输入 Base URL（如 `https://api.openai.com`）
- 系统自动补全 `/v1` 路径前缀
- 如果用户输入的 URL 已包含 `/v1`，则直接使用
- 请求转发时自动拼接完整路径

**浏览模型：**
- 点击「浏览模型」按钮调用 `/v1/models` 接口获取可用模型列表
- 显示模型 ID 和提供者信息

**对话测试：**
- 点击「对话测试」按钮弹出模型选择窗口
- 自动获取端点的可用模型列表
- 用户选择模型后发送测试请求（内容为 "hi"）
- 显示模型回复结果

### 2.2 端点池管理

**功能描述：** 多个端点可组成一个池子，池内端点共享调度策略。

**池配置项：**
- **名称** - 池的显示名称
- **描述** - 池的用途说明
- **调度算法** - 该池使用的请求分发策略
- **模型名称传递模式** - 透传模式或映射模式
- **重试机制** - 无重试、原地重试或端点重试

**端点与池的关系：**
- 每个端点可归属多个池（`pool_ids` 数组）
- 从池中移除端点不会删除端点本身，只是从池的 `pool_ids` 中移除
- 移除的端点会重新出现在「选择端点」的可选列表中
- 「选择端点」只显示未分配到当前池的端点

**层级关系：**
```
端点池 (Pool)
  ├── 代理端点 A (可属于多个池)
  ├── 代理端点 B (可属于多个池)
  └── 代理端点 C (可属于多个池)
```

**池一键测试：**
- 每个池卡片上显示「一键测试」按钮
- 支持两种模型选择模式：
  - **自动选择（推荐）** - 每个端点自动选用其第一个可用模型，无需手动选择
  - **手动指定模型** - 从池内端点模型列表中手动选择一个模型名称，后端自动通过最长后缀匹配逻辑将选中的模型名映射为各端点的实际模型名
- 测试流程：点击按钮 → 选择模式（自动/手动） → 点击开始测试 → 对池内每个端点依次发送 "hi" 对话请求 → 展示汇总结果（成功/失败数、每个端点的回复内容、使用的模型名、HTTP状态码）

### 2.3 调度算法

**功能描述：** 支持多种请求调度算法，每个端点池可独立配置。

#### 2.3.1 轮询 (Round Robin)
- **逻辑：** 依次将请求转发给池中的每个端点
- **跳过规则：** 当某个端点的 Token 限额耗尽后，自动跳过该端点
- **适用场景：** 多端点负载均衡，公平分配请求量

#### 2.3.2 轮换 (Failover)
- **逻辑：** 优先将所有请求转发到当前活跃端点
- **切换规则：** 当该端点的 Token 限额耗尽后，自动切换到下一个可用端点
- **适用场景：** 主备模式，优先使用某个端点直到用尽

#### 2.3.3 随机 (Random)
- **逻辑：** 每次请求随机选择一个可用端点
- **跳过规则：** 额度耗尽的端点会被跳过
- **适用场景：** 高可用场景，随机负载均衡

### 2.4 重试机制

**功能描述：** 支持配置重试策略，每个端点池可独立配置。

**重试模式：**
- **无重试 (None)** - 异常直接返回错误
- **原地重试 (Same)** - 异常时继续向原端点重试，直到达到重试次数
- **端点重试 (Pool)** - 异常时切换到池内其他端点，每个端点依次尝试

**异常判断规则：**
- 非200响应码
- 连接超时
- 连接失败（DNS解析失败、连接被拒绝等）
- 请求错误
- 其他网络异常
- 响应体中包含错误（支持多种格式，见 2.8 错误检测）

**重试次数：** 可配置 1-10 次

**全池失败：** 返回统一提示：「端点池所有接口均不可用，请检查后重试」

### 2.5 API 接口管理

**功能描述：** 对外暴露的统一 API 接口，支持自定义前缀和独立认证。

**接口配置项：**
- **名称** - 接口的显示名称
- **URL 前缀** - 客户端请求时使用的路径前缀（如 `/v1`、`/v1/gpt4`、`/v1/responses`）
- **接口类型** - 对外暴露的接口格式（openai / anthropic / openai-responses）
- **关联池** - 该接口关联到哪个端点池
- **API 认证密钥** - 调用此接口时需要提供的认证密钥，留空则不需要认证
- **启用/禁用** - 可动态切换接口状态

**调用示例显示：**
- 接口列表中显示完整调用 URL（http 格式）
- 输入前缀时实时显示完整调用路径

**对话测试：**
- 点击「对话测试」按钮弹出模型选择窗口
- 自动从关联池的端点获取可用模型列表
- 用户选择模型后进行测试
- 显示模型回复结果

**路由机制：**
- 请求路径匹配前缀后，转发到关联池中的端点
- 自动剥离前缀，拼接目标路径

### 2.6 Token 统计与限额管理

**功能描述：** 自动统计并持久化每个端点已消耗的 Token 和请求次数。

**统计内容：**
- 每个端点的 Token 使用量
- 每个端点的请求次数（当前周期 + 累计）
- 每个端点的错误次数
- 每个端点的最后使用时间
- 每个池的汇总统计
- 全局汇总统计

**Token 限额管理：**
- 支持为每个端点设置 Token 限额
- 限额为空表示无上限（显示为「无上限」）
- 支持手动重置单个或所有端点的 Token 使用量
- 支持每日零点自动重置（Daily 策略）
- 支持 5 小时滚动重置（Rolling5h 策略）：滑动窗口，仅统计最近 5 小时的消耗，旧消耗自动失效

**请求次数限额管理：**
- 支持为每个端点设置请求次数上限
- 限制为空表示无上限（显示为「无上限」）
- 支持手动重置单个端点的请求次数
- 支持每日零点自动重置
- 支持 5 小时滚动重置（滑动窗口，仅统计最近 5 小时的请求数）
- 达到限制后端点自动标记为不可用，窗口期后恢复

**限额规则：**
- 限额为空：不自动清零，列表页面显示为「无上限」
- 限额不为空 + 手动重置：已使用达到限额时标记为不可用，不自动清零
- 限额不为空 + 每日重置：每天零点自动清零，达到限额时标记为不可用
- 限额不为空 + 5小时滚动：滑动窗口计算有效消耗，超出窗口的旧消耗自动失效
- 同时设置 Token 限额和请求次数限制时，任一达到上限则端点不可用
- 概览页统计排除无限制端点的已使用和限额

**持久化：**
- 运行时状态（已使用 Token、已请求次数、Token 滑动窗口历史、请求滑动窗口历史、错误数、请求数、最后使用时间）每 10 秒自动保存到 `state.json`
- 重启后自动恢复

**Token 解析：**
- OpenAI 格式：从响应中解析 `usage.total_tokens`
- Anthropic 格式：从响应中解析 `usage.input_tokens + usage.output_tokens`
- 流式响应：从最后一个 SSE 事件中解析 usage 字段

### 2.7 模型名称传递

**功能描述：** 池支持配置模型名称传递模式，控制客户端请求的模型名称如何传递给端点。

**传递模式：**
- **透传模式 (Passthrough)** - 自动匹配端点支持的模型名称，不区分大小写，包含即匹配
- **映射模式 (Mapping)** - 需手动配置模型名称映射关系

**透传模式匹配规则：**
- 首先检查是否有完全一致（包括大小写）的模型，如果有则不替换
- 然后进行模糊匹配（不区分大小写，包含即匹配）
- 如果匹配到多个模型，返回错误提示
- 如果没有匹配到，使用原始名称

**映射模式：**
- 在选择端点到池时配置映射关系
- 每个映射包含：客户端模型名称 → 端点模型名称
- 精确匹配客户端模型名称

**模型缓存：**
- 端点模型列表自动缓存
- 添加/编辑端点时自动更新缓存
- 每小时定时更新缓存
- 请求时发现无缓存自动获取

### 2.8 错误检测

**功能描述：** 支持检测多种格式的 API 错误，用于触发重试机制。

**支持的错误格式：**
- **OpenAI 格式：** `{"error": {"code": "...", "message": "..."}}`
- **Anthropic 格式：** `{"type": "error", "error": {...}}`
- **NVIDIA 格式：** `{"status": 429, "title": "Too Many Requests"}`
- **顶层 code+message：** `{"code": 429, "message": "..."}`
- **SSE 格式：** `data: {"error": {...}}`
- **Anthropic SSE：** `event: error` + `data: {...}`
- **内容中嵌入的错误：** 检查 choices 中的 content 是否包含错误信息
- **纯文本错误关键词：** 请求负载过高、请稍后再试、rate limit 等（仅对短内容 <200 字符检测）

**检测范围：**
- 普通 JSON 响应
- SSE 流式响应（逐行检查 `data:` 事件）
- 正常回复中不误判（跳过有 `choices` 或 `id` 字段的响应）

### 2.9 API 格式转换

**功能描述：** 支持三种接口类型之间的请求体和响应体自动转换。

**支持的接口类型：**
- OpenAI Chat Completions (`openai`)
- OpenAI Responses (`openai-responses`)
- Anthropic Messages (`anthropic`)

**转换场景：**
- 客户端使用接口 A 类型，端点使用接口 B 类型
- 代理自动转换请求体和响应体格式
- 流式响应支持逐块转换

**转换规则：**
- 请求体：`input` → `messages`，`max_output_tokens` → `max_tokens` 等
- 响应体：`choices` → `output`，`usage` 字段映射等
- 流式响应：SSE 事件类型和格式自动转换

### 2.10 Web 管理后台

**功能描述：** 提供 Web 管理界面，支持密码认证登录。

**页面结构：**

#### 概览页
- 端点总数 / 活跃端点数
- 已用 Token / Token 限额（排除无限制端点）
- Token 使用率（进度条）
- 总请求数 / 错误数
- 端点池数 / API 接口数
- 端点池概览列表
- API 接口概览列表
- 端点状态详情（包含请求次数限额和当前已请求次数）
- **图表统计：**
  - 端点状态分布图（正常/异常/禁用）
  - Token 使用量图表（按端点展示使用率）
  - 请求量统计图表（按端点展示请求数）

#### 端点管理页
- 端点列表（显示所有端点，不受池的限制）
- 添加端点功能
- 每个端点卡片包含：
  - 编辑 / 启用禁用 / 重置Token / 重置请求 / 浏览模型 / 删除 操作按钮
  - 「浏览模型」按钮调用 API 获取真实模型列表
  - 「对话测试」按钮（弹出模型选择后测试）
- 编辑端点时回显完整的 API Key
- 显示已使用 Token、Token 限额、请求次数限额、当前已请求次数
- 无限制端点显示「无上限」

#### 池管理页
- 端点池列表
- 添加池功能
- 每个池卡片包含：
  - 池内端点列表
  - 「一键测试」按钮 - 对池内所有端点进行批量对话测试（支持自动/手动模型选择）
  - 「选择端点」按钮 - 从端点列表中选择未分配到当前池的端点添加到池
  - 端点操作：编辑 / 从池中移除
- 「从池中移除」只从池的 `pool_ids` 中移除，不删除端点
- 编辑池时配置模型名称传递模式、重试机制、模型映射

#### 接口管理页
- 对外接口列表
- 每个接口显示完整调用 URL（http 格式）
- 添加 / 编辑 / 删除接口
- 启用 / 禁用接口
- 添加/编辑页面包含：
  - URL 前缀输入后实时显示完整调用路径
  - 「对话测试」按钮（弹出模型选择后测试）

#### 设置页
- 修改密码
- 重置所有端点 Token

**认证机制：**
- Cookie 方式保持登录状态
- 登录后可修改密码
- 24小时会话有效期

### 2.11 API 认证

**功能描述：** 对外暴露的统一 API 接口支持可选的认证。

**认证方式：**
- 请求头：`Authorization: Bearer <api_key>`（OpenAI/Responses 格式）
- 请求头：`x-api-key: <api_key>`（Anthropic 格式）
- 每个 API 接口可独立配置认证密钥
- 密钥为空则不需要认证

---

## 三、技术架构

### 3.1 技术栈
- **后端语言：** Rust
- **Web 框架：** Actix-web 4
- **HTTP 客户端：** Reqwest（rustls-tls）
- **序列化：** Serde / Serde JSON
- **异步运行时：** Tokio
- **配置存储：** TOML 文件
- **运行时状态：** JSON 文件（state.json）
- **前端：** 原生 HTML / CSS / JavaScript

### 3.2 项目结构
```
tokenhub/
├── src/
│   ├── main.rs        # 入口，路由配置
│   ├── models.rs      # 数据模型定义
│   ├── config.rs      # 配置文件管理
│   ├── state.rs       # 应用状态管理
│   ├── scheduler.rs   # 调度算法实现
│   ├── proxy.rs       # API 代理转发（含 URL 自动补全、格式转换）
│   ├── converter.rs   # API 格式转换器（三种格式互转）
│   ├── auth.rs        # 认证中间件（支持 Authorization 和 x-api-key）
│   ├── admin.rs       # 管理后台 API（含浏览模型、对话测试）
│   ├── validator.rs   # 输入验证工具
│   └── error.rs       # 错误类型定义
├── static/
│   ├── index.html     # 管理后台页面
│   ├── style.css      # 样式文件
│   └── app.js         # 前端逻辑
├── data/
│   ├── config.toml    # 配置文件
│   └── state.json     # 运行时状态持久化
└── Cargo.toml
```

### 3.3 配置文件格式 (config.toml)
```toml
listen_addr = "0.0.0.0"
listen_port = 8080
admin_password = "admin123"

# 端点池
[[pools]]
id = "pool-1"
name = "默认池"
description = "默认端点池"
schedule_algorithm = "round_robin"
model_mode = "passthrough"
retry_mode = "none"
retry_count = 3
created_at = "2026-01-01T00:00:00Z"

# 对外暴露的 API
[[exposed_apis]]
id = "api-1"
name = "默认 OpenAI API"
prefix = "/v1"
api_type = "openai"
enabled = true
pool_id = "pool-1"
created_at = "2026-01-01T00:00:00Z"

# 代理端点
[[endpoints]]
id = "ep-1"
name = "GPT-4"
url = "https://api.openai.com"
api_type = "openai"
api_key = "sk-xxx"
token_limit = 100000
reset_policy = "manual"
request_limit = 0
request_reset_policy = "manual"
enabled = true
pool_ids = ["pool-1"]
timeout = 300
model_mappings = []
```

---

## 四、部署说明

### 4.1 系统要求
- Linux 操作系统
- 已安装 Rust 编译环境

### 4.2 编译与部署
```bash
# 编译
cd tokenhub
cargo build --release

# 部署
mkdir -p /opt/tokenhub/data
cp target/release/tokenhub /opt/tokenhub/
cp -r static /opt/tokenhub/

# 配置 systemd 服务
cp tokenhub.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable tokenhub
systemctl start tokenhub
```

### 4.3 访问地址
- **管理后台：** `http://your-ip:8080/`
- **默认密码：** `admin123`
- **API 端点：** `http://your-ip:8080/{prefix}/chat/completions`

---

## 五、使用示例

### 5.1 创建端点池
```bash
curl -X POST -b "admin_logged_in=true" \
  http://localhost:8080/admin/api/pools \
  -H "Content-Type: application/json" \
  -d '{
    "name": "我的池",
    "description": "用于 GPT-4 模型",
    "schedule_algorithm": "round_robin",
    "model_mode": "passthrough",
    "retry_mode": "pool",
    "retry_count": 3
  }'
```

### 5.2 添加端点
```bash
curl -X POST -b "admin_logged_in=true" \
  http://localhost:8080/admin/api/endpoints \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI GPT-4",
    "url": "https://api.openai.com",
    "api_type": "openai",
    "api_key": "sk-xxx",
    "token_limit": 100000,
    "reset_policy": "manual",
    "request_limit": 500,
    "request_reset_policy": "Rolling5h",
    "enabled": true,
    "pool_ids": ["池ID"],
    "timeout": 300
  }'
```

### 5.3 创建 API 接口
```bash
curl -X POST -b "admin_logged_in=true" \
  http://localhost:8080/admin/api/exposed-apis \
  -H "Content-Type: application/json" \
  -d '{
    "name": "GPT-4 API",
    "prefix": "/v1/gpt4",
    "api_type": "openai",
    "pool_id": "池ID",
    "api_key": "my-secret-key",
    "enabled": true
  }'
```

### 5.4 调用 API（OpenAI 格式）
```bash
curl http://localhost:8080/v1/gpt4/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-secret-key" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 50
  }'
```

### 5.5 调用 API（Anthropic 格式）
```bash
curl http://localhost:8080/v1/claude/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: my-secret-key" \
  -d '{
    "model": "claude-3",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 50
  }'
```

### 5.6 调用 API（Responses 格式）
```bash
curl http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-secret-key" \
  -d '{
    "model": "gpt-4",
    "input": "Hello",
    "max_output_tokens": 50
  }'
```

---

## 六、管理 API 端点列表

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | /admin/api/login | 管理员登录 |
| POST | /admin/api/logout | 管理员登出 |
| GET | /admin/api/auth/status | 检查登录状态 |
| POST | /admin/api/password | 修改密码 |
| GET | /admin/api/stats | 获取统计数据 |
| GET | /admin/api/config | 获取全局配置 |
| PUT | /admin/api/config | 更新全局配置 |
| GET | /admin/api/endpoints | 获取端点列表 |
| POST | /admin/api/endpoints | 创建端点 |
| GET | /admin/api/endpoints/{id} | 获取端点详情 |
| PUT | /admin/api/endpoints/{id} | 更新端点 |
| DELETE | /admin/api/endpoints/{id} | 删除端点 |
| POST | /admin/api/endpoints/{id}/toggle | 切换端点状态 |
| POST | /admin/api/endpoints/{id}/reset | 重置端点 Token |
| POST | /admin/api/endpoints/{id}/reset-requests | 重置端点请求次数 |
| POST | /admin/api/endpoints/reset-all | 重置所有端点 Token |
| POST | /admin/api/endpoints/check | 对话测试 |
| POST | /admin/api/endpoints/models | 浏览模型列表 |
| GET | /admin/api/pools | 获取池列表 |
| POST | /admin/api/pools | 创建池 |
| PUT | /admin/api/pools/{id} | 更新池 |
| DELETE | /admin/api/pools/{id} | 删除池 |
| GET | /admin/api/pools/{id}/models | 获取池内端点模型列表 |
| POST | /admin/api/pools/{id}/test-all | 池一键测试 |
| GET | /admin/api/exposed-apis | 获取 API 接口列表 |
| POST | /admin/api/exposed-apis | 创建 API 接口 |
| PUT | /admin/api/exposed-apis/{id} | 更新 API 接口 |
| DELETE | /admin/api/exposed-apis/{id} | 删除 API 接口 |
| POST | /admin/api/exposed-apis/{id}/toggle | 切换接口状态 |

---

## 七、版本历史

### v1.9 (2026-07-02)
- **池一键测试**
  - 池管理页面新增「一键测试」按钮，可对池内所有端点进行批量对话测试
  - 支持两种测试模式：自动选择（每个端点自动选用第一个可用模型）和手动指定（用户选择模型名，后端通过最长后缀匹配自动映射到各端点的实际模型名）
  - 测试结果展示每个端点的成功/失败状态、使用的模型名、HTTP 状态码和回复内容
  - 新增 API：`GET /admin/api/pools/{id}/models` 获取池内端点模型列表
  - 新增 API：`POST /admin/api/pools/{id}/test-all` 批量测试端点

### v1.8 (2026-06-29)
- **请求次数限额管理**
  - 端点新增 `request_limit` / `request_reset_policy` 配置项
  - 支持三种重置策略：手动重置 / 每日自动重置 / 5小时滚动重置
  - 5小时滚动重置：滑动窗口算法，仅统计最近5小时的请求数，旧请求自动失效
  - `is_available()` 同时检查 Token 限额和请求次数限额，任一达到上限则不可用
  - 端点列表和概览页展示请求次数限额和当前周期已请求次数
  - 新增「重置请求」按钮，仅归零请求次数，不影响 Token 使用量
- **标签统一**
  - "已使用" → "已使用Token"
  - "限额" → "Token限额"
  - "请求限额" → "请求次数限额"
  - "已请求" → "已请求次数"
  - "请求数" → "总请求数"

### v1.7 (2026-06-24)
- **API 格式转换**
  - 新增 converter.rs 模块，支持三种接口类型互相转换
  - OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 格式互转
  - 非流式请求自动转换请求体和响应体
  - 流式请求支持逐块 SSE 格式转换
  - 转换路径自动映射（如 `/chat/completions` → `/messages`）
- **API 认证增强**
  - 支持 Anthropic 的 `x-api-key` 头认证
  - 同时支持 `Authorization: Bearer xxx` 和 `x-api-key: xxx`
- **概览页图表统计**
  - 新增端点状态分布图（正常/异常/禁用）
  - 新增 Token 使用量图表（按端点展示使用率）
  - 新增请求量统计图表（按端点展示请求数）
  - 统计卡片添加图标和渐变背景
  - 图表支持响应式布局

### v1.6 (2026-06-21)
- **重试机制**
  - 池支持配置重试模式：无重试、原地重试、端点重试
  - 无重试：异常直接返回错误
  - 原地重试：异常时继续向原端点重试，直到达到重试次数
  - 端点重试：异常时切换到池内其他端点，每个端点依次尝试
  - 重试次数可配置（1-10）
  - 调度算法中的随机模式不再内置重试逻辑，改由重试机制统一管理
- **错误检测增强**
  - 支持 NVIDIA 格式错误：`{"status":429,"title":"Too Many Requests"}`
  - 支持纯文本错误关键词检测：请求负载过高、请稍后再试等
  - 支持 Anthropic SSE 格式：`event: error`
  - 支持 SSE 内容中嵌入的 JSON 错误对象
  - 全池失败时返回统一提示：「端点池所有接口均不可用，请检查后重试」
- **流式响应优化**
  - 只缓冲第一个 chunk 检查错误，后续直接转发，保留低延迟
- **前端优化**
  - 池编辑页面添加重试机制配置
  - 模型名称传递模式和重试机制说明改为选中后展示对应说明
  - 调度算法随机模式说明更新

### v1.5 (2026-06-19)
- **模型映射功能**
  - 池支持两种模型参数传递模式：透传模式和映射模式
  - 透传模式：自动匹配端点支持的模型，不区分大小写，包含即匹配
  - 映射模式：需手动配置模型名称映射关系
  - 匹配到多个模型时返回错误提示
- **模型缓存机制**
  - 端点模型列表自动缓存
  - 添加/编辑端点时自动更新缓存
  - 每小时定时更新缓存
  - 请求时发现无缓存自动获取
- **Token 限额优化**
  - 限额为空时重置方式自动设为手动重置
  - 限额为空：不自动清零，列表页面显示为「无上限」
  - 限额不为空 + 手动重置：已使用达到限额时标记为不可用，不自动清零
  - 限额不为空 + 每日重置：每天零点自动清零，达到限额时标记为不可用
  - 概览页统计排除无限制端点
- **前端优化**
  - 池编辑页面添加模型参数模式选择
  - 端点管理页面显示已使用和限额
  - 选择端点到池时支持配置模型映射（映射模式下）
  - 编辑端点时限额无限制显示手动重置且不可修改
  - 池管理编辑对话框中显示端点映射配置
  - 端点管理编辑页面不显示模型映射配置
  - 模型映射配置中端点模型改为下拉选择（从模型列表接口获取）
  - 移除池管理列表中端点的重置和禁用按钮
- **后端优化**
  - 添加模型缓存数据结构
  - 添加模型名称匹配函数
  - 修改代理转发逻辑支持模型映射
  - 修复 MutexGuard 跨 await 点问题
  - 取消循环清零逻辑

### v1.4 (2026-06-19)
- **限额重置优化**
  - 限额为空时，重置方式固定为「手动重置」，不可修改
  - 限额不为空时，可自由选择重置方式
  - 手动重置按钮点击后立即将已使用清零
- **端点管理页面优化**
  - 显示字段改为「已使用」和「限额」
  - 无限制端点不显示进度条
  - 无限制端点状态始终显示为「正常」
- **每日自动重置**
  - 每日凌晨自动将每日重置端点的已使用清零
  - 概览页统计排除无限制端点的已使用

### v1.3 (2026-06-19)
- **Token 限额优化**
  - 空限额默认设置为 999999999999（12 个 9）
  - 重置方式默认为手动重置
  - 概览页统计排除无限制端点
  - 剩余数量接近 12 个 9 时统一显示
- **接口管理优化**
  - 接口列表显示调用 URL（http 格式）
  - 移除 Nginx，直接使用 HTTP 访问
- **Bug 修复**
  - 修复路由重复添加 /v1 问题
  - 修复转发请求时认证头覆盖问题
  - 修复剩余显示不一致问题

### v1.2 (2026-06-19)
- **对话测试优化**
  - 对话测试前先弹出模型选择窗口
  - 自动获取端点的可用模型列表
  - 用户选择模型后再进行测试
- **对外接口对话测试**
  - 接口管理页面新增「对话测试」按钮
  - 使用关联池的端点进行测试
- **URL 显示优化**
  - 接口管理列表显示完整调用 URL（http 格式）
  - 输入前缀时实时显示完整调用路径
- **API Key 显示**
  - 编辑端点时回显完整的 API Key（不再掩码）

### v1.1 (2026-06-19)
- **菜单重构**
  - 新增「端点管理」一级菜单，集中管理所有端点
  - 原「端点」菜单改名为「池管理」，只管理端点池
  - 原「API接口」菜单改名为「接口管理」
- **URL 自动补全**
  - 添加端点时只需输入 Base URL（如 `https://api.openai.com`）
  - 后端自动补全 `/v1` 路径前缀
  - 验证接口时自动测试 `/v1/models` 端点
- **端点归属关系优化**
  - 每个端点可归属多个池（`pool_ids` 数组）
  - 从池中移除端点不删除端点本身
  - 「选择端点」只显示未分配到当前池的端点
- **UI/UX 优化**
  - 验证按钮美化（详细结果展示）
  - 取消按钮统一样式
  - 浏览模型按钮集成到每个端点卡片
- **Bug 修复**
  - 修复选择端点到池时缺少必填字段的问题
  - 修复从池中移除端点时缺少必填字段的问题

### v1.0 (2026-06-19)
- 初始版本发布
- 支持多端点代理池管理
- 支持三种调度算法（轮询/轮换/随机）
- 支持 OpenAI / Anthropic / OpenAI Responses 接口类型
- 支持端点超时时间配置
- 支持 Token 限额管理（手动/每日自动重置）
- 支持自定义 API 前缀路由
- 支持独立的 API 认证密钥
- 支持流式响应转发和 Token 统计
- 深色模式 Web 管理后台

---

*文档更新时间：2026-07-02*
