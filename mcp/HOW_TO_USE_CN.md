# Cowboy MCP 服务器 — 使用指南

本 MCP 服务器允许 AI 代理（Claude 等）控制正在运行的 Cowboy 游戏中的任意玩家（A/B/C/D）。

## 前置条件

- Python 3.11+
- Cowboy 服务栈正在运行（在项目根目录执行 `make up`）
- 已通过前端 `http://localhost:8000` 创建并开始游戏

## 安装

在项目根目录执行：

```bash
pip install -e mcp/
```

## 配置

MCP 服务器配置是一段 JSON，需要添加到你所使用客户端的相应配置文件中。

```json
{
  "mcpServers": {
    "cowboy": {
      "command": "python",
      "args": ["-m", "cowboy_mcp.server"],
      "env": {
        "COWBOY_BASE_URL": "http://localhost:8000"
      }
    }
  }
}
```

### Claude Code

添加到项目根目录的 `.mcp.json` 文件中。本仓库已包含该文件，重启 Claude Code 即可加载。

### Claude Desktop

添加到你的 `claude_desktop_config.json`：
- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`
- **Linux:** `~/.config/Claude/claude_desktop_config.json`

### 其他 MCP 客户端

任何支持 stdio 传输的 MCP 客户端都可以使用本服务器。配置如下：
- **命令:** `python -m cowboy_mcp.server`
- **环境变量:** `COWBOY_BASE_URL=http://localhost:8000`

## 可用工具

### bind_player

绑定到游戏中的某个玩家，开始控制会话。

```
bind_player(game_id="<uuid>", player_name="B")
bind_player(game_id="<uuid>", player_name="B", autoplay=False)
```

- `game_id` — 游戏的 UUID（显示在前端 URL 或游戏创建响应中）
- `player_name` — `A`、`B`、`C` 或 `D` 之一
- `autoplay` — 可选，默认为 `true`；启用后，绑定后 MCP 会在后台自动进行回合操作

返回初始游戏状态快照。

### get_game_state

获取最新缓存的游戏状态。

```
get_game_state()
```

返回完整地图、所有玩家的位置/生命值/护盾/存活状态，以及当前轮到哪位玩家。

### wait_for_my_turn

阻塞等待，直到轮到绑定的玩家行动。

```
wait_for_my_turn(timeout_seconds=120)
```

在轮到你时返回游戏状态。如果游戏结束或超时，则提前返回。

### submit_action

提交游戏操作。

```
submit_action(command_type="move", direction="up")
submit_action(command_type="shoot", direction="left")
submit_action(command_type="shield", direction="down")
submit_action(command_type="speak", speak_text="hello")
```

- `command_type` — `move`（移动）、`shoot`（射击）、`shield`（护盾）或 `speak`（发言）
- `direction` — move/shoot/shield 必填：`up`（上）、`down`（下）、`left`（左）、`right`（右）
- `speak_text` — speak 必填

### get_session_info

检查会话状态和 WebSocket 连接健康度。

```
get_session_info()
```

### set_autoplay

启用或禁用当前绑定会话的自动游玩。

```
set_autoplay(enabled=True)
set_autoplay(enabled=False)
```

### get_autoplay_status

获取自动游玩状态和最近一次自动决策的元数据（包括决策思考过程）。

```
get_autoplay_status()
```

返回数据包括：
- `last_decision.reasoning.summary` — 选择该行动的简要说明
- `last_decision.reasoning.steps` — 逐步决策过程
- `last_decision.reasoning.meta` — 结构化详情（视线扫描、移动候选、距离等）

### explain_next_autoplay_move

预览自动游玩的下一步操作并返回结构化推理，不会实际提交任何操作。

```
explain_next_autoplay_move()
```

适用于在自动游玩执行前，查看它为什么会选择 `move`、`shoot` 或 `shield` 操作。
预览响应包含与 `last_decision.reasoning` 结构相同的顶层 `reasoning` 字段。

## 游戏循环示例

典型的手动代理游戏循环如下：

```
1. bind_player(game_id, "B")        → 获取初始状态
2. wait_for_my_turn()               → 阻塞等待直到轮到玩家 B
3. get_game_state()                 → 查看棋盘
4. submit_action("move", "right")   → 执行操作
5. 回到第 2 步                       → 重复直到游戏结束
```

启用默认自动游玩后，你只需：

```
1. bind_player(game_id, "B")        → 自动游玩自动开始
2. get_autoplay_status()            → 查看当前自动游玩的推理过程
```

**重要 — 展示推理详情：**

启用自动游玩后，你**必须**定期调用 `get_autoplay_status()` 并向用户展示每次自动决策的完整推理详情。包括：

1. 执行的操作（`command_type` + `direction`）
2. 原因代码（如 `enemy_in_line_of_sight`、`close_distance`、`defensive_fallback`）
3. 逐步思考过程（`last_decision.reasoning.steps`）
4. 视线分析或移动候选（`last_decision.reasoning.meta`）

绑定并启用自动游玩后，立即调用 `get_autoplay_status()` 并以可读的表格/摘要形式渲染推理过程。持续定期轮询，向用户展示自动游玩在做什么以及为什么。

## 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `COWBOY_BASE_URL` | `http://localhost:8000` | nginx 反向代理的基础 URL，用于访问所有游戏服务 |
| `COWBOY_MCP_AUTOPLAY_ON_BIND` | `true` | 调用 `bind_player` 时如未指定 `autoplay` 参数，是否自动启用自动游玩 |
| `COWBOY_MCP_AUTOPLAY_WAIT_TIMEOUT_SECONDS` | `120` | 自动游玩循环中等待回合状态的超时时间（秒） |

## 故障排除

**"No active session"** — 在使用其他工具前先调用 `bind_player`。

**"Failed to fetch game"** — 检查游戏服务栈是否正在运行（`make up`）以及 game_id 是否正确。

**"Not your turn"** — 使用 `wait_for_my_turn` 等待，或通过 `get_game_state` 查看当前轮到谁。

**WebSocket 断开** — 服务器会以指数退避策略自动重连。通过 `get_session_info` 检查连接健康度。

**自动游玩开启时进行手动控制** — 在发送手动 `submit_action` 命令前，先调用 `set_autoplay(enabled=False)`。
