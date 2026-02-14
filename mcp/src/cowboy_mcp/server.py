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

import json
import logging
import os
import sys

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

mcp_server = FastMCP("Cowboy Game Controller")

# Module-level state — single session per process
_client = GameClient(BASE_URL)
_session: Session | None = None
_ws: GameWebSocket | None = None


def _require_session() -> Session:
    if _session is None:
        raise ValueError("No active session. Call bind_player first.")
    return _session


@mcp_server.tool()
async def bind_player(game_id: str, player_name: str) -> str:
    """Bind to a player in a Cowboy game to start controlling them.

    Args:
        game_id: The UUID of the game to join.
        player_name: The player to control — one of A, B, C, or D.

    Returns:
        Session info and the initial game state snapshot.
    """
    global _session, _ws

    player_name = player_name.upper().strip()
    if player_name not in VALID_PLAYER_NAMES:
        return f"Invalid player_name '{player_name}'. Must be one of: A, B, C, D"

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

    logger.info(
        "Bound to player %s (%s) in game %s", player_name, player_id, game_id
    )

    return json.dumps(
        {
            "bound": True,
            "game_id": game_id,
            "player_name": player_name,
            "player_id": player_id,
            "game_status": _session.game_status,
            "snapshot": snapshot,
        },
        indent=2,
    )


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
        return json.dumps({"bound": False, "message": "No active session."}, indent=2)

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
        },
        indent=2,
    )


def main() -> None:
    """Entry point — run the MCP server over stdio."""
    logger.info("Starting Cowboy MCP server (base_url=%s)", BASE_URL)
    mcp_server.run(transport="stdio")


if __name__ == "__main__":
    main()
