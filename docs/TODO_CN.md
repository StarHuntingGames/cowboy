# Cowboy V2 剩余待办事项

最后更新：2026-02-10

状态字段说明：
- `Done`：代码中的实现已完成
- `Tested`：自动化/手动测试已执行并通过
- `Verified`：端到端行为已确认（包括适用的前端可见效果）

| ID | 任务 | Done | Tested | Verified | 备注 |
| --- | --- | --- | --- | --- | --- |
| T1 | Timer Service：消费已应用的步骤事件并维护每回合计时器（启动/重置/取消） | [x] | [x] | [x] | 2026-02-09：在 `backend/timer-service/src/main.rs` 中实现；`cargo test -p timer-service` 通过；无头运行中观察到超时驱动的回合推进（`/tmp/cowboy-e2e-timeout-headless.png`）。 |
| T2 | Timer Service：计时器到期时将 `TIMEOUT` 命令发布到每个游戏的 input Kafka topic | [x] | [x] | [x] | 2026-02-09：超时命令生产者已实现；通过 `/tmp/e2e-output-topic.log` 中 `source:\"timer\"` 的 `TIMEOUT_APPLIED` 事件验证。 |
| T3 | Web Service：将 `LoggingPublisher` 替换为真实的 Kafka 生产者发布到游戏 input topic | [x] | [x] | [x] | 2026-02-09：`KafkaCommandPublisher` 已添加到 `backend/web-service/src/main.rs`；`cargo test -p web-service` 通过；命令在无头 E2E 中被接受并处理。 |
| T4 | Game Service：从每个游戏的 input Kafka topic 消费命令，替代仅 HTTP 的 apply 流程 | [x] | [x] | [x] | 2026-02-09：Kafka 正则消费者（`game.commands.<game_id>.v1`）在 `backend/game-service/src/main.rs` 中实现；`cargo test -p game-service` 通过（0 个测试，构建/检查）；E2E 命令路径端到端使用 Kafka。 |
| T5 | Game Service：将已应用/忽略的步骤事件发布到每个游戏的 output Kafka topic | [x] | [x] | [x] | 2026-02-09：每个游戏的 output 生产者和 `publish_and_persist` 已添加；在 `/tmp/e2e-output-topic.log` 中验证了 applied 和 ignored 两种结果。 |
| T6 | Game Service：实现超时/迟到命令行为（`timeout` 推进回合；迟到的用户命令被忽略但仍记录） | [x] | [x] | [x] | 2026-02-09：超时和过期/迟到处理已实现（`ResultStatus::TimeoutApplied` / `IgnoredTimeout`）；通过 output 日志中的迟到命令 id `late-e2e-1770650727781` + Dynamo 记录验证。 |
| T7 | 持久化：记录已消费的命令和结果步骤记录，包括 applied 和 ignored 两种结果 | [x] | [x] | [x] | 2026-02-09：DynamoDB 步骤持久化在 `persist_step_record` 中实现；在 `/tmp/dynamo-late-cmd.json` 中验证了 `IGNORED_TIMEOUT`，在 `/tmp/dynamo-game-steps-e2e.json` 中验证了多个 `TIMEOUT_APPLIED`。 |
| T8 | Game Manager Service：实际发布 `GAME_STARTED` 事件（当前仅准备/记录） | [x] | [x] | [x] | 2026-02-09：`start_game_handler` 现在将 `GAME_STARTED` 发布到每个游戏的 output topic；`cargo test -p game-manager-service` 通过，包括 `start_game_publishes_game_started_event_to_output_topic`。 |
| T9 | Game Watcher Service：消费每个游戏的 output topics（或路由流），替代固定的共享 `game.steps.v1` | [x] | [x] | [x] | 2026-02-09：watcher Kafka 消费者现在订阅 `game.output.*.v1`；通过 WebSocket 广播 `TIMEOUT`/`GAME_FINISHED`；前端超时/结束动画在无头截图中确认。 |
| T10 | E2E 测试：超时流程、迟到命令被忽略但记录、watcher 重连/快照同步 | [x] | [x] | [x] | 2026-02-09：通过无头产物验证：`/tmp/cowboy-e2e-timeout-summary.json`、`/tmp/cowboy-e2e-full-summary.json`、`/tmp/cowboy-e2e-finish-summary.json` 以及 `/tmp/` 中的截图。 |
| T11 | 添加 `speak` 命令端到端支持（前端 -> web/input Kafka -> game-service -> output Kafka -> watcher WebSocket -> 前端日志） | [x] | [x] | [x] | 2026-02-10：在共享 schema 和服务中实现了 `speak` + `speak_text`；无头 Playwright 通过（`/tmp/cowboy-e2e-speak-summary.json`、`/tmp/cowboy-e2e-speak-headless.png`）；验证了 Kafka input/output 日志（`/tmp/speak-input-topic.log`、`/tmp/speak-output-topic.log`）和 Dynamo 持久化（`/tmp/speak-dynamo-game.json`）。 |
| T12 | 玩家身份重构：前端显示名称 `A/B/C/D`，后端使用 UUID `player_id`，`New`/`Start` 控件顺序 + 样式更新 | [x] | [x] | [x] | 2026-02-10：`PlayerId` 改为 UUID 字符串，暴露每个玩家的名称，创建/启动 UI 更新（`New` 在 `Start` 之前，相同的主样式），命令提交在无头 Playwright 中使用 UUID player id 验证通过（`/tmp/cowboy-e2e-player-uuid-summary.json`、`/tmp/cowboy-e2e-player-uuid-headless.png`）。 |

## 更新规则

更新任务时：
1. 仅在代码实现完成后将 `Done` 设为 `[x]`。
2. 仅在测试运行并通过后将 `Tested` 设为 `[x]`。
3. 仅在端到端行为确认后将 `Verified` 设为 `[x]`。
4. 在任务 `Notes` 单元格中添加简短的带日期备注，描述验证了什么。
