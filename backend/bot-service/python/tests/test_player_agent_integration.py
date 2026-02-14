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

import os
import sys

import httpx
import pytest

AGENT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if AGENT_DIR not in sys.path:
    sys.path.insert(0, AGENT_DIR)

import player_agent  # noqa: E402


@pytest.mark.asyncio
async def test_init_and_decide_http(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("BOT_AGENT_USE_DEEPAGENTS", "0")
    player_agent.PLAYER = None

    transport = httpx.ASGITransport(app=player_agent.app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        init_payload = {
            "bot_id": "bot-1",
            "game_id": "game-1",
            "player_name": "B",
            "player_id": "player-1",
        }
        init_response = await client.post("/init", json=init_payload)
        assert init_response.status_code == 200
        assert init_response.json()["ok"] is True

        decide_payload = {
            "force_speak": True,
            "game": {"state": {"players": [{"player_id": "player-1"}]}},
        }
        decide_response = await client.post("/decide", json=decide_payload)
        assert decide_response.status_code == 200
        body = decide_response.json()
        assert body["ok"] is True
        assert body["decision"]["command_type"] == "speak"
        assert body["decision"]["speak_text"]
