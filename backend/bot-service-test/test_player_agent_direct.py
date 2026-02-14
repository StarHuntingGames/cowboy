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

"""Integration tests that call Player methods directly (no HTTP, no subprocess)."""
import os
import sys
import uuid
from datetime import datetime, timezone
from typing import Any, Dict

import pytest

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
AGENT_DIR = os.path.join(REPO_ROOT, "backend", "bot-service", "python")
if AGENT_DIR not in sys.path:
    sys.path.insert(0, AGENT_DIR)

from player_agent import (
    Player,
    fallback_decision,
    normalize_decision,
    parse_text_command,
    resolve_system_prompt,
    render_user_prompt_template,
)


def iso_ts() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def load_llm_config() -> Dict[str, str]:
    import yaml

    cfg = yaml.safe_load(open(os.path.join(REPO_ROOT, "bot-manager-llm.yaml"))) or {}
    default = cfg.get("default") or {}
    api_key = (default.get("api_key") or "").strip()
    if not api_key:
        api_key = os.environ.get("OPENROUTER_API_KEY", "") or os.environ.get("OPENAI_API_KEY", "")
    return {
        "base_url": (default.get("base_url") or "").strip(),
        "model": (default.get("model") or "").strip(),
        "api_key": api_key,
    }


def load_prompts_env() -> None:
    """Set prompt env vars from bot-service-prompts.yaml, matching what Rust bot-service does."""
    import yaml

    path = os.path.join(REPO_ROOT, "bot-service-prompts.yaml")
    if not os.path.isfile(path):
        return
    cfg = yaml.safe_load(open(path)) or {}
    if cfg.get("system_prompt"):
        os.environ["BOT_AGENT_SYSTEM_PROMPT"] = cfg["system_prompt"]
    if cfg.get("user_prompt"):
        os.environ["BOT_AGENT_USER_PROMPT_TEMPLATE"] = cfg["user_prompt"]
    if cfg.get("custom_system_prompt"):
        os.environ["BOT_AGENT_CUSTOM_SYSTEM_PROMPT"] = cfg["custom_system_prompt"]
    if cfg.get("custom_user_prompt"):
        os.environ["BOT_AGENT_CUSTOM_USER_PROMPT"] = cfg["custom_user_prompt"]


def make_player(llm: Dict[str, str]) -> Player:
    return Player(
        bot_id=f"bot-{uuid.uuid4()}",
        game_id=f"game-{uuid.uuid4()}",
        player_name="B",
        player_id=f"player-{uuid.uuid4()}",
        llm_base_url=llm["base_url"] or None,
        llm_model=llm["model"] or None,
        llm_api_key=llm["api_key"] or None,
    )


def make_game(player_id: str, *, two_players: bool = True) -> Dict[str, Any]:
    players = [
        {
            "player_name": "B",
            "player_id": player_id,
            "hp": 10,
            "row": 1,
            "col": 0,
            "shield": "left",
            "alive": True,
        },
    ]
    if two_players:
        players.insert(
            0,
            {
                "player_name": "A",
                "player_id": "player-a",
                "hp": 10,
                "row": 0,
                "col": 1,
                "shield": "up",
                "alive": True,
            },
        )
    return {
        "game_id": f"game-{uuid.uuid4()}",
        "status": "RUNNING",
        "map_source": "CUSTOM",
        "turn_timeout_seconds": 30,
        "turn_no": 1,
        "round_no": 1,
        "current_player_id": player_id,
        "created_at": iso_ts(),
        "started_at": iso_ts(),
        "state": {
            "map": {"rows": 3, "cols": 3, "cells": [[0, 0, 0], [0, 0, 0], [0, 0, 0]]},
            "players": players,
        },
    }


def make_step_event(game: Dict[str, Any], step_seq: int = 1) -> Dict[str, Any]:
    return {
        "game_id": game["game_id"],
        "step_seq": step_seq,
        "turn_no": 1,
        "round_no": 1,
        "event_type": "STEP_APPLIED",
        "result_status": "APPLIED",
        "command": None,
        "state_after": game["state"],
        "created_at": iso_ts(),
    }


@pytest.fixture(autouse=True)
def _setup_prompts_env():
    load_prompts_env()


# ---------------------------------------------------------------------------
# Test: decide returns a valid command (mirrors test_player_agent_decide_direct_real_llm)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_decide_returns_valid_command() -> None:
    llm = load_llm_config()
    player = make_player(llm)
    game = make_game(player.player_id, two_players=False)

    decision = await player.decide(game, force_speak=False)

    assert decision.get("command_type") in {"move", "shoot", "shield", "speak"}


# ---------------------------------------------------------------------------
# Test: decision_source is not fallback (mirrors test_player_agent_decision_source_not_fallback)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_decide_decision_source_not_fallback() -> None:
    llm = load_llm_config()
    assert llm["api_key"], (
        "LLM API key must be set in bot-manager-llm.yaml "
        "or via OPENROUTER_API_KEY / OPENAI_API_KEY env var"
    )
    player = make_player(llm)
    game = make_game(player.player_id, two_players=True)

    decision = await player.decide(game, force_speak=False)

    assert decision.get("decision_source") != "python_fallback", (
        f"Expected LLM decision but got python_fallback: {decision}"
    )
    assert not decision.get("llm_error"), f"LLM error: {decision.get('llm_error')}"
    assert decision.get("command_type") in {"move", "shoot", "shield", "speak"}


# ---------------------------------------------------------------------------
# Test: force_speak returns speak command
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_decide_force_speak() -> None:
    llm = load_llm_config()
    player = make_player(llm)
    game = make_game(player.player_id, two_players=True)

    decision = await player.decide(game, force_speak=True)

    assert decision.get("command_type") == "speak"
    assert decision.get("speak_text")
    assert len(decision["speak_text"]) <= 140


# ---------------------------------------------------------------------------
# Test: update processes a step event (mirrors the update portion of test_bot_service_interfaces_with_real_llm)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_update_processes_step_event() -> None:
    llm = load_llm_config()
    assert llm["api_key"], "LLM API key required"
    player = make_player(llm)
    game = make_game(player.player_id, two_players=True)
    step = make_step_event(game)

    result = await player.update(
        game,
        step_event_type=step["event_type"],
        step_seq=step["step_seq"],
        step_turn_no=step["turn_no"],
        step_round_no=step["round_no"],
        command=step["command"],
        is_bot_turn=True,
    )

    assert isinstance(result, dict)
    assert result.get("update_source") in {"deepagents", "python_fallback"}
    assert result.get("memory_size", 0) >= 1


# ---------------------------------------------------------------------------
# Test: custom prompts are included in resolved prompts
# ---------------------------------------------------------------------------

def test_custom_system_prompt_appended() -> None:
    old = os.environ.get("BOT_AGENT_CUSTOM_SYSTEM_PROMPT")
    try:
        os.environ["BOT_AGENT_CUSTOM_SYSTEM_PROMPT"] = "Kill player C first."
        result = resolve_system_prompt()
        assert "Kill player C first." in result
    finally:
        if old is None:
            os.environ.pop("BOT_AGENT_CUSTOM_SYSTEM_PROMPT", None)
        else:
            os.environ["BOT_AGENT_CUSTOM_SYSTEM_PROMPT"] = old


def test_custom_user_prompt_appended() -> None:
    old_custom = os.environ.get("BOT_AGENT_CUSTOM_USER_PROMPT")
    try:
        os.environ["BOT_AGENT_CUSTOM_USER_PROMPT"] = "Retaliate against {player_name}."
        player = Player(
            bot_id="bot-1",
            game_id="game-1",
            player_name="B",
            player_id="player-b",
        )
        game = make_game(player.player_id, two_players=True)
        result = render_user_prompt_template(player, game, force_speak=False)
        assert "Retaliate against B." in result
    finally:
        if old_custom is None:
            os.environ.pop("BOT_AGENT_CUSTOM_USER_PROMPT", None)
        else:
            os.environ["BOT_AGENT_CUSTOM_USER_PROMPT"] = old_custom


def test_empty_custom_prompts_do_not_append() -> None:
    old_sys = os.environ.get("BOT_AGENT_CUSTOM_SYSTEM_PROMPT")
    old_usr = os.environ.get("BOT_AGENT_CUSTOM_USER_PROMPT")
    try:
        os.environ["BOT_AGENT_CUSTOM_SYSTEM_PROMPT"] = ""
        os.environ["BOT_AGENT_CUSTOM_USER_PROMPT"] = ""
        system = resolve_system_prompt()
        assert not system.endswith("\n\n")

        player = Player(
            bot_id="bot-1",
            game_id="game-1",
            player_name="B",
            player_id="player-b",
        )
        game = make_game(player.player_id, two_players=False)
        user = render_user_prompt_template(player, game, force_speak=False)
        assert not user.endswith("\n\n")
    finally:
        if old_sys is None:
            os.environ.pop("BOT_AGENT_CUSTOM_SYSTEM_PROMPT", None)
        else:
            os.environ["BOT_AGENT_CUSTOM_SYSTEM_PROMPT"] = old_sys
        if old_usr is None:
            os.environ.pop("BOT_AGENT_CUSTOM_USER_PROMPT", None)
        else:
            os.environ["BOT_AGENT_CUSTOM_USER_PROMPT"] = old_usr


# ---------------------------------------------------------------------------
# Test: decide without force_speak returns a tactical command, not a speak fallback
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_decide_no_speak_fallback_without_force_speak() -> None:
    """When force_speak=False the LLM must return move/shoot/shield.

    If the LLM output cannot be parsed as a valid command, invoke_deepagents_sync
    silently wraps it as a speak command (decision_source stays 'deepagents').
    This test catches that: a speak result without force_speak means the LLM
    failed to produce a real tactical command.
    """
    llm = load_llm_config()
    assert llm["api_key"], "LLM API key required"
    player = make_player(llm)
    game = make_game(player.player_id, two_players=True)

    decision = await player.decide(game, force_speak=False)

    parsed_source = decision.get("parsed_source", "")
    assert parsed_source != "speak_fallback", (
        f"LLM output was not parseable as a command and fell back to speak. "
        f"parsed_source={parsed_source!r} "
        f"command_type={decision.get('command_type')!r} "
        f"llm_output={decision.get('llm_output', '')!r}"
    )
    assert parsed_source in {"json", "text_command"}, (
        f"Expected parsed_source to be 'json' or 'text_command' but got {parsed_source!r}"
    )
    assert decision.get("command_type") in {"move", "shoot", "shield", "speak"}


# ---------------------------------------------------------------------------
# Test: Player state tracks decisions and memory
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_player_state_tracks_decisions() -> None:
    llm = load_llm_config()
    player = make_player(llm)
    game = make_game(player.player_id, two_players=False)

    assert player.decision_count == 0
    assert player.memory == []

    await player.decide(game, force_speak=False)

    assert player.decision_count == 1
    assert len(player.memory) == 1
    assert player.last_turn == 1
    assert player.last_command in {"move", "shoot", "shield", "speak"}


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
