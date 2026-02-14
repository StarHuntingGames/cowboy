#!/usr/bin/env python3
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

import argparse
import asyncio
import ast
import json
import logging
import os
import random
import re
import time
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Dict, Optional

import boto3
import uvicorn
from fastapi import FastAPI
from pydantic import BaseModel


ALLOWED_COMMANDS = {"move", "shoot", "shield", "speak"}
ALLOWED_DIRECTIONS = {"up", "left", "down", "right"}
DIRECTION_ALIASES = {
    "1": "up",
    "up": "up",
    "u": "up",
    "2": "left",
    "left": "left",
    "l": "left",
    "3": "down",
    "down": "down",
    "d": "down",
    "4": "right",
    "right": "right",
    "r": "right",
}
SPEAK_PHRASES = [
    "I am moving.",
    "Watch this turn.",
    "Shield up.",
    "Taking the shot.",
    "No mercy.",
]
LOGGER = logging.getLogger("player-agent")
DEFAULT_SYSTEM_PROMPT = (
    "You are an expert Cowboy game bot. "
    "Return exactly one command line only (no JSON). "
    "Valid commands: move <direction>, shoot <direction>, shield <direction>, speak <text>. "
    "Directions: up, left, down, right. "
    "Shoot fires a laser into the adjacent cell in that direction; the laser then sweeps "
    "both perpendicular directions from that cell (e.g. shoot right → laser enters the cell "
    "to your right, then sweeps up and down). Each sweep stops at the first wall or player it "
    "hits. If the adjacent cell is blocked by a wall or player, the shot fails. "
    "For speak, provide a short text <= 140 chars."
)
DEFAULT_USER_PROMPT_TEMPLATE = (
    "bot_id={bot_id} player_name={player_name} player_id={player_id} "
    "turn={turn_no} round={round_no} force_speak={force_speak}. "
    "Pick the strongest legal action for this turn.\n"
    "GAME_JSON={game_json}"
)
DEFAULT_UPDATE_USER_PROMPT_TEMPLATE = (
    "You are tracking Cowboy game state for a bot. "
    "Summarize the latest change in one short sentence, focusing on tactical impact "
    "for the bot player. \n\n"
    "bot_id={bot_id} player_name={player_name} player_id={player_id} "
    "event={step_event_type} step_seq={step_seq} step_turn={step_turn_no} "
    "step_round={step_round_no} is_bot_turn={is_bot_turn}\n"
    "LATEST_COMMAND={command_json}\n"
    "GAME_JSON={game_json}"
)
JSON_FENCE_RE = re.compile(r"```(?:json)?\s*(\{.*?\})\s*```", re.DOTALL | re.IGNORECASE)


def pick_direction() -> str:
    return random.choice(tuple(ALLOWED_DIRECTIONS))


def normalize_direction(value: Any) -> Optional[str]:
    direction = str(value or "").strip().lower()
    return DIRECTION_ALIASES.get(direction)


def find_player(game: Dict[str, Any], player_id: str) -> Optional[Dict[str, Any]]:
    players = game.get("state", {}).get("players", [])
    if not isinstance(players, list):
        return None
    for player in players:
        if isinstance(player, dict) and player.get("player_id") == player_id:
            return player
    return None


def parse_json_candidate(raw: str) -> Optional[Any]:
    try:
        return json.loads(raw)
    except Exception:
        pass
    try:
        return ast.literal_eval(raw)
    except Exception:
        return None


def iter_json_objects(text: str) -> list[str]:
    matches: list[str] = []
    for match in JSON_FENCE_RE.finditer(text):
        matches.append(match.group(1))

    depth = 0
    start = None
    in_string = False
    escape = False
    for idx, ch in enumerate(text):
        if in_string:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_string = False
            continue
        if ch == '"':
            in_string = True
            continue
        if ch == "{":
            if depth == 0:
                start = idx
            depth += 1
        elif ch == "}":
            if depth > 0:
                depth -= 1
                if depth == 0 and start is not None:
                    matches.append(text[start : idx + 1])
                    start = None
    return matches


def extract_json(text: str) -> Optional[Dict[str, Any]]:
    text = (text or "").strip()
    if not text:
        return None

    parsed = parse_json_candidate(text)
    if isinstance(parsed, dict):
        return parsed

    fallback: Optional[Dict[str, Any]] = None
    for candidate in iter_json_objects(text):
        parsed = parse_json_candidate(candidate)
        if isinstance(parsed, dict):
            if "command_type" in parsed:
                return parsed
            inner = parsed.get("text")
            if isinstance(inner, str):
                nested = extract_json(inner)
                if isinstance(nested, dict):
                    return nested
            if fallback is None:
                fallback = parsed
        elif isinstance(parsed, list):
            for entry in parsed:
                if isinstance(entry, dict) and "text" in entry:
                    nested = extract_json(str(entry.get("text") or ""))
                    if isinstance(nested, dict):
                        return nested
    return fallback


def parse_text_command(text: str) -> Optional[Dict[str, Any]]:
    normalized = " ".join((text or "").strip().split())
    if not normalized:
        return None

    command_match = re.match(r"^(move|shoot|shield)\s+([^\s]+)", normalized, re.IGNORECASE)
    if command_match:
        command_type = command_match.group(1).strip().lower()
        direction = normalize_direction(command_match.group(2))
        if direction:
            return {"command_type": command_type, "direction": direction, "speak_text": None}
        return None

    speak_match = re.match(r"^speak(?:\s+(.+))?$", normalized, re.IGNORECASE)
    if speak_match:
        raw_text = (speak_match.group(1) or "").strip()
        if (
            len(raw_text) >= 2
            and raw_text[0] == raw_text[-1]
            and raw_text[0] in {"'", '"'}
        ):
            raw_text = raw_text[1:-1].strip()
        if not raw_text:
            return None
        return {"command_type": "speak", "direction": None, "speak_text": raw_text[:140]}

    return None


def normalize_decision(
    raw: Dict[str, Any], game: Dict[str, Any], player_id: str, force_speak: bool
) -> Dict[str, Any]:
    command_type = str(raw.get("command_type") or "").strip().lower()
    if command_type not in ALLOWED_COMMANDS:
        decision = fallback_decision(game, player_id, force_speak)
        decision["_normalization_error"] = (
            f"invalid command_type: {command_type or '<empty>'}"
        )
        return decision

    if force_speak:
        command_type = "speak"

    if command_type == "speak":
        speak_text = str(raw.get("speak_text") or "").strip()
        if not speak_text:
            decision = fallback_decision(game, player_id, force_speak)
            decision["_normalization_error"] = "missing speak_text for speak command"
            return decision
        return {"command_type": "speak", "direction": None, "speak_text": speak_text[:140]}

    direction = normalize_direction(raw.get("direction"))
    if not direction:
        decision = fallback_decision(game, player_id, force_speak)
        decision["_normalization_error"] = (
            f"invalid direction for {command_type}: "
            f"{str(raw.get('direction') or '').strip() or '<empty>'}"
        )
        return decision
    return {"command_type": command_type, "direction": direction, "speak_text": None}


def fallback_decision(game: Dict[str, Any], player_id: str, force_speak: bool) -> Dict[str, Any]:
    if force_speak:
        return {
            "command_type": "speak",
            "direction": None,
            "speak_text": random.choice(SPEAK_PHRASES),
        }

    me = find_player(game, player_id)
    hp = int(me.get("hp", 10)) if isinstance(me, dict) else 10
    if hp <= 3:
        shield = (me or {}).get("shield", pick_direction())
        return {
            "command_type": "shield",
            "direction": normalize_direction(shield) or pick_direction(),
            "speak_text": None,
        }

    roll = random.randint(0, 99)
    if roll < 20:
        return {
            "command_type": "speak",
            "direction": None,
            "speak_text": random.choice(SPEAK_PHRASES),
        }
    if roll < 50:
        return {"command_type": "move", "direction": pick_direction(), "speak_text": None}
    if roll < 80:
        return {"command_type": "shoot", "direction": pick_direction(), "speak_text": None}
    shield = (me or {}).get("shield", pick_direction())
    return {
        "command_type": "shield",
        "direction": normalize_direction(shield) or pick_direction(),
        "speak_text": None,
    }


def parse_model_spec(player: "Player") -> tuple[str, str]:
    raw = (player.llm_model or os.getenv("BOT_AGENT_MODEL", "openai:gpt-4o-mini")).strip()
    if not raw:
        raw = "openai:gpt-4o-mini"

    if ":" in raw:
        provider, model_name = raw.split(":", 1)
        provider = provider.strip().lower()
        model_name = model_name.strip()
        if provider in {"openai", "anthropic"} and model_name:
            return provider, model_name

    # No provider prefix: treat as OpenAI-compatible model id.
    return "openai", raw


def resolve_api_key(provider: str, explicit_api_key: Optional[str]) -> Optional[str]:
    if explicit_api_key:
        return explicit_api_key
    if provider == "anthropic":
        return os.getenv("ANTHROPIC_API_KEY")
    return os.getenv("OPENAI_API_KEY")


def has_provider_credentials(provider: str, api_key: Optional[str]) -> bool:
    if api_key:
        return True
    if provider == "anthropic":
        return bool(os.getenv("ANTHROPIC_API_KEY"))
    return bool(os.getenv("OPENAI_API_KEY"))


def build_chat_model(
    provider: str,
    model_name: str,
    api_key: Optional[str],
    base_url: Optional[str],
) -> Any:
    if provider == "anthropic":
        from langchain_anthropic import ChatAnthropic

        kwargs: Dict[str, Any] = {"model": model_name, "temperature": 0}
        if api_key:
            kwargs["api_key"] = api_key
        if base_url:
            kwargs["base_url"] = base_url
        return ChatAnthropic(**kwargs)

    # Default to OpenAI-compatible transport (works for OpenRouter and similar gateways).
    from langchain_openai import ChatOpenAI

    kwargs = {"model": model_name, "temperature": 0}
    if api_key:
        kwargs["api_key"] = api_key
    if base_url:
        kwargs["base_url"] = base_url
    return ChatOpenAI(**kwargs)


def stringify_agent_content(content: Any) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        # Reasoning models return a list of blocks; only use the last text block.
        last_text = None
        for block in content:
            if isinstance(block, str):
                last_text = block
            elif isinstance(block, dict) and block.get("type") == "text":
                last_text = block.get("text", "")
        if last_text is not None:
            lines = [l for l in last_text.strip().splitlines() if l.strip()]
            return lines[-1].strip() if lines else last_text
        try:
            return json.dumps(content, ensure_ascii=True)
        except Exception:
            return str(content)
    if isinstance(content, dict):
        if content.get("type") == "text":
            return content.get("text", "")
        try:
            return json.dumps(content, ensure_ascii=True)
        except Exception:
            return str(content)
    return str(content)


def truncate_text(value: Optional[str], max_chars: int) -> str:
    text = (value or "").strip()
    if len(text) <= max_chars:
        return text
    return text[:max_chars] + "...[truncated]"


def resolve_system_prompt(custom_override: Optional[str] = None) -> str:
    configured = (os.getenv("BOT_AGENT_SYSTEM_PROMPT") or "").strip()
    base = configured if configured else DEFAULT_SYSTEM_PROMPT
    custom = (custom_override or os.getenv("BOT_AGENT_CUSTOM_SYSTEM_PROMPT") or "").strip()
    if custom:
        return base + "\n\n" + custom
    return base


def resolve_output_mode() -> str:
    configured = (os.getenv("BOT_AGENT_OUTPUT_MODE") or "").strip().lower()
    return configured if configured else "command_text"



def resolve_update_timeout_ms() -> int:
    configured = (os.getenv("BOT_AGENT_UPDATE_TIMEOUT_MS") or "").strip()
    if configured.isdigit():
        return max(1, int(configured))
    fallback = (os.getenv("BOT_AGENT_TIMEOUT_MS") or "").strip()
    if fallback.isdigit():
        return max(1, int(fallback))
    return 120000


def render_user_prompt_template(player: "Player", game: Dict[str, Any], force_speak: bool) -> str:
    template = (os.getenv("BOT_AGENT_USER_PROMPT_TEMPLATE") or "").strip()
    if not template:
        template = DEFAULT_USER_PROMPT_TEMPLATE

    values = {
        "bot_id": player.bot_id,
        "player_name": player.player_name,
        "player_id": player.player_id,
        "turn_no": str(game.get("turn_no")),
        "round_no": str(game.get("round_no")),
        "force_speak": str(force_speak),
        "game_json": json.dumps(game, separators=(",", ":"), ensure_ascii=True),
    }
    rendered = template
    for key, value in values.items():
        rendered = rendered.replace("{" + key + "}", value)

    custom = (player.custom_user_prompt or os.getenv("BOT_AGENT_CUSTOM_USER_PROMPT") or "").strip()
    if custom:
        custom_rendered = custom
        for key, value in values.items():
            custom_rendered = custom_rendered.replace("{" + key + "}", value)
        rendered = rendered + "\n\n" + custom_rendered

    return rendered


def render_update_user_prompt_template(
    player: "Player",
    game: Dict[str, Any],
    step_event_type: str,
    step_seq: int,
    step_turn_no: int,
    step_round_no: int,
    command: Optional[Dict[str, Any]],
    is_bot_turn: bool,
) -> str:
    template = (os.getenv("BOT_AGENT_UPDATE_USER_PROMPT_TEMPLATE") or "").strip()
    if not template:
        template = DEFAULT_UPDATE_USER_PROMPT_TEMPLATE

    values = {
        "bot_id": player.bot_id,
        "player_name": player.player_name,
        "player_id": player.player_id,
        "step_event_type": str(step_event_type),
        "step_seq": str(step_seq),
        "step_turn_no": str(step_turn_no),
        "step_round_no": str(step_round_no),
        "is_bot_turn": str(is_bot_turn),
        "command_json": json.dumps(command or {}, separators=(",", ":"), ensure_ascii=True),
        "game_json": json.dumps(game, separators=(",", ":"), ensure_ascii=True),
    }
    rendered = template
    for key, value in values.items():
        rendered = rendered.replace("{" + key + "}", value)
    return rendered


def invoke_deepagents_chat_sync(
    player: "Player", system_prompt: str, user_prompt: str,
    agent: Any = None,
) -> Dict[str, Any]:
    trace: Dict[str, Any] = {
        "model": None,
        "system": None,
        "input": None,
        "output": None,
        "error": None,
    }
    enabled = os.getenv("BOT_AGENT_USE_DEEPAGENTS", "1").lower() not in {
        "0",
        "false",
        "no",
    }
    if not enabled:
        trace["error"] = "deepagents disabled by BOT_AGENT_USE_DEEPAGENTS"
        return trace

    provider, model_name = parse_model_spec(player)
    api_key = resolve_api_key(provider, player.llm_api_key)
    trace["model"] = player._model_spec or f"{provider}:{model_name}"
    if not has_provider_credentials(provider, api_key):
        trace["error"] = f"missing credentials for provider={provider}"
        return trace

    trace["system"] = system_prompt
    trace["input"] = user_prompt

    if agent is None:
        try:
            from deepagents import create_deep_agent
        except Exception as error:
            trace["error"] = f"failed to import deepagents: {error}"
            return trace

        try:
            model = build_chat_model(provider, model_name, api_key, player.llm_base_url)
            agent = create_deep_agent(model=model, system_prompt=system_prompt)
        except Exception as error:
            trace["error"] = f"deepagents agent creation failed: {error}"
            return trace

    try:
        result = agent.invoke({"messages": [{"role": "user", "content": user_prompt}]})
    except Exception as error:
        trace["error"] = f"deepagents invoke failed: {error}"
        return trace

    messages = result.get("messages", []) if isinstance(result, dict) else []
    if not messages:
        trace["error"] = "deepagents returned no messages"
        return trace
    last = messages[-1]
    if isinstance(last, dict):
        content = stringify_agent_content(last.get("content", ""))
    else:
        content = stringify_agent_content(getattr(last, "content", ""))
    trace["raw_output"] = content
    lines = [l for l in content.strip().splitlines() if l.strip()]
    trace["output"] = lines[-1].strip() if lines else content
    return trace


def invoke_deepagents_sync(player: "Player", game: Dict[str, Any], force_speak: bool) -> Dict[str, Any]:
    trace = invoke_deepagents_chat_sync(
        player,
        resolve_system_prompt(player.custom_system_prompt),
        render_user_prompt_template(player, game, force_speak),
        agent=player._agent,
    )
    output_text = (trace.get("output") if isinstance(trace, dict) else "") or ""
    output_mode = resolve_output_mode()
    parsed: Optional[Dict[str, Any]] = None
    parsed_source = "none"

    if output_text.strip():
        json_candidate = extract_json(output_text)
        if isinstance(json_candidate, dict) and "command_type" in json_candidate:
            parsed = json_candidate
            parsed_source = "json"
        if parsed is None:
            parsed = parse_text_command(output_text)
            if parsed is not None:
                parsed_source = "text_command"
        if parsed is None and output_mode == "command_text":
            parsed = {
                "command_type": "speak",
                "direction": None,
                "speak_text": output_text.strip()[:140],
            }
            parsed_source = "speak_fallback"

    trace["parsed_source"] = parsed_source
    if isinstance(parsed, dict):
        trace["decision"] = parsed
        return trace

    trace["decision"] = None
    if not trace.get("error"):
        trace["error"] = "deepagents output was empty"
    return trace


@dataclass
class Player:
    bot_id: str
    game_id: str
    player_name: str
    player_id: str
    decision_count: int = 0
    last_turn: int = 0
    last_command: Optional[str] = None
    memory: list[str] = field(default_factory=list)
    llm_base_url: Optional[str] = None
    llm_model: Optional[str] = None
    llm_api_key: Optional[str] = None
    custom_user_prompt: Optional[str] = None
    custom_system_prompt: Optional[str] = None
    _table: Any = field(init=False, default=None)
    _agent: Any = field(init=False, default=None)
    _model_spec: Optional[str] = field(init=False, default=None)

    def __post_init__(self):
        table_name = os.getenv("BOT_LLM_LOGS_TABLE")
        if table_name:
            try:
                endpoint_url = os.getenv("DYNAMODB_ENDPOINT")
                region_name = os.getenv("AWS_REGION", "us-east-1")
                dynamodb = boto3.resource('dynamodb', endpoint_url=endpoint_url, region_name=region_name)
                self._table = dynamodb.Table(table_name)
            except Exception as e:
                LOGGER.warning("failed to initialize dynamodb table: %s", e)
        self._init_agents()

    def _init_agents(self) -> None:
        enabled = os.getenv("BOT_AGENT_USE_DEEPAGENTS", "1").lower() not in {
            "0", "false", "no",
        }
        if not enabled:
            return

        provider, model_name = parse_model_spec(self)
        api_key = resolve_api_key(provider, self.llm_api_key)
        self._model_spec = f"{provider}:{model_name}"

        if not has_provider_credentials(provider, api_key):
            return

        try:
            from deepagents import create_deep_agent
        except Exception as error:
            LOGGER.warning("failed to import deepagents at init: %s", error)
            return

        try:
            model = build_chat_model(provider, model_name, api_key, self.llm_base_url)
            self._agent = create_deep_agent(
                model=model, system_prompt=resolve_system_prompt(self.custom_system_prompt),
            )
            LOGGER.info(
                "pre-built deepagents agent model=%s", self._model_spec,
            )
        except Exception as error:
            LOGGER.warning("failed to pre-build deepagents agent: %s", error)

    async def persist_llm_log(self, game: Dict[str, Any], data: Dict[str, Any], step_seq: Optional[int] = None):
        if self._table is None:
            return

        try:
            table = self._table

            turn_no = int(game.get("turn_no", 0) or 0)
            if step_seq is not None:
                log_key = f"{turn_no:010}#seq-{step_seq:010}#{self.player_id}"
            else:
                log_key = f"{turn_no:010}#{self.player_id}"

            item = {
                "game_id": self.game_id,
                "log_key": log_key,
                "player_id": self.player_id,
                "turn_no": turn_no,
                "created_at": datetime.utcnow().isoformat() + "Z",
            }

            if data.get("llm_model"):
                item["llm_model"] = data["llm_model"]
            if data.get("llm_system"):
                item["llm_system"] = data["llm_system"]
            if data.get("llm_input"):
                item["llm_input"] = data["llm_input"]
            if data.get("llm_output"):
                item["llm_output"] = data["llm_output"]
            if data.get("llm_error"):
                item["llm_error"] = data["llm_error"]

            await asyncio.to_thread(table.put_item, Item=item)

        except Exception as e:
            LOGGER.warning("failed to persist llm log to dynamodb: %s", e)

    async def decide(self, game: Dict[str, Any], force_speak: bool) -> Dict[str, Any]:
        self.decision_count += 1
        turn_no = int(game.get("turn_no", 0) or 0)
        self.last_turn = turn_no
        trace = await asyncio.to_thread(invoke_deepagents_sync, self, game, force_speak)
        decision = trace.get("decision") if isinstance(trace, dict) else None
        decision_source = "deepagents"
        if not isinstance(decision, dict):
            decision = fallback_decision(game, self.player_id, force_speak)
            decision_source = "python_fallback"
        normalized = normalize_decision(decision, game, self.player_id, force_speak)
        normalization_error = str(normalized.pop("_normalization_error", "")).strip()
        if normalization_error:
            decision_source = "python_fallback"
            if isinstance(trace, dict) and not trace.get("error"):
                trace["error"] = normalization_error
        normalized["decision_source"] = decision_source
        normalized["llm_model"] = (trace.get("model") if isinstance(trace, dict) else None) or ""
        normalized["llm_system"] = (trace.get("system") if isinstance(trace, dict) else None) or ""
        normalized["llm_input"] = (trace.get("input") if isinstance(trace, dict) else None) or ""
        normalized["llm_output"] = (trace.get("output") if isinstance(trace, dict) else None) or ""
        normalized["llm_raw_output"] = (trace.get("raw_output") if isinstance(trace, dict) else None) or ""
        normalized["llm_error"] = (trace.get("error") if isinstance(trace, dict) else None) or ""
        normalized["parsed_source"] = (trace.get("parsed_source") if isinstance(trace, dict) else None) or ""

        await self.persist_llm_log(game, normalized)

        LOGGER.info(
            "llm conversation source=%s model=%s error=%s system=%s input=%s output=%s raw_output=%s",
            decision_source,
            normalized["llm_model"],
            normalized["llm_error"],
            truncate_text(normalized["llm_system"], 800),
            truncate_text(normalized["llm_input"], 1400),
            truncate_text(normalized["llm_output"], 1400),
            truncate_text(normalized["llm_raw_output"], 1400),
        )
        if decision_source == "python_fallback":
            LOGGER.warning(
                "using python_fallback decision model=%s error=%s",
                normalized["llm_model"],
                normalized["llm_error"],
            )
        self.last_command = normalized.get("command_type")
        self.memory.append(f"turn={turn_no} cmd={self.last_command}")
        if len(self.memory) > 25:
            self.memory = self.memory[-25:]
        return normalized

    async def update(
        self,
        game: Dict[str, Any],
        step_event_type: str,
        step_seq: int,
        step_turn_no: int,
        step_round_no: int,
        command: Optional[Dict[str, Any]],
        is_bot_turn: bool,
    ) -> Dict[str, Any]:
        return {
            "update_source": "",
            "summary": "",
            "memory_size": 0,
            "llm_model": "",
            "llm_system": "",
            "llm_input": "",
            "llm_output": "",
            "llm_error": "",
        }

        # user_prompt = render_update_user_prompt_template(
        #     self,
        #     game,
        #     step_event_type,
        #     step_seq,
        #     step_turn_no,
        #     step_round_no,
        #     command,
        #     is_bot_turn,
        # )
        # timeout_ms = resolve_update_timeout_ms()
        # start_ts = time.monotonic()
        # try:
        #     trace = await asyncio.wait_for(
        #         asyncio.to_thread(
        #             invoke_deepagents_chat_sync,
        #             self,
        #             resolve_system_prompt(self.custom_system_prompt),
        #             user_prompt,
        #             agent=self._agent,
        #         ),
        #         timeout=timeout_ms / 1000.0,
        #     )
        # except asyncio.TimeoutError:
        #     elapsed_ms = int((time.monotonic() - start_ts) * 1000)
        #     LOGGER.warning(
        #         "llm update timed out after %sms (configured=%sms)",
        #         elapsed_ms,
        #         timeout_ms,
        #     )
        #     trace = {
        #         "model": None,
        #         "system": resolve_system_prompt(self.custom_system_prompt),
        #         "input": user_prompt,
        #         "output": "",
        #         "error": f"update timed out after {timeout_ms}ms",
        #     }
        # else:
        #     elapsed_ms = int((time.monotonic() - start_ts) * 1000)
        #     LOGGER.info("llm update completed in %sms", elapsed_ms)
        # update_source = "deepagents"
        # summary = ((trace.get("output") if isinstance(trace, dict) else None) or "").strip()
        # if not summary:
        #     update_source = "python_fallback"
        #     command_type = ""
        #     if isinstance(command, dict):
        #         command_type = str(command.get("command_type") or "").strip()
        #     summary = (
        #         f"event={step_event_type} step_seq={step_seq} "
        #         f"turn={step_turn_no} round={step_round_no} command={command_type or 'none'}"
        #     )
        # if (trace.get("error") if isinstance(trace, dict) else None):
        #     update_source = "python_fallback"
        #
        # self.memory.append(f"update step={step_seq} {truncate_text(summary, 240)}")
        # if len(self.memory) > 25:
        #     self.memory = self.memory[-25:]
        #
        # llm_model = (trace.get("model") if isinstance(trace, dict) else None) or ""
        # llm_system = (trace.get("system") if isinstance(trace, dict) else None) or ""
        # llm_input = (trace.get("input") if isinstance(trace, dict) else None) or ""
        # llm_output = (trace.get("output") if isinstance(trace, dict) else None) or ""
        # llm_error = (trace.get("error") if isinstance(trace, dict) else None) or ""
        #
        # log_data = {
        #     "llm_model": llm_model,
        #     "llm_system": llm_system,
        #     "llm_input": llm_input,
        #     "llm_output": llm_output,
        #     "llm_error": llm_error,
        # }
        # await self.persist_llm_log(game, log_data, step_seq=step_seq)
        #
        # LOGGER.info(
        #     "llm update source=%s model=%s error=%s event=%s step_seq=%s input=%s output=%s",
        #     update_source,
        #     llm_model,
        #     llm_error,
        #     step_event_type,
        #     step_seq,
        #     truncate_text(llm_input, 1400),
        #     truncate_text(llm_output, 1400),
        # )
        # if update_source == "python_fallback":
        #     LOGGER.warning(
        #         "using python_fallback update model=%s error=%s event=%s step_seq=%s",
        #         llm_model,
        #         llm_error,
        #         step_event_type,
        #         step_seq,
        #     )
        #
        # return {
        #     "update_source": update_source,
        #     "summary": truncate_text(summary, 280),
        #     "memory_size": len(self.memory),
        #     "llm_model": llm_model,
        #     "llm_system": llm_system,
        #     "llm_input": llm_input,
        #     "llm_output": llm_output,
        #     "llm_error": llm_error,
        # }


class InitRequest(BaseModel):
    bot_id: str
    game_id: str
    player_name: str
    player_id: str
    llm_base_url: Optional[str] = None
    llm_model: Optional[str] = None
    llm_api_key: Optional[str] = None


class DecideRequest(BaseModel):
    force_speak: bool = False
    game: Dict[str, Any]


class UpdateRequest(BaseModel):
    game: Dict[str, Any]
    step_event_type: str
    step_seq: int
    step_turn_no: int
    step_round_no: int
    command: Optional[Dict[str, Any]] = None
    is_bot_turn: bool = False


class PromptRequest(BaseModel):
    custom_user_prompt: str


class SystemPromptRequest(BaseModel):
    custom_system_prompt: str


def ok_response(
    decision: Optional[Dict[str, Any]] = None,
    update: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    response: Dict[str, Any] = {"ok": True}
    if decision is not None:
        response["decision"] = decision
    if update is not None:
        response["update"] = update
    return response


def err_response(message: str) -> Dict[str, Any]:
    return {"ok": False, "error": message}


app = FastAPI()
PLAYER: Optional[Player] = None
PLAYER_LOCK = asyncio.Lock()
PENDING_SYSTEM_PROMPT: Optional[str] = None


@app.get("/health")
async def health() -> Dict[str, Any]:
    return {"ok": True, "service": "player-agent"}


@app.post("/init")
async def init(payload: InitRequest) -> Dict[str, Any]:
    global PLAYER
    if not payload.bot_id or not payload.game_id or not payload.player_id:
        return err_response("init requires bot_id, game_id, player_id")

    async with PLAYER_LOCK:
        PLAYER = Player(
            bot_id=payload.bot_id.strip(),
            game_id=payload.game_id.strip(),
            player_name=payload.player_name.strip(),
            player_id=payload.player_id.strip(),
            llm_base_url=(payload.llm_base_url or "").strip() or None,
            llm_model=(payload.llm_model or "").strip() or None,
            llm_api_key=(payload.llm_api_key or "").strip() or None,
            custom_system_prompt=PENDING_SYSTEM_PROMPT,
        )
    return ok_response()


@app.post("/decide")
async def decide(payload: DecideRequest) -> Dict[str, Any]:
    async with PLAYER_LOCK:
        player = PLAYER
    if player is None:
        return err_response("player is not initialized")
    if not isinstance(payload.game, dict):
        return err_response("decide requires game object")

    try:
        decision = await player.decide(payload.game, payload.force_speak)
    except Exception as error:
        return err_response(f"decide failed: {error}")
    return ok_response(decision)


@app.post("/update")
async def update(payload: UpdateRequest) -> Dict[str, Any]:
    async with PLAYER_LOCK:
        player = PLAYER
    if player is None:
        return err_response("player is not initialized")
    if not isinstance(payload.game, dict):
        return err_response("update requires game object")

    try:
        result = await player.update(
            payload.game,
            payload.step_event_type,
            int(payload.step_seq),
            int(payload.step_turn_no),
            int(payload.step_round_no),
            payload.command,
            bool(payload.is_bot_turn),
        )
    except Exception as error:
        return err_response(f"update failed: {error}")
    return ok_response(update=result)


@app.post("/prompt/user")
async def prompt_user(payload: PromptRequest) -> Dict[str, Any]:
    async with PLAYER_LOCK:
        player = PLAYER
    if player is None:
        return err_response("player is not initialized")
    player.custom_user_prompt = payload.custom_user_prompt.strip() or None
    LOGGER.info("updated custom_user_prompt for %s", player.player_id)
    return ok_response()


@app.post("/prompt/system")
async def prompt_system(payload: SystemPromptRequest) -> Dict[str, Any]:
    global PENDING_SYSTEM_PROMPT
    value = payload.custom_system_prompt.strip() or None
    PENDING_SYSTEM_PROMPT = value
    LOGGER.info("updated pending system prompt (will apply on next /init)")
    async with PLAYER_LOCK:
        player = PLAYER
    if player is not None:
        player.custom_system_prompt = value
        LOGGER.info("also updated custom_system_prompt on current player %s", player.player_id)
    return ok_response()


@app.post("/shutdown")
async def shutdown() -> Dict[str, Any]:
    return ok_response()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Cowboy bot python player-agent HTTP server")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8098)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    uvicorn.run(
        app,
        host=args.host,
        port=args.port,
        log_level=os.getenv("BOT_AGENT_LOG_LEVEL", "warning"),
        access_log=False,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
