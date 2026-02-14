# V3 Architecture and Design (Bot + Bot Manager)

## 1. Goal

Add AI-controlled players to Cowboy V3 so humans can play with bots.
Default assignment policy:
- player `A` is human
- players `B/C/D` are bots

Key outcomes:
- `bot-manager-service` owns bot lifecycle for each game.
- `bot-service` runs bot actors and decides commands.
- Bots read game events from each game's output Kafka topic and write commands to that game's input Kafka topic.
- On game end, bot workers are stopped and cleaned up.

## 2. Current System Context

Current V2 flow (already implemented):
- Per-game topics are created at game creation:
  - input topic: `game.commands.<game_id>.v1`
  - output topic: `game.output.<game_id>.v1`
- `web-service` publishes human commands to input topic.
- `timer-service` publishes timeout commands to input topic.
- `game-service` consumes input topic and emits step events to output topic.
- `game-watcher-service` consumes output topic and pushes websocket events to frontend.
- `game-manager-service` creates/starts/finishes game and deletes per-game topics on finish.

Bot feature extends this flow without breaking existing behavior.

## 3. New Services

## 3.1 bot-manager-service

Responsibilities:
- Observe game lifecycle (start/end) and orchestrate bots per game.
- Assign bot/human roles for each player in a game.
- Provide default assignment API that binds `B/C/D` as bots.
- Create bots for assigned bot players.
- Choose which bot-service instance each bot is bound to.
- Resolve per-player LLM settings (`base_url`, `model`, `api_key`) from YAML config and pass them at bot creation.
- Send game-teaching payload to each new bot (`teach_game`) and wait for `READY`.
- Bind each ready bot to `(game_id, player_id)`.
- Stop and destroy bots when game ends.
- Optionally retry failed bot startup/onboarding.

State managed by bot-manager:
- Bot registry: `bot_id -> process/session/status`.
- Binding map: `(game_id, player_id) -> (bot_id, bot_service_base_url)`.
- Onboarding version: `game_guide_version` per bot.
- Instance capacity target: `BOTS_PER_INSTANCE_CAPACITY` (default `2`).

## 3.2 bot-service

Responsibilities:
- One bot-service OS process serves many bot bindings concurrently.
- Run one bot actor per `(game_id, player_id)` binding.
- Maintain one persistent Python `Player` object per binding (destroyed when the game/binding ends).
- Accept onboarding payload and keep game guide in bot context.
- Consume the bound game's output topic.
- Detect whether it is the bot's turn.
- Generate one valid command (`move`, `shoot`, `shield`, `speak`) using DeepAgents.
- Publish command to the game's input topic.
- Ignore events when game is not running or when not the bot's turn.

## 4. Data and Control Flow

## 4.1 Game start
1. Human clicks `New`.
2. Role assignment is configured via bot-manager:
   - default API assigns `B/C/D` as bots, or
   - custom bind API assigns explicit bot-player mappings.
3. Human clicks `Start`.
4. `game-manager-service` marks game as running and emits `GAME_STARTED`.
5. `bot-manager-service` is notified and reconciles active bot bindings.
6. Bot manager creates missing bot workers for bot-assigned players.
7. Bot manager selects target bot-service instance per player with least-load + capacity target.
8. Bot manager sends `teach_game` payload to each created/restarted bot.
9. Each bot replies `READY`.
10. Bot manager activates bindings.

## 4.2 During turns
1. `game-service` emits step events to game output topic.
2. `game-watcher-service` broadcasts timeout/game-finished/speak to websocket clients.
3. Each bot worker consumes output topic events.
4. If snapshot says current player is this bot:
   - bot generates command with DeepAgents
   - bot publishes to input topic.
5. `game-service` processes bot command exactly like a human command.

## 4.3 Game end
1. `game-service` triggers finish through game manager.
2. `game-manager-service` emits `GAME_FINISHED` and deletes per-game topics.
3. `bot-manager-service` receives end signal and destroys all bots for the game.
4. Bindings and bot runtime state are removed.

## 5. Bot Onboarding (`teach_game`)

Problem:
- New bots do not know game rules by default.

Solution:
- Bot manager must teach each bot after creation and before activation.

Required payload fields:
- `game_guide_version`
- command schema and validation rules
- turn model and timeout behavior
- map/players snapshot
- this bot's `player_id` and `player_name`
- examples of valid commands

Activation rule:
- Bot is not allowed to publish commands until onboarding status is `READY`.

Re-teach triggers:
- Bot restarts
- `game_guide_version` change
- explicit manager re-sync request

## 6. DeepAgents Integration Design

Bot decision loop:
1. Build prompt context:
   - latest snapshot
   - last N events
   - bot identity (`player_id`, `player_name`)
2. Request model output with strict JSON shape:
   - `command_type`: `move|shoot|shield|speak`
   - `direction` when required
   - `speak_text` when `speak`
3. Validate output before publish.
4. If invalid:
   - retry once with validation error feedback
   - fallback to safe default command (`shield` with current shield direction or `up`).

Guardrails:
- Exactly one command per turn.
- Never emit reserved commands (`timeout`, `game_started`).
- Never publish with missing `player_id`, `turn_no`, or `command_id`.

## 7. Persistence and Reliability

MVP:
- In-memory bot registry + bindings in bot-manager.

Recommended:
- DynamoDB table `bot_bindings`:
  - PK: `game_id`
  - SK: `player_id`
  - attributes: `bot_id`, `status`, `game_guide_version`, `created_at`, `updated_at`

Idempotency:
- Bot manager start operation for same game must be idempotent.
- Destroy operation must be safe if bots are already absent.

## 8. Failure Handling

Failure cases and behavior:
- Bot creation fails: retry with backoff; report health degradation.
- `teach_game` fails: recreate bot or keep unbound; do not activate.
- Kafka unavailable: bot pauses publish and retries.
- Manager restart: reconstruct active games from source of truth and re-bind/re-teach.
- Late bot command: allowed in transport, ignored by `game-service` if stale; still persisted.

## 9. Security and Operations

Initial scope:
- Internal-only APIs (same network as backend services).
- No end-user auth changes required.

Operational requirements:
- Per-service health endpoint.
- Structured logs with `game_id`, `player_id`, `bot_id`, `command_id`.
- Config via env vars:
  - model/provider settings
  - Kafka bootstrap/prefix
  - manager base URLs
  - bot-service instance list (`BOT_SERVICE_BASE_URLS`)
  - per-instance capacity target (`BOTS_PER_INSTANCE_CAPACITY`, default `2`)
  - bot-manager YAML path (`BOT_MANAGER_LLM_CONFIG_PATH`) for default/per-player model settings
  - bot-service LangSmith YAML path (`BOT_AGENT_LANGSMITH_CONFIG_PATH`) for deepagents tracing settings
  - onboarding timeout/retry

## 10. Test Strategy

Unit tests:
- Bot manager lifecycle state transitions.
- Bot onboarding gate (`READY` required).
- Command validation and fallback.

Integration tests:
- Consume output topic and publish valid bot command to input topic.
- Re-teach on guide-version change.
- Destroy all bots on game end.

End-to-end tests (headless by default):
- Human + 3 bots game loop.
- Bot `speak` shows in websocket/frontend log.
- Timeout and finish events remain visible in frontend.
