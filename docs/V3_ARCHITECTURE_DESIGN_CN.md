# V3 架构与设计（Bot + Bot Manager）

## 1. 目标

为 Cowboy V3 添加 AI 控制的玩家，使人类玩家可以与机器人一起游戏。
默认分配策略：
- 玩家 `A` 为人类
- 玩家 `B/C/D` 为机器人

关键成果：
- `bot-manager-service` 负责每局游戏的机器人生命周期管理。
- `bot-service` 运行机器人 Actor 并决定指令。
- 机器人从每局游戏的输出 Kafka 主题读取游戏事件，并将指令写入该游戏的输入 Kafka 主题。
- 游戏结束时，机器人工作者被停止并清理。

## 2. 当前系统上下文

当前 V2 流程（已实现）：
- 游戏创建时为每局游戏创建主题：
  - 输入主题：`game.commands.<game_id>.v1`
  - 输出主题：`game.output.<game_id>.v1`
- `web-service` 将人类指令发布到输入主题。
- `timer-service` 将超时指令发布到输入主题。
- `game-service` 消费输入主题并将步骤事件发送到输出主题。
- `game-watcher-service` 消费输出主题并通过 WebSocket 将事件推送到前端。
- `game-manager-service` 创建/启动/结束游戏，并在结束时删除每局游戏的主题。

机器人功能在不破坏现有行为的前提下扩展此流程。

## 3. 新服务

## 3.1 bot-manager-service

职责：
- 观察游戏生命周期（开始/结束）并协调每局游戏的机器人。
- 为游戏中的每个玩家分配机器人/人类角色。
- 提供默认分配 API，将 `B/C/D` 绑定为机器人。
- 为被分配为机器人的玩家创建机器人。
- 选择每个机器人绑定到哪个 bot-service 实例。
- 从 YAML 配置中解析每个玩家的 LLM 设置（`base_url`、`model`、`api_key`）并在创建机器人时传递。
- 向每个新机器人发送游戏教学负载（`teach_game`）并等待 `READY` 状态。
- 将每个就绪的机器人绑定到 `(game_id, player_id)`。
- 游戏结束时停止并销毁机器人。
- 可选地重试失败的机器人启动/入职流程。

bot-manager 管理的状态：
- 机器人注册表：`bot_id -> process/session/status`。
- 绑定映射：`(game_id, player_id) -> (bot_id, bot_service_base_url)`。
- 入职版本：每个机器人的 `game_guide_version`。
- 实例容量目标：`BOTS_PER_INSTANCE_CAPACITY`（默认 `2`）。

## 3.2 bot-service

职责：
- 一个 bot-service 操作系统进程并发服务多个机器人绑定。
- 为每个 `(game_id, player_id)` 绑定运行一个机器人 Actor。
- 为每个绑定维护一个持久的 Python `Player` 对象（游戏/绑定结束时销毁）。
- 接受入职负载并在机器人上下文中保留游戏指南。
- 消费所绑定游戏的输出主题。
- 检测是否轮到该机器人行动。
- 使用 DeepAgents 生成一个有效指令（`move`、`shoot`、`shield`、`speak`）。
- 将指令发布到游戏的输入主题。
- 当游戏未运行或非该机器人回合时忽略事件。

## 4. 数据与控制流

## 4.1 游戏开始
1. 人类点击 `New`。
2. 通过 bot-manager 配置角色分配：
   - 默认 API 将 `B/C/D` 分配为机器人，或
   - 自定义绑定 API 指定明确的机器人-玩家映射。
3. 人类点击 `Start`。
4. `game-manager-service` 将游戏标记为运行中并发出 `GAME_STARTED` 事件。
5. `bot-manager-service` 收到通知并协调活跃的机器人绑定。
6. Bot manager 为被分配为机器人的玩家创建缺失的机器人工作者。
7. Bot manager 根据最低负载 + 容量目标为每个玩家选择目标 bot-service 实例。
8. Bot manager 向每个已创建/重启的机器人发送 `teach_game` 负载。
9. 每个机器人回复 `READY`。
10. Bot manager 激活绑定。

## 4.2 回合进行中
1. `game-service` 将步骤事件发送到游戏输出主题。
2. `game-watcher-service` 通过 WebSocket 向客户端广播超时/游戏结束/发言事件。
3. 每个机器人工作者消费输出主题事件。
4. 如果快照显示当前玩家是该机器人：
   - 机器人使用 DeepAgents 生成指令
   - 机器人将指令发布到输入主题。
5. `game-service` 处理机器人指令的方式与处理人类指令完全相同。

## 4.3 游戏结束
1. `game-service` 通过 game manager 触发结束。
2. `game-manager-service` 发出 `GAME_FINISHED` 事件并删除每局游戏的主题。
3. `bot-manager-service` 接收结束信号并销毁该游戏的所有机器人。
4. 绑定和机器人运行时状态被移除。

## 5. 机器人入职（`teach_game`）

问题：
- 新创建的机器人默认不了解游戏规则。

解决方案：
- Bot manager 必须在创建机器人之后、激活之前对其进行教学。

必需的负载字段：
- `game_guide_version`
- 指令模式和验证规则
- 回合模型和超时行为
- 地图/玩家快照
- 该机器人的 `player_id` 和 `player_name`
- 有效指令示例

激活规则：
- 机器人在入职状态变为 `READY` 之前不允许发布指令。

重新教学触发条件：
- 机器人重启
- `game_guide_version` 变更
- 管理器显式重新同步请求

## 6. DeepAgents 集成设计

机器人决策循环：
1. 构建提示上下文：
   - 最新快照
   - 最近 N 个事件
   - 机器人身份（`player_id`、`player_name`）
2. 请求模型输出严格的 JSON 格式：
   - `command_type`：`move|shoot|shield|speak`
   - 需要时包含 `direction`
   - `speak` 时包含 `speak_text`
3. 发布前验证输出。
4. 如果无效：
   - 带验证错误反馈重试一次
   - 回退到安全默认指令（使用当前护盾方向或 `up` 的 `shield`）。

防护措施：
- 每回合仅发出一条指令。
- 禁止发出保留指令（`timeout`、`game_started`）。
- 禁止发布缺少 `player_id`、`turn_no` 或 `command_id` 的指令。

## 7. 持久化与可靠性

MVP 阶段：
- bot-manager 中使用内存机器人注册表 + 绑定。

推荐方案：
- DynamoDB 表 `bot_bindings`：
  - PK：`game_id`
  - SK：`player_id`
  - 属性：`bot_id`、`status`、`game_guide_version`、`created_at`、`updated_at`

幂等性：
- 对同一游戏的 Bot manager 启动操作必须是幂等的。
- 如果机器人已不存在，销毁操作必须安全执行。

## 8. 故障处理

故障场景与行为：
- 机器人创建失败：使用退避策略重试；报告健康状况下降。
- `teach_game` 失败：重新创建机器人或保持未绑定状态；不激活。
- Kafka 不可用：机器人暂停发布并重试。
- 管理器重启：从数据源重建活跃游戏并重新绑定/重新教学。
- 延迟的机器人指令：传输层允许，若过期则被 `game-service` 忽略；仍会持久化。

## 9. 安全与运维

初始范围：
- 仅限内部 API（与后端服务同一网络）。
- 无需更改终端用户认证。

运维要求：
- 每个服务提供健康检查端点。
- 包含 `game_id`、`player_id`、`bot_id`、`command_id` 的结构化日志。
- 通过环境变量配置：
  - 模型/提供商设置
  - Kafka bootstrap/前缀
  - manager 基础 URL
  - bot-service 实例列表（`BOT_SERVICE_BASE_URLS`）
  - 每实例容量目标（`BOTS_PER_INSTANCE_CAPACITY`，默认 `2`）
  - bot-manager YAML 路径（`BOT_MANAGER_LLM_CONFIG_PATH`），用于默认/每玩家模型设置
  - bot-service LangSmith YAML 路径（`BOT_AGENT_LANGSMITH_CONFIG_PATH`），用于 DeepAgents 追踪设置
  - 入职超时/重试

## 10. 测试策略

单元测试：
- Bot manager 生命周期状态转换。
- 机器人入职门控（需要 `READY` 状态）。
- 指令验证和回退。

集成测试：
- 消费输出主题并将有效机器人指令发布到输入主题。
- 游戏指南版本变更时重新教学。
- 游戏结束时销毁所有机器人。

端到端测试（默认无头模式）：
- 人类 + 3 个机器人的游戏循环。
- 机器人 `speak` 在 WebSocket/前端日志中显示。
- 超时和结束事件在前端中保持可见。
