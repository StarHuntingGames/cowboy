# V3 API 新增与修改

## 1. 范围

本文档定义了：
- `bot-manager-service` 和 `bot-service` 所需的新 API。
- 支持机器人功能所需的现有 API/事件契约变更。
- 向后兼容性指南。

## 2. 需要修改的现有 API

## 2.1 `POST /v2/games`（game-manager-service）

当前行为：
- 创建游戏。
- 返回 `game_id`、初始状态、超时时间、玩家信息和回合元数据。

所需修改：
- 保持游戏创建与角色无关。
- 请求和响应中不包含玩家 `kind`。
- 继续返回玩家身份信息（`player_name`、`player_id`），以便 bot-manager 后续进行分配。

## 2.2 `GET /v2/games/{game_id}`（game-manager-service）

当前行为：
- 返回完整的游戏实例和状态。

所需修改：
- 保持 game-manager 与角色无关。
- 确保 bot-manager 可以从此响应中获取所有分配所需的输入：
  - `state.players[*].player_name`
  - `state.players[*].player_id`
  - `input_topic`
  - `output_topic`

## 2.3 `POST /v2/games/{game_id}/start`（game-manager-service）

当前行为：
- 启动游戏并发出 `GAME_STARTED` 事件。

所需修改：
- 保持现有行为。
- 可选地在成功启动后通知 bot-manager 协调活跃分配。

方案选择：
- 方案 A：game-manager 同步调用 bot-manager API。
- 方案 B：game-manager 发布 `GAME_STARTED` 事件，bot-manager 消费输出主题。

推荐方案：
- 方案 B，降低耦合度且更易于重试。

## 3. 新 API：bot-manager-service

基础 URL 示例：`http://bot-manager-service:8090`

## 3.1 健康检查
- `GET /health`

响应：
```json
{ "ok": true, "service": "bot-manager-service" }
```

## 3.2 为游戏分配默认玩家
- `POST /internal/v3/games/{game_id}/assignments/default`

用途：
- 使用默认规则分配玩家：
  - `A` 为人类
  - `B/C/D` 为机器人
- 为被分配为机器人的玩家创建并绑定机器人。

请求：
```json
{
  "apply_immediately": true,
  "game_guide_version": "v1",
  "force_recreate": false
}
```

响应：
```json
{
  "assigned": true,
  "game_id": "uuid",
  "humans": [
    { "player_name": "A", "player_id": "uuid-a" }
  ],
  "bindings": [
    { "player_name": "B", "player_id": "uuid-b", "bot_id": "bot-1", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_name": "C", "player_id": "uuid-c", "bot_id": "bot-2", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_name": "D", "player_id": "uuid-d", "bot_id": "bot-3", "bot_service_base_url": "http://bot-service:8091", "status": "READY" }
  ]
}
```

## 3.3 将一个机器人绑定到一个玩家
- `POST /internal/v3/games/{game_id}/bindings`

用途：
- 将机器人显式绑定到特定玩家。

请求：
```json
{
  "player_id": "uuid-b",
  "bot_id": "bot-1",
  "create_bot_if_missing": true,
  "game_guide_version": "v1"
}
```

响应：
```json
{
  "bound": true,
  "game_id": "uuid",
  "player_id": "uuid-b",
  "bot_id": "bot-1",
  "bot_service_base_url": "http://bot-service:8091",
  "status": "READY"
}
```

## 3.4 显式分配玩家（批量）
- `POST /internal/v3/games/{game_id}/assignments`

用途：
- 使用自定义的人类/机器人方案分配所有玩家。

请求：
```json
{
  "human_player_ids": ["uuid-a"],
  "bot_player_ids": ["uuid-b", "uuid-c", "uuid-d"],
  "game_guide_version": "v1",
  "force_recreate": false
}
```

响应：
```json
{
  "assigned": true,
  "game_id": "uuid",
  "humans": [
    { "player_id": "uuid-a" }
  ],
  "bindings": [
    { "player_id": "uuid-b", "bot_id": "bot-1", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_id": "uuid-c", "bot_id": "bot-2", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_id": "uuid-d", "bot_id": "bot-3", "bot_service_base_url": "http://bot-service:8091", "status": "READY" }
  ]
}
```

## 3.5 停止游戏的机器人
- `POST /internal/v3/games/{game_id}/bots/stop`

用途：
- 游戏结束时销毁所有机器人并清除绑定。

请求：
```json
{
  "reason": "GAME_FINISHED"
}
```

响应：
```json
{
  "stopped": true,
  "game_id": "uuid",
  "destroyed_bot_count": 3
}
```

## 3.6 查询活跃分配和绑定
- `GET /internal/v3/games/{game_id}/assignments`

响应：
```json
{
  "game_id": "uuid",
  "humans": [
    { "player_name": "A", "player_id": "uuid-a" }
  ],
  "bindings": [
    { "player_name": "B", "player_id": "uuid-b", "bot_id": "bot-1", "bot_service_base_url": "http://bot-service:8091", "status": "READY" }
  ]
}
```

调度/配置说明：
- Bot manager 通过 `BOT_SERVICE_BASE_URLS`（逗号分隔的 URL）支持多个 bot-service 实例。
- Bot manager 使用 `BOTS_PER_INSTANCE_CAPACITY`（默认 `2`）作为每实例容量目标。
- 当所有实例达到或超过目标时，manager 仍会分配到负载最低的实例。
- Bot manager 可从 `BOT_MANAGER_LLM_CONFIG_PATH` 加载每玩家的 LLM 配置。
- Bot service 可从 `BOT_AGENT_LANGSMITH_CONFIG_PATH` 加载 LangSmith/DeepAgents 追踪配置。
- YAML 支持 `default` 默认配置以及每玩家覆盖配置（`A/B/C/D`），可覆盖 `base_url`、`model`、`api_key`。

YAML 示例：
```yaml
default:
  base_url: "https://api.openai.com/v1"
  model: "openai:gpt-4o-mini"
  api_key: "sk-..."
players:
  B:
    model: "openai:gpt-4o"
  C:
    model: "openai:gpt-4o-mini"
  D:
    base_url: "https://compatible-endpoint/v1"
    model: "openai:gpt-4o-mini"
    api_key: "sk-..."
```

## 4. 新 API：bot-service

基础 URL 示例：`http://bot-service:8091`

## 4.1 健康检查
- `GET /health`

响应：
```json
{ "ok": true, "service": "bot-service" }
```

## 4.2 创建机器人 Actor
- `POST /internal/v3/bots`

请求：
```json
{
  "game_id": "uuid",
  "player_name": "B",
  "player_id": "uuid-b",
  "input_topic": "game.commands.uuid.v1",
  "output_topic": "game.output.uuid.v1",
  "llm_base_url": "https://api.openai.com/v1",
  "llm_model": "openai:gpt-4o-mini",
  "llm_api_key": "sk-..."
}
```

字段说明：
- `llm_base_url`、`llm_model`、`llm_api_key` 为可选字段，由 bot-manager 按玩家设置。
- 同一 bot-service 实例中的不同玩家可以使用不同的模型设置。

响应：
```json
{
  "bot_id": "bot-1",
  "status": "CREATED"
}
```

## 4.3 教授机器人游戏规则（必需）
- `POST /internal/v3/bots/{bot_id}/teach-game`

请求：
```json
{
  "game_guide_version": "v1",
  "rules_markdown": "....",
  "command_schema": {
    "allowed": ["move", "shoot", "shield", "speak"],
    "direction_required_for": ["move", "shoot", "shield"],
    "speak_text_required_for": ["speak"]
  },
  "examples": [
    { "command_type": "move", "direction": "up" },
    { "command_type": "speak", "speak_text": "hello" }
  ]
}
```

响应：
```json
{
  "bot_id": "bot-1",
  "status": "READY",
  "game_guide_version": "v1"
}
```

行为说明：
- 在此端点成功返回之前，机器人不得发布游戏指令。

## 4.4 销毁机器人 Actor
- `DELETE /internal/v3/bots/{bot_id}`

响应：
```json
{
  "deleted": true,
  "bot_id": "bot-1"
}
```

## 5. WebSocket/事件契约变更

无需新增 WebSocket 端点。现有端点保持不变：
- `GET /v2/games/{game_id}/stream`（升级为 WebSocket）

所需的事件使用：
- 现有的 `SPEAK` 事件已适用于机器人。
- 前端应使用 `player_id` 查找来统一渲染人类和机器人的发言。

`SPEAK` 事件格式（已兼容）：
```json
{
  "event_type": "SPEAK",
  "game_id": "uuid",
  "turn_no": 12,
  "player_id": "uuid-b",
  "speak_text": "Bot says hello",
  "snapshot": { "...": "..." }
}
```

## 6. Kafka 契约变更

复用现有的每局游戏主题：
- 输入：`game.commands.<game_id>.v1`
- 输出：`game.output.<game_id>.v1`

所需的约定更新：
- 机器人指令必须使用：
  - `source: "user"` 或新增 `"bot"` 来源。

推荐方案：
- 新增来源枚举值 `"bot"`，以便更清晰地进行分析和调试。

建议的指令信封示例：
```json
{
  "command_id": "uuid",
  "source": "bot",
  "game_id": "uuid",
  "player_id": "uuid-b",
  "command_type": "speak",
  "direction": null,
  "speak_text": "I will win",
  "turn_no": 12,
  "sent_at": "2026-02-10T12:00:00Z"
}
```

## 7. DynamoDB 模型变更

## 7.1 现有 `game_steps` 表

无需更改表结构。机器人指令与人类指令以相同方式持久化。

推荐新增：
- `source` 属性支持 `source = BOT` 值。
- 保持 `speak_text` 持久化（发言功能已存在）。

## 7.2 新增可选表 `bot_bindings`

用途：
- 持久化映射关系，支持 bot-manager 重启后恢复。

建议的表结构：
- PK：`game_id` (S)
- SK：`player_id` (S)
- 属性：
  - `player_name` (S)
  - `bot_id` (S)
  - `status` (S)（`CREATED|READY|STOPPED|FAILED`）
  - `game_guide_version` (S)
  - `created_at` (S)
  - `updated_at` (S)

## 8. 向后兼容性

兼容性规则：
- 现有前端指令和流 API 保持有效。
- 现有游戏创建流程保持不变且与角色无关。
- 现有消费者应安全忽略未知字段。

迁移顺序：
1. 部署 bot-service + bot-manager-service。
2. 添加 bot-manager 分配 API（`default`、`bulk`、`bind`）。
3. 为机器人协调和清理接入游戏开始/结束钩子。
4. 启用前端在启动前调用默认或自定义分配 API。

## 9. 待定决策

1. 机器人指令应使用 `source: "bot"` 还是复用 `source: "user"`？
2. bot-manager 应仅采用事件驱动、仅采用 API 驱动，还是混合模式？
3. MVP 阶段机器人绑定状态应仅保存在内存中，还是现在就持久化到 DynamoDB？
4. 分配应仅通过显式 API 调用进行，还是在不存在分配时由启动操作自动触发默认分配？
