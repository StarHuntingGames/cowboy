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

import json
import os
import signal
import subprocess
import threading
import time
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict

import httpx
import pytest

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BOT_SERVICE_BIN = ["cargo", "run", "-p", "bot-service", "--manifest-path", "backend/Cargo.toml"]
PLAYER_AGENT_BIN = [
    os.path.join(REPO_ROOT, ".venv", "bin", "python"),
    os.path.join(REPO_ROOT, "backend", "bot-service", "python", "player_agent.py"),
]


def iso_ts() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


class GameManagerHandler(BaseHTTPRequestHandler):
    game_payload: Dict[str, Any] = {}

    def log_message(self, fmt: str, *args: Any) -> None:
        return

    def _send_json(self, status: int, payload: Dict[str, Any]) -> None:
        data = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        if self.path == "/health":
            self._send_json(200, {"ok": True})
            return
        if self.path.startswith("/v2/games/"):
            self._send_json(200, self.game_payload)
            return
        self._send_json(404, {"error": "not found"})


@pytest.fixture(scope="module")
def kafka_stack() -> None:
    yield


def wait_http_ok(url: str, timeout_s: float = 30.0) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            if httpx.get(url, timeout=2.0).status_code == 200:
                return
        except Exception:
            pass
        time.sleep(0.25)
    raise RuntimeError(f"timeout waiting for {url}")


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


def load_langsmith_env() -> Dict[str, str]:
    import yaml

    cfg = yaml.safe_load(open(os.path.join(REPO_ROOT, "bot-service-langsmith.yaml"))) or {}
    enabled = bool(cfg.get("enabled", True))
    env = {
        "LANGSMITH_TRACING": "true" if enabled else "false",
        "LANGCHAIN_TRACING_V2": "true" if enabled else "false",
    }

    def add(key: str, value: Any) -> None:
        if value is None:
            return
        text = str(value).strip()
        if text:
            env[key] = text

    add("LANGSMITH_API_KEY", cfg.get("api_key"))
    add("LANGSMITH_ENDPOINT", cfg.get("endpoint"))
    add("LANGSMITH_PROJECT", cfg.get("project"))
    add("LANGSMITH_WORKSPACE_ID", cfg.get("workspace_id"))
    extra = cfg.get("extra_env") or {}
    if isinstance(extra, dict):
        for key, value in extra.items():
            add(str(key).strip(), value)
    return env


def start_bot_service(bind: str, manager_url: str) -> subprocess.Popen:
    llm = load_llm_config()
    env = os.environ.copy()
    env.update(load_langsmith_env())
    if llm["api_key"]:
        env["OPENAI_API_KEY"] = llm["api_key"]
    env.update(
        {
            "BOT_SERVICE_BIND": bind,
            "RUST_LOG": "bot_service=info,tower_http=info",
            "KAFKA_BOOTSTRAP_SERVERS": "localhost:1",
            "BOT_SERVICE_MOCK_KAFKA": "1",
            "GAME_MANAGER_BASE_URL": manager_url,
            "BOT_AGENT_USE_DEEPAGENTS": "1",
            "BOT_AGENT_OUTPUT_MODE": "command_text",
            "BOT_AGENT_TIMEOUT_MS": "120000",
            "BOT_AGENT_UPDATE_TIMEOUT_MS": "120000",
            "BOT_AGENT_PYTHON_BIN": os.path.join(REPO_ROOT, ".venv", "bin", "python"),
            "BOT_AGENT_SCRIPT_PATH": os.path.join(
                REPO_ROOT, "backend", "bot-service", "python", "player_agent.py"
            ),
            "BOT_AGENT_REQUIREMENTS_PATH": os.path.join(
                REPO_ROOT, "backend", "bot-service", "python", "requirements.txt"
            ),
            "BOT_AGENT_AUTO_INSTALL_REQUIREMENTS": "0",
            "BOT_AGENT_PROMPTS_CONFIG_PATH": os.path.join(
                REPO_ROOT, "bot-service-prompts.yaml"
            ),
            "BOT_AGENT_LANGSMITH_CONFIG_PATH": os.path.join(
                REPO_ROOT, "bot-service-langsmith.yaml"
            ),
        }
    )
    return subprocess.Popen(
        BOT_SERVICE_BIN,
        cwd=REPO_ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )


def stop_process(proc: subprocess.Popen) -> None:
    if proc.poll() is not None:
        return
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        proc.kill()


def start_player_agent(bind: str) -> subprocess.Popen:
    host, port = bind.split(":")
    llm = load_llm_config()
    env = os.environ.copy()
    env.update(load_langsmith_env())
    env.update(
        {
            "BOT_AGENT_USE_DEEPAGENTS": "1",
            "BOT_AGENT_OUTPUT_MODE": "command_text",
            "BOT_AGENT_TIMEOUT_MS": "120000",
            "BOT_AGENT_UPDATE_TIMEOUT_MS": "120000",
            "BOT_AGENT_PROMPTS_CONFIG_PATH": os.path.join(
                REPO_ROOT, "bot-service-prompts.yaml"
            ),
            "BOT_AGENT_LANGSMITH_CONFIG_PATH": os.path.join(
                REPO_ROOT, "bot-service-langsmith.yaml"
            ),
        }
    )
    if llm["api_key"]:
        env["OPENAI_API_KEY"] = llm["api_key"]
    return subprocess.Popen(
        PLAYER_AGENT_BIN + ["--host", host, "--port", port],
        cwd=REPO_ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )


def build_game_payload(game_id: str, player_b_id: str, input_topic: str, output_topic: str) -> Dict[str, Any]:
    return {
        "game_id": game_id,
        "status": "RUNNING",
        "map_source": "CUSTOM",
        "turn_timeout_seconds": 30,
        "turn_no": 1,
        "round_no": 1,
        "current_player_id": player_b_id,
        "created_at": iso_ts(),
        "started_at": iso_ts(),
        "input_topic": input_topic,
        "output_topic": output_topic,
        "state": {
            "map": {"rows": 3, "cols": 3, "cells": [[0, 0, 0], [0, 0, 0], [0, 0, 0]]},
            "players": [
                {
                    "player_name": "A",
                    "player_id": "player-a",
                    "hp": 10,
                    "row": 0,
                    "col": 1,
                    "shield": "up",
                    "alive": True,
                },
                {
                    "player_name": "B",
                    "player_id": player_b_id,
                    "hp": 10,
                    "row": 1,
                    "col": 0,
                    "shield": "left",
                    "alive": True,
                },
            ],
        },
    }


def build_step_event(game_id: str, state_after: Dict[str, Any], step_seq: int) -> Dict[str, Any]:
    return {
        "game_id": game_id,
        "step_seq": step_seq,
        "turn_no": 1,
        "round_no": 1,
        "event_type": "STEP_APPLIED",
        "result_status": "APPLIED",
        "command": None,
        "state_after": state_after["state"],
        "created_at": iso_ts(),
    }


def test_bot_service_interfaces_with_real_llm(kafka_stack: None) -> None:
    game_id = f"game-{uuid.uuid4()}"
    bot_id = f"bot-{uuid.uuid4()}"
    player_b_id = f"player-b-{uuid.uuid4()}"
    input_topic = f"bot-commands-{uuid.uuid4()}"
    output_topic = f"bot-steps-{uuid.uuid4()}"

    game_payload = build_game_payload(game_id, player_b_id, input_topic, output_topic)
    GameManagerHandler.game_payload = game_payload

    server = ThreadingHTTPServer(("127.0.0.1", 0), GameManagerHandler)
    server_host, server_port = server.server_address
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    manager_url = f"http://{server_host}:{server_port}"
    bot_service = start_bot_service("127.0.0.1:18091", manager_url)
    client: httpx.Client | None = None

    try:
        wait_http_ok("http://127.0.0.1:18091/health", timeout_s=60)
        wait_http_ok(f"{manager_url}/health", timeout_s=10)

        llm = load_llm_config()
        client = httpx.Client(base_url="http://127.0.0.1:18091", timeout=30)

        create_payload = {
            "bot_id": bot_id,
            "game_id": game_id,
            "player_name": "B",
            "player_id": player_b_id,
            "input_topic": input_topic,
            "output_topic": output_topic,
            "llm_base_url": llm["base_url"],
            "llm_model": llm["model"],
            "llm_api_key": llm["api_key"],
        }
        create_resp = client.post("/internal/v3/bots", json=create_payload)
        assert create_resp.status_code == 200

        get_resp = client.get(f"/internal/v3/bots/{bot_id}")
        assert get_resp.status_code == 200
        assert get_resp.json()["bot_id"] == bot_id

        teach_resp = client.post(
            f"/internal/v3/bots/{bot_id}/teach-game",
            json={"game_guide_version": "e2e-test"},
        )
        assert teach_resp.status_code == 200

        update_payload = {
            "step": build_step_event(game_id, game_payload, 1),
        }
        update_resp = client.post(f"/internal/v3/bots/{bot_id}/update", json=update_payload)
        assert update_resp.status_code == 200
        assert update_resp.json().get("accepted") is True

        delete_resp = client.delete(f"/internal/v3/bots/{bot_id}")
        assert delete_resp.status_code == 200
        assert delete_resp.json().get("deleted") is True
    finally:
        if client is not None:
            client.close()
        stop_process(bot_service)
        server.shutdown()


def test_player_agent_decide_direct_real_llm() -> None:
    llm = load_llm_config()
    player_agent = start_player_agent("127.0.0.1:18092")
    client: httpx.Client | None = None
    try:
        wait_http_ok("http://127.0.0.1:18092/health", timeout_s=60)
        client = httpx.Client(base_url="http://127.0.0.1:18092", timeout=120)

        init_payload = {
            "bot_id": f"bot-{uuid.uuid4()}",
            "game_id": f"game-{uuid.uuid4()}",
            "player_name": "B",
            "player_id": f"player-{uuid.uuid4()}",
            "llm_base_url": llm["base_url"],
            "llm_model": llm["model"],
            "llm_api_key": llm["api_key"],
        }
        init_resp = client.post("/init", json=init_payload)
        assert init_resp.status_code == 200
        assert init_resp.json().get("ok") is True

        decide_payload = {
            "force_speak": False,
            "game": {
                "game_id": init_payload["game_id"],
                "status": "RUNNING",
                "map_source": "CUSTOM",
                "turn_timeout_seconds": 30,
                "turn_no": 1,
                "round_no": 1,
                "current_player_id": init_payload["player_id"],
                "created_at": iso_ts(),
                "started_at": iso_ts(),
                "state": {
                    "map": {
                        "rows": 3,
                        "cols": 3,
                        "cells": [[0, 0, 0], [0, 0, 0], [0, 0, 0]],
                    },
                    "players": [
                        {
                            "player_name": "B",
                            "player_id": init_payload["player_id"],
                            "hp": 10,
                            "row": 1,
                            "col": 0,
                            "shield": "left",
                            "alive": True,
                        }
                    ],
                },
            },
        }
        decide_resp = client.post("/decide", json=decide_payload)
        assert decide_resp.status_code == 200
        body = decide_resp.json()
        assert body.get("ok") is True
        decision = body.get("decision") or {}
        assert decision.get("command_type") in {"move", "shoot", "shield", "speak"}
    finally:
        if client is not None:
            client.close()
        stop_process(player_agent)


def test_player_agent_decision_source_not_fallback() -> None:
    """Verify the LLM actually responds and decision_source is not python_fallback."""
    llm = load_llm_config()
    assert llm["api_key"], (
        "LLM API key must be set in bot-manager-llm.yaml "
        "or via OPENROUTER_API_KEY / OPENAI_API_KEY env var"
    )
    player_agent = start_player_agent("127.0.0.1:18093")
    client: httpx.Client | None = None
    try:
        wait_http_ok("http://127.0.0.1:18093/health", timeout_s=60)
        client = httpx.Client(base_url="http://127.0.0.1:18093", timeout=120)

        player_id = f"player-{uuid.uuid4()}"
        init_payload = {
            "bot_id": f"bot-{uuid.uuid4()}",
            "game_id": f"game-{uuid.uuid4()}",
            "player_name": "B",
            "player_id": player_id,
            "llm_base_url": llm["base_url"],
            "llm_model": llm["model"],
            "llm_api_key": llm["api_key"],
        }
        init_resp = client.post("/init", json=init_payload)
        assert init_resp.status_code == 200
        assert init_resp.json().get("ok") is True

        decide_payload = {
            "force_speak": False,
            "game": {
                "game_id": init_payload["game_id"],
                "status": "RUNNING",
                "map_source": "CUSTOM",
                "turn_timeout_seconds": 30,
                "turn_no": 1,
                "round_no": 1,
                "current_player_id": player_id,
                "created_at": iso_ts(),
                "started_at": iso_ts(),
                "state": {
                    "map": {
                        "rows": 3,
                        "cols": 3,
                        "cells": [[0, 0, 0], [0, 0, 0], [0, 0, 0]],
                    },
                    "players": [
                        {
                            "player_name": "A",
                            "player_id": "player-a",
                            "hp": 10,
                            "row": 0,
                            "col": 1,
                            "shield": "up",
                            "alive": True,
                        },
                        {
                            "player_name": "B",
                            "player_id": player_id,
                            "hp": 10,
                            "row": 1,
                            "col": 0,
                            "shield": "left",
                            "alive": True,
                        },
                    ],
                },
            },
        }
        decide_resp = client.post("/decide", json=decide_payload)
        assert decide_resp.status_code == 200
        body = decide_resp.json()
        assert body.get("ok") is True
        decision = body.get("decision") or {}
        assert decision.get("decision_source") != "python_fallback", (
            f"Expected LLM decision but got python_fallback: {decision}"
        )
        assert not decision.get("llm_error"), (
            f"LLM error: {decision.get('llm_error')}"
        )
        assert decision.get("command_type") in {"move", "shoot", "shield", "speak"}
    finally:
        if client is not None:
            client.close()
        stop_process(player_agent)


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__]))
