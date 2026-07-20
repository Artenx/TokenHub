# TokenHub 开发者指南

## 环境

项目使用 Rust 2021、Cargo 和静态前端资源。默认服务端口为 `8080`，配置文件路径可由 `CONFIG_PATH` 环境变量指定；缺省配置文件名为 `config.toml`。

```bash
cargo run

cargo test

cargo check
```

访问 `http://localhost:8080/admin/` 使用管理后台。开发环境提供的默认管理密码由 `AppConfig::default` 定义，部署前应在配置中更新。

## 代码组织

- 在 `models.rs` 增加可序列化的领域模型与默认值。
- 在 `state.rs` 增加运行时状态、启动恢复、保存逻辑及同模块测试。
- 在 `admin.rs` 添加管理接口，并先调用 `check_admin_auth`。
- 在 `main.rs` 注册管理路由，保持兜底代理路由位于最后。
- 在 `static/` 中同步维护页面结构、交互逻辑与样式。

## 持久化约定

持久化状态使用配置目录中的 JSON 文件。新状态域应使用独立状态文件，避免增加 `state.json` 的写入负担。`AppState::new` 负责启动恢复，`save_runtime_state` 负责周期性保存触发。

技能仓库状态文件为 `skill_repository.json`：

- `AppConfig.skill_repository` 保存根目录与导入容量限制。
- `SkillRepositoryState` 保存来源、已导入技能元数据和审计记录。
- `SkillImportPreview` 仅保存在内存中，读取时自动清理过期预览。

本地技能包服务位于 `src/skill_repository.rs`。归档预览和本地扫描统一校验 `SKILL.md`、相对路径、符号链接、单文件容量、总容量和文件数量。导入操作先写入同级临时目录，再通过重命名提交目标目录。

公开来源适配器位于 `src/skill_sources.rs`。适配器以统一 `SkillSourceAdapter` 接口提供搜索能力；GitHub 使用公开代码搜索，SkillHub 使用 `https://api.skillhub.cn/api/skills`，自定义来源使用 JSON 文档中的 `skills` 数组。聚合搜索会隔离单个来源的超时和协议错误。

## 测试

单元测试与相应模块共同维护，状态层测试位于 `src/state.rs`，本地技能包服务测试位于 `src/skill_repository.rs`，公开来源适配器测试位于 `src/skill_sources.rs`。提交前运行：

```bash
cargo test

cargo check

git diff --check
```

当前工具链未提供 `cargo-fmt` 组件时，格式检查会失败；完成 Rust 工具链组件安装后运行 `cargo fmt --check`。

## 添加管理功能

1. 在 `models.rs` 定义请求、响应或持久化模型。
2. 在 `state.rs` 实现状态变更与持久化。
3. 在 `admin.rs` 添加鉴权接口与接口测试。
4. 在 `main.rs` 注册路由。
5. 在 `static/` 增加导航、视图、交互和样式。
6. 运行全量测试与静态检查。
