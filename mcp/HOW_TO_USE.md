# Cowboy MCP Server — How To Use

This MCP server lets an AI agent (Claude, etc.) control any player (A/B/C/D) in a running Cowboy game.

## Prerequisites

- Python 3.11+
- The Cowboy stack running (`make up` from project root)
- A game created and started via the frontend at `http://localhost:8000`

## Install

From the project root:

```bash
pip install -e mcp/
```

## Configure

The MCP server config is a JSON block you add to the appropriate config file for your client.

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

Add to `.mcp.json` in the project root. This file is already included in this repo — just restart Claude Code to load it.

### Claude Desktop

Add to your `claude_desktop_config.json`:
- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`
- **Linux:** `~/.config/Claude/claude_desktop_config.json`

### Other MCP Clients

Any MCP client that supports stdio transport can use this server. Point it at:
- **Command:** `python -m cowboy_mcp.server`
- **Environment variable:** `COWBOY_BASE_URL=http://localhost:8000`

## Available Tools

### bind_player

Start a control session by binding to a player in a game.

```
bind_player(game_id="<uuid>", player_name="B")
bind_player(game_id="<uuid>", player_name="B", autoplay=False)
```

- `game_id` — the UUID of the game (shown in the frontend URL or game creation response)
- `player_name` — one of `A`, `B`, `C`, or `D`
- `autoplay` — optional, defaults to `true`; when enabled, MCP auto-plays turns in the background after bind

Returns the initial game state snapshot.

### get_game_state

Get the latest cached game state.

```
get_game_state()
```

Returns the full map, all players' positions/HP/shields/alive status, and whose turn it is.

### wait_for_my_turn

Block until it's the bound player's turn.

```
wait_for_my_turn(timeout_seconds=120)
```

Returns the game state when it becomes your turn. Returns early if the game finishes or the timeout expires.

### submit_action

Submit a game action.

```
submit_action(command_type="move", direction="up")
submit_action(command_type="shoot", direction="left")
submit_action(command_type="shield", direction="down")
submit_action(command_type="speak", speak_text="hello")
```

- `command_type` — `move`, `shoot`, `shield`, or `speak`
- `direction` — required for move/shoot/shield: `up`, `down`, `left`, `right`
- `speak_text` — required for speak

### get_session_info

Check session status and WebSocket connection health.

```
get_session_info()
```

### set_autoplay

Enable or disable autoplay for the current bound session.

```
set_autoplay(enabled=True)
set_autoplay(enabled=False)
```

### get_autoplay_status

Get autoplay status and last autoplay decision metadata (including decision thinking).

```
get_autoplay_status()
```

The payload includes:
- `last_decision.reasoning.summary` — concise explanation of why the action was chosen
- `last_decision.reasoning.steps` — step-by-step decision trail
- `last_decision.reasoning.meta` — structured details (line-of-sight scan, move candidates, distances)

### explain_next_autoplay_move

Preview autoplay's next command and return structured reasoning without submitting anything.

```
explain_next_autoplay_move()
```

Useful when you want to inspect exactly why autoplay would choose a `move`, `shoot`, or `shield` action before letting it act.
This preview response now includes a top-level `reasoning` block with the same structure as `last_decision.reasoning`.

## Game Loop Example

A typical manual agent game loop looks like:

```
1. bind_player(game_id, "B")        → get initial state
2. wait_for_my_turn()               → blocks until it's player B's turn
3. get_game_state()                 → inspect the board
4. submit_action("move", "right")   → make a move
5. goto 2                           → repeat until game ends
```

With default autoplay enabled, you can simply:

```
1. bind_player(game_id, "B")        → autoplay starts automatically
2. get_autoplay_status()            → inspect current autoplay reasoning
```

**IMPORTANT — Showing Reasoning Details:**

When autoplay is enabled, you MUST periodically call `get_autoplay_status()` and display the full reasoning details of every autoplay decision to the user. This includes:

1. The action taken (`command_type` + `direction`)
2. The reason code (e.g., `enemy_in_line_of_sight`, `close_distance`, `defensive_fallback`)
3. The step-by-step thinking (`last_decision.reasoning.steps`)
4. The line-of-sight analysis or move candidates from `last_decision.reasoning.meta`

After binding with autoplay, immediately call `get_autoplay_status()` and render the reasoning in a readable table/summary. Continue polling periodically to show the user what the autoplay is doing and why.

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `COWBOY_BASE_URL` | `http://localhost:8000` | Base URL of the nginx reverse proxy serving all game services |
| `COWBOY_MCP_AUTOPLAY_ON_BIND` | `true` | Enable autoplay automatically when `bind_player` is called without `autoplay` argument |
| `COWBOY_MCP_AUTOPLAY_WAIT_TIMEOUT_SECONDS` | `120` | Wait timeout used by the autoplay loop before re-checking turn state |

## Troubleshooting

**"No active session"** — call `bind_player` before using other tools.

**"Failed to fetch game"** — check that the game stack is running (`make up`) and the game_id is correct.

**"Not your turn"** — use `wait_for_my_turn` to wait, or check `get_game_state` to see whose turn it is.

**WebSocket disconnects** — the server auto-reconnects with exponential backoff. Check `get_session_info` for connection health.

**Manual control while autoplay is on** — call `set_autoplay(enabled=False)` before sending manual `submit_action` commands.
