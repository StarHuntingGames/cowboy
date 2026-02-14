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

import pytest

AGENT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if AGENT_DIR not in sys.path:
    sys.path.insert(0, AGENT_DIR)

import player_agent  # noqa: E402


def reset_player() -> None:
    player_agent.PLAYER = None


@pytest.mark.asyncio
async def test_init_rejects_empty_fields() -> None:
    reset_player()
    payload = player_agent.InitRequest(
        bot_id="",
        game_id="game-1",
        player_name="B",
        player_id="player-1",
    )
    response = await player_agent.init(payload)
    assert response["ok"] is False
    assert response["error"] == "init requires bot_id, game_id, player_id"


@pytest.mark.asyncio
async def test_decide_requires_init() -> None:
    reset_player()
    payload = player_agent.DecideRequest(force_speak=True, game={})
    response = await player_agent.decide(payload)
    assert response["ok"] is False
    assert response["error"] == "player is not initialized"


@pytest.mark.asyncio
async def test_decide_force_speak_returns_decision(monkeypatch: pytest.MonkeyPatch) -> None:
    reset_player()
    monkeypatch.setenv("BOT_AGENT_USE_DEEPAGENTS", "0")

    payload = player_agent.InitRequest(
        bot_id="bot-1",
        game_id="game-1",
        player_name="B",
        player_id="player-1",
    )
    init_response = await player_agent.init(payload)
    assert init_response["ok"] is True

    decide_payload = player_agent.DecideRequest(
        force_speak=True,
        game={"state": {"players": [{"player_id": "player-1"}]}},
    )
    decide_response = await player_agent.decide(decide_payload)
    assert decide_response["ok"] is True
    decision = decide_response["decision"]
    assert decision["command_type"] == "speak"
    assert decision["speak_text"]
