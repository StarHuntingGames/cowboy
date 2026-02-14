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
import time

import httpx
import pytest

AGENT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if AGENT_DIR not in sys.path:
    sys.path.insert(0, AGENT_DIR)

import player_agent  # noqa: E402


@pytest.mark.asyncio
async def test_update_timeout_falls_back(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("BOT_AGENT_USE_DEEPAGENTS", "1")
    monkeypatch.setenv("BOT_AGENT_UPDATE_TIMEOUT_MS", "10")
    player_agent.PLAYER = None

    def slow_invoke(*_args, **_kwargs):
        time.sleep(0.05)
        return {
            "model": "openai:test",
            "system": "test-system",
            "input": "test-input",
            "output": "slow output",
            "error": None,
        }

    monkeypatch.setattr(player_agent, "invoke_deepagents_chat_sync", slow_invoke)

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

        update_payload = {
            "game": {"state": {"players": [{"player_id": "player-1"}]}, "turn_no": 1},
            "step_event_type": "STEP_APPLIED",
            "step_seq": 1,
            "step_turn_no": 1,
            "step_round_no": 1,
            "command": {"command_type": "move", "direction": "up"},
            "is_bot_turn": False,
        }
        update_response = await client.post("/update", json=update_payload)
        assert update_response.status_code == 200
        body = update_response.json()
        assert body["ok"] is True
        update = body["update"]
        assert update["update_source"] == "python_fallback"
        assert "timed out" in (update.get("llm_error") or "")
