# 运行指南 (Make + Docker Compose)

本指南说明如何在本地启动和管理完整的服务栈。

## 前置条件
- Docker Desktop（或 Docker Engine + Compose 插件）
- `make`

检查是否已安装：
```bash
docker --version
docker compose version
make --version
```

## 一键启动（推荐）
在项目根目录下执行：
```bash
cd cowboy

# 创建 .env 文件并填入你的 API 密钥（仅需配置一次）
cat > .env <<EOF
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
EOF

make
```

等同于：
```bash
make up
```

该命令将启动以下服务：
- Kafka + Zookeeper
- Kafka 主题初始化（`game.commands.v1`、`game.steps.v1`）
- DynamoDB Local
- DynamoDB 表初始化（`default_maps`、`game_instances`、`game_steps`、`bot_players`、`bot_llm_logs`）
- 后端服务（game-manager、web、game、timer、game-watcher、bot-manager、bot-service）
- 前端（nginx）

## 常用 Make 命令
在项目根目录下执行：

启动全部服务：
```bash
make up
```

查看运行中的服务：
```bash
make ps
```

查看日志（所有服务）：
```bash
make logs
```

停止服务：
```bash
make down
```

停止并删除数据卷（完全重置）：
```bash
make clean
```

重启所有服务：
```bash
make restart
```

仅启动基础设施（Kafka/DynamoDB + 初始化任务）：
```bash
make init
```

重启单个 Bot 服务：
```bash
make restart-bot            # 重新构建并重启 bot-service
make restart-bot-manager    # 重启 bot-manager-service
```

后端 Rust 辅助命令：
```bash
make backend-fmt
make backend-check
```

## 直接使用 Docker Compose 命令
如果你不想使用 `make`，可以直接运行以下命令：

启动全部服务：
```bash
docker compose -f docker-compose.yml up --build -d
```

查看状态：
```bash
docker compose -f docker-compose.yml ps
```

跟踪日志：
```bash
docker compose -f docker-compose.yml logs -f --tail=200
```

停止：
```bash
docker compose -f docker-compose.yml down --remove-orphans
```

完全重置：
```bash
docker compose -f docker-compose.yml down -v --remove-orphans
```

## 本地访问地址
所有服务通过 nginx 的 8000 端口统一访问：
- 前端页面：[http://localhost:8000](http://localhost:8000)
- 游戏 API：`http://localhost:8000/v2/games`
- WebSocket：`ws://localhost:8000/v2/games/{game_id}/stream`
- Kafka（主机访问）：`localhost:29092`

内部服务端口（默认不对外暴露）：
- Game Manager Service：`8081`
- Web Service：`8082`
- Game Watcher Service：`8083`
- Game Service：`8084`
- Bot Manager Service：`8090`
- Bot Service：`8091`

## 常见工作流程
1. 启动服务栈：
```bash
make up
```

2. 打开游戏页面：
```
http://localhost:8000
```

3. 测试时查看日志：
```bash
make logs
```

4. 完成后停止服务：
```bash
make down
```

## 配置说明

### API 密钥
在项目根目录下创建 `.env` 文件（已添加到 `.gitignore` 中）：
```
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
```

Docker Compose 会自动读取该文件，并将值传递到容器中。

### LLM Bot 配置
编辑 `conf/bot-manager-llm.yaml` 来配置每个玩家的 LLM 模型。

### LangSmith 追踪
编辑 `conf/bot-service-langsmith.yaml` 来启用或禁用 LLM 追踪。

### Bot 提示词
编辑 `conf/bot-service-prompts.yaml` 来自定义 Bot 的行为。

## 故障排查
如果端口被占用：
- 修改 `docker-compose.yml` 中的端口映射，然后重新运行 `make up`。

如果容器在初始化时失败：
```bash
make down
make up
make logs
```

如果需要完全重置：
```bash
make clean
make up
```

如果 Bot 的 LLM 调用报错"缺少凭证"：
- 检查 `.env` 文件中是否填入了有效的 API 密钥。
- 运行 `make restart` 以使用更新后的密钥重新创建容器。
