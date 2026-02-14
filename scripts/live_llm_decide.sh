#!/usr/bin/env bash
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

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-8106}"
BASE_URL="http://${HOST}:${PORT}"

PYTHON_BIN="${ROOT_DIR}/.venv/bin/python"
if [[ ! -x "${PYTHON_BIN}" ]]; then
  PYTHON_BIN="python3"
fi

TMP_DIR="$(mktemp -d)"
LOG_FILE="${TMP_DIR}/player_agent.log"
ENV_FILE="${TMP_DIR}/langsmith.env"
PID=""

cleanup() {
  if [[ -n "${PID}" ]] && kill -0 "${PID}" 2>/dev/null; then
    kill "${PID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

if [[ ! -f "${ROOT_DIR}/bot-manager-llm.yaml" ]]; then
  echo "E2E_FAIL: missing bot-manager-llm.yaml"
  exit 1
fi

if [[ ! -f "${ROOT_DIR}/bot-service-langsmith.yaml" ]]; then
  echo "E2E_FAIL: missing bot-service-langsmith.yaml"
  exit 1
fi

read -r LLM_BASE_URL LLM_MODEL LLM_API_KEY <<EOF
$(${PYTHON_BIN} - <<'PY'
import pathlib
import sys

try:
    import yaml
except Exception as exc:
    raise SystemExit(f"E2E_FAIL: PyYAML required to read bot-manager-llm.yaml: {exc}")

cfg = yaml.safe_load(pathlib.Path("bot-manager-llm.yaml").read_text()) or {}
default = cfg.get("default") or {}
base = (default.get("base_url") or "").strip()
model = (default.get("model") or "").strip()
key = (default.get("api_key") or "").strip()
print(base, model, key)
PY
)
EOF

${PYTHON_BIN} - <<'PY' >"${ENV_FILE}"
import pathlib
import sys

try:
    import yaml
except Exception as exc:
    raise SystemExit(f"E2E_FAIL: PyYAML required to read bot-service-langsmith.yaml: {exc}")

cfg = yaml.safe_load(pathlib.Path("bot-service-langsmith.yaml").read_text()) or {}
enabled = bool(cfg.get("enabled", True))
print(f"LANGSMITH_TRACING={'true' if enabled else 'false'}")
print(f"LANGCHAIN_TRACING_V2={'true' if enabled else 'false'}")

def emit(key, value):
    if value is None:
        return
    text = str(value).strip()
    if text:
        print(f"{key}={text}")

emit("LANGSMITH_API_KEY", cfg.get("api_key"))
emit("LANGSMITH_ENDPOINT", cfg.get("endpoint"))
emit("LANGSMITH_PROJECT", cfg.get("project"))
emit("LANGSMITH_WORKSPACE_ID", cfg.get("workspace_id"))
extra = cfg.get("extra_env") or {}
if isinstance(extra, dict):
    for key, value in extra.items():
        emit(str(key).strip(), value)
PY

set -a
source "${ENV_FILE}"
set +a

export BOT_AGENT_USE_DEEPAGENTS=1
export BOT_AGENT_OUTPUT_MODE=command_text

"${PYTHON_BIN}" backend/bot-service/python/player_agent.py --host "${HOST}" --port "${PORT}" >"${LOG_FILE}" 2>&1 &
PID="$!"

for _ in $(seq 1 40); do
  if curl -fsS "${BASE_URL}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

if ! curl -fsS "${BASE_URL}/health" >/dev/null 2>&1; then
  echo "E2E_FAIL: player-agent did not become ready"
  cat "${LOG_FILE}" || true
  exit 1
fi

init_payload=$(cat <<EOF
{
  "bot_id": "local-bot-1",
  "game_id": "local-game-1",
  "player_name": "B",
  "player_id": "player-b",
  "llm_base_url": "${LLM_BASE_URL}",
  "llm_model": "${LLM_MODEL}",
  "llm_api_key": "${LLM_API_KEY}"
}
EOF
)

curl -fsS -X POST -H 'content-type: application/json' "${BASE_URL}/init" -d "${init_payload}" >/dev/null

mock_game=$(cat <<'EOF'
{
  "turn_no": 1,
  "round_no": 1,
  "current_player_id": "player-b",
  "status": "RUNNING",
  "state": {
    "players": [
      {"player_id": "player-b", "hp": 8, "shield": "up"},
      {"player_id": "player-a", "hp": 10}
    ],
    "grid": [[0]]
  }
}
EOF
)

decide_payload=$(cat <<EOF
{
  "force_speak": false,
  "game": ${mock_game}
}
EOF
)

curl -fsS -X POST -H 'content-type: application/json' "${BASE_URL}/decide" -d "${decide_payload}"
