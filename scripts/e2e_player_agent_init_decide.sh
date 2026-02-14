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

HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-8099}"
BASE_URL="http://${HOST}:${PORT}"

TMP_DIR="$(mktemp -d)"
LOG_FILE="${TMP_DIR}/player_agent.log"
PID=""

cleanup() {
  if [[ -n "${PID}" ]] && kill -0 "${PID}" 2>/dev/null; then
    kill "${PID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

export BOT_AGENT_USE_DEEPAGENTS=0

if [[ -x ".venv/bin/python" ]]; then
  PYTHON_BIN=".venv/bin/python"
else
  PYTHON_BIN="python3"
fi

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

init_payload='{"bot_id":"bot-1","game_id":"game-1","player_name":"B","player_id":"player-1"}'
init_response="$(curl -fsS -X POST -H 'content-type: application/json' "${BASE_URL}/init" -d "${init_payload}")"
if [[ -z "${init_response}" ]]; then
  echo "E2E_FAIL: init returned empty response"
  cat "${LOG_FILE}" || true
  exit 1
fi
python3 -c 'import json,sys; payload=json.loads(sys.argv[1]); import sys as _sys; _sys.exit("E2E_FAIL: init response not ok") if not payload.get("ok") else None; print("init=PASS")' "${init_response}"

decide_payload='{"force_speak":true,"game":{"state":{"players":[{"player_id":"player-1"}]}}}'
decide_response="$(curl -fsS -X POST -H 'content-type: application/json' "${BASE_URL}/decide" -d "${decide_payload}")"
if [[ -z "${decide_response}" ]]; then
  echo "E2E_FAIL: decide returned empty response"
  cat "${LOG_FILE}" || true
  exit 1
fi
python3 -c 'import json,sys; payload=json.loads(sys.argv[1]); import sys as _sys; _sys.exit("E2E_FAIL: decide response not ok") if not payload.get("ok") else None; _sys.exit("E2E_FAIL: decide did not return speak") if payload.get("decision", {}).get("command_type") != "speak" else None; print("decide=PASS")' "${decide_response}"

echo "player_agent_init_decide_e2e=PASS"
