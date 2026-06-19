# Token Pool

> LLM API Token Pool Manager - 让 Token 自由更近一步

## 功能特性

- 多端点代理池管理
- 三种调度算法（轮询/轮换/随机）
- 支持 OpenAI / Anthropic / OpenAI Responses 接口类型
- Token 限额管理与实时统计
- 自定义 API 前缀路由
- Web 管理后台
- HTTPS 支持（Nginx 反向代理）

## 快速部署

### 1. 环境要求

- Linux 操作系统
- Rust 编译环境（1.75+）
- Nginx（可选，用于 HTTPS）

### 2. 编译

```bash
cd token-pool
cargo build --release
```

### 3. 部署

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

### 4. 验证

```bash
# 检查服务状态
systemctl status token-pool

# 健康检查
curl http://localhost:8080/health
```

## 访问地址

| 项目 | 地址 |
|------|------|
| 管理后台 | `http://your-ip:8080/admin/` |
| 默认密码 | `admin123` |
| API 端点 | `http://your-ip:8080/v1/chat/completions` |

## Nginx HTTPS 配置

```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.com;

    ssl_certificate /etc/nginx/ssl/cert.pem;
    ssl_certificate_key /etc/nginx/ssl/key.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_read_timeout 300s;
    }
}
```

## 使用示例

```bash
# 调用 API
curl https://your-domain.com/v1/chat/completions \
  -H "Content-Type: application/json" \
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
