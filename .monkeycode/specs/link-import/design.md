# 通过链接导入技能

Feature Name: link-import
Updated: 2026-07-21

## 描述

链接导入为技能仓库增加独立预览入口。前端提交 URL 后，后端将 GitHub 链接规范化为仓库归档和目标目录，或将公开 HTTPS ZIP 直接交给既有技能包预览器。成功预览仍需要管理员明确确认才会写入本地仓库。

## 架构

```mermaid
flowchart LR
    A[管理员粘贴链接] --> B[链接预览接口]
    B --> C[GitHub 链接规范化]
    B --> D[公开 ZIP 地址校验]
    C --> E[受控下载]
    D --> E
    E --> F[技能 ZIP 预览校验]
    F --> G[确认导入]
```

## 组件与接口

- `static/index.html` 提供链接输入框和预览按钮。
- `static/app.js` 提交 URL，复用 `showSkillImportPreview` 展示预览结果。
- `POST /admin/api/skill-links/preview` 接收 `{ "url": "https://..." }` 并返回现有导入预览模型。
- `src/admin.rs` 解析 GitHub 路径、校验公开 ZIP 地址、下载内容并调用 `preview_zip_archive`。

## 正确性约束

- 仅管理员会话可以创建链接预览。
- 只有 HTTPS URL 可以进入下载流程。
- URL 不包含用户名或密码，端口固定为 HTTPS 默认端口。
- 域名解析结果只包含公开单播地址。
- 下载客户端关闭重定向，归档总量沿用仓库限制。
- GitHub 提取后的归档只包含指定技能目录，且目录必须包含 `SKILL.md`。

## 错误处理

- URL 格式、协议、主机、端口和地址校验失败返回 `400`。
- GitHub 路径不包含目标目录、分支或标签时返回 `400`。
- 远端 HTTP 非成功状态返回包含状态码的 `400`。
- ZIP 结构校验继续使用本地技能包服务的错误信息。

## 测试策略

- 单元测试 GitHub 目录与 `SKILL.md` 链接的路径解析。
- 单元测试 URL 协议、凭据、端口和受限 IP 地址校验。
- 管理接口测试无管理员会话、非法 URL 和有效预览流程。
- 前端语法检查与浏览器预览验证链接表单、预览弹窗和冲突替换。

## 参考

- `src/admin.rs`
- `src/skill_repository.rs`
- `static/app.js`
- `static/index.html`
