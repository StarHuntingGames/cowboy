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
```

- `game_id` — the UUID of the game (shown in the frontend URL or game creation response)
- `player_name` — one of `A`, `B`, `C`, or `D`

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

## Game Loop Example

A typical agent game loop looks like:

```
1. bind_player(game_id, "B")        → get initial state
2. wait_for_my_turn()               → blocks until it's player B's turn
3. get_game_state()                 → inspect the board
4. submit_action("move", "right")   → make a move
5. goto 2                           → repeat until game ends
```

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `COWBOY_BASE_URL` | `http://localhost:8000` | Base URL of the nginx reverse proxy serving all game services |

## Troubleshooting

**"No active session"** — call `bind_player` before using other tools.

**"Failed to fetch game"** — check that the game stack is running (`make up`) and the game_id is correct.

**"Not your turn"** — use `wait_for_my_turn` to wait, or check `get_game_state` to see whose turn it is.

**WebSocket disconnects** — the server auto-reconnects with exponential backoff. Check `get_session_info` for connection health.
