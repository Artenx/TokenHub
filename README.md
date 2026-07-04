# TokenHub

> LLM API 接口池管理工具 — 多端点负载均衡、Token 限额管控

[English](./README.en.md)

## 功能特性

- **多端点代理池** - 端点按池分组，支持透传和映射两种模型名称模式
- **负载均衡调度** - 轮询、轮换、随机三种算法，随机模式失败自动重试
- **Token 限额管控** - 支持无上限、手动重置、每日自动重置、5小时滚动重置
- **请求次数限额** - 独立于 Token 的请求次数上限，支持手动/每日/5小时滚动/每分钟自动重置
- **滑动窗口算法** - 5小时滚动窗口，旧消耗自动失效，无需手动清零
- **端点延迟统计** - 自动采集每端点请求延迟，提供 P50/P90/P95 等聚合指标与排行榜
- **IP 限流防扫描** - 对未命中对外 API 的请求按 IP 限流，秒级阈值与累计封禁
- **自定义路由** - 配置对外 API 前缀，支持独立认证密钥
- **Web 管理后台** - 可视化管理端点、池、接口、延迟排行和系统设置

## 快速部署

### 方式一：直接编译部署

#### 1. 环境要求

- Linux 操作系统
- Rust 编译环境（1.75+）

#### 2. 编译

```bash
cd tokenhub
cargo build --release
```

#### 3. 部署

```bash
# 创建部署目录
mkdir -p /opt/tokenhub

# 复制文件
cp target/release/tokenhub /opt/tokenhub/
cp -r static /opt/tokenhub/

# 安装 systemd 服务
cp tokenhub.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable tokenhub
systemctl start tokenhub
```

#### 4. 验证

```bash
# 检查服务状态
systemctl status tokenhub

# 健康检查
curl http://localhost:8080/health
```

---

### 方式二：Docker 部署

#### 1. 环境要求

- Docker
- Docker Compose（可选）

#### 2. 使用 Docker Compose（推荐）

```bash
cd tokenhub

# 创建数据目录
mkdir -p data

# 构建并启动
docker-compose up -d

# 查看日志
docker-compose logs -f

# 停止服务
docker-compose down
```

#### 3. 使用 Docker 命令

```bash
# 构建镜像
docker build -t tokenhub .

# 运行容器
docker run -d \
  --name tokenhub \
  -p 8080:8080 \
  -v $(pwd)/data:/app/data \
  --restart unless-stopped \
  tokenhub
```

#### 4. 验证

```bash
# 检查容器状态
docker ps | grep tokenhub

# 健康检查
curl http://localhost:8080/health
```

## 访问地址

| 项目 | 地址 |
|------|------|
| 管理后台 | `http://your-ip:8080/admin/` |
| 默认密码 | `admin123` |
| API 端点 | `http://your-ip:8080/{prefix}/chat/completions` |

## 使用示例

```bash
# 通过接口管理的前缀调用 API
curl http://your-ip:8080/your-prefix/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-api-key" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 50
  }'
```

## 配置文件

配置文件位于 `/opt/tokenhub/config.toml`，首次启动自动生成。

## 详细文档

完整需求说明请查看 [REQUIREMENTS.md](./REQUIREMENTS.md)

## License

MIT License - 详见 [LICENSE](./LICENSE)

## 致谢

本项目创世版本由小米 MiMo-v2.5-Pro 生成。
