# Cow Boy Game Rules

## Goal
Be the **last alive player**.

## Players and Map
- The game supports 1–4 players (configurable at game creation, default: 2):
  - Player A (Up)
  - Player B (Left)
  - Player C (Down)
  - Player D (Right)
- Player A is human-controlled; the rest are AI bots.
- The map is a grid with:
  - `empty` cells (players can move through)
  - `block` cells (players cannot move through)
- Starting positions are the center cells of each map side (up, left, down, right).

## Turn Order
- The game is turn-based.
- Fixed turn order:
  - **Up -> Left -> Down -> Right**
- One action per player per turn.
- After each player acts once, a new round starts.

## Player Stats
- Every player has:
  - HP (default: `10`)
  - One shield with a facing direction: `up`, `left`, `down`, or `right`
  - One laser gun

## Actions (one per turn)
- `move`
- `shield`
- `shoot`

Each action also uses one direction: `up`, `left`, `down`, `right`.

## Action Rules

### 1) Move
- Move exactly one cell in the chosen direction.
- Move fails if:
  - the target cell is out of map bounds
  - the target cell has a block
  - the target cell is occupied by another player
- Failed move does not consume the turn.

### 2) Move Shield
- Change shield facing to one of:
  - `up`, `left`, `down`, `right`
- This consumes the turn.

### 3) Shoot
- Shooter fires in a straight line (row or column) in the chosen direction.
- Laser stops at the first blocker in line:
  - first block, or
  - first player
- One shot can affect at most one target.
- A player **cannot shoot in the same direction as their own shield**.
  - This is invalid and does not consume the turn.

## Shield and Damage Rules
- If a shot reaches a player:
  - The game checks the target shield direction.
  - If shield faces the incoming side, the shot is blocked and no HP is lost.
  - Otherwise, target loses `1` HP.
- At `0` HP, player is eliminated.

## Block Strength Rules
- Blocks have strength values:
  - `-1` means indestructible (can block forever)
  - `N > 0` means destructible
- Each time a destructible block is shot:
  - strength decreases by 1
  - when strength reaches 0, the block is destroyed and the cell becomes empty

## Win Condition
- The match ends when only one player is alive.
- That player wins.

## How to Play in This UI
- Open the game at `http://localhost:8000`.
- Click **New** to create a game, then **Start** to begin.
- On your turn (Player A), choose a command and direction, then click **Execute**.
- Bot players (B/C/D) act automatically via LLM-powered AI.

