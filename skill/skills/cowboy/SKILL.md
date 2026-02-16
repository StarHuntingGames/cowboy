---
name: cowboy
description: Use when the user wants to play a Cowboy game, join a game as a player, or have Claude autonomously play the Cowboy turn-based game. Triggers on "play cowboy", "join game", "play as player", "cowboy game", "bind to player", "play game".
version: 1.2.0
---

# Cowboy Autonomous Player

This skill enables you to autonomously play the Cowboy turn-based multiplayer game by connecting directly to the backend server via WebSocket and HTTP. No MCP service needed.

## Quick Start

The user provides: **game_id**, **player_name** (A/B/C/D), and optionally a **server_url** (default: `http://localhost:8000`).

You then enter an autonomous game loop: a persistent WebSocket listener runs in the background, you wait for your turn, analyze the board, submit the best action via HTTP, repeat until the game ends.

## Initialization

Before entering the game loop, resolve the player_id:

```bash
curl -s "$SERVER/v2/games/$GAME_ID" | jq -r ".state.players[] | select(.player_name==\"$PLAYER\") | .player_id"
```

Store `GAME_ID`, `PLAYER_ID`, and `SERVER` for all subsequent calls. Verify the player_id is not empty. If empty, tell the user the player was not found.

## Game Loop

The game loop uses a **persistent background process** for WebSocket listening. This avoids spawning a new Python process each turn.

### Setup: Start the WebSocket Listener

Run this **once** at the start of the game, using `run_in_background=true`:

```bash
python3 skill/scripts/wait_for_turn.py --loop "$SERVER" "$GAME_ID" "$PLAYER_ID"
```

The script creates `/tmp/cowboy_<GAME_ID>/` and stays connected to the WebSocket. Each time it's your turn, it writes a snapshot file. When the game ends, it writes `game_over.json` and exits.

Initialize your turn counter: `TURN_COUNTER=1`

### Step 1: Wait For My Turn

Block until the next turn snapshot is ready:

```bash
python3 skill/scripts/wait_for_turn.py --wait-for "$GAME_ID" $TURN_COUNTER
```

This blocks until `/tmp/cowboy_<GAME_ID>/my_turn_<N>.json` or `game_over.json` appears, then prints its content as JSON to stdout.

Parse the output:
- If `"status": "FINISHED"` → announce the game result and stop.
- If `"error"` field exists → the listener may have died; restart it.
- Otherwise → it's your turn. Proceed to Step 2.

### Step 2: Analyze Board & Choose Action

Parse the snapshot JSON to extract:
- `turn_no` — current turn number
- `state.map.cells` — 2D grid array
- `state.map.rows`, `state.map.cols` — grid dimensions
- `state.players[]` — each player's `row`, `col`, `shield`, `hp`, `alive`, `player_id`, `player_name`

Then follow the **Decision Framework** below to choose your action.

### Step 3: Submit Command

First, generate a UUID and timestamp:

```bash
python3 -c "import uuid, datetime; print(uuid.uuid4()); print(datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.%f')[:-3]+'Z')"
```

This prints two lines: the UUID (line 1) and the timestamp (line 2). Use them in the curl command:

```bash
curl -s -X POST "$SERVER/v2/games/$GAME_ID/commands" \
  -H "Content-Type: application/json" \
  -d '{
    "command_id": "<UUID>",
    "player_id": "<PLAYER_ID>",
    "command_type": "<move|shoot|shield>",
    "direction": "<up|down|left|right>",
    "speak_text": null,
    "turn_no": <TURN_NO>,
    "client_sent_at": "<TIMESTAMP>"
  }'
```

After submitting, increment `TURN_COUNTER` and go back to Step 1.

### Cleanup

When the game ends (FINISHED status), the background listener exits automatically. If you need to stop early:

```bash
kill $(cat /tmp/cowboy_<GAME_ID>/pid) 2>/dev/null
```

## Board Visualization

Each turn, display the board to the user. Render the grid as ASCII art:

- `.` = empty cell (value 0)
- `#` = indestructible wall (value -1)
- `1`-`9` = destructible block (value = HP remaining)
- `A`, `B`, `C`, `D` = player positions (use lowercase `a`-`d` if dead)
- Add a legend showing each player's HP, shield direction, and alive status

Example output:
```
Turn 5 | Your turn (Player B) | HP: 3 | Shield: right

  0 1 2 3 4 5 6 7 8 9 10
0 # # # # # # # # # # #
1 # . . . . . . . . . #
2 # . . . A↑. . . . . #
3 # . . . . . . . . . #
4 # . . . . 2 . . . . #
5 # . . B→. . . . C↓. #
6 # . . . . . . . . . #
7 # . . . . . . . . . #
8 # . . . . . . D←. . #
9 # . . . . . . . . . #
10 # # # # # # # # # # #

Players:
  A: (2,4) HP=3 shield=up   ALIVE
  B: (5,3) HP=3 shield=right ALIVE  ← YOU
  C: (5,8) HP=2 shield=down  ALIVE
  D: (8,7) HP=3 shield=left  ALIVE
```

Use arrow indicators for shield direction: `↑` up, `↓` down, `←` left, `→` right.

## T-Shaped Shooting — Critical Mechanic

**Shooting is NOT a straight line.** It forms a T-shape:

1. The laser enters the **adjacent cell** in the shoot direction.
2. From that cell, the laser sweeps **BOTH perpendicular directions**.
3. Each sweep travels until it hits a wall, block, or player.

```
Example: shoot right (player at X)

        ↑ sweep up
        |
  X --- L --- (laser enters L, then sweeps up AND down)
        |
        ↓ sweep down
```

**You CANNOT hit someone directly in your line of sight. The sweep is perpendicular.**

### Checking if a Shot is Valid

For each potential shoot direction (up/down/left/right):

1. **Shield conflict?** — Cannot shoot in the same direction as your current shield. Skip.
2. **Adjacent cell check** — The cell one step in the shoot direction must be:
   - In bounds (not off the grid edge)
   - Empty (cell value == 0)
   - Unoccupied (no player standing there)
   - If ANY of these fail, this shoot direction is invalid. Skip.
3. **Trace perpendicular sweeps** — From the adjacent cell, trace both perpendicular directions:
   - `shoot up/down` → sweep left AND right
   - `shoot left/right` → sweep up AND down
   - For each sweep direction, move cell-by-cell:
     - If cell is out of bounds → sweep stops (no hit)
     - If cell value != 0 (wall/block) → sweep stops (hits block, not player)
     - If cell has an alive enemy player → **HIT!** Check shield.
4. **Shield blocking** — A hit is blocked if the enemy's shield faces OPPOSITE to the sweep direction:
   - Sweep going up → shield `down` blocks
   - Sweep going down → shield `up` blocks
   - Sweep going left → shield `right` blocks
   - Sweep going right → shield `left` blocks

A shot is **effective** if at least one hit is NOT blocked by a shield.

## Decision Framework

Follow this priority order strictly. Take the first action that applies:

### Priority 1: Shoot (T-shaped hit available)

Check all 4 shoot directions from your current position. If any produces an unblocked hit, take the shot with the most unblocked hits.

**Show your analysis:** For each valid shoot direction, list the laser cell, perpendicular sweeps, and any enemies hit (blocked or unblocked).

### Priority 2: Change Shield to Enable Shot

If no shot is available because your shield blocks the only valid shoot direction:
- For each alternative shield direction, re-check if that shield would allow a T-shaped shot.
- If yes, change your shield this turn (you can shoot next turn).

### Priority 3: Move to Create Shot Opportunity

For each passable move direction (prefer directions toward the nearest enemy):
- Calculate the new position after moving.
- From that new position, check if any T-shaped shots would be available (using your current shield).
- If a move creates a shot opportunity, take it.

### Priority 4: Close Distance

If no shot or positioning move is available:
- Calculate Manhattan distance to the nearest alive enemy.
- Move in the direction that reduces this distance the most.
- Only move if the distance actually decreases.

### Priority 5: Defensive Shield

If no useful move is available (all directions blocked):
- Point your shield toward the nearest enemy.
- To find "toward": if the enemy is more rows away, shield up/down. If more cols away, shield left/right.

## Coordinate System

- `up` = row - 1 (toward row 0)
- `down` = row + 1 (toward max row)
- `left` = col - 1 (toward col 0)
- `right` = col + 1 (toward max col)
- Direction vectors: up=(-1,0), down=(1,0), left=(0,-1), right=(0,1)

## Communication Protocol

Each turn, show the user:
1. The board visualization
2. Your analysis (what shots are available, distances to enemies)
3. Your chosen action and reasoning
4. The server's response to your command

Keep it concise but informative. The user should understand WHY you chose each action.

## Error Handling

- If a command submission fails (HTTP error), retry once. If it fails again, show the error and continue to the next turn.
- If the `--wait-for` returns an error (listener died), restart the background listener with `--loop` and retry.
- If the game status changes to FINISHED mid-loop, announce the result immediately.
- If the player is eliminated (hp=0 or alive=false), announce it and stop.

## API Reference

### GET /v2/games/{game_id}
Returns game info including players and their IDs.

### GET /v2/games/{game_id}/snapshot?from_turn_no=1
Returns the latest game state snapshot:
```json
{
  "status": "RUNNING",
  "turn_no": 5,
  "current_player_id": "uuid-here",
  "state": {
    "map": {
      "rows": 11,
      "cols": 11,
      "cells": [[0, -1, ...], ...]
    },
    "players": [
      {
        "player_id": "uuid",
        "player_name": "A",
        "row": 2,
        "col": 4,
        "hp": 3,
        "shield": "up",
        "alive": true
      }
    ]
  }
}
```

### WebSocket /v2/games/{game_id}/stream?from_turn_no=1
Real-time event stream. Messages are JSON with structure:
```json
{
  "event_type": "STEP_APPLIED",
  "snapshot": {
    "status": "RUNNING",
    "turn_no": 5,
    "current_player_id": "uuid",
    "state": { "map": {...}, "players": [...] }
  }
}
```
Event types: `CONNECTED`, `SNAPSHOT`, `GAME_STARTED`, `STEP_APPLIED`, `GAME_FINISHED`.

### POST /v2/games/{game_id}/commands
Submit a player command. Body:
```json
{
  "command_id": "new-uuid",
  "player_id": "your-player-id",
  "command_type": "move|shoot|shield|speak",
  "direction": "up|down|left|right",
  "speak_text": null,
  "turn_no": 5,
  "client_sent_at": "2026-01-01T00:00:00.000Z"
}
```

## Script Dependencies

The `wait_for_turn.py` script requires the `websockets` package for instant turn detection. Install with:

```bash
pip install websockets
```

If `websockets` is not installed, the script automatically falls back to HTTP polling (2-second intervals).

## Script Modes Reference

| Mode | Command | Purpose |
|---|---|---|
| Single-shot | `python3 wait_for_turn.py <server> <game_id> <player_id> [timeout]` | Wait for one turn, print snapshot, exit |
| Loop | `python3 wait_for_turn.py --loop <server> <game_id> <player_id>` | Background process, writes turn files |
| Wait-for | `python3 wait_for_turn.py --wait-for <game_id> <N> [timeout]` | Block until turn N file or game_over exists |
