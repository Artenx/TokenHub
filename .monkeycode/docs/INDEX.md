# TokenHub 项目文档

本文档覆盖 TokenHub 的系统结构、管理接口和本地开发方式，面向维护代理服务及管理后台的开发者。

## 核心文档

- `ARCHITECTURE.md`：系统模块、请求链路和持久化方式。
- `INTERFACES.md`：公共接口、管理 API 和技能仓库接口。
- `DEVELOPER_GUIDE.md`：本地运行、状态持久化和开发流程。

## 规格

- `.monkeycode/specs/api-data-replay/`：API 数据回放。
- `.monkeycode/specs/model-benchmark/`：模型评测。
- `.monkeycode/specs/skill-repository/`：本地技能管理与公开来源搜索。

## 核心模块

| 模块 | 职责 |
| --- | --- |
| `src/main.rs` | 服务启动、路由与后台任务。 |
| `src/admin.rs` | 管理后台接口。 |
| `src/state.rs` | 共享状态与 JSON 持久化。 |
| `src/models.rs` | 配置、端点、评测和技能仓库模型。 |
| `src/proxy.rs` | API 代理与流式响应处理。 |
| `src/scheduler.rs` | 端点池调度。 |
| `src/converter.rs` | API 协议转换。 |
