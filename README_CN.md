[English](README.md) | 中文

# Cowboy

一款回合制多人对战游戏，支持 1-4 名玩家在网格地图上竞技。玩家可以移动、发射激光和放置护盾。玩家 A 由人类控制，其余玩家为 AI 机器人，由 LLM（GPT、Gemini、Claude 等）通过 LangChain 驱动。

最后存活的玩家获胜。

**本项目的全部内容——架构、后端、前端、基础设施、测试和文档——均由 AI 代理（Claude Code）100% 开发。** 没有任何一行代码由人类编写。人类的参与仅限于向 AI 提供高层指令和审查结果。

## 演示

https://github.com/user-attachments/assets/demo.mp4

## 快速开始

**前置条件：** Docker 和 Docker Compose。

```bash
# 1. 克隆并配置
git clone https://github.com/StarHuntingGames/cowboy.git
cd cowboy

# 2. 添加你的 API 密钥 — 在项目根目录创建 .env 文件
cat > .env <<EOF
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
EOF

# 3. 启动所有服务
make up

# 4. 打开游戏
#    http://localhost:8000
```

停止服务：`make down` | 完全重置：`make clean` | 重启：`make restart`

## 架构

事件驱动微服务架构 — 8 个 Rust 服务 + 1 个 Python 子进程，通过 Kafka 和 REST 进行通信。状态存储在 DynamoDB Local 中。前端为 nginx 提供的静态 HTML/JS 页面。

```
┌─────────────┐         ┌──────────────────┐        ┌──────────────┐
│   Frontend   │◄──WS──►│ game-watcher (WS) │◄─Kafka─│ game-service  │
│  :8000       │         │ :8083             │        │ :8084 (logic) │
└──────┬───────┘         └──────────────────┘        └──────┬───────┘
       │ HTTP                                                │ Kafka
       ▼                                                     ▼
┌──────────────┐  Kafka  ┌──────────────────┐        ┌──────────────┐
│ web-service   │───────►│ game-manager      │        │ timer-service │
│ :8082 (API)  │         │ :8081 (lifecycle) │        │ (timeouts)   │
└──────────────┘         └──────────────────┘        └──────────────┘
                                                           │
                         ┌──────────────────┐        ┌──────────────┐
                         │ bot-manager       │───────►│ bot-service   │
                         │ :8090 (orchestr.) │  HTTP  │ :8091 (LLM)  │
                         └──────────────────┘        └──────┬───────┘
                                                            │
                                                     Python subprocess
                                                     (LangChain agent)
```

### 数据流

1. 人类指令 → `web-service` → Kafka 输入主题
2. 机器人指令 → `bot-service`（Python LLM 决策）→ Kafka 输入主题
3. `game-service` 消费输入 → 应用游戏规则 → 发布步骤事件
4. `game-watcher-service` 消费事件 → 通过 WebSocket 广播到前端
5. `bot-service` 消费事件 → 在机器人回合时触发下一步决策

### Kafka 主题

每局游戏的主题：`game.commands.<game_id>.v1`（输入）、`game.output.<game_id>.v1`（输出）。

## 游戏规则

- **网格地图**，包含空白格和可破坏/不可破坏的障碍物
- **回合顺序：** A → B → C → D（固定）
- **行动（每回合一次）：** 移动、射击或重新放置护盾
- **射击**沿直线发射激光 — 在遇到第一个障碍物或玩家时停止
- **护盾**可以阻挡来自一个方向的射击
- **生命值：** 初始为 10，每次未被阻挡的命中损失 1 点，降至 0 时淘汰

完整规则请参阅 [docs/GAME_RULES.md](docs/GAME_RULES.md)。

## 配置

### API 密钥

在项目根目录创建 `.env` 文件（已包含在 `.gitignore` 中）：

```
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
```

Docker Compose 会自动读取此文件。`conf/` 中的 YAML 配置文件通过 `${BOT_LLM_API_KEY}` 和 `${LANGSMITH_API_KEY}` 占位符引用这些变量，并在运行时展开。

### LLM 机器人配置

编辑 `conf/bot-manager-llm.yaml` 为每个玩家设置 LLM 提供商：

```yaml
default:
  base_url: "https://openrouter.ai/api/v1"
  model: "google/gemini-3-flash-preview"
  api_key: "${BOT_LLM_API_KEY}"

players:
  B:
    model: "openai/gpt-5.2-codex"
    api_key: "${BOT_LLM_API_KEY}"
```

### LangSmith 追踪（可选）

编辑 `conf/bot-service-langsmith.yaml` 以启用 LLM 追踪。

### 机器人提示词

在 `conf/bot-service-prompts.yaml` 中自定义机器人行为。

## 开发

### 构建命令

```bash
make up                    # 构建并启动所有服务
make down                  # 停止服务
make restart               # 完全重启（down + clean + up）
make clean                 # 停止并删除卷（完全重置）
make logs                  # 查看所有日志
make ps                    # 显示运行中的服务
make init                  # 仅启动基础设施（Kafka、DynamoDB）
make restart-bot           # 仅重新构建并重启 bot-service
make restart-bot-manager   # 仅重启 bot-manager-service
```

### Rust 后端

```bash
make backend-fmt           # cargo fmt
make backend-check         # cargo check
cargo build --manifest-path backend/Cargo.toml
```

### Python 测试

```bash
.venv/bin/pytest backend/bot-service/python/tests/
```

### 端到端测试

需要 Docker Compose 技术栈正在运行（先执行 `make up`）。

```bash
make e2e-llm-failure-speak        # LLM 不可用时的机器人回退测试
make e2e-verify-bot-config-wiring  # 配置传播验证
make e2e-llm-connection-test       # 真实 LLM 连接测试
```

## 技术栈

- **后端：** Rust (axum)、Python (FastAPI、LangChain)
- **消息队列：** Apache Kafka
- **存储：** DynamoDB Local
- **前端：** HTML5 Canvas、原生 JS
- **基础设施：** Docker Compose、nginx

## 文档

- [游戏规则](docs/GAME_RULES.md)
- [架构设计](docs/V3_ARCHITECTURE_DESIGN.md)
- [运行指南](docs/HOW_TO_RUN_V2.md)
- [API 变更](docs/V3_API_CHANGES.md)
- [测试用例](docs/TEST_CASES.md)

## 联系方式

StarHuntingGames — starhuntinggames@gmail.com

## 许可证

本项目基于 GNU 通用公共许可证 v3.0 授权 — 详见 [LICENSE](LICENSE) 文件。
