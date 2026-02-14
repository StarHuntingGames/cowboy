# Cowboy V2 Remaining TODO

Last updated: 2026-02-10

Status fields:
- `Done`: implementation completed in code
- `Tested`: automated/manual test executed and passed
- `Verified`: behavior confirmed end-to-end (including frontend-visible effect where applicable)

| ID | Task | Done | Tested | Verified | Notes |
| --- | --- | --- | --- | --- | --- |
| T1 | Timer Service: consume applied step events and maintain per-turn timers (start/reset/cancel) | [x] | [x] | [x] | 2026-02-09: implemented in `backend/timer-service/src/main.rs`; `cargo test -p timer-service` passed; headless run observed turn advancement by timeout (`/tmp/cowboy-e2e-timeout-headless.png`). |
| T2 | Timer Service: publish `TIMEOUT` command to per-game input Kafka topic when timer expires | [x] | [x] | [x] | 2026-02-09: timeout command producer implemented; validated by `TIMEOUT_APPLIED` events with `source:\"timer\"` in `/tmp/e2e-output-topic.log`. |
| T3 | Web Service: replace `LoggingPublisher` with real Kafka producer to the game input topic | [x] | [x] | [x] | 2026-02-09: `KafkaCommandPublisher` added in `backend/web-service/src/main.rs`; `cargo test -p web-service` passed; commands accepted and processed in headless E2E. |
| T4 | Game Service: consume commands from per-game input Kafka topic instead of HTTP-only apply flow | [x] | [x] | [x] | 2026-02-09: Kafka regex consumer (`game.commands.<game_id>.v1`) implemented in `backend/game-service/src/main.rs`; `cargo test -p game-service` passed (0 tests, build/check); E2E command path uses Kafka end-to-end. |
| T5 | Game Service: publish applied/ignored step events to per-game output Kafka topic | [x] | [x] | [x] | 2026-02-09: per-game output producer and `publish_and_persist` added; verified in `/tmp/e2e-output-topic.log` for both applied and ignored outcomes. |
| T6 | Game Service: implement timeout/late-command behavior (`timeout` advances turn; late user command ignored but still recorded) | [x] | [x] | [x] | 2026-02-09: timeout and stale/late handling implemented (`ResultStatus::TimeoutApplied` / `IgnoredTimeout`); verified by late command id `late-e2e-1770650727781` in output log + Dynamo record. |
| T7 | Persistence: record consumed commands and resulting step records for both applied and ignored outcomes | [x] | [x] | [x] | 2026-02-09: DynamoDB step persistence implemented in `persist_step_record`; verified `IGNORED_TIMEOUT` in `/tmp/dynamo-late-cmd.json` and multiple `TIMEOUT_APPLIED` in `/tmp/dynamo-game-steps-e2e.json`. |
| T8 | Game Manager Service: actually publish `GAME_STARTED` event (currently prepared/logged only) | [x] | [x] | [x] | 2026-02-09: `start_game_handler` now publishes `GAME_STARTED` to per-game output topic; `cargo test -p game-manager-service` passed including `start_game_publishes_game_started_event_to_output_topic`. |
| T9 | Game Watcher Service: consume per-game output topics (or routed stream) instead of fixed shared `game.steps.v1` | [x] | [x] | [x] | 2026-02-09: watcher Kafka consumer now subscribes `game.output.*.v1`; broadcasts `TIMEOUT`/`GAME_FINISHED` over websocket; frontend timeout/finish animations confirmed in headless screenshots. |
| T10 | E2E tests: timeout flow, late-command ignored-but-recorded, watcher reconnect/snapshot sync | [x] | [x] | [x] | 2026-02-09: verified via headless artifacts `/tmp/cowboy-e2e-timeout-summary.json`, `/tmp/cowboy-e2e-full-summary.json`, `/tmp/cowboy-e2e-finish-summary.json` and screenshots in `/tmp/`. |
| T11 | Add `speak` command end-to-end (frontend -> web/input Kafka -> game-service -> output Kafka -> watcher websocket -> frontend log) | [x] | [x] | [x] | 2026-02-10: implemented `speak` + `speak_text` in shared schemas and services; headless Playwright passed (`/tmp/cowboy-e2e-speak-summary.json`, `/tmp/cowboy-e2e-speak-headless.png`); verified Kafka input/output logs (`/tmp/speak-input-topic.log`, `/tmp/speak-output-topic.log`) and Dynamo persistence (`/tmp/speak-dynamo-game.json`). |
| T12 | Player identity refactor: frontend shows names `A/B/C/D`, backend uses UUID `player_id`, and `New`/`Start` control order+style updated | [x] | [x] | [x] | 2026-02-10: `PlayerId` changed to UUID string, per-player names exposed, create/start UI updated (`New` before `Start`, same primary style), and command submission verified with UUID player id in headless Playwright (`/tmp/cowboy-e2e-player-uuid-summary.json`, `/tmp/cowboy-e2e-player-uuid-headless.png`). |

## Update Rules

When updating a task:
1. Set `Done` to `[x]` only after code is implemented.
2. Set `Tested` to `[x]` only after tests are run and pass.
3. Set `Verified` to `[x]` only after end-to-end behavior is confirmed.
4. Add a short dated note in the task `Notes` cell describing what was validated.
