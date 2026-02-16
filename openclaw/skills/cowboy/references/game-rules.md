# Cowboy Game Rules — Complete Reference for AI Players

## Overview

Cowboy is a turn-based multiplayer game on a grid map. 1–4 players (A/B/C/D) compete. Last player alive wins. Each turn, you choose exactly ONE action.

## Map

- Grid of rows x cols (typically 11x11).
- Cell values:
  - `0` = empty, passable
  - `-1` = indestructible wall (cannot be destroyed or passed)
  - `N > 0` = destructible block with N HP (blocks movement and laser sweeps; loses 1 HP per hit; becomes empty when HP reaches 0)
- Players occupy cells but do not change cell values.
- Borders are typically walls (`-1`).

## Coordinate System

- **row** = vertical position. Row 0 is the top.
- **col** = horizontal position. Col 0 is the left.
- `up` = row - 1 (toward row 0)
- `down` = row + 1 (toward max row)
- `left` = col - 1 (toward col 0)
- `right` = col + 1 (toward max col)
- Direction vectors: up=(-1,0), down=(1,0), left=(0,-1), right=(0,1)

## Actions

### move \<direction\>

Move exactly 1 cell in the given direction.

**Illegal if the target cell is:**
- Out of bounds (off the grid edge)
- A wall or block (cell value != 0)
- Occupied by another player (alive or dead)

### shield \<direction\>

Set your shield to face the given direction. Your shield stays in this direction until you change it.

**No restrictions** — you can shield in any direction at any time.

### shoot \<direction\>

> **!!! CRITICAL: SHOOTING IS T-SHAPED, NOT A STRAIGHT LINE !!!**

The laser does NOT travel in a straight line. It forms a **T-shape**:

1. The laser enters the **adjacent cell** in the chosen direction (one step only).
2. From that cell, the laser sweeps **BOTH perpendicular directions** simultaneously.
3. Each perpendicular sweep travels cell-by-cell until it hits a wall, block, or player.

**Perpendicular sweep directions for each shoot direction:**
- `shoot up` → laser at (row-1, col), sweeps **left** AND **right**
- `shoot down` → laser at (row+1, col), sweeps **left** AND **right**
- `shoot left` → laser at (row, col-1), sweeps **up** AND **down**
- `shoot right` → laser at (row, col+1), sweeps **up** AND **down**

```
Example: Player at X shoots right

           ↑ sweep up (travels upward from L)
           |
  X ----→ L  (laser enters L = one cell right of X)
           |
           ↓ sweep down (travels downward from L)

The laser does NOT continue right past L.
The sweeps go perpendicular (up and down) from L.
```

```
Example: Player at X shoots up

                 L  (laser enters L = one cell above X)
                / \
  sweep left ←     → sweep right
                 |
                 X

The laser does NOT continue upward past L.
The sweeps go perpendicular (left and right) from L.
```

> **You CANNOT hit a player by shooting directly at them in a straight line.**
> **You MUST position so the PERPENDICULAR SWEEP reaches them.**

**Shoot is ILLEGAL if:**
- The adjacent cell in the chosen direction is **out of bounds**
- The adjacent cell is a **wall or block** (cell value != 0)
- The adjacent cell is **occupied by another player**
- You are shooting in the **same direction as your current shield**

> **!!! ABSOLUTE RULE: YOU CANNOT SHOOT TOWARD AN OCCUPIED ADJACENT CELL !!!**
>
> The adjacent cell MUST be completely empty (cell value == 0, no player standing there) for the laser to enter it.
>
> - Player directly above you? → `shoot up` is **ILLEGAL**
> - Player directly below you? → `shoot down` is **ILLEGAL**
> - Player directly left of you? → `shoot left` is **ILLEGAL**
> - Player directly right of you? → `shoot right` is **ILLEGAL**
> - Wall/block next to you? → Same rule applies. Cannot shoot into it.
>
> **You must pick a different shoot direction or reposition first.**

### speak \<text\>

Send a chat message (max 140 chars). No direction needed. Does not end your turn in the sense that it's a free action — but it IS your action for the turn.

## Damage

- Each perpendicular sweep can hit **one target** independently.
- A sweep stops at the first wall, block, or player it encounters.
- If the target is a **player**:
  - Check shield blocking (see below)
  - If NOT blocked: player loses **1 HP**
  - If blocked: no damage
- If the target is a **destructible block** (cell value > 0):
  - Block loses 1 HP (cell value decreases by 1)
  - When cell value reaches 0, the block is destroyed and becomes empty
- **HP 0 = eliminated.** Dead players are removed from the game.
- A single shoot action can deal damage to **2 targets** (one per perpendicular sweep), or to **1 target** hit by both sweeps (2 damage), or to **0 targets** if both sweeps miss.

## Shield Blocking

The shield blocks a sweep when it faces **OPPOSITE to the sweep's travel direction** — i.e., TOWARD the direction the attack is coming FROM.

| Sweep traveling... | Shield direction that blocks |
|---|---|
| Upward (from below) | `down` (facing toward below, where attack comes from) |
| Downward (from above) | `up` (facing toward above, where attack comes from) |
| Leftward (from right) | `right` (facing toward right, where attack comes from) |
| Rightward (from left) | `left` (facing toward left, where attack comes from) |

**Key rule: Shield direction = OPPOSITE of sweep travel direction.**

A shield pointing `up` does NOT block upward sweeps. It blocks **downward** sweeps (attacks coming from above).

## T-Shaped Shooting Strategy

### How to position for a shot

Since shooting is T-shaped, you need **perpendicular positioning**:

**To hit an enemy, you must be offset by exactly 1 row OR 1 column**, then shoot so the perpendicular sweep passes through their cell.

### Worked examples

**Enemy at (3, 5). Where can you stand to hit them?**

```
Option 1: Stand at (4, 4), shoot up
  → Laser enters (3, 4)
  → Sweep right from (3, 4): (3,5) = ENEMY HIT!
  → Sweep left from (3, 4): travels left until wall

Option 2: Stand at (4, 6), shoot up
  → Laser enters (3, 6)
  → Sweep left from (3, 6): (3,5) = ENEMY HIT!
  → Sweep right from (3, 6): travels right until wall

Option 3: Stand at (2, 4), shoot down
  → Laser enters (3, 4)
  → Sweep right from (3, 4): (3,5) = ENEMY HIT!
  → Sweep left from (3, 4): travels left until wall

Option 4: Stand at (2, 6), shoot down
  → Laser enters (3, 6)
  → Sweep left from (3, 6): (3,5) = ENEMY HIT!

Option 5: Stand at (4, 5), shoot up
  → Laser enters (3, 5) — INVALID! Cell is occupied by enemy.

Option 6: Stand at (3, 4), shoot right
  → Laser enters (3, 5) — INVALID! Cell is occupied by enemy.

Option 7: Stand at (3, 6), shoot left
  → Laser enters (3, 5) — INVALID! Cell is occupied by enemy.
```

**Pattern:** To hit someone at (r, c):
- Stand at (r+1, c-N) or (r-1, c-N), shoot up/down → sweep right hits them (if no obstacles between laser cell and enemy)
- Stand at (r+1, c+N) or (r-1, c+N), shoot up/down → sweep left hits them
- Stand at (r-N, c+1) or (r-N, c-1), shoot left/right → sweep down hits them
- Stand at (r+N, c+1) or (r+N, c-1), shoot left/right → sweep up hits them

Where N >= 1, and the laser's adjacent cell must be empty, and the sweep path must be clear.

### The "ideal" shooting position

The best position is **diagonally adjacent** to the enemy (offset by 1 row AND 1 col):
- Standing at (r±1, c±1) gives you a shoot direction where the laser enters at (r, c±1) or (r±1, c), and the perpendicular sweep has only 1 cell to travel to hit the enemy.
- This minimizes the chance of obstacles blocking the sweep.

### Double-hit opportunity

If two enemies are on the same row (or same column), you can position so both perpendicular sweeps hit different enemies:
- Enemies at (3, 2) and (3, 8). Stand at (4, 5), shoot up → laser at (3, 5), sweep left hits (3, 2), sweep right hits (3, 8). **Two hits with one shot!**

## Shot Validation Checklist

Before shooting, verify ALL of the following:

1. **Shield conflict?** Cannot shoot in the same direction as your shield. If shield=right, cannot shoot right.
2. **Adjacent cell in bounds?** The cell one step in the shoot direction must be within the grid.
3. **Adjacent cell empty?** Cell value must be 0 (not wall, not block).
4. **Adjacent cell unoccupied?** No player (alive) standing in that cell.
5. **Perpendicular sweeps — trace each one:**
   - From the laser cell, move cell-by-cell in each perpendicular direction
   - If cell is out of bounds → sweep stops, no hit
   - If cell value != 0 (wall/block) → sweep stops, hits the block (not a player)
   - If cell has an alive enemy → **HIT!** Check shield blocking.
6. **Shield check:** Is the hit blocked? (shield OPPOSITE to sweep direction = blocked)

## Tactical Tips

- **Prefer guaranteed damage.** If you can shoot and hit an unshielded enemy, do it.
- **Diagonal positioning is key.** Being diagonally adjacent to an enemy gives the shortest sweep path.
- **Shield toward the most likely attack.** If enemies are mostly to your right, shield left (to block rightward sweeps coming from the left... wait, no). Think about WHERE the sweep will come from relative to you, then point shield OPPOSITE to the sweep's direction.
- **Avoid wasting moves.** If no shot is available, reposition toward a shooting position rather than shielding randomly.
- **Break destructible blocks** that are blocking your sweep paths. Shoot through them to clear the way.
- **Mind the T-shape at all times.** The most common mistake is thinking you can hit someone in a straight line. You cannot. The sweep is always perpendicular.
