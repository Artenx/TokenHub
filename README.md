# Token Pool

> LLM API 代理池管理工具 — 多端点负载均衡、Token 限额管控、自定义路由

## 功能特性

- **多端点代理池** - 端点按池分组，支持透传和映射两种模型名称模式
- **负载均衡调度** - 轮询、轮换、随机三种算法，随机模式失败自动重试
- **Token 限额管控** - 支持无上限、手动重置、每日自动重置
- **自定义路由** - 配置对外 API 前缀，支持独立认证密钥
- **Web 管理后台** - 可视化管理端点、池、接口和系统设置

## 快速部署

### 方式一：直接编译部署

#### 1. 环境要求

- Linux 操作系统
- Rust 编译环境（1.75+）

#### 2. 编译

```bash
cd token-pool
cargo build --release
```

#### 3. 部署

```bash
# 创建部署目录
mkdir -p /opt/token-pool

# 复制文件
cp target/release/token-pool /opt/token-pool/
cp -r static /opt/token-pool/

# 安装 systemd 服务
cp token-pool.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable token-pool
systemctl start token-pool
```

#### 4. 验证

```bash
# 检查服务状态
systemctl status token-pool

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
cd token-pool

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
docker build -t token-pool .

# 运行容器
docker run -d \
  --name token-pool \
  -p 8080:8080 \
  -v $(pwd)/data:/app/data \
  --restart unless-stopped \
  token-pool
```

#### 4. 验证

```bash
# 检查容器状态
docker ps | grep token-pool

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

配置文件位于 `/opt/token-pool/config.toml`，首次启动自动生成。

## 详细文档

完整需求说明请查看 [REQUIREMENTS.md](./REQUIREMENTS.md)

## License

Crafted by Artenx
