# Requirements Document

## Introduction

TokenHub 的「接口管理」页面用于管理对外暴露的统一 API 接口（ExposedApi）。当前系统仅能记录每次调用的元数据（时间、IP、路径、状态码、Token 用量），无法查看请求体与响应体的完整内容，导致线上问题排查与联调核对困难。

本特性为每个对外接口增加独立的「数据回放」能力：用户在接口卡片上开启回放开关后，系统自动捕获该接口后续每次调用的完整请求体与响应体，并在管理后台以格式化方式展示，便于调试、审计与问题复现。

## Glossary

- **ExposedApi**：对外暴露的统一 API 接口，由前缀、接口类型、关联池、认证密钥等配置项组成。
- **Replay（数据回放）**：在代理转发链路中同步捕获请求体与响应体并临时保存，供事后查看的能力。
- **Replay 开关**：位于 ExposedApi 上的布尔型配置项，控制是否对该接口启用 Replay。
- **Replay 记录**：一次调用产生的请求/响应快照，包含时间、方法、路径、状态码、耗时、请求体、响应体、错误信息。
- **管理后台**：TokenHub 内置的 Web 管理界面（`/admin`）。
- **调用方**：通过 ExposedApi 前缀发起请求的客户端。

## Requirements

### Requirement 1 — Replay 开关配置

**User Story:** AS 管理员，I want 在每个对外接口上独立开启或关闭数据回放，so that 我只为需要排查的接口记录完整请求/响应内容，避免不必要的存储开销。

#### Acceptance Criteria

1. WHEN 管理员在接口管理页面点击某个 ExposedApi 的「数据回放」开关，THE 系统 SHALL 切换该接口的 `replay_enabled` 状态并立即生效。
2. WHEN 管理员创建或编辑 ExposedApi，THE 系统 SHALL 提供 `replay_enabled` 配置项，且默认值为关闭。
3. WHILE `replay_enabled` 处于开启状态，THE 系统 SHALL 在接口卡片上以高亮样式展示「回放中」标识。
4. WHEN 系统加载历史配置文件且配置中不存在 `replay_enabled` 字段，THE 系统 SHALL 将该字段视为关闭，不中断启动流程。
5. WHEN 管理员关闭 Replay 开关，THE 系统 SHALL 停止为该接口写入新的 Replay 记录，且保留已存在的 Replay 记录。

### Requirement 2 — 请求/响应内容捕获

**User Story:** AS 管理员，I want 系统在接口开启回放后自动捕获每次调用的请求体与响应体，so that 我可以看到一次调用完整、真实的数据内容。

#### Acceptance Criteria

1. WHEN 调用方通过 `replay_enabled = true` 的 ExposedApi 完成一次非流式调用，THE 系统 SHALL 记录原始请求体、最终返回给调用方的响应体、HTTP 状态码、耗时与发生时间。
2. WHEN 调用方通过 `replay_enabled = true` 的 ExposedApi 完成一次流式调用，THE 系统 SHALL 在流结束后将所有响应数据块按顺序拼接为完整响应体并记录。
3. WHEN 调用因上游错误、池内无可用端点等原因失败，THE 系统 SHALL 仍然记录请求体、错误信息与 HTTP 状态码，响应体字段记录上游返回的错误内容或系统生成的错误描述。
4. WHEN 请求体或响应体的字节长度超过配置的截断阈值（默认 1 MB），THE 系统 SHALL 仅保留前该阈值长度的内容并在记录中标记为「已截断」。
5. WHEN 调用经过端点重试，THE 系统 SHALL 仅记录最终返回给调用方的那一次请求/响应内容，不记录中间失败尝试。

### Requirement 3 — Replay 记录存储、保留与可配置参数

**User Story:** AS 管理员，I want Replay 记录按接口隔离保存、持久化到独立文件，且保留条数与截断阈值可配置，so that 我可以根据磁盘空间与调试需求灵活调整，且服务重启后仍能回看历史回放。

#### Acceptance Criteria

1. THE 系统 SHALL 按 ExposedApi ID 隔离存储 Replay 记录，接口之间互不可见。
2. WHEN 某个 ExposedApi 的 Replay 记录数量达到配置的上限值，THE 系统 SHALL 在写入新记录时淘汰最旧的一条记录，使总数保持不超过该上限。
3. WHEN 管理员删除某个 ExposedApi，THE 系统 SHALL 同步清除该接口对应的全部 Replay 记录。
4. THE 系统 SHALL 将 Replay 记录持久化到独立的 JSON 文件，并在服务重启后自动恢复。
5. WHEN 管理员点击「清空回放」，THE 系统 SHALL 删除该接口当前全部 Replay 记录并返回成功响应。
6. THE 系统 SHALL 在全局配置文件中提供 `replay.max_records_per_api` 配置项，用于控制每个接口最多保留的记录条数，默认值为 50。
7. THE 系统 SHALL 在全局配置文件中提供 `replay.state_file_path` 配置项，用于控制回放记录持久化文件的存放路径，默认值为配置文件所在目录下的 `replay_state.json`。
8. THE 系统 SHALL 在全局配置文件中提供 `replay.max_body_size_kb` 配置项，用于控制请求体/响应体的截断阈值（单位 KB），默认值为 1024（即 1 MB）。
9. WHEN 系统加载配置文件且配置中不存在 `replay` 配置节，THE 系统 SHALL 使用全部默认值，不中断启动流程。
10. WHEN 系统向持久化文件写入 Replay 记录，THE 系统 SHALL 仅序列化每个 ExposedApi 最近 `replay.max_records_per_api` 条记录。

### Requirement 4 — Replay 记录查询与格式化展示

**User Story:** AS 管理员，I want 在接口管理页面查看每个接口的回放记录并以 JSON 格式美化展示请求体与响应体，so that 我可以快速阅读与核对数据。

#### Acceptance Criteria

1. WHEN 管理员在接口卡片上点击「回放记录」，THE 系统 SHALL 展示该接口的 Replay 记录列表，按发生时间倒序排列。
2. WHEN Replay 记录列表为空，THE 系统 SHALL 显示「暂无回放记录」的占位提示。
3. WHEN 管理员展开某条 Replay 记录，THE 系统 SHALL 分别展示请求体与响应体；当内容为合法 JSON 时，系统 SHALL 以带缩进的美化格式渲染，当内容为非 JSON 文本时，系统 SHALL 以原文展示。
4. WHEN 某条 Replay 记录被标记为「已截断」，THE 系统 SHALL 在该记录详情区域显示明显的截断提示。
5. WHEN 某条 Replay 记录为失败调用，THE 系统 SHALL 在列表项上以错误样式标识，并在详情区域展示错误信息。
6. WHEN 管理员点击「刷新」，THE 系统 SHALL 重新拉取该接口最新的 Replay 记录列表。

### Requirement 5 — 安全与权限

**User Story:** AS 系统所有者，I want Replay 数据只能被已通过管理后台认证的访问者读取，so that 调用方数据不被未授权泄露。

#### Acceptance Criteria

1. WHEN 未认证客户端请求 Replay 记录的读取、清空或开关切换接口，THE 系统 SHALL 返回 HTTP 401 响应。
2. WHEN 系统在 Replay 记录中保存请求体，THE 系统 SHALL 完整保留请求体原文，不对其中的 Authorization、api-key 等字段做自动脱敏，并在界面上以提示告知管理员该数据可能包含敏感信息。
