# Token Pool - LLM API Token 池管理系统

## 需求说明文档

---

## 一、项目概述

Token Pool 是一个基于 Rust 开发的 Web 服务项目，用于代理和管理多个大模型（LLM）API 端点，形成一个统一的 Token 池管理系统。系统对外暴露统一的 API 接口，内部将请求智能分发到不同的后端端点，并实时统计 Token 使用情况。

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
- **超时时间** - 请求超时时间（秒），默认 300 秒
- **限额重置方式**：
  - `manual` - 一次性手动重置
  - `daily` - 每日零点自动重置
- **启用/禁用** - 可动态切换端点状态
- **所属池** - 端点归属于某个端点池（每个端点只能归属一个池）

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

**端点与池的关系：**
- 每个端点只能归属一个池
- 从池中移除端点不会删除端点本身，只是清除端点的 pool_id
- 移除的端点会重新出现在「选择端点」的可选列表中
- 「选择端点」只显示未分配到任何池的端点

**层级关系：**
```
端点池 (Pool)
  ├── 代理端点 A
  ├── 代理端点 B
  └── 代理端点 C
```

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
- **重试规则：** 当请求失败（非200响应）时，自动重试下一个端点，依次尝试直到成功
- **跳过规则：** 额度耗尽的端点会被跳过
- **适用场景：** 高可用场景，自动故障转移

### 2.4 API 接口管理

**功能描述：** 对外暴露的统一 API 接口，支持自定义前缀和独立认证。

**接口配置项：**
- **名称** - 接口的显示名称
- **URL 前缀** - 客户端请求时使用的路径前缀（如 `/v1`、`/v1/gpt4`、`/v1/responses`）
- **接口类型** - 对外暴露的接口格式（openai / anthropic / openai-responses）
- **关联池** - 该接口关联到哪个端点池
- **API 认证密钥** - 调用此接口时需要提供的认证密钥，留空则不需要认证
- **启用/禁用** - 可动态切换接口状态

**调用示例显示：**
- 接口列表中显示完整调用 URL（https 格式）
- 输入前缀时实时显示完整调用路径

**对话测试：**
- 点击「对话测试」按钮弹出模型选择窗口
- 自动从关联池的端点获取可用模型列表
- 用户选择模型后进行测试
- 显示模型回复结果

**路由机制：**
- 请求路径匹配前缀后，转发到关联池中的端点
- 自动剥离前缀，拼接目标路径

### 2.5 Token 统计与限额管理

**功能描述：** 自动统计并记住每个端点已消耗的 Token。

**统计内容：**
- 每个端点的 Token 使用量
- 每个端点的请求次数
- 每个端点的错误次数
- 每个池的汇总统计
- 全局汇总统计

**限额管理：**
- 支持为每个端点设置 Token 限额
- 限额为 0 表示无上限
- 支持手动重置单个或所有端点的 Token 使用量
- 支持每日零点自动重置（Daily 策略）

**Token 解析：**
- OpenAI 格式：从响应中解析 `usage.total_tokens`
- Anthropic 格式：从响应中解析 `usage.input_tokens + usage.output_tokens`
- 流式响应：从最后一个 SSE 事件中解析 usage 字段

### 2.6 Web 管理后台

**功能描述：** 提供 Web 管理界面，支持密码认证登录。

**页面结构：**

#### 概览页
- 端点总数 / 活跃端点数
- 已用 Token / Token 限额
- Token 使用率（进度条）
- 总请求数 / 错误数
- 端点池数 / API 接口数
- 端点池概览列表
- API 接口概览列表
- 端点状态详情

#### 端点管理页
- 端点列表（显示所有端点，不受池的限制）
- 添加端点功能
- 每个端点卡片包含：
  - 编辑 / 启用禁用 / 重置Token / 浏览模型 / 删除 操作按钮
  - 「浏览模型」按钮调用 API 获取真实模型列表
  - 「对话测试」按钮（弹出模型选择后测试）
- 编辑端点时回显完整的 API Key

#### 池管理页
- 端点池列表
- 添加池功能
- 每个池卡片包含：
  - 池内端点列表
  - 「选择端点」按钮 - 从端点列表中选择未分配的端点添加到池
  - 端点操作：编辑 / 启用禁用 / 重置 / 从池中移除
- 「从池中移除」只清除端点的池关联，不删除端点

#### 接口管理页
- 对外接口列表
- 每个接口显示完整调用 URL（https 格式）
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

### 2.7 API 认证

**功能描述：** 对外暴露的统一 API 接口支持可选的认证。

**认证方式：**
- 请求头：`Authorization: Bearer <api_key>`
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
- **前端：** 原生 HTML / CSS / JavaScript

### 3.2 项目结构
```
token-pool/
├── src/
│   ├── main.rs        # 入口，路由配置
│   ├── models.rs      # 数据模型定义
│   ├── config.rs      # 配置文件管理
│   ├── state.rs       # 应用状态管理
│   ├── scheduler.rs   # 调度算法实现
│   ├── proxy.rs       # API 代理转发（含 URL 自动补全）
│   ├── auth.rs        # 认证中间件
│   ├── admin.rs       # 管理后台 API（含浏览模型、对话测试）
│   ├── validator.rs   # 输入验证工具
│   └── error.rs       # 错误类型定义
├── static/
│   ├── index.html     # 管理后台页面
│   ├── style.css      # 样式文件
│   └── app.js         # 前端逻辑
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
url = "https://api.openai.com"  # Base URL，系统自动补全 /v1
api_type = "openai"
api_key = "sk-xxx"
token_limit = 100000
reset_policy = "manual"
enabled = true
pool_id = "pool-1"
timeout = 300
```

---

## 四、部署说明

### 4.1 系统要求
- Linux 操作系统
- 已安装 Rust 编译环境
- 已安装 Nginx（用于 HTTPS 反向代理）

### 4.2 编译与部署
```bash
# 编译
cd token-pool
cargo build --release

# 部署
mkdir -p /opt/token-pool
cp target/release/token-pool /opt/token-pool/
cp -r static /opt/token-pool/

# 配置 systemd 服务
cp token-pool.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable token-pool
systemctl start token-pool
```

### 4.3 Nginx 配置（HTTPS）
```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.com;
    
    ssl_certificate /etc/nginx/ssl/cert.pem;
    ssl_certificate_key /etc/nginx/ssl/key.pem;
    
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_read_timeout 300s;
    }
}
```

### 4.4 访问地址
- **管理后台：** `https://your-domain.com/admin/`
- **默认密码：** `admin123`
- **API 端点：** `https://your-domain.com/v1/chat/completions`

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
    "schedule_algorithm": "round_robin"
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
    "enabled": true,
    "pool_id": "池ID",
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

### 5.4 调用 API
```bash
curl https://your-domain.com/v1/gpt4/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-secret-key" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 50
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
| POST | /admin/api/endpoints/reset-all | 重置所有端点 Token |
| POST | /admin/api/endpoints/check | 对话测试 |
| POST | /admin/api/endpoints/models | 浏览模型列表 |
| GET | /admin/api/pools | 获取池列表 |
| POST | /admin/api/pools | 创建池 |
| PUT | /admin/api/pools/{id} | 更新池 |
| DELETE | /admin/api/pools/{id} | 删除池 |
| GET | /admin/api/exposed-apis | 获取 API 接口列表 |
| POST | /admin/api/exposed-apis | 创建 API 接口 |
| PUT | /admin/api/exposed-apis/{id} | 更新 API 接口 |
| DELETE | /admin/api/exposed-apis/{id} | 删除 API 接口 |
| POST | /admin/api/exposed-apis/{id}/toggle | 切换接口状态 |

---

## 七、版本历史

### v1.5 (2026-06-19)
- **模型映射功能**
  - 池支持两种模型参数传递模式：透传模式和映射模式
  - 透传模式：客户端直接使用端点支持的模型名称
  - 映射模式：客户端使用统一名称，后端自动匹配到端点实际模型
  - 匹配规则：不区分大小写，后缀相同即认为匹配
  - 如果完全一致（包括大小写）则不替换
- **模型缓存机制**
  - 端点模型列表自动缓存
  - 添加/编辑端点时自动更新缓存
  - 每小时定时更新缓存
  - 请求时发现无缓存自动获取
- **前端优化**
  - 池编辑页面添加模型参数模式选择
  - 端点管理页面显示今日已用和限额
- **后端优化**
  - 添加模型缓存数据结构
  - 添加模型名称匹配函数
  - 修改代理转发逻辑支持模型映射
  - 修复 MutexGuard 跨 await 点问题

### v1.4 (2026-06-19)
- **限额重置优化**
  - 限额为空时，重置方式固定为「每日自动重置」，不可修改
  - 限额不为空时，可自由选择重置方式
  - 手动重置按钮点击后立即将已使用清零
- **端点管理页面优化**
  - 显示字段改为「今日已用」和「限额」
  - 无限制端点不显示进度条
  - 无限制端点状态始终显示为「正常」
- **每日自动重置**
  - 每日凌晨自动将每日重置端点的已使用清零
  - 概览页统计排除无限制端点的已使用

### v1.3 (2026-06-19)
- **Token 限额优化**
  - 空限额默认设置为 999999999999（12 个 9）
  - 重置方式默认为每日零点自动重置
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
  - 接口管理列表显示完整调用 URL（https 格式）
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
  - 每个端点只能归属一个池
  - 从池中移除端点不删除端点本身
  - 「选择端点」只显示未分配的端点
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
- 登录页面酷炫动画效果
- Nginx HTTPS 反向代理支持

---

*文档更新时间：2026-06-19*
