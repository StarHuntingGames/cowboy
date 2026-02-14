# Cowboy MCP Server — HTTP Game Client
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

import uuid
from datetime import datetime, timezone
from typing import Any

import httpx


class GameClient:
    """HTTP client for Cowboy game services via nginx reverse proxy."""

    def __init__(self, base_url: str = "http://localhost:8000") -> None:
        self.base_url = base_url.rstrip("/")
        self._http = httpx.AsyncClient(base_url=self.base_url, timeout=10.0)

    async def get_game(self, game_id: str) -> dict[str, Any]:
        """GET /v2/games/{game_id} — fetch game instance info (via game-manager)."""
        resp = await self._http.get(f"/v2/games/{game_id}")
        resp.raise_for_status()
        return resp.json()

    async def get_snapshot(self, game_id: str, from_turn_no: int = 1) -> dict[str, Any]:
        """GET /v2/games/{game_id}/snapshot — fetch latest game snapshot (via game-watcher)."""
        resp = await self._http.get(
            f"/v2/games/{game_id}/snapshot",
            params={"from_turn_no": from_turn_no},
        )
        resp.raise_for_status()
        return resp.json()

    async def submit_command(
        self,
        game_id: str,
        player_id: str,
        command_type: str,
        turn_no: int,
        direction: str | None = None,
        speak_text: str | None = None,
    ) -> dict[str, Any]:
        """POST /v2/games/{game_id}/commands — submit a player command (via web-service)."""
        payload: dict[str, Any] = {
            "command_id": str(uuid.uuid4()),
            "player_id": player_id,
            "command_type": command_type,
            "direction": direction,
            "speak_text": speak_text,
            "turn_no": turn_no,
            "client_sent_at": datetime.now(timezone.utc).isoformat(),
        }
        resp = await self._http.post(f"/v2/games/{game_id}/commands", json=payload)
        resp.raise_for_status()
        return resp.json()

    async def close(self) -> None:
        await self._http.aclose()
