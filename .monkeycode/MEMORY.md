# 用户指令记忆

本文件记录了用户的指令、偏好和教导，用于在未来的交互中提供参考。

## 格式

### 用户指令条目
用户指令条目应遵循以下格式：

[用户指令摘要]
- Date: [YYYY-MM-DD]
- Context: [提及的场景或时间]
- Instructions:
  - [用户教导或指示的内容，逐行描述]

### 项目知识条目
Agent 在任务执行过程中发现的条目应遵循以下格式：

[项目知识摘要]
- Date: [YYYY-MM-DD]
- Context: Agent 在执行 [具体任务描述] 时发现
- Category: [运维部署|构建方法|测试方法|排错调试|工作流协作|环境配置]
- Instructions:
  - [具体的知识点，逐行描述]

## 去重策略
- 添加新条目前，检查是否存在相似或相同的指令
- 若发现重复，跳过新条目或与已有条目合并
- 合并时，更新上下文或日期信息
- 这有助于避免冗余条目，保持记忆文件整洁

## 条目

### TokenHub 38 服务器部署
- Date: 2026-07-16
- Context: Agent 在恢复 TokenHub Nginx 路由时发现
- Category: 运维部署
- Instructions:
  - 38.76.209.172 上的 TokenHub 由 `tokenhub.service` 管理，服务监听 `127.0.0.1:8080`。
  - Nginx 容器名为 `nginx`，使用 Docker `host` 网络，配置文件由 `/opt/lantern-notes/nginx.conf` 挂载至容器的 `/etc/nginx/conf.d/default.conf`。
  - 保留 `/wxapi/` 到 `127.0.0.1:9090` 的独立代理；其他路径代理至 `127.0.0.1:8080`。

### 修改后自动推送到 main 分支
- Date: 2026-07-03
- Context: 用户明确指示，完成代码修改后直接推送到 main 分支，不再创建功能分支或 Merge Request
- Instructions:
  - 每次修改完成后，执行 git add、commit 并直接 push 到 origin/main 分支
  - 不创建临时功能分支，不通过 Merge Request 合并
