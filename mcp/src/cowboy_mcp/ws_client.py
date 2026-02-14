# Cowboy MCP Server — WebSocket Client
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
from typing import Any, TYPE_CHECKING

import websockets

if TYPE_CHECKING:
    from cowboy_mcp.session import Session

logger = logging.getLogger("cowboy_mcp.ws")

SNAPSHOT_EVENTS = {"SNAPSHOT", "GAME_STARTED", "GAME_FINISHED", "STEP_APPLIED"}


class GameWebSocket:
    """Maintains a WebSocket connection to the game-watcher stream and keeps
    the session snapshot up to date."""

    def __init__(self, base_url: str, session: Session) -> None:
        # Convert http(s) to ws(s) scheme
        ws_base = base_url.replace("https://", "wss://").replace("http://", "ws://")
        self._url = f"{ws_base}/v2/games/{session.game_id}/stream?from_turn_no=1"
        self._session = session
        self._task: asyncio.Task[None] | None = None
        self._stop = asyncio.Event()

    async def start(self) -> None:
        """Start the background listener task."""
        self._stop.clear()
        self._task = asyncio.create_task(self._listen_loop())

    async def stop(self) -> None:
        """Stop the background listener."""
        self._stop.set()
        if self._task is not None:
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
            self._task = None
        self._session.ws_connected = False

    async def _listen_loop(self) -> None:
        """Connect and reconnect with exponential backoff."""
        backoff = 1.0
        max_backoff = 30.0

        while not self._stop.is_set():
            try:
                async with websockets.connect(self._url) as ws:
                    self._session.ws_connected = True
                    backoff = 1.0
                    logger.info("WebSocket connected to %s", self._url)

                    async for raw in ws:
                        if self._stop.is_set():
                            break
                        self._handle_message(raw)

            except asyncio.CancelledError:
                break
            except Exception:
                logger.warning(
                    "WebSocket disconnected, reconnecting in %.1fs", backoff, exc_info=True
                )
                self._session.ws_connected = False
                try:
                    await asyncio.wait_for(self._stop.wait(), timeout=backoff)
                    break  # stop was requested during backoff
                except asyncio.TimeoutError:
                    pass
                backoff = min(backoff * 2, max_backoff)

        self._session.ws_connected = False

    def _handle_message(self, raw: str | bytes) -> None:
        """Parse a WebSocket message and update session state."""
        try:
            msg = json.loads(raw)
        except json.JSONDecodeError:
            logger.warning("Non-JSON WebSocket message: %s", raw[:200])
            return

        event_type = msg.get("event_type", "")

        # Record all game events (exclude internal CONNECTED/SNAPSHOT/ERROR)
        if event_type not in ("CONNECTED", "SNAPSHOT", "ERROR", ""):
            event_record: dict[str, Any] = {"event_type": event_type}
            for key in ("player_id", "command_type", "direction", "speak_text",
                        "result_status", "turn_no", "round_no", "step_seq"):
                if key in msg:
                    event_record[key] = msg[key]
            self._session.add_event(event_record)
            logger.info("Event: %s %s", event_type, {
                k: v for k, v in event_record.items() if k != "event_type"
            })

        if event_type == "GAME_FINISHED":
            self._session.game_status = "FINISHED"
            snapshot = msg.get("snapshot")
            if snapshot:
                self._session.update_snapshot(snapshot)
            else:
                self._session._turn_event.set()
                self._session._turn_event.clear()
            return

        # Events that carry a snapshot
        snapshot = msg.get("snapshot")
        if snapshot:
            self._session.update_snapshot(snapshot)
            logger.debug("Snapshot updated: turn=%s", snapshot.get("turn_no"))
