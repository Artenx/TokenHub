# 技能在线更新

Feature Name: skill-online-update
Updated: 2026-07-22

## Description

技能导入预览在校验完成后计算 ZIP 归档 SHA-256 摘要，并将摘要写入技能来源元数据。管理页面使用已有的链接预览接口重新获取来源内容，前端比较摘要后复用导入替换流程。

## Components and Interfaces

- `SkillOrigin.content_digest`：保存经过目标目录提取后的归档摘要。
- `POST /admin/api/skill-links/preview`：作为在线更新检查接口，返回当前远端内容的预览及摘要。
- 技能详情弹窗：对 HTTPS 来源显示检查在线更新按钮。
- Nginx：设置 `client_max_body_size 50m`，与应用技能仓库的 50MB 单文件限制一致。

## Correctness Properties

- 相同来源内容产生相同 SHA-256 摘要。
- 摘要相同时，系统保持本地技能目录和元数据。
- 摘要变化后，系统仅在管理员确认替换时修改本地技能内容。

## Test Strategy

- 测试链接预览返回 SHA-256 来源摘要。
- 测试前端对相同摘要展示已是最新状态。
- 使用 1.5MB ZIP 请求验证 Nginx 将上传转发给应用服务。
