# Token Pool

> LLM API Token Pool Manager - 让 Token 自由更近一步

## 功能特性

- **多端点代理池** - 端点可分配到不同池，支持透传模式和映射模式
- **三种调度算法** - 轮询、轮换、随机（随机模式请求失败时自动重试其他端点）
- **模型名称透传** - 透传模式不区分大小写自动匹配模型名称，映射模式按配置精确匹配
- **端点模型缓存** - 启动时自动获取端点模型列表并缓存，每小时更新
- **Token 限额** - 支持无上限、手动重置、每日零点自动重置
- **自定义 API 前缀** - 通过接口管理配置对外访问路径，支持独立认证密钥
- **对话测试** - 端点和接口页面支持在线发送测试请求
- **Web 管理后台** - 端点管理、池管理、接口管理、设置

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
