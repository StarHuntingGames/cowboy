# V3 API Additions and Modifications

## 1. Scope

This document defines:
- New APIs required for `bot-manager-service` and `bot-service`.
- Existing API/event contract changes needed to support bots.
- Backward-compatibility guidance.

## 2. Existing APIs That Must Be Modified

## 2.1 `POST /v2/games` (game-manager-service)

Current behavior:
- Creates game.
- Returns `game_id`, initial state, timeout, players, and turn metadata.

Required modification:
- Keep game creation role-agnostic.
- Do not include player `kind` in request or response.
- Continue returning player identities (`player_name`, `player_id`) so bot-manager can assign later.

## 2.2 `GET /v2/games/{game_id}` (game-manager-service)

Current behavior:
- Returns full game instance and state.

Required modification:
- Keep game-manager role-agnostic.
- Ensure bot-manager can derive all assignment inputs from this response:
  - `state.players[*].player_name`
  - `state.players[*].player_id`
  - `input_topic`
  - `output_topic`

## 2.3 `POST /v2/games/{game_id}/start` (game-manager-service)

Current behavior:
- Starts game and emits `GAME_STARTED`.

Required modification:
- Keep existing behavior.
- Optionally notify bot-manager to reconcile active assignments after successful start.

Options:
- Option A: game-manager calls bot-manager API synchronously.
- Option B: game-manager publishes `GAME_STARTED` and bot-manager consumes the output topic.

Recommendation:
- Option B for lower coupling and easier retries.

## 3. New API: bot-manager-service

Base URL example: `http://bot-manager-service:8090`

## 3.1 Health
- `GET /health`

Response:
```json
{ "ok": true, "service": "bot-manager-service" }
```

## 3.2 Assign default players for a game
- `POST /internal/v3/games/{game_id}/assignments/default`

Purpose:
- Assign players using default rule:
  - `A` as human
  - `B/C/D` as bots
- Create and bind bots for bot-assigned players.

Request:
```json
{
  "apply_immediately": true,
  "game_guide_version": "v1",
  "force_recreate": false
}
```

Response:
```json
{
  "assigned": true,
  "game_id": "uuid",
  "humans": [
    { "player_name": "A", "player_id": "uuid-a" }
  ],
  "bindings": [
    { "player_name": "B", "player_id": "uuid-b", "bot_id": "bot-1", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_name": "C", "player_id": "uuid-c", "bot_id": "bot-2", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_name": "D", "player_id": "uuid-d", "bot_id": "bot-3", "bot_service_base_url": "http://bot-service:8091", "status": "READY" }
  ]
}
```

## 3.3 Bind one bot to one player
- `POST /internal/v3/games/{game_id}/bindings`

Purpose:
- Explicitly bind a bot to a specific player.

Request:
```json
{
  "player_id": "uuid-b",
  "bot_id": "bot-1",
  "create_bot_if_missing": true,
  "game_guide_version": "v1"
}
```

Response:
```json
{
  "bound": true,
  "game_id": "uuid",
  "player_id": "uuid-b",
  "bot_id": "bot-1",
  "bot_service_base_url": "http://bot-service:8091",
  "status": "READY"
}
```

## 3.4 Assign players explicitly (bulk)
- `POST /internal/v3/games/{game_id}/assignments`

Purpose:
- Assign all players with a custom human/bot plan.

Request:
```json
{
  "human_player_ids": ["uuid-a"],
  "bot_player_ids": ["uuid-b", "uuid-c", "uuid-d"],
  "game_guide_version": "v1",
  "force_recreate": false
}
```

Response:
```json
{
  "assigned": true,
  "game_id": "uuid",
  "humans": [
    { "player_id": "uuid-a" }
  ],
  "bindings": [
    { "player_id": "uuid-b", "bot_id": "bot-1", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_id": "uuid-c", "bot_id": "bot-2", "bot_service_base_url": "http://bot-service:8091", "status": "READY" },
    { "player_id": "uuid-d", "bot_id": "bot-3", "bot_service_base_url": "http://bot-service:8091", "status": "READY" }
  ]
}
```

## 3.5 Stop bots for a game
- `POST /internal/v3/games/{game_id}/bots/stop`

Purpose:
- Destroy all bots and clear bindings when game ends.

Request:
```json
{
  "reason": "GAME_FINISHED"
}
```

Response:
```json
{
  "stopped": true,
  "game_id": "uuid",
  "destroyed_bot_count": 3
}
```

## 3.6 Query active assignments and bindings
- `GET /internal/v3/games/{game_id}/assignments`

Response:
```json
{
  "game_id": "uuid",
  "humans": [
    { "player_name": "A", "player_id": "uuid-a" }
  ],
  "bindings": [
    { "player_name": "B", "player_id": "uuid-b", "bot_id": "bot-1", "bot_service_base_url": "http://bot-service:8091", "status": "READY" }
  ]
}
```

Scheduling/config notes:
- Bot manager supports multiple bot-service instances via `BOT_SERVICE_BASE_URLS` (comma-separated URLs).
- Bot manager uses `BOTS_PER_INSTANCE_CAPACITY` (default `2`) as the per-instance capacity target.
- When all instances are at/over target, manager still assigns to the least-loaded instance.
- Bot manager can load per-player LLM config from `BOT_MANAGER_LLM_CONFIG_PATH`.
- Bot service can load LangSmith/deepagents tracing config from `BOT_AGENT_LANGSMITH_CONFIG_PATH`.
- YAML supports `default` plus per-player overrides (`A/B/C/D`) for `base_url`, `model`, `api_key`.

YAML example:
```yaml
default:
  base_url: "https://api.openai.com/v1"
  model: "openai:gpt-4o-mini"
  api_key: "sk-..."
players:
  B:
    model: "openai:gpt-4o"
  C:
    model: "openai:gpt-4o-mini"
  D:
    base_url: "https://compatible-endpoint/v1"
    model: "openai:gpt-4o-mini"
    api_key: "sk-..."
```

## 4. New API: bot-service

Base URL example: `http://bot-service:8091`

## 4.1 Health
- `GET /health`

Response:
```json
{ "ok": true, "service": "bot-service" }
```

## 4.2 Create bot actor
- `POST /internal/v3/bots`

Request:
```json
{
  "game_id": "uuid",
  "player_name": "B",
  "player_id": "uuid-b",
  "input_topic": "game.commands.uuid.v1",
  "output_topic": "game.output.uuid.v1",
  "llm_base_url": "https://api.openai.com/v1",
  "llm_model": "openai:gpt-4o-mini",
  "llm_api_key": "sk-..."
}
```

Field notes:
- `llm_base_url`, `llm_model`, `llm_api_key` are optional and set by bot-manager per player.
- Different players in the same bot-service instance can use different model settings.

Response:
```json
{
  "bot_id": "bot-1",
  "status": "CREATED"
}
```

## 4.3 Teach bot game rules (required)
- `POST /internal/v3/bots/{bot_id}/teach-game`

Request:
```json
{
  "game_guide_version": "v1",
  "rules_markdown": "....",
  "command_schema": {
    "allowed": ["move", "shoot", "shield", "speak"],
    "direction_required_for": ["move", "shoot", "shield"],
    "speak_text_required_for": ["speak"]
  },
  "examples": [
    { "command_type": "move", "direction": "up" },
    { "command_type": "speak", "speak_text": "hello" }
  ]
}
```

Response:
```json
{
  "bot_id": "bot-1",
  "status": "READY",
  "game_guide_version": "v1"
}
```

Behavior:
- Bot must not publish gameplay commands before this endpoint succeeds.

## 4.4 Destroy bot actor
- `DELETE /internal/v3/bots/{bot_id}`

Response:
```json
{
  "deleted": true,
  "bot_id": "bot-1"
}
```

## 5. WebSocket/Event Contract Changes

No new websocket endpoint is required. Existing endpoint remains:
- `GET /v2/games/{game_id}/stream` (upgrade to websocket)

Required event usage:
- Existing `SPEAK` event already fits bots.
- Frontend should render human and bot speak uniformly using `player_id` lookup.

`SPEAK` event shape (already compatible):
```json
{
  "event_type": "SPEAK",
  "game_id": "uuid",
  "turn_no": 12,
  "player_id": "uuid-b",
  "speak_text": "Bot says hello",
  "snapshot": { "...": "..." }
}
```

## 6. Kafka Contract Changes

Existing per-game topics are reused:
- input: `game.commands.<game_id>.v1`
- output: `game.output.<game_id>.v1`

Required convention updates:
- Bot commands must use:
  - `source: "user"` or add `"bot"` source.

Recommendation:
- Add new source enum value `"bot"` for clearer analytics and debugging.

Proposed command envelope example:
```json
{
  "command_id": "uuid",
  "source": "bot",
  "game_id": "uuid",
  "player_id": "uuid-b",
  "command_type": "speak",
  "direction": null,
  "speak_text": "I will win",
  "turn_no": 12,
  "sent_at": "2026-02-10T12:00:00Z"
}
```

## 7. DynamoDB Model Changes

## 7.1 Existing `game_steps` table

No schema break required. Bot commands are persisted like human commands.

Recommended additions:
- `source = BOT` value support in `source` attribute.
- Keep `speak_text` persistence (already present for speak).

## 7.2 New optional table `bot_bindings`

Purpose:
- Durable mapping and recovery after bot-manager restart.

Suggested schema:
- PK: `game_id` (S)
- SK: `player_id` (S)
- Attributes:
  - `player_name` (S)
  - `bot_id` (S)
  - `status` (S) (`CREATED|READY|STOPPED|FAILED`)
  - `game_guide_version` (S)
  - `created_at` (S)
  - `updated_at` (S)

## 8. Backward Compatibility

Compatibility rules:
- Existing frontend command and stream APIs remain valid.
- Existing game creation stays unchanged and role-agnostic.
- Existing consumers should ignore unknown fields safely.

Migration order:
1. Deploy bot-service + bot-manager-service.
2. Add bot-manager assignment APIs (`default`, `bulk`, `bind`).
3. Wire game start/end hooks for bot reconciliation and teardown.
4. Enable frontend to call default or custom assignment API before start.

## 9. Open Decisions

1. Should bot commands use `source: "bot"` or reuse `source: "user"`?
2. Should bot-manager be event-driven only, API-driven only, or hybrid?
3. Should bot binding state be in-memory only for MVP or persisted in DynamoDB now?
4. Should assignment happen only via explicit API call, or should start trigger auto-default assignment when none exists?
