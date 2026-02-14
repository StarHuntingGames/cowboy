# How To Run (Make + Docker Compose)

This guide explains how to start and manage the full stack locally.

## Prerequisites
- Docker Desktop (or Docker Engine + Compose plugin)
- `make`

Check:
```bash
docker --version
docker compose version
make --version
```

## One Command Start (Recommended)
From project root:
```bash
cd cowboy

# Create .env with your API keys (one-time setup)
cat > .env <<EOF
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
EOF

make
```

This is the same as:
```bash
make up
```

It runs:
- Kafka + Zookeeper
- Kafka topic init (`game.commands.v1`, `game.steps.v1`)
- DynamoDB Local
- DynamoDB table init (`default_maps`, `game_instances`, `game_steps`, `bot_players`, `bot_llm_logs`)
- Backend services (game-manager, web, game, timer, game-watcher, bot-manager, bot-service)
- Frontend (nginx)

## Useful Make Commands
From project root:

Start all:
```bash
make up
```

Show running services:
```bash
make ps
```

View logs (all services):
```bash
make logs
```

Stop services:
```bash
make down
```

Stop + remove volumes (clean reset):
```bash
make clean
```

Restart everything:
```bash
make restart
```

Init infra only (Kafka/DynamoDB + init jobs):
```bash
make init
```

Restart individual bot services:
```bash
make restart-bot            # Rebuild and restart bot-service
make restart-bot-manager    # Restart bot-manager-service
```

Backend Rust helpers:
```bash
make backend-fmt
make backend-check
```

## Direct Docker Compose Commands
If you prefer not to use `make`:

Start all:
```bash
docker compose -f docker-compose.yml up --build -d
```

Show status:
```bash
docker compose -f docker-compose.yml ps
```

Tail logs:
```bash
docker compose -f docker-compose.yml logs -f --tail=200
```

Stop:
```bash
docker compose -f docker-compose.yml down --remove-orphans
```

Clean reset:
```bash
docker compose -f docker-compose.yml down -v --remove-orphans
```

## Local Endpoints
All services are accessed through nginx on port 8000:
- Frontend: [http://localhost:8000](http://localhost:8000)
- Game API: `http://localhost:8000/v2/games`
- WebSocket: `ws://localhost:8000/v2/games/{game_id}/stream`
- Kafka (host access): `localhost:29092`

Internal service ports (not exposed to host by default):
- Game Manager Service: `8081`
- Web Service: `8082`
- Game Watcher Service: `8083`
- Game Service: `8084`
- Bot Manager Service: `8090`
- Bot Service: `8091`

## Common Workflow
1. Start stack:
```bash
make up
```

2. Open the game:
```
http://localhost:8000
```

3. Watch logs while testing:
```bash
make logs
```

4. Stop when done:
```bash
make down
```

## Configuration

### API Keys
Create a `.env` file in the project root (already in `.gitignore`):
```
BOT_LLM_API_KEY=your-llm-api-key-here
LANGSMITH_API_KEY=your-langsmith-api-key-here
```

Docker Compose reads this file automatically and passes the values into containers.

### LLM Bot Config
Edit `conf/bot-manager-llm.yaml` to configure per-player LLM models.

### LangSmith Tracing
Edit `conf/bot-service-langsmith.yaml` to enable/disable LLM tracing.

### Bot Prompts
Edit `conf/bot-service-prompts.yaml` to customize bot behavior.

## Troubleshooting
If ports are busy:
- Change published ports in `docker-compose.yml`, then run `make up` again.

If containers fail during init:
```bash
make down
make up
make logs
```

If you want a full clean restart:
```bash
make clean
make up
```

If bot LLM calls fail with "missing credentials":
- Verify your `.env` file has valid API keys.
- Run `make restart` to recreate containers with the updated keys.
