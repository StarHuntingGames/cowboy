# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Cowboy is a turn-based multiplayer game where 1–4 players (A/B/C/D) compete on a grid map. Players can move, shoot, and shield. The last player alive wins. Player A is typically human; the rest are AI bots powered by LLMs via DeepAgents/LangChain. The number of players is configurable at game creation time (default: 2).

## Architecture

**Event-driven microservices** — 8 Rust services + 1 Python subprocess, communicating via Kafka topics and REST APIs. State stored in DynamoDB Local. Frontend is static HTML/JS served by nginx.

### Service Map

| Service | Port | Role |
|---|---|---|
| game-manager-service | 8081 | Game lifecycle (create/start/finish) |
| web-service | 8082 | REST API for human command submission |
| game-watcher-service | 8083 | WebSocket broadcaster for game events |
| game-service | 8084 | Game logic engine, turn processing |
| bot-manager-service | 8090 | Bot lifecycle orchestration, player assignment |
| bot-service | 8091 | Bot execution — Rust HTTP API wrapping Python subprocess per bot |
| timer-service | — | Auto-publishes timeout commands when turns expire |
| frontend | 8080 | Nginx serving static HTML/CSS/JS from `frontend/` |

### Data Flow

1. Human commands → web-service → Kafka input topic
2. Bot commands → bot-service (Python LLM decision) → Kafka input topic
3. game-service consumes input topic → applies rules → publishes step events to output topic
4. game-watcher-service consumes output topic → broadcasts via WebSocket to frontend
5. bot-service consumes output topic → triggers next bot decision when it's the bot's turn
6. bot-manager-service listens for GAME_STARTED → creates/assigns bots via bot-service API

### Kafka Topics

Per-game topics: `game.commands.<game_id>.v1` (input), `game.output.<game_id>.v1` (output). Global topics: `game.commands.v1`, `game.steps.v1`.

### DynamoDB Tables

`game_instances`, `game_steps`, `default_maps`, `bot_players`, `bot_llm_logs`

## Build & Run Commands

### Full Stack (Docker Compose)

```bash
make up          # Build and start all services
make down        # Stop services
make clean       # Stop + remove volumes (full reset)
make restart     # down + clean + up
make logs        # Tail logs (all services)
make ps          # Show running services
make init        # Start only infra (Kafka, DynamoDB) + init jobs
```

### Rust Backend

```bash
make backend-fmt                    # cargo fmt --all
make backend-check                  # cargo check
cargo build --manifest-path backend/Cargo.toml
cargo build --release -p bot-service --manifest-path backend/Cargo.toml  # Single service
```

Workspace root: `backend/Cargo.toml`. Shared types live in `cowboy-common` crate.

### Python (bot-service agent)

```bash
.venv/bin/pytest backend/bot-service/python/tests/                         # All Python tests
.venv/bin/pytest backend/bot-service/python/tests/test_player_agent_unit.py  # Single test file
.venv/bin/pytest backend/bot-service/python/tests/test_player_agent_unit.py::test_init  # Single test
```

Python venv is at `.venv/`. Dependencies in `backend/bot-service/python/requirements.txt`.

### E2E & Integration Tests

```bash
make e2e-llm-failure-speak          # Tests bot fallback when LLM unavailable
make e2e-verify-bot-config-wiring   # Validates config propagation
make e2e-llm-connection-test        # Tests real LLM connectivity
make integration-live-llm-output    # Python integration (live LLM output)
make integration-update-timeout     # Python integration (timeout behavior)
```

E2E scripts are in `scripts/`. They require the Docker Compose stack running (`make up` first).

## Key Files

- `backend/cowboy-common/src/lib.rs` — Shared types: PlayerName, CommandType, CommandEnvelope, StepEvent, GameStateSnapshot
- `backend/bot-service/python/player_agent.py` — LLM decision engine (FastAPI, DeepAgents, LangChain)
- `backend/bot-manager-service/src/main.rs` — Bot assignment logic, per-player LLM config loading
- `backend/bot-service/src/main.rs` — Bot actor management, Python subprocess orchestration, Kafka integration
- `conf/bot-manager-llm.yaml` — Per-player LLM model configuration (base_url, model, api_key)
- `conf/bot-service-langsmith.yaml` — LangSmith tracing configuration
- `conf/bot-service-prompts.yaml` — Bot LLM system/user prompt configuration
- `conf/default-map.yaml` — Default game map layout
- `frontend/index.html` — Main game UI page
- `frontend/game.js` — Game rendering, WebSocket client, command submission
- `frontend/styles.css` — Game UI styles
- `docker-compose.yml` — Full stack definition (11+ containers)

## Conventions

- **Rust edition 2024**, workspace resolver v2. All services are axum-based HTTP servers.
- Each Rust service has a single `src/main.rs` (no lib.rs split).
- Python uses `pytest-asyncio` for async tests. DeepAgents can be disabled (`BOT_AGENT_USE_DEEPAGENTS=0`) for fallback-only testing.
- LLM provider format: `"openai:gpt-4o-mini"` or `"anthropic:claude-3-sonnet"` — parsed by `player_agent.py`.
- Bot decisions have a fallback strategy: if LLM fails, Python generates random safe commands (shield if low HP).
- Kafka uses `rdkafka` crate in Rust. Per-game topic naming enables horizontal scaling.
- Config hierarchy: YAML defaults → per-player YAML overrides → environment variables.
- **Never perform git write operations** (commit, push, reset, checkout, rebase, etc.) — leave all git operations to the user.
- **GPL v3 license headers** — every source file must include the GPL v3 copyright header with `Copyright (C) 2026 StarHuntingGames`. Use the GitHub organization name `StarHuntingGames` for all copyright notices. When creating new files, always add the appropriate GPL header.
