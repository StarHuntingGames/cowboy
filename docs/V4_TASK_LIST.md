# V4 Task List — MCP Server for Player Control

Status values used in this file:
- `TODO`: not started
- `IN_PROGRESS`: partially done, or done but not fully validated
- `DONE`: completed and verified for that status column

| ID | Task | Coding | Testing | Finished | Notes |
|---|---|---|---|---|---|
| MCP-001 | Create `mcp/` project skeleton (pyproject.toml, package structure, dependencies) | DONE | DONE | DONE | Python MCP server using official SDK, stdio transport. |
| MCP-002 | Implement `game_client.py` — HTTP client for game-manager and web-service via nginx (:8000) | DONE | DONE | DONE | Single base URL, httpx async client. GET game info, GET snapshot, POST commands. |
| MCP-003 | Implement `ws_client.py` — WebSocket client for game-watcher stream via nginx (:8000) | DONE | DONE | DONE | Background asyncio task, auto-reconnect, parses SNAPSHOT/GAME_STARTED/GAME_FINISHED events, updates session state. |
| MCP-004 | Implement `session.py` — Session state management | DONE | DONE | DONE | Stores game_id, player_id, player_name, latest_snapshot, game_status. Asyncio Event for turn signaling. |
| MCP-005 | Implement `server.py` — MCP server with tool definitions (bind_player, get_game_state, wait_for_my_turn, submit_action, get_session_info) | DONE | DONE | DONE | Wires session, ws_client, game_client together. Stdio transport. Package installs and all 5 tools register correctly. |
| MCP-006 | Add GPL v3 license headers to all source files | DONE | DONE | DONE | Copyright (C) 2026 StarHuntingGames. |
| MCP-007 | Manual integration test: bind to a running game and play a turn via MCP tools | TODO | TODO | TODO | Requires `make up` stack running. |

## Design Summary

- **Language:** Python
- **Transport:** stdio
- **Architecture:** Single-session stateful server (one instance = one player)
- **Base URL:** `COWBOY_BASE_URL` env var (default `http://localhost:8000`), all services accessed via nginx reverse proxy
- **Tools exposed:**
  - `bind_player(game_id, player_name)` — start a control session
  - `get_game_state()` — latest cached snapshot
  - `wait_for_my_turn(timeout_seconds)` — blocks until it's the bound player's turn
  - `submit_action(command_type, direction, speak_text)` — submit a command
  - `get_session_info()` — session status and connection health

## Update Rules

When a task progresses, update columns in this order:
1. Set `Coding` to `DONE` after implementation is complete.
2. Set `Testing` to `DONE` after relevant automated/manual tests pass.
3. Set `Finished` to `DONE` only when coding + testing are done and verified.
