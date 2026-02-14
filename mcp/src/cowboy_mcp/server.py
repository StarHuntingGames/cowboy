# Cowboy MCP Server — Main Server & Tool Definitions
# Copyright (C) 2026 StarHuntingGames
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
from typing import Any

from mcp.server.fastmcp import FastMCP

from cowboy_mcp.game_client import GameClient
from cowboy_mcp.session import Session
from cowboy_mcp.ws_client import GameWebSocket

# Route logs to stderr so they don't corrupt stdio JSON-RPC
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(name)s %(levelname)s %(message)s",
    stream=sys.stderr,
)
logger = logging.getLogger("cowboy_mcp")

BASE_URL = os.environ.get("COWBOY_BASE_URL", "http://localhost:8000")

VALID_PLAYER_NAMES = {"A", "B", "C", "D"}
VALID_COMMAND_TYPES = {"move", "shield", "shoot", "speak"}
VALID_DIRECTIONS = {"up", "down", "left", "right"}
DIRECTION_VECTORS = {
    "up": (-1, 0),
    "down": (1, 0),
    "left": (0, -1),
    "right": (0, 1),
}
# Perpendicular sweep directions for each shoot direction (T-shaped mechanic)
_PERP_DIRS: dict[str, list[str]] = {
    "up": ["left", "right"],
    "down": ["left", "right"],
    "left": ["up", "down"],
    "right": ["up", "down"],
}
# Shield direction that blocks a sweep traveling in a given direction
# (shield must face OPPOSITE to sweep travel direction)
_SHIELD_BLOCKS: dict[str, str] = {
    "up": "down",       # sweep going up → shield "down" blocks
    "down": "up",       # sweep going down → shield "up" blocks
    "left": "right",    # sweep going left → shield "right" blocks
    "right": "left",    # sweep going right → shield "left" blocks
}

# Load game rules from GAME_RULES.md
_GAME_RULES_PATH = os.path.join(os.path.dirname(__file__), "..", "..", "GAME_RULES.md")


def _load_game_rules() -> str:
    """Load game rules from GAME_RULES.md file."""
    try:
        with open(_GAME_RULES_PATH) as f:
            return f.read()
    except FileNotFoundError:
        logger.warning("Game rules file not found at %s", _GAME_RULES_PATH)
        return "Game rules file not found."


_GAME_RULES_CONTENT: str = _load_game_rules()


def _bool_env(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.strip().lower() in {"1", "true", "yes", "on"}


AUTO_PLAY_DEFAULT_ON_BIND = _bool_env("COWBOY_MCP_AUTOPLAY_ON_BIND", True)
AUTO_PLAY_WAIT_TIMEOUT_SECONDS = float(
    os.environ.get("COWBOY_MCP_AUTOPLAY_WAIT_TIMEOUT_SECONDS", "120")
)

mcp_server = FastMCP("Cowboy Game Controller")

# Module-level state — single session per process
_client = GameClient(BASE_URL)
_session: Session | None = None
_ws: GameWebSocket | None = None
_autoplay_enabled = False
_autoplay_task: asyncio.Task[None] | None = None
_autoplay_last_decision: dict[str, Any] | None = None


def _require_session() -> Session:
    if _session is None:
        raise ValueError("No active session. Call bind_player first.")
    return _session


def _autoplay_status_payload() -> dict[str, Any]:
    running = _autoplay_task is not None and not _autoplay_task.done()
    payload: dict[str, Any] = {
        "enabled": _autoplay_enabled,
        "running": running,
        "default_on_bind": AUTO_PLAY_DEFAULT_ON_BIND,
        "wait_timeout_seconds": AUTO_PLAY_WAIT_TIMEOUT_SECONDS,
        "last_decision": _autoplay_last_decision,
    }
    if _session is None:
        payload["bound"] = False
    else:
        payload["bound"] = True
        payload["game_id"] = _session.game_id
        payload["player_name"] = _session.player_name
    return payload


def _cell_value(cells: list[list[Any]], row: int, col: int) -> Any:
    try:
        return cells[row][col]
    except Exception:
        return 1


def _get_shield_direction(
    my_row: int, my_col: int, enemy_row: int, enemy_col: int
) -> str:
    delta_row = enemy_row - my_row
    delta_col = enemy_col - my_col
    if abs(delta_row) >= abs(delta_col):
        return "down" if delta_row > 0 else "up"
    return "right" if delta_col > 0 else "left"


def _autoplay_decision(
    command_type: str,
    direction: str | None,
    reason: str,
    thinking_steps: list[str],
    thinking_meta: dict[str, Any] | None = None,
) -> dict[str, Any]:
    action_label = command_type if not direction else f"{command_type} {direction}"
    reason_summary = {
        "t_shaped_shot": "T-shaped sweep hits enemy",
        "shield_change_for_shot": "changing shield to enable T-shaped shot next turn",
        "positioning_for_shot": "moving to create T-shaped shot opportunity",
        "close_distance": "no shot available, closing distance to enemy",
        "defensive_fallback": "no strong move available, so protect against nearest threat",
        "fallback_default": "snapshot data missing, so use a safe default",
    }.get(reason, reason)
    summary = f"Selected `{action_label}` because {reason_summary}."
    payload: dict[str, Any] = {
        "command_type": command_type,
        "direction": direction,
        "speak_text": None,
        "reason": reason,
        "thinking": {
            "strategy": "heuristic_v2_t_shaped",
            "steps": thinking_steps,
        },
        "reasoning": {
            "version": "autoplay_reasoning_v2",
            "summary": summary,
            "selected_action": {
                "command_type": command_type,
                "direction": direction,
            },
            "reason_code": reason,
            "strategy": "heuristic_v2_t_shaped",
            "steps": thinking_steps,
        },
    }
    if thinking_meta:
        payload["thinking"]["meta"] = thinking_meta
        payload["reasoning"]["meta"] = thinking_meta
    return payload


def _passability_check(
    row: int,
    col: int,
    rows: int,
    cols: int,
    cells: list[list[Any]],
    occupied: set[tuple[int, int]],
) -> tuple[bool, str | None]:
    if row < 0 or row >= rows or col < 0 or col >= cols:
        return False, "out_of_bounds"
    if (row, col) in occupied:
        return False, "occupied_by_enemy"
    value = _cell_value(cells, row, col)
    if value != 0:
        return False, f"blocked_cell_value_{value}"
    return True, None


def _find_t_shaped_shots(
    my_row: int,
    my_col: int,
    my_shield: str,
    rows: int,
    cols: int,
    cells: list[list[Any]],
    alive_enemies: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Find all valid T-shaped shots from a position.

    For each shoot direction, checks if the adjacent cell is empty and in bounds,
    then sweeps both perpendicular directions from that cell to find enemy hits.
    Returns a list of shot options with hit details.
    """
    enemy_positions: dict[tuple[int, int], dict[str, Any]] = {
        (int(e.get("row", -1)), int(e.get("col", -1))): e
        for e in alive_enemies
    }

    shots: list[dict[str, Any]] = []
    for shoot_dir, (dr, dc) in DIRECTION_VECTORS.items():
        # Can't shoot same direction as shield
        if shoot_dir == my_shield:
            continue

        # Adjacent cell must be in bounds and empty (value 0, no player)
        adj_r = my_row + dr
        adj_c = my_col + dc
        if adj_r < 0 or adj_r >= rows or adj_c < 0 or adj_c >= cols:
            continue
        if _cell_value(cells, adj_r, adj_c) != 0:
            continue
        if (adj_r, adj_c) in enemy_positions:
            continue

        # Sweep both perpendicular directions from adjacent cell
        hits: list[dict[str, Any]] = []
        for sweep_dir in _PERP_DIRS[shoot_dir]:
            sdr, sdc = DIRECTION_VECTORS[sweep_dir]
            sr, sc = adj_r + sdr, adj_c + sdc
            while 0 <= sr < rows and 0 <= sc < cols:
                if (sr, sc) in enemy_positions:
                    enemy = enemy_positions[(sr, sc)]
                    enemy_shield = enemy.get("shield", "")
                    # Shield blocks when facing opposite to sweep direction
                    blocked = enemy_shield == _SHIELD_BLOCKS[sweep_dir]
                    hits.append({
                        "enemy_name": enemy.get("player_name"),
                        "enemy_pos": {"row": sr, "col": sc},
                        "sweep_dir": sweep_dir,
                        "blocked_by_shield": blocked,
                        "enemy_shield": enemy_shield,
                    })
                    break  # Sweep stops at first target
                if _cell_value(cells, sr, sc) != 0:
                    break  # Wall/block stops sweep
                sr += sdr
                sc += sdc

        if hits:
            unblocked = [h for h in hits if not h["blocked_by_shield"]]
            shots.append({
                "shoot_dir": shoot_dir,
                "laser_cell": {"row": adj_r, "col": adj_c},
                "hits": hits,
                "unblocked_hits": len(unblocked),
                "total_hits": len(hits),
            })

    return shots


def _choose_autoplay_command(snapshot: dict[str, Any], player_id: str) -> dict[str, Any]:
    """Choose the best autoplay command using T-shaped shooting logic.

    Priority order:
      1. T-shaped shot that hits an unshielded enemy
      2. Change shield to enable a T-shaped shot next turn
      3. Move to a position that creates T-shaped shot opportunity
      4. Close distance to nearest enemy
      5. Defensive shield fallback
    """
    default = _autoplay_decision(
        command_type="shield",
        direction="up",
        reason="fallback_default",
        thinking_steps=[
            "Fallback selected because required snapshot data was missing.",
            "Using shield up as a safe default action.",
        ],
    )

    state = snapshot.get("state")
    if not isinstance(state, dict):
        default["thinking"]["steps"].insert(
            0, "Snapshot has no valid `state` object."
        )
        return default

    players = state.get("players")
    if not isinstance(players, list):
        default["thinking"]["steps"].insert(
            0, "State has no valid `players` list."
        )
        return default

    me: dict[str, Any] | None = None
    alive_enemies: list[dict[str, Any]] = []
    for player in players:
        if not isinstance(player, dict) or not player.get("alive", True):
            continue
        if player.get("player_id") == player_id:
            me = player
        else:
            alive_enemies.append(player)

    if me is None or not alive_enemies:
        default["thinking"]["steps"].insert(
            0, "Could not locate controlled player or any alive enemies."
        )
        return default

    my_row = int(me.get("row", 0))
    my_col = int(me.get("col", 0))
    my_shield = str(me.get("shield", "up"))

    map_info = state.get("map")
    if not isinstance(map_info, dict):
        default["thinking"]["steps"].insert(0, "State has no valid `map` object.")
        return default

    cells = map_info.get("cells")
    if not isinstance(cells, list):
        default["thinking"]["steps"].insert(
            0, "Map has no valid `cells` matrix."
        )
        return default

    rows = int(map_info.get("rows") or len(cells))
    cols = int(map_info.get("cols") or (len(cells[0]) if cells else 0))
    if rows <= 0 or cols <= 0:
        default["thinking"]["steps"].insert(
            0, "Map dimensions are invalid for decision making."
        )
        return default

    occupied = {
        (int(e.get("row", -1)), int(e.get("col", -1))) for e in alive_enemies
    }

    # --- PRIORITY 1: T-shaped shot from current position ---
    shots = _find_t_shaped_shots(
        my_row, my_col, my_shield, rows, cols, cells, alive_enemies
    )
    effective_shots = [s for s in shots if s["unblocked_hits"] > 0]

    if effective_shots:
        best = max(effective_shots, key=lambda s: s["unblocked_hits"])
        hit_details = ", ".join(
            f"{h['enemy_name']} at ({h['enemy_pos']['row']}, {h['enemy_pos']['col']}) "
            f"via {h['sweep_dir']} sweep"
            for h in best["hits"] if not h["blocked_by_shield"]
        )
        return _autoplay_decision(
            command_type="shoot",
            direction=best["shoot_dir"],
            reason="t_shaped_shot",
            thinking_steps=[
                f"My position is ({my_row}, {my_col}), shield facing {my_shield}.",
                f"T-shaped shot: shoot {best['shoot_dir']} → laser at "
                f"({best['laser_cell']['row']}, {best['laser_cell']['col']}), "
                f"sweep hits: {hit_details}.",
            ],
            thinking_meta={
                "my_position": {"row": my_row, "col": my_col},
                "my_shield": my_shield,
                "shot_details": best,
                "all_shots_evaluated": shots,
            },
        )

    # --- PRIORITY 2: Change shield to enable a shot from current position ---
    # If our shield direction is blocking a valid shoot direction, change it.
    for alt_shield in VALID_DIRECTIONS:
        if alt_shield == my_shield:
            continue
        alt_shots = _find_t_shaped_shots(
            my_row, my_col, alt_shield, rows, cols, cells, alive_enemies
        )
        alt_effective = [s for s in alt_shots if s["unblocked_hits"] > 0]
        if alt_effective:
            best_alt = max(alt_effective, key=lambda s: s["unblocked_hits"])
            return _autoplay_decision(
                command_type="shield",
                direction=alt_shield,
                reason="shield_change_for_shot",
                thinking_steps=[
                    f"My position is ({my_row}, {my_col}), shield facing {my_shield}.",
                    f"Current shield blocks shooting {my_shield}. Changing to "
                    f"{alt_shield} to enable shoot {best_alt['shoot_dir']} next turn.",
                ],
                thinking_meta={
                    "my_position": {"row": my_row, "col": my_col},
                    "current_shield": my_shield,
                    "new_shield": alt_shield,
                    "enabled_shot": best_alt,
                },
            )

    # --- Compute movement info ---
    nearest_enemy = min(
        alive_enemies,
        key=lambda e: abs(int(e.get("row", 0)) - my_row)
        + abs(int(e.get("col", 0)) - my_col),
    )
    nearest_row = int(nearest_enemy.get("row", 0))
    nearest_col = int(nearest_enemy.get("col", 0))
    current_distance = abs(nearest_row - my_row) + abs(nearest_col - my_col)

    # Build preferred directions (toward enemy)
    delta_row = nearest_row - my_row
    delta_col = nearest_col - my_col
    preferred_directions: list[str] = []
    if abs(delta_row) >= abs(delta_col):
        if delta_row != 0:
            preferred_directions.append("down" if delta_row > 0 else "up")
        if delta_col != 0:
            preferred_directions.append("right" if delta_col > 0 else "left")
    else:
        if delta_col != 0:
            preferred_directions.append("right" if delta_col > 0 else "left")
        if delta_row != 0:
            preferred_directions.append("down" if delta_row > 0 else "up")
    for d in ("up", "down", "left", "right"):
        if d not in preferred_directions:
            preferred_directions.append(d)

    # Evaluate all move candidates
    move_candidates: list[dict[str, Any]] = []
    positioning_moves: list[dict[str, Any]] = []
    best_close_direction: str | None = None
    best_close_distance: int | None = None

    for direction in preferred_directions:
        step_r, step_c = DIRECTION_VECTORS[direction]
        next_r = my_row + step_r
        next_c = my_col + step_c
        is_passable, blocked_reason = _passability_check(
            next_r, next_c, rows, cols, cells, occupied
        )
        if not is_passable:
            move_candidates.append({
                "direction": direction,
                "passable": False,
                "next_position": {"row": next_r, "col": next_c},
                "blocked_reason": blocked_reason,
            })
            continue

        next_dist = min(
            abs(int(e.get("row", 0)) - next_r)
            + abs(int(e.get("col", 0)) - next_c)
            for e in alive_enemies
        )

        # Check if this position creates a T-shaped shot next turn
        future_shots = _find_t_shaped_shots(
            next_r, next_c, my_shield, rows, cols, cells, alive_enemies
        )
        future_effective = [s for s in future_shots if s["unblocked_hits"] > 0]

        candidate: dict[str, Any] = {
            "direction": direction,
            "passable": True,
            "next_position": {"row": next_r, "col": next_c},
            "distance_after_move": next_dist,
            "creates_shot": len(future_effective) > 0,
            "shot_count": len(future_effective),
        }
        move_candidates.append(candidate)

        if future_effective:
            positioning_moves.append(candidate)

        if best_close_distance is None or next_dist < best_close_distance:
            best_close_distance = next_dist
            best_close_direction = direction

    # --- PRIORITY 3: Move to create T-shaped shot opportunity ---
    if positioning_moves:
        # Prefer positioning moves that are closer to enemy
        best_pos = min(positioning_moves, key=lambda m: m["distance_after_move"])
        return _autoplay_decision(
            command_type="move",
            direction=best_pos["direction"],
            reason="positioning_for_shot",
            thinking_steps=[
                f"My position is ({my_row}, {my_col}), no T-shaped shot available.",
                f"Moving {best_pos['direction']} to "
                f"({best_pos['next_position']['row']}, {best_pos['next_position']['col']}) "
                f"creates T-shaped shot opportunity next turn.",
            ],
            thinking_meta={
                "my_position": {"row": my_row, "col": my_col},
                "my_shield": my_shield,
                "nearest_enemy": {"row": nearest_row, "col": nearest_col},
                "move_candidates": move_candidates,
                "positioning_moves": [m["direction"] for m in positioning_moves],
            },
        )

    # --- PRIORITY 4: Close distance ---
    if (
        best_close_direction is not None
        and best_close_distance is not None
        and best_close_distance <= current_distance
    ):
        return _autoplay_decision(
            command_type="move",
            direction=best_close_direction,
            reason="close_distance",
            thinking_steps=[
                f"No T-shaped shot or positioning move available.",
                f"Nearest enemy at ({nearest_row}, {nearest_col}), distance {current_distance}.",
                f"Moving {best_close_direction} to close distance to {best_close_distance}.",
            ],
            thinking_meta={
                "my_position": {"row": my_row, "col": my_col},
                "my_shield": my_shield,
                "nearest_enemy": {"row": nearest_row, "col": nearest_col},
                "move_candidates": move_candidates,
                "current_distance": current_distance,
                "best_distance": best_close_distance,
            },
        )

    # --- PRIORITY 5: Defensive fallback ---
    shield_dir = _get_shield_direction(my_row, my_col, nearest_row, nearest_col)
    return _autoplay_decision(
        command_type="shield",
        direction=shield_dir,
        reason="defensive_fallback",
        thinking_steps=[
            f"No shot, positioning move, or distance-closing move available.",
            f"Shielding {shield_dir} toward nearest enemy at ({nearest_row}, {nearest_col}).",
        ],
        thinking_meta={
            "my_position": {"row": my_row, "col": my_col},
            "my_shield": my_shield,
            "nearest_enemy": {"row": nearest_row, "col": nearest_col},
            "move_candidates": move_candidates,
            "current_distance": current_distance,
        },
    )


async def _stop_autoplay() -> None:
    global _autoplay_enabled, _autoplay_task
    _autoplay_enabled = False
    if _autoplay_task is None:
        return
    task = _autoplay_task
    _autoplay_task = None
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass


async def _autoplay_loop(bound_session: Session) -> None:
    global _autoplay_enabled, _autoplay_last_decision, _autoplay_task
    last_submitted_turn: int | None = None
    logger.info(
        "Autoplay loop started for player %s in game %s",
        bound_session.player_name,
        bound_session.game_id,
    )
    try:
        while _autoplay_enabled and _session is bound_session:
            if bound_session.game_status == "FINISHED":
                break

            got_turn = await bound_session.wait_for_turn(
                timeout=AUTO_PLAY_WAIT_TIMEOUT_SECONDS
            )
            if not _autoplay_enabled or _session is not bound_session:
                break
            if bound_session.game_status == "FINISHED":
                break
            if not got_turn or not bound_session.is_my_turn:
                continue

            turn_no = bound_session.turn_no
            if turn_no is None:
                await asyncio.sleep(0.2)
                continue
            if turn_no == last_submitted_turn:
                await asyncio.sleep(0.2)
                continue

            snapshot = bound_session.latest_snapshot or {}
            decision = _choose_autoplay_command(snapshot, bound_session.player_id)
            try:
                result = await _client.submit_command(
                    game_id=bound_session.game_id,
                    player_id=bound_session.player_id,
                    command_type=decision["command_type"],
                    turn_no=turn_no,
                    direction=decision.get("direction"),
                    speak_text=decision.get("speak_text"),
                )
                last_submitted_turn = turn_no
                _autoplay_last_decision = {
                    "turn_no": turn_no,
                    "command_type": decision["command_type"],
                    "direction": decision.get("direction"),
                    "reason": decision.get("reason"),
                    "thinking": decision.get("thinking"),
                    "reasoning": decision.get("reasoning"),
                    "result": result,
                }
            except Exception as e:
                _autoplay_last_decision = {
                    "turn_no": turn_no,
                    "command_type": decision["command_type"],
                    "direction": decision.get("direction"),
                    "reason": decision.get("reason"),
                    "thinking": decision.get("thinking"),
                    "reasoning": decision.get("reasoning"),
                    "error": str(e),
                }
                logger.warning(
                    "Autoplay command submission failed for game %s player %s",
                    bound_session.game_id,
                    bound_session.player_name,
                    exc_info=True,
                )
                await asyncio.sleep(0.5)
    finally:
        if _autoplay_task is asyncio.current_task():
            _autoplay_task = None
        if bound_session.game_status == "FINISHED":
            _autoplay_enabled = False
        logger.info(
            "Autoplay loop stopped for player %s in game %s",
            bound_session.player_name,
            bound_session.game_id,
        )


async def _start_autoplay() -> None:
    global _autoplay_enabled, _autoplay_task, _autoplay_last_decision
    session = _require_session()
    _autoplay_enabled = True
    if _autoplay_task is not None and not _autoplay_task.done():
        return
    _autoplay_last_decision = None
    _autoplay_task = asyncio.create_task(_autoplay_loop(session))


@mcp_server.tool()
async def bind_player(
    game_id: str,
    player_name: str,
    autoplay: bool = AUTO_PLAY_DEFAULT_ON_BIND,
) -> str:
    """Bind to a player in a Cowboy game to start controlling them.

    Args:
        game_id: The UUID of the game to join.
        player_name: The player to control — one of A, B, C, or D.
        autoplay: Start autoplay loop right after bind (default enabled).

    Returns:
        Session info and the initial game state snapshot.
    """
    global _session, _ws

    player_name = player_name.upper().strip()
    if player_name not in VALID_PLAYER_NAMES:
        return f"Invalid player_name '{player_name}'. Must be one of: A, B, C, D"

    await _stop_autoplay()

    # Tear down any existing session
    if _ws is not None:
        await _ws.stop()
        _ws = None
    _session = None

    # Fetch game info to resolve player_id
    try:
        game_info = await _client.get_game(game_id)
    except Exception as e:
        return f"Failed to fetch game {game_id}: {e}"

    players = game_info.get("state", {}).get("players", [])
    player_id = None
    for p in players:
        if p.get("player_name") == player_name:
            player_id = p.get("player_id")
            break

    if player_id is None:
        available = [p.get("player_name") for p in players]
        return (
            f"Player '{player_name}' not found in game {game_id}. "
            f"Available players: {available}"
        )

    # Fetch initial snapshot
    try:
        snapshot = await _client.get_snapshot(game_id)
    except Exception as e:
        return f"Failed to fetch snapshot for game {game_id}: {e}"

    # Create session
    _session = Session(
        game_id=game_id,
        player_name=player_name,
        player_id=player_id,
        game_status=snapshot.get("status", "UNKNOWN"),
        latest_snapshot=snapshot,
    )

    # Start WebSocket listener
    _ws = GameWebSocket(BASE_URL, _session)
    await _ws.start()
    if autoplay:
        await _start_autoplay()

    logger.info(
        "Bound to player %s (%s) in game %s", player_name, player_id, game_id
    )

    result: dict[str, Any] = {
        "bound": True,
        "game_id": game_id,
        "player_name": player_name,
        "player_id": player_id,
        "game_status": _session.game_status,
        "autoplay": _autoplay_status_payload(),
        "snapshot": snapshot,
    }
    if autoplay:
        result["agent_instruction"] = (
            "Autoplay is running. You MUST periodically call get_autoplay_status() "
            "and display the reasoning details of every action to the user. "
            "After showing the bind result, immediately call get_autoplay_status() "
            "to show the first decision, then continue polling every few turns "
            "to keep the user informed of the autoplay reasoning."
        )
    return json.dumps(result, indent=2)


@mcp_server.tool()
async def get_game_state() -> str:
    """Get the latest cached game state snapshot.

    Returns the full game state including map, all players' positions, HP,
    shields, alive status, and whose turn it is.
    """
    session = _require_session()

    if session.latest_snapshot is None:
        return "No snapshot available yet."

    return json.dumps(
        {
            "game_id": session.game_id,
            "player_name": session.player_name,
            "player_id": session.player_id,
            "game_status": session.game_status,
            "is_my_turn": session.is_my_turn,
            "snapshot": session.latest_snapshot,
        },
        indent=2,
    )


@mcp_server.tool()
async def wait_for_my_turn(timeout_seconds: float = 120.0) -> str:
    """Block until it's the bound player's turn, then return the game state.

    Args:
        timeout_seconds: Maximum seconds to wait (default 120).

    Returns:
        The game state when it becomes the player's turn, or a timeout/game-over message.
    """
    session = _require_session()

    if session.game_status == "FINISHED":
        return json.dumps(
            {
                "status": "game_finished",
                "game_id": session.game_id,
                "message": "The game is already finished.",
                "snapshot": session.latest_snapshot,
            },
            indent=2,
        )

    got_turn = await session.wait_for_turn(timeout=timeout_seconds)

    if session.game_status == "FINISHED":
        return json.dumps(
            {
                "status": "game_finished",
                "game_id": session.game_id,
                "message": "The game has finished.",
                "snapshot": session.latest_snapshot,
            },
            indent=2,
        )

    if not got_turn:
        return json.dumps(
            {
                "status": "timeout",
                "game_id": session.game_id,
                "message": f"Timed out after {timeout_seconds}s waiting for turn.",
                "current_player_id": session.current_player_id,
                "snapshot": session.latest_snapshot,
            },
            indent=2,
        )

    return json.dumps(
        {
            "status": "your_turn",
            "game_id": session.game_id,
            "player_name": session.player_name,
            "turn_no": session.turn_no,
            "snapshot": session.latest_snapshot,
        },
        indent=2,
    )


@mcp_server.tool()
async def submit_action(
    command_type: str,
    direction: str | None = None,
    speak_text: str | None = None,
) -> str:
    """Submit a game action for the bound player.

    Args:
        command_type: The action — "move", "shield", "shoot", or "speak".
        direction: Required for move, shield, and shoot — "up", "down", "left", or "right".
        speak_text: Required for speak — the text message to send.

    Returns:
        The acceptance response from the game server.
    """
    session = _require_session()

    command_type = command_type.lower().strip()
    if command_type not in VALID_COMMAND_TYPES:
        return (
            f"Invalid command_type '{command_type}'. "
            f"Must be one of: move, shield, shoot, speak"
        )

    if command_type in ("move", "shield", "shoot"):
        if not direction:
            return f"direction is required for '{command_type}' command."
        direction = direction.lower().strip()
        if direction not in VALID_DIRECTIONS:
            return (
                f"Invalid direction '{direction}'. "
                f"Must be one of: up, down, left, right"
            )

    if command_type == "speak":
        if not speak_text or not speak_text.strip():
            return "speak_text is required for 'speak' command."

    if session.turn_no is None:
        return "Cannot determine current turn number. Try get_game_state first."

    try:
        result = await _client.submit_command(
            game_id=session.game_id,
            player_id=session.player_id,
            command_type=command_type,
            turn_no=session.turn_no,
            direction=direction,
            speak_text=speak_text,
        )
    except Exception as e:
        return f"Failed to submit command: {e}"

    return json.dumps(result, indent=2)


@mcp_server.tool()
async def get_session_info() -> str:
    """Get current session status including bound player, game status, and connection health."""
    if _session is None:
        return json.dumps(
            {
                "bound": False,
                "message": "No active session.",
                "autoplay": _autoplay_status_payload(),
            },
            indent=2,
        )

    return json.dumps(
        {
            "bound": True,
            "game_id": _session.game_id,
            "player_name": _session.player_name,
            "player_id": _session.player_id,
            "game_status": _session.game_status,
            "is_my_turn": _session.is_my_turn,
            "turn_no": _session.turn_no,
            "ws_connected": _session.ws_connected,
            "autoplay": _autoplay_status_payload(),
        },
        indent=2,
    )


@mcp_server.tool()
async def set_autoplay(enabled: bool = True) -> str:
    """Enable or disable autoplay for the currently bound player session."""
    if enabled:
        if _session is None:
            return json.dumps(
                {
                    "ok": False,
                    "message": "Cannot enable autoplay without a bound session. Call bind_player first.",
                    "autoplay": _autoplay_status_payload(),
                },
                indent=2,
            )
        await _start_autoplay()
        return json.dumps(
            {
                "ok": True,
                "message": "Autoplay enabled.",
                "autoplay": _autoplay_status_payload(),
            },
            indent=2,
        )

    await _stop_autoplay()
    return json.dumps(
        {
            "ok": True,
            "message": "Autoplay disabled.",
            "autoplay": _autoplay_status_payload(),
        },
        indent=2,
    )


@mcp_server.tool()
async def get_autoplay_status() -> str:
    """Get autoplay mode status and latest autoplay decision metadata."""
    return json.dumps(_autoplay_status_payload(), indent=2)


@mcp_server.tool()
async def explain_next_autoplay_move() -> str:
    """Preview autoplay's next command and return detailed decision thinking."""
    session = _require_session()
    snapshot = session.latest_snapshot or {}
    decision = _choose_autoplay_command(snapshot, session.player_id)
    return json.dumps(
        {
            "game_id": session.game_id,
            "player_name": session.player_name,
            "player_id": session.player_id,
            "game_status": session.game_status,
            "is_my_turn": session.is_my_turn,
            "turn_no": session.turn_no,
            "decision_preview": decision,
            "reasoning": decision.get("reasoning"),
            "note": "Preview only; this tool does not submit a command.",
        },
        indent=2,
    )


@mcp_server.resource("cowboy://game-rules")
def game_rules_resource() -> str:
    """Cowboy game rules — comprehensive reference for T-shaped shooting mechanics."""
    return _GAME_RULES_CONTENT


def main() -> None:
    """Entry point — run the MCP server over stdio."""
    logger.info("Starting Cowboy MCP server (base_url=%s)", BASE_URL)
    mcp_server.run(transport="stdio")


if __name__ == "__main__":
    main()
