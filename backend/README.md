# Cow Boy V2 Backend (Rust Workspace)

This folder contains the Version 2 backend services.

## Workspace Members
- `cowboy-common`: shared API/domain contracts.
- `game-manager-service`: create/start game APIs.
- `web-service`: command ingest API.
- `game-service`: game command processing service (scaffolded).
- `timer-service`: timeout scheduling service (scaffolded).
- `game-watcher-service`: snapshot/ws service (scaffolded).

## Current Status
Implemented now:
- Shared contracts and game data models.
- Game manager APIs:
  - `POST /v2/games`
  - `POST /v2/games/{game_id}/start`
  - `GET /v2/games/{game_id}`
  - `GET /v2/maps/default`
- Web service API:
  - `POST /v2/games/{game_id}/commands`

Pending integration (tracked in `../V2_TASK_LIST.md`):
- Kafka producer/consumer wiring.
- DynamoDB persistence wiring.
- Full game rules processing in `game-service`.
- Timer and watcher real pipelines.

## Run (once Rust crates are resolvable)
From `backend/`:

```bash
cargo run -p game-manager-service
cargo run -p web-service
cargo run -p game-watcher-service
```

Optional binds (env vars):
- `GAME_MANAGER_BIND` (default `0.0.0.0:8081`)
- `WEB_SERVICE_BIND` (default `0.0.0.0:8082`)
- `WATCHER_SERVICE_BIND` (default `0.0.0.0:8083`)

## Docker Compose (recommended for local V2 stack)
From repo root:

```bash
make
```

This runs `docker compose up --build -d` and starts:
- Kafka (+ topic init)
- DynamoDB Local (+ table init)
- game-manager-service
- web-service
- game-service
- timer-service
- game-watcher-service
- frontend (nginx serving current HTML/JS/CSS)

Useful commands:
- `make logs`
- `make ps`
- `make down`
- `make clean`

## AWS Lambda Compatibility (Rust Runtime)
The following services now support AWS Lambda runtime mode using
[`lambda_http`](https://github.com/awslabs/aws-lambda-rust-runtime):
- `game-manager-service`
- `web-service`
- `game-watcher-service`

Each service supports dual mode:
- Local mode: starts Axum HTTP server (current behavior).
- Lambda mode: auto-enabled when `AWS_LAMBDA_RUNTIME_API` is present, and runs `lambda_http::run(app)`.

### Build Lambda Artifacts
From `backend/`:

```bash
cargo build --release -p game-manager-service
cargo build --release -p web-service
cargo build --release -p game-watcher-service
```

For real Lambda deployment, compile for a Lambda target (example):

```bash
cargo build --release --target x86_64-unknown-linux-gnu -p game-manager-service
```

Then package each binary as `bootstrap` (or use your preferred Lambda Rust packaging workflow).

Note:
- `game-watcher-service` websocket streaming endpoint is still scaffold-level and not yet integrated with the Kafka event stream.
- For production websocket-on-lambda, use API Gateway WebSocket integration plus connection management flow.

## Note About Build in This Environment
If `cargo check` fails with crates.io DNS/network errors, code scaffolding is still generated correctly; dependency download just cannot complete in this environment.
