# Bot Feature Task List (V3)

Status values used in this file:
- `TODO`: not started
- `IN_PROGRESS`: partially done, or done but not fully validated
- `DONE`: completed and verified for that status column

| ID | Task | Coding | Testing | Finished | Notes |
|---|---|---|---|---|---|
| BOT-001 | Create `bot-manager-service` project skeleton (config, startup, health endpoint, logging) | DONE | DONE | DONE | Service boots and health endpoint responds. |
| BOT-002 | Create `bot-service` project skeleton based on DeepAgents integration | DONE | DONE | DONE | Python DeepAgents runtime wired in bot-service with fallback policy; validated in headless E2E. |
| BOT-003 | Add game lifecycle hook so bot manager is triggered when a game starts | DONE | DONE | DONE | Consumer on output topics handles `GAME_STARTED`. |
| BOT-004 | Implement bot creation flow for assigned bot players (default policy: B/C/D) | DONE | DONE | DONE | Implemented via default assignment API. |
| BOT-005 | Implement bot destroy flow when game ends | DONE | DONE | DONE | Verified via `GAME_FINISHED` event: `/tmp/cowboy-v3-bot-teardown-summary.json`. |
| BOT-006 | Implement bot-player binding model `(game_id, player_id, bot_id)` | DONE | DONE | DONE | Stored in bot-manager assignment map and queryable via API. |
| BOT-007 | Add bot onboarding API/message `teach_game` sent by bot manager after bot creation | DONE | DONE | DONE | Bot manager calls bot-service `teach-game` after create/bind. |
| BOT-008 | Implement bot-side game-guide ingestion and readiness ack (`READY`) | DONE | DONE | DONE | Bot lifecycle transitions to `READY` after teach call. |
| BOT-009 | Include versioned game guide (`game_guide_version`) and re-teach logic on restart/version change | DONE | IN_PROGRESS | IN_PROGRESS | Version field is wired; explicit re-teach coverage by test is pending. |
| BOT-010 | Subscribe bot workers to per-game output Kafka topic | DONE | DONE | DONE | Worker subscribes to each assigned game's output topic. |
| BOT-011 | Implement turn detection logic so bot acts only on its own turn | DONE | DONE | DONE | Worker gates on `current_player_id` and `turn_no`. |
| BOT-012 | Implement command generation with DeepAgents (`move`, `shoot`, `shield`, `speak`) | DONE | DONE | DONE | DeepAgents-backed Python decision path with strict normalization and fallback; observed bot `speak`/turn progression in `/tmp/cowboy-v3-bot-concurrency-summary.json`. |
| BOT-013 | Add command validation + fallback policy (retry once, then safe default) | TODO | TODO | TODO | Not implemented yet. |
| BOT-014 | Publish bot commands to per-game input Kafka topic | DONE | DONE | DONE | Bot worker publishes command envelopes to input topic. |
| BOT-015 | Ensure `speak` text is preserved end-to-end in output events and websocket broadcast | DONE | DONE | DONE | Verified in headless UI log: `/tmp/cowboy-v3-bot-e2e-summary.json`. |
| BOT-016 | Add configuration and secrets management for LLM provider used by bot service | DONE | DONE | DONE | `.env` file for API keys, `${BOT_LLM_API_KEY}` / `${LANGSMITH_API_KEY}` expansion in YAML configs, env vars passed through docker-compose. |
| BOT-017 | Update `docker-compose` and local run scripts for bot services | DONE | DONE | DONE | `bot-service` and `bot-manager-service` wired into compose. |
| BOT-018 | Add unit tests for bot manager lifecycle and binding logic | TODO | TODO | TODO | No dedicated unit tests yet. |
| BOT-019 | Add unit tests for bot command generation/validation/fallback | TODO | TODO | TODO | No dedicated unit tests yet. |
| BOT-020 | Add integration test for Kafka flow (output consume -> bot action -> input publish) | TODO | TODO | TODO | Manual verification done; automated integration test is pending. |
| BOT-021 | Add end-to-end test (default headless Chrome) for human + bots gameplay loop | IN_PROGRESS | DONE | IN_PROGRESS | Manual headless E2E passed, but checked-in automated test file is pending. |
| BOT-022 | Add end-to-end test for bot teardown on game end (workers stop + no further commands) | IN_PROGRESS | DONE | IN_PROGRESS | Manual teardown E2E passed; checked-in automated test file is pending. |
| BOT-023 | Update docs (`README`/service docs) with architecture, env vars, and run/test steps | DONE | DONE | DONE | `V3_ARCHITECTURE_DESIGN.md` and `V3_API_CHANGES.md` updated. |
| BOT-024 | Final verification pass: all tests green, logs clean, and acceptance criteria met | IN_PROGRESS | IN_PROGRESS | IN_PROGRESS | Core manual E2E checks passed; remaining DeepAgents/test automation tasks pending. |
| BOT-025 | Add bot-manager default assignment API to bind `B/C/D` as bots and `A` as human | DONE | DONE | DONE | Endpoint implemented and validated in headless E2E flow. |
| BOT-026 | Add bot-manager bind/assignment APIs for explicit bot-player mapping | DONE | DONE | DONE | Endpoints implemented: bind, bulk assignment, query assignment. |
| BOT-027 | Bot-service lifecycle: one persistent Python `Player` object per bot/player, destroyed on game end | DONE | DONE | DONE | Added line-protocol player agent (`backend/bot-service/python/player_agent.py`) and persistent session integration in `backend/bot-service/src/main.rs`; validated by headless E2E run. |
| BOT-028 | Bot-manager instance scheduling: select bot-service instance per binding with configurable capacity (default `2`) | DONE | DONE | DONE | Added `BOT_SERVICE_BASE_URLS`, `BOTS_PER_INSTANCE_CAPACITY`, least-load selection, and per-binding `bot_service_base_url` tracking in `backend/bot-manager-service/src/main.rs`. |
| BOT-029 | Headless Chrome E2E for v3 bot lifecycle and assignment metadata | DONE | DONE | DONE | Playwright headless validation passed: `/tmp/cowboy-v3-bot-concurrency-summary.json`, screenshot `/tmp/cowboy-v3-bot-concurrency-headless.png`. |
| BOT-030 | Per-player model config in same bot-service instance (`base_url`, `model`, `api_key`) via bot-manager YAML and create-bot payload | DONE | DONE | DONE | Added YAML loader in `bot-manager-service` (`BOT_MANAGER_LLM_CONFIG_PATH`), forwarded config in create-bot API, wired bot-service/player-agent per-player LLM fields; verified by headless E2E `/tmp/cowboy-v3-bot-llm-summary.json` + `/tmp/cowboy-v3-bot-llm-headless.png`. |
| BOT-031 | Persist bot lifecycle/state in DynamoDB `bot_players` table (`game_id`, `player_id`, `model`, `base_url`, `api_key`, timestamps) and update `player_state`/`game_state` over lifecycle | DONE | DONE | DONE | Added table + bot-manager DynamoDB integration, with create/update on assign/start/stop/game-finished. Verified via `/tmp/bot_players_describe.json`, `/tmp/bot_players_game_query_running.json`, `/tmp/bot_players_game_query_stopped.json`, `/tmp/bot_players_after_finish_lower.json`, and headless UI run `/tmp/bot_players_headless_e2e.json` + `/tmp/cowboy-bot-players-headless.png`. |

## Update Rules

When a task progresses, update columns in this order:
1. Set `Coding` to `DONE` after implementation is complete.
2. Set `Testing` to `DONE` after relevant automated/manual tests pass.
3. Set `Finished` to `DONE` only when coding + testing are done and verified.
