# Bot 功能任务列表（V3）

本文件使用的状态值：
- `TODO`：未开始
- `IN_PROGRESS`：部分完成，或已完成但尚未完全验证
- `DONE`：已完成并针对该状态列验证通过

| ID | 任务 | Coding | Testing | Finished | 备注 |
|---|---|---|---|---|---|
| BOT-001 | 创建 `bot-manager-service` 项目骨架（配置、启动、健康检查端点、日志） | DONE | DONE | DONE | 服务已启动，健康检查端点正常响应。 |
| BOT-002 | 基于 DeepAgents 集成创建 `bot-service` 项目骨架 | DONE | DONE | DONE | Python DeepAgents 运行时已接入 bot-service，具备回退策略；已在无头 E2E 中验证。 |
| BOT-003 | 添加游戏生命周期钩子，使 bot manager 在游戏启动时被触发 | DONE | DONE | DONE | output topics 上的消费者处理 `GAME_STARTED`。 |
| BOT-004 | 实现已分配 bot 玩家的 bot 创建流程（默认策略：B/C/D） | DONE | DONE | DONE | 通过默认分配 API 实现。 |
| BOT-005 | 实现游戏结束时的 bot 销毁流程 | DONE | DONE | DONE | 通过 `GAME_FINISHED` 事件验证：`/tmp/cowboy-v3-bot-teardown-summary.json`。 |
| BOT-006 | 实现 bot-玩家绑定模型 `(game_id, player_id, bot_id)` | DONE | DONE | DONE | 存储在 bot-manager 分配映射中，可通过 API 查询。 |
| BOT-007 | 添加 bot 上线 API/消息 `teach_game`，由 bot manager 在 bot 创建后发送 | DONE | DONE | DONE | Bot manager 在 create/bind 后调用 bot-service 的 `teach-game`。 |
| BOT-008 | 实现 bot 端游戏指南接收和就绪确认（`READY`） | DONE | DONE | DONE | Bot 生命周期在 teach 调用后转换为 `READY`。 |
| BOT-009 | 包含版本化的游戏指南（`game_guide_version`）和重启/版本变更时的重新教学逻辑 | DONE | IN_PROGRESS | IN_PROGRESS | 版本字段已接入；测试对显式重新教学的覆盖待完成。 |
| BOT-010 | 订阅 bot worker 到每个游戏的 output Kafka topic | DONE | DONE | DONE | Worker 订阅每个已分配游戏的 output topic。 |
| BOT-011 | 实现回合检测逻辑，使 bot 仅在自己的回合行动 | DONE | DONE | DONE | Worker 基于 `current_player_id` 和 `turn_no` 进行门控。 |
| BOT-012 | 使用 DeepAgents 实现命令生成（`move`、`shoot`、`shield`、`speak`） | DONE | DONE | DONE | 基于 DeepAgents 的 Python 决策路径，具备严格的标准化和回退机制；在 `/tmp/cowboy-v3-bot-concurrency-summary.json` 中观察到 bot `speak`/回合推进。 |
| BOT-013 | 添加命令验证 + 回退策略（重试一次，然后使用安全默认值） | TODO | TODO | TODO | 尚未实现。 |
| BOT-014 | 将 bot 命令发布到每个游戏的 input Kafka topic | DONE | DONE | DONE | Bot worker 将命令信封发布到 input topic。 |
| BOT-015 | 确保 `speak` 文本在 output 事件和 WebSocket 广播中端到端保留 | DONE | DONE | DONE | 在无头 UI 日志中验证：`/tmp/cowboy-v3-bot-e2e-summary.json`。 |
| BOT-016 | 添加 bot service 使用的 LLM 提供商的配置和密钥管理 | DONE | DONE | DONE | `.env` 文件用于 API 密钥，YAML 配置中支持 `${BOT_LLM_API_KEY}` / `${LANGSMITH_API_KEY}` 展开，环境变量通过 docker-compose 传递。 |
| BOT-017 | 更新 `docker-compose` 和本地运行脚本以支持 bot 服务 | DONE | DONE | DONE | `bot-service` 和 `bot-manager-service` 已接入 compose。 |
| BOT-018 | 添加 bot manager 生命周期和绑定逻辑的单元测试 | TODO | TODO | TODO | 尚无专门的单元测试。 |
| BOT-019 | 添加 bot 命令生成/验证/回退的单元测试 | TODO | TODO | TODO | 尚无专门的单元测试。 |
| BOT-020 | 添加 Kafka 流程的集成测试（output 消费 -> bot 动作 -> input 发布） | TODO | TODO | TODO | 已完成手动验证；自动化集成测试待完成。 |
| BOT-021 | 添加端到端测试（默认无头 Chrome）用于人类 + bot 游戏循环 | IN_PROGRESS | DONE | IN_PROGRESS | 手动无头 E2E 已通过，但已提交的自动化测试文件待完成。 |
| BOT-022 | 添加端到端测试用于游戏结束时的 bot 清理（worker 停止 + 不再发送命令） | IN_PROGRESS | DONE | IN_PROGRESS | 手动清理 E2E 已通过；已提交的自动化测试文件待完成。 |
| BOT-023 | 更新文档（`README`/服务文档）包含架构、环境变量和运行/测试步骤 | DONE | DONE | DONE | `V3_ARCHITECTURE_DESIGN.md` 和 `V3_API_CHANGES.md` 已更新。 |
| BOT-024 | 最终验证检查：所有测试通过、日志干净、验收标准满足 | IN_PROGRESS | IN_PROGRESS | IN_PROGRESS | 核心手动 E2E 检查已通过；剩余 DeepAgents/测试自动化任务待完成。 |
| BOT-025 | 添加 bot-manager 默认分配 API，将 `B/C/D` 绑定为 bot，`A` 绑定为人类 | DONE | DONE | DONE | 端点已实现，在无头 E2E 流程中验证通过。 |
| BOT-026 | 添加 bot-manager 绑定/分配 API 用于显式的 bot-玩家映射 | DONE | DONE | DONE | 已实现端点：绑定、批量分配、查询分配。 |
| BOT-027 | Bot-service 生命周期：每个 bot/玩家一个持久化的 Python `Player` 对象，游戏结束时销毁 | DONE | DONE | DONE | 添加了行协议 player agent（`backend/bot-service/python/player_agent.py`）和 `backend/bot-service/src/main.rs` 中的持久化会话集成；通过无头 E2E 运行验证。 |
| BOT-028 | Bot-manager 实例调度：按绑定选择 bot-service 实例，可配置容量（默认 `2`） | DONE | DONE | DONE | 添加了 `BOT_SERVICE_BASE_URLS`、`BOTS_PER_INSTANCE_CAPACITY`、最少负载选择，以及 `backend/bot-manager-service/src/main.rs` 中每个绑定的 `bot_service_base_url` 跟踪。 |
| BOT-029 | 用于 v3 bot 生命周期和分配元数据的无头 Chrome E2E | DONE | DONE | DONE | Playwright 无头验证通过：`/tmp/cowboy-v3-bot-concurrency-summary.json`，截图 `/tmp/cowboy-v3-bot-concurrency-headless.png`。 |
| BOT-030 | 同一 bot-service 实例中每个玩家的模型配置（`base_url`、`model`、`api_key`），通过 bot-manager YAML 和 create-bot 载荷传递 | DONE | DONE | DONE | 在 `bot-manager-service` 中添加了 YAML 加载器（`BOT_MANAGER_LLM_CONFIG_PATH`），在 create-bot API 中转发配置，在 bot-service/player-agent 中接入每个玩家的 LLM 字段；通过无头 E2E 验证：`/tmp/cowboy-v3-bot-llm-summary.json` + `/tmp/cowboy-v3-bot-llm-headless.png`。 |
| BOT-031 | 在 DynamoDB `bot_players` 表中持久化 bot 生命周期/状态（`game_id`、`player_id`、`model`、`base_url`、`api_key`、时间戳），并在生命周期中更新 `player_state`/`game_state` | DONE | DONE | DONE | 添加了表 + bot-manager DynamoDB 集成，在 assign/start/stop/game-finished 时执行 create/update。通过以下产物验证：`/tmp/bot_players_describe.json`、`/tmp/bot_players_game_query_running.json`、`/tmp/bot_players_game_query_stopped.json`、`/tmp/bot_players_after_finish_lower.json`，以及无头 UI 运行 `/tmp/bot_players_headless_e2e.json` + `/tmp/cowboy-bot-players-headless.png`。 |

## 更新规则

当任务进展时，按以下顺序更新各列：
1. 实现完成后将 `Coding` 设为 `DONE`。
2. 相关自动化/手动测试通过后将 `Testing` 设为 `DONE`。
3. 仅当编码 + 测试均已完成并验证后，才将 `Finished` 设为 `DONE`。
