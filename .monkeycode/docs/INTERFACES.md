# TokenHub 接口

所有 `/admin/api/*` 接口由管理员会话保护。登录成功后，客户端通过 Cookie 维持会话。对外代理接口由对外 API 的认证配置决定访问方式。

## 公共接口

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/health` | 返回服务健康状态。 |
| `GET` | `/` | 重定向至 `/admin/`。 |
| `*` | `/{exposed-api-prefix}` | 匹配对外 API 后转发至端点池。 |

## 管理认证

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `POST` | `/admin/api/login` | 创建管理员会话。 |
| `POST` | `/admin/api/logout` | 销毁当前管理员会话。 |
| `GET` | `/admin/api/auth/status` | 查询会话状态。 |
| `POST` | `/admin/api/password` | 修改管理密码。 |

## 端点与端点池

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET`、`POST` | `/admin/api/endpoints` | 查询或创建上游端点。 |
| `GET`、`PUT`、`DELETE` | `/admin/api/endpoints/{id}` | 查询、更新或删除端点。 |
| `POST` | `/admin/api/endpoints/check` | 测试端点。 |
| `POST` | `/admin/api/endpoints/models` | 浏览端点模型。 |
| `GET`、`POST` | `/admin/api/pools` | 查询或创建端点池。 |
| `PUT`、`DELETE` | `/admin/api/pools/{id}` | 更新或删除端点池。 |

## 对外 API 与运行记录

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET`、`POST` | `/admin/api/exposed-apis` | 查询或创建对外 API。 |
| `GET`、`PUT`、`DELETE` | `/admin/api/exposed-apis/{id}` | 管理单个对外 API。 |
| `POST` | `/admin/api/exposed-apis/{id}/replay-toggle` | 切换数据回放。 |
| `GET`、`DELETE` | `/admin/api/exposed-apis/{id}/replay-records` | 查询或清空回放记录。 |
| `GET`、`PUT` | `/admin/api/replay-config` | 管理回放配置。 |
| `GET` | `/admin/api/logs` | 查询最近调用日志。 |
| `GET` | `/admin/api/latency-leaderboard` | 查询端点延迟统计。 |

## 模型评测

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET`、`POST` | `/admin/api/model-benchmarks` | 查询或创建评测任务。 |
| `GET` | `/admin/api/model-benchmarks/candidates` | 查询候选端点及模型。 |
| `GET` | `/admin/api/model-benchmarks/{id}` | 查询任务详情与汇总。 |
| `POST` | `/admin/api/model-benchmarks/{id}/cancel` | 取消执行中的任务。 |

## 技能仓库接口规划

技能仓库的数据模型、本地服务与公开来源适配器已实现。下列管理接口等待后续任务注册：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/admin/api/skills` | 查询本地技能列表。 |
| `GET` | `/admin/api/skills/{id}` | 查询技能详情与文件清单。 |
| `POST` | `/admin/api/skills/upload-preview` | 校验上传内容并生成导入预览。 |
| `POST` | `/admin/api/skills/import` | 确认导入预览。 |
| `GET` | `/admin/api/skill-sources/search` | 搜索公开技能来源。 |
