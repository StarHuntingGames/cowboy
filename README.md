English | [中文](README_CN.md)

# Cowboy

A turn-based multiplayer battle game where 1–4 players compete on a grid map. Players can move, shoot lasers, and position shields. Player A is human-controlled; the rest are AI bots powered by LLMs (GPT, Gemini, Claude, etc.) via LangChain.

Last player standing wins.

**This entire project — architecture, backend, frontend, infrastructure, tests, and documentation — is 100% developed by AI agents (Claude Code).** No line of code was written by a human. Human involvement was limited to directing the AI with high-level instructions and reviewing results.

## Demo

[<video src="https://raw.githubusercontent.com/StarHuntingGames/cowboy/refs/heads/main/demo.mp4" autoplay loop muted playsinline width="100%"></video>](https://github.com/user-attachments/assets/d108a53e-adeb-46fe-8240-ce14deb28c2c)

## Quick Start

**Prerequisites:** Docker and Docker Compose.

```bash
# 1. Clone and configure
git clone https://github.com/StarHuntingGames/cowboy.git
cd cowboy

# 2. Add your API keys — create a .env file in the project root
cat > .env <<EOF
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
EOF

# 3. Start everything
make up

# 4. Open the game
#    http://localhost:8000
```

To stop: `make down` | Full reset: `make clean` | Restart: `make restart`

## Architecture

Event-driven microservices — 8 Rust services + 1 Python subprocess, communicating via Kafka and REST. State stored in DynamoDB Local. Frontend is static HTML/JS served by nginx.

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

### Data Flow

1. Human commands → `web-service` → Kafka input topic
2. Bot commands → `bot-service` (Python LLM decision) → Kafka input topic
3. `game-service` consumes input → applies game rules → publishes step events
4. `game-watcher-service` consumes events → broadcasts via WebSocket to frontend
5. `bot-service` consumes events → triggers next bot decision on bot's turn

### Kafka Topics

Per-game topics: `game.commands.<game_id>.v1` (input), `game.output.<game_id>.v1` (output).

## Game Rules

- **Grid map** with empty cells and destructible/indestructible blocks
- **Turn order:** A → B → C → D (fixed)
- **Actions (one per turn):** move, shoot, or reposition shield
- **Shooting** fires a laser in a straight line — stops at the first block or player
- **Shield** blocks incoming shots from one direction
- **HP:** starts at 10, lose 1 per unblocked hit, eliminated at 0

See [docs/GAME_RULES.md](docs/GAME_RULES.md) for full rules.

## MCP Server (AI Agent Control)

The project includes an MCP server that lets any AI agent (Claude, etc.) play Cowboy by controlling a player via the [Model Context Protocol](https://modelcontextprotocol.io/). The agent can bind to any player (A/B/C/D) in a running game, observe the board state in real time, and submit actions — move, shoot, shield, or speak.

### Control Human Players with AI

Instead of playing manually through the browser, you can let an AI agent take over a human player slot via MCP. This means you can watch Claude, GPT, or any MCP-compatible AI agent play the game for you — or even pit multiple AI agents against each other, each controlling a different human player.

**How it works:**

1. Start a game from the frontend and set the number of human players (e.g., 2 humans, 2 bots)
2. Connect your AI agent (e.g., Claude Code) to the MCP server
3. The agent binds to a human player slot (A, B, C, or D) using `bind_player`
4. The agent receives all game events in real time — moves, shots, shields, speaks, and more
5. The agent decides and submits actions each turn, either via built-in autoplay heuristics or its own reasoning

```
Frontend (browser)          MCP Server              AI Agent (Claude Code, etc.)
       │                        │                           │
       │   Create game          │                           │
       │   (2 humans, 2 bots)   │                           │
       │                        │    bind_player(game, "A") │
       │                        │◄──────────────────────────│
       │   Start game           │                           │
       │                        │    ── game events ──────► │
       │                        │    ◄── submit_action ──── │
       │   Watch the battle!    │                           │
```

Available tools: `bind_player`, `get_game_state`, `wait_for_my_turn`, `submit_action`, `get_session_info`, `set_autoplay`, `get_autoplay_status`, `explain_next_autoplay_move`.

See [mcp/HOW_TO_USE.md](mcp/HOW_TO_USE.md) for setup, configuration, and usage details.

## Configuration

### API Keys

Create a `.env` file in the project root (already in `.gitignore`):

```
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
```

Docker Compose reads this file automatically. The YAML configs in `conf/` reference these via `${BOT_LLM_API_KEY}` and `${LANGSMITH_API_KEY}` placeholders, which are expanded at runtime.

### LLM Bot Config

Edit `conf/bot-manager-llm.yaml` to set per-player LLM providers:

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

### LangSmith Tracing (Optional)

Edit `conf/bot-service-langsmith.yaml` to enable LLM tracing.

### Bot Prompts

Customize bot behavior in `conf/bot-service-prompts.yaml`.

## Development

### Build Commands

```bash
make up                    # Build and start all services
make down                  # Stop services
make restart               # Full restart (down + clean + up)
make clean                 # Stop + remove volumes (full reset)
make logs                  # Tail all logs
make ps                    # Show running services
make init                  # Start only infra (Kafka, DynamoDB)
make restart-bot           # Rebuild and restart bot-service only
make restart-bot-manager   # Restart bot-manager-service only
```

### Rust Backend

```bash
make backend-fmt           # cargo fmt
make backend-check         # cargo check
cargo build --manifest-path backend/Cargo.toml
```

### Python Tests

```bash
.venv/bin/pytest backend/bot-service/python/tests/
```

### E2E Tests

Require the Docker Compose stack running (`make up` first).

```bash
make e2e-llm-failure-speak        # Bot fallback when LLM unavailable
make e2e-verify-bot-config-wiring  # Config propagation
make e2e-llm-connection-test       # Real LLM connectivity
```

## Tech Stack

- **Backend:** Rust (axum), Python (FastAPI, LangChain)
- **Messaging:** Apache Kafka
- **Storage:** DynamoDB Local
- **Frontend:** HTML5 Canvas, vanilla JS
- **Infrastructure:** Docker Compose, nginx

## Documentation

- [Game Rules](docs/GAME_RULES.md)
- [MCP Server Guide](mcp/HOW_TO_USE.md)
- [Architecture Design](docs/V3_ARCHITECTURE_DESIGN.md)
- [How to Run](docs/HOW_TO_RUN_V2.md)
- [API Changes](docs/V3_API_CHANGES.md)
- [Test Cases](docs/TEST_CASES.md)

## Contact

StarHuntingGames — starhuntinggames@gmail.com

## License

This project is licensed under the GNU General Public License v3.0 — see the [LICENSE](LICENSE) file.
