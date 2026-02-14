# Cowboy MCP Server — Session State Management
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
from dataclasses import dataclass, field
from typing import Any


@dataclass
class Session:
    """Holds the state for a single player-game binding."""

    game_id: str
    player_name: str
    player_id: str
    game_status: str = "UNKNOWN"
    latest_snapshot: dict[str, Any] | None = None
    ws_connected: bool = False

    # Signaled whenever the snapshot updates (turn change, game event, etc.)
    _turn_event: asyncio.Event = field(default_factory=asyncio.Event, repr=False)

    @property
    def current_player_id(self) -> str | None:
        if self.latest_snapshot is None:
            return None
        return self.latest_snapshot.get("current_player_id")

    @property
    def turn_no(self) -> int | None:
        if self.latest_snapshot is None:
            return None
        return self.latest_snapshot.get("turn_no")

    @property
    def is_my_turn(self) -> bool:
        return self.current_player_id == self.player_id

    def update_snapshot(self, snapshot: dict[str, Any]) -> None:
        """Update the cached snapshot and signal waiters."""
        self.latest_snapshot = snapshot
        self.game_status = snapshot.get("status", self.game_status)
        self._turn_event.set()
        self._turn_event.clear()

    async def wait_for_turn(self, timeout: float = 120.0) -> bool:
        """Block until it's this player's turn, or the game ends, or timeout.

        Returns True if it's the player's turn, False on timeout or game end.
        """
        if self.is_my_turn:
            return True
        if self.game_status == "FINISHED":
            return False

        deadline = asyncio.get_event_loop().time() + timeout
        while True:
            remaining = deadline - asyncio.get_event_loop().time()
            if remaining <= 0:
                return False
            try:
                await asyncio.wait_for(self._turn_event.wait(), timeout=remaining)
            except asyncio.TimeoutError:
                return False
            if self.is_my_turn or self.game_status == "FINISHED":
                return self.is_my_turn
