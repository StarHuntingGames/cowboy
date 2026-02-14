# Cowboy Game Rules — Quick Reference for AI Players

## Map

- Grid of rows x cols (typically 11x11).
- Cell values: `0` = empty/passable, `-1` = indestructible wall, `N > 0` = destructible block (HP = N).
- Players: A, B, C, D. Last player alive wins.

## Actions

Each turn, a player chooses exactly one action:

### move \<direction\>
- Move exactly 1 cell in the given direction (up/down/left/right).
- **Illegal if** the target cell is: out of bounds, a wall/block (cell != 0), or occupied by another player.

### shield \<direction\>
- Set your shield to face the given direction.
- Shields block incoming laser sweeps (see Shield Blocking below).

### shoot \<direction\>

> **!!! CRITICAL: THE SHOOTING IS T-SHAPED, NOT A STRAIGHT LINE !!!**

- The laser enters the **adjacent cell** in the chosen direction, then **sweeps BOTH perpendicular directions** from that cell, forming a **T-shape**.
- **`shoot right`** → laser goes to the cell immediately to your right, then sweeps **up** and **down** from that cell. NOT further right!
- **`shoot up`** → laser goes to the cell immediately above you, then sweeps **left** and **right** from that cell. NOT further up!
- **`shoot left`** → laser goes to the cell immediately to your left, then sweeps **up** and **down** from that cell.
- **`shoot down`** → laser goes to the cell immediately below you, then sweeps **left** and **right** from that cell.
- Each perpendicular sweep travels until it hits a wall or a player.

> **You CANNOT hit a player by shooting directly at them in a straight line.**
> **You MUST position so the PERPENDICULAR SWEEP reaches them.**

- **Illegal if:**
  - The adjacent cell in the chosen direction is out of bounds, a wall, or occupied by another player.
  - You are shooting in the same direction as your current shield.

> **!!! ABSOLUTE RULE: YOU CANNOT SHOOT TOWARD AN OCCUPIED ADJACENT CELL !!!**
>
> If someone (or a wall/block) is in your adjacent cell, you **CANNOT** shoot in that direction. The adjacent cell MUST be completely empty for the laser to enter it.
>
> - Player next to you on the UP side? **`shoot up` is ILLEGAL.**
> - Player next to you on the DOWN side? **`shoot down` is ILLEGAL.**
> - Player next to you on the LEFT side? **`shoot left` is ILLEGAL.**
> - Player next to you on the RIGHT side? **`shoot right` is ILLEGAL.**
>
> **You must pick a different shoot direction or reposition first.**

### speak \<text\>
- Send a chat message (max 140 chars). No direction needed.

## Damage

- Each perpendicular sweep can hit **one target** (wall or player) independently.
- If a sweep reaches a player and the player's shield blocks it (see below), no damage.
- Otherwise, the target loses **1 HP**.
- HP 0 = eliminated.
- Destructible blocks (cell value > 0) decrease by 1 when hit by a sweep.

## Shield Blocking

The shield blocks damage when it faces the **source direction** — the direction the sweep is coming FROM.

- If a sweep is traveling **upward** (coming from below), the player's shield must face **down** to block it.
- If a sweep is traveling **downward** (coming from above), the player's shield must face **up** to block it.
- If a sweep is traveling **to the right** (coming from the left), the player's shield must face **left** to block it.
- If a sweep is traveling **to the left** (coming from the right), the player's shield must face **right** to block it.

**In other words: shield direction must be OPPOSITE to the sweep's travel direction to block. Face your shield TOWARD where the attack comes from.**

## Shooting Strategy (T-Shaped Positioning)

Since **shooting is T-shaped**, optimal positioning requires understanding perpendicular geometry:

- To hit a target, you do NOT aim directly at them. Instead, you position so the **perpendicular sweep** from the laser's entry point passes through the target's cell.
- Example: If enemy is at (3, 5):
  - Standing at (3, 4) and `shoot right` → laser at (3, 5) — **INVALID** (cell occupied by enemy).
  - Standing at (2, 5) and `shoot down` → laser at (3, 5) — **INVALID** (cell occupied by enemy).
  - Standing at (4, 4) and `shoot up` → laser at (3, 4), sweeps right → hits enemy at (3, 5). **VALID!**
  - Standing at (4, 6) and `shoot up` → laser at (3, 6), sweeps left → hits enemy at (3, 5). **VALID!**
  - Standing at (2, 4) and `shoot down` → laser at (3, 4), sweeps right → hits enemy at (3, 5). **VALID!**

**Key insight: Position yourself one row/column offset from the target, then shoot perpendicular to reach them with the sweep.**

## Direction Reference

- `up` = row decreases (toward row 0)
- `down` = row increases (toward max row)
- `left` = col decreases (toward col 0)
- `right` = col increases (toward max col)
