# Cow Boy Test Cases

Last updated: 2026-02-14

## V1 Frontend Gameplay (Manual)

### TC-V1-001: Start match and initial turn
- Preconditions: Open `http://localhost:8000`.
- Steps:
1. Click `Start`.
- Expected:
1. Turn panel shows active player `Player A (Up)`.
2. `Execute` button is enabled.
3. Log shows match started with turn order `Up -> Left -> Down -> Right`.

### TC-V1-002: Move action
- Preconditions: Match started, active player A.
- Steps:
1. Select command `move`.
2. Select direction `down`.
3. Click `Execute`.
- Expected:
1. Player A moves one tile down if target tile is empty.
2. Active player changes to Player B.
3. Log contains move result.

### TC-V1-003: Move blocked by block tile
- Preconditions: Match started; active player has a block adjacent in selected direction.
- Steps:
1. Select command `move`.
2. Select blocked direction.
3. Click `Execute`.
- Expected:
1. Player stays in same tile.
2. Log explains move is blocked.
3. Turn still advances.

### TC-V1-004: Shoot destroys finite-strength block
- Preconditions: Shooter has line-of-sight to a block with strength `1` or `2`.
- Steps:
1. Select command `shoot`.
2. Select direction toward that block.
3. Click `Execute`.
- Expected:
1. Laser stops at first blocker in line.
2. Block strength decreases; if it reaches 0, block is removed.
3. Log records block hit/destroyed.

### TC-V1-005: Shoot blocked by infinite-strength block
- Preconditions: Shooter has line-of-sight to `-1` block.
- Steps:
1. Select command `shoot`.
2. Select direction toward `-1` block.
3. Click `Execute`.
- Expected:
1. Laser stops at the `-1` block.
2. Block remains unchanged.
3. No player behind that block takes damage.

### TC-V1-006: Shield move and protected shot
- Preconditions: Target player exists in same row or column.
- Steps:
1. On target turn, set shield direction to face attacker and execute `shield`.
2. On attacker turn, execute `shoot` toward target.
- Expected:
1. Protected target does not lose HP.
2. No shake/sound effect for protected target.
3. Log reports shield protection.

### TC-V1-007: Self-shield shooting rule
- Preconditions: Active player shield is facing one direction.
- Steps:
1. Select `shoot` in the same direction as own shield.
2. Click `Execute`.
- Expected:
1. Shot is rejected by game rule.
2. No laser damage is applied.
3. Log explains invalid shoot direction.

### TC-V1-008: Turn order and round increment
- Preconditions: Match started.
- Steps:
1. Execute one action for A, then B, then C, then D.
- Expected:
1. Turn order remains `A -> B -> C -> D`.
2. After D action, round increments by 1 and next active player is A.

### TC-V1-009: Last alive wins
- Preconditions: Match in progress with low HP players.
- Steps:
1. Continue actions until only one player remains alive.
- Expected:
1. Game status changes to finished.
2. Winner is announced in log/turn panel.
3. Further execute actions are prevented.

## V2 Backend APIs (Automated/Manual)

### TC-V2-001: Create game with default map
- Endpoint: `POST /v2/games`
- Input: `{ "turn_timeout_seconds": 10 }`
- Expected:
1. `200` response with `status=CREATED`.
2. `map_source=DEFAULT`.
3. `turn_no=1`, `round_no=1`, `current_player_id=up`.

### TC-V2-002: Create game with custom map
- Endpoint: `POST /v2/games`
- Input: includes `map`.
- Expected:
1. `200` response with `map_source=CUSTOM`.
2. Stored game state map matches input dimensions/content.

### TC-V2-003: Start game
- Endpoint: `POST /v2/games/{game_id}/start`
- Expected:
1. First call sets `status=RUNNING`, `started=true`.
2. Second call returns `started=false`, reason `ALREADY_RUNNING`.

### TC-V2-004: Submit command validation
- Endpoint: `POST /v2/games/{game_id}/commands`
- Input variants:
1. `command_type=timeout` (invalid from user).
2. `move/shield/shoot` without `direction`.
- Expected:
1. `400` for invalid user command type.
2. `400` when required direction is missing.

### TC-V2-005: Submit valid command
- Endpoint: `POST /v2/games/{game_id}/commands`
- Input: valid `move|shield|shoot` with direction.
- Expected:
1. `200` with `accepted=true`.
2. Response includes same `command_id`.

### TC-V2-006: Watcher health endpoint
- Endpoint: `GET /health` on watcher service.
- Expected:
1. `200` with `{ "ok": true, "service": "game-watcher-service" }`.

### TC-V2-007: Watcher snapshot
- Endpoint: `GET /v2/games/{game_id}/snapshot`
- Expected:
1. `200` with latest map/players snapshot.
2. Includes `status`, `turn_no`, `round_no`, `current_player_id`.
3. If game already ended, `status=FINISHED`.

### TC-V2-008: Finish transition + watcher event
- Preconditions: running game with two players at low HP.
- Steps:
1. Submit a valid `shoot` that eliminates the second-to-last player.
2. Call watcher snapshot endpoint.
3. Observe watcher websocket stream for the same game.
- Expected:
1. Game Service transitions game status to `FINISHED`.
2. Snapshot response returns `status=FINISHED`.
3. Websocket emits `GAME_FINISHED`.
4. Frontend shows winner message and finish animation.

## Current Automated Coverage
- `cowboy-common`:
1. Spawn positions at side centers.
2. Generated map safe spawn tiles.
3. Generated map value domain (`-1,0,1,2`).
4. Built-in default map shape + safe spawn tiles.
- `web-service`:
1. Command validation rules.
2. Successful publish path.
3. Publisher failure path.
- `game-manager-service`:
1. Create game default/custom map flows.
2. Start idempotency behavior.
3. Not-found behavior.
4. Default map retrieval stability.
- `game-watcher-service`:
1. Health payload.
2. Snapshot conversion with `turn_no` cursor semantics.
