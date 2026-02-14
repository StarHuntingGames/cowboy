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

MANAGER_BASE_URL="${MANAGER_BASE_URL:-http://localhost:8081}"
BOT_SERVICE_BASE_URL="${BOT_SERVICE_BASE_URL:-http://localhost:8091}"
WEB_SERVICE_BASE_URL="${WEB_SERVICE_BASE_URL:-http://localhost:8082}"
WATCHER_WS_BASE_URL="${WATCHER_WS_BASE_URL:-ws://localhost:8083}"
WS_TIMEOUT_SECONDS="${WS_TIMEOUT_SECONDS:-70}"
DDB_WAIT_SECONDS="${DDB_WAIT_SECONDS:-25}"
DDB_POLL_INTERVAL_SECONDS="${DDB_POLL_INTERVAL_SECONDS:-2}"

TMP_DIR="$(mktemp -d)"
STEP_QUERY_RAW="${TMP_DIR}/step_query_raw.json"
WS_MATCH_FILE="${TMP_DIR}/ws_match.json"
WS_LOG_FILE="${TMP_DIR}/ws_listener.log"
DDB_MATCH_FILE="${TMP_DIR}/ddb_match.json"
BOT_ID=""
GAME_ID=""
WS_PID=""

if [[ -x ".venv/bin/python" ]]; then
  PYTHON_WS_BIN="${PYTHON_WS_BIN:-.venv/bin/python}"
else
  PYTHON_WS_BIN="${PYTHON_WS_BIN:-python3}"
fi

cleanup() {
  if [[ -n "${WS_PID}" ]] && kill -0 "${WS_PID}" 2>/dev/null; then
    kill "${WS_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${BOT_ID}" ]]; then
    curl -sS -X DELETE "${BOT_SERVICE_BASE_URL}/internal/v3/bots/${BOT_ID}" >/dev/null || true
  fi
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

curl -fsS "${MANAGER_BASE_URL}/health" >/dev/null
curl -fsS "${BOT_SERVICE_BASE_URL}/health" >/dev/null
curl -fsS "${WEB_SERVICE_BASE_URL}/health" >/dev/null

create_game_response="$(
  curl -fsS \
    -X POST \
    -H "content-type: application/json" \
    "${MANAGER_BASE_URL}/v2/games" \
    -d '{"turn_timeout_seconds":60,"bot_players":[]}'
)"
GAME_ID="$(printf '%s' "${create_game_response}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["game_id"])')"

game_response="$(curl -fsS "${MANAGER_BASE_URL}/v2/games/${GAME_ID}")"
read -r PLAYER_A_ID PLAYER_B_ID INPUT_TOPIC OUTPUT_TOPIC <<EOF
$(printf '%s' "${game_response}" | python3 -c 'import json,sys
game=json.load(sys.stdin)
player_a=next(player for player in game["state"]["players"] if player["player_name"]=="A")
player_b=next(player for player in game["state"]["players"] if player["player_name"]=="B")
print(player_a["player_id"], player_b["player_id"], game["input_topic"], game["output_topic"])')
EOF

BOT_ID="e2e-llm-failure-${GAME_ID}"
create_bot_payload="$(cat <<EOF
{
  "bot_id": "${BOT_ID}",
  "game_id": "${GAME_ID}",
  "player_name": "B",
  "player_id": "${PLAYER_B_ID}",
  "input_topic": "${INPUT_TOPIC}",
  "output_topic": "${OUTPUT_TOPIC}",
  "llm_base_url": "http://127.0.0.1:1/v1",
  "llm_model": "openai:gpt-4o-mini",
  "llm_api_key": "e2e-invalid-key"
}
EOF
)"
curl -fsS \
  -X POST \
  -H "content-type: application/json" \
  "${BOT_SERVICE_BASE_URL}/internal/v3/bots" \
  -d "${create_bot_payload}" >/dev/null

curl -fsS \
  -X POST \
  -H "content-type: application/json" \
  "${BOT_SERVICE_BASE_URL}/internal/v3/bots/${BOT_ID}/teach-game" \
  -d '{"game_guide_version":"e2e-llm-failure-speak"}' >/dev/null

# Let the worker subscribe before the game starts (consumer offset is latest).
sleep 1

WATCHER_WS_URL="${WATCHER_WS_BASE_URL%/}/v2/games/${GAME_ID}/stream?from_turn_no=0"
"${PYTHON_WS_BIN}" - "${WATCHER_WS_URL}" "${PLAYER_B_ID}" "${WS_TIMEOUT_SECONDS}" "${WS_MATCH_FILE}" >"${WS_LOG_FILE}" 2>&1 <<'PY' &
import asyncio
import json
import pathlib
import sys

try:
    import websockets
except Exception as error:
    print(f"ws listener setup failed: cannot import websockets: {error}", file=sys.stderr)
    sys.exit(3)

ws_url = sys.argv[1]
target_player_id = sys.argv[2]
timeout_seconds = float(sys.argv[3])
out_file = pathlib.Path(sys.argv[4])

async def main() -> int:
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout_seconds
    try:
        async with websockets.connect(ws_url, ping_interval=20, ping_timeout=20, max_size=2_000_000) as ws:
            while True:
                remaining = deadline - loop.time()
                if remaining <= 0:
                    print("ws listener timed out waiting for SPEAK event", file=sys.stderr)
                    return 1
                message = await asyncio.wait_for(ws.recv(), timeout=remaining)
                if isinstance(message, bytes):
                    message = message.decode("utf-8", errors="replace")
                try:
                    payload = json.loads(message)
                except Exception:
                    continue
                if payload.get("event_type") != "SPEAK":
                    continue
                if payload.get("player_id") != target_player_id:
                    continue
                speak_text = str(payload.get("speak_text") or "")
                if not speak_text.startswith("bot fail:"):
                    continue
                if "deepagents invoke failed" not in speak_text:
                    continue
                matched = {
                    "event_type": payload.get("event_type"),
                    "turn_no": payload.get("turn_no"),
                    "step_seq": payload.get("step_seq"),
                    "player_id": payload.get("player_id"),
                    "speak_text": speak_text,
                }
                out_file.write_text(json.dumps(matched, ensure_ascii=True))
                print(json.dumps(matched, ensure_ascii=True))
                return 0
    except Exception as error:
        print(f"ws listener failed: {error}", file=sys.stderr)
        return 2

raise SystemExit(asyncio.run(main()))
PY
WS_PID="$!"

start_response="$(curl -fsS -X POST "${MANAGER_BASE_URL}/v2/games/${GAME_ID}/start")"
read -r TURN_NO CURRENT_PLAYER_ID <<EOF
$(printf '%s' "${start_response}" | python3 -c 'import json,sys
payload=json.load(sys.stdin)
print(payload["turn_no"], payload["current_player_id"])')
EOF

if [[ "${CURRENT_PLAYER_ID}" != "${PLAYER_A_ID}" ]]; then
  echo "E2E_FAIL: expected player A to be current at game start"
  echo "current_player_id=${CURRENT_PLAYER_ID}"
  echo "player_a_id=${PLAYER_A_ID}"
  exit 1
fi

client_sent_at="$(date -u +"%Y-%m-%dT%H:%M:%S.%3NZ")"
open_command_payload="$(cat <<EOF
{
  "command_id": "e2e-open-${GAME_ID}",
  "player_id": "${PLAYER_A_ID}",
  "command_type": "speak",
  "direction": null,
  "speak_text": "e2e-open",
  "turn_no": ${TURN_NO},
  "client_sent_at": "${client_sent_at}"
}
EOF
)"
curl -fsS \
  -X POST \
  -H "content-type: application/json" \
  "${WEB_SERVICE_BASE_URL}/v2/games/${GAME_ID}/commands" \
  -d "${open_command_payload}" >/dev/null

if ! wait "${WS_PID}"; then
  echo "E2E_FAIL: watcher websocket did not receive the expected SPEAK event"
  echo "websocket_listener_log:"
  cat "${WS_LOG_FILE}" || true
  echo "Recent watcher/bot logs:"
  docker compose logs --tail=120 game-watcher-service bot-service || true
  exit 1
fi
WS_PID=""

deadline_epoch="$(( $(date +%s) + DDB_WAIT_SECONDS ))"
ddb_matched=0
while [[ "$(date +%s)" -lt "${deadline_epoch}" ]]; do
  expression_values="$(printf '{\":g\":{\"S\":\"%s\"}}' "${GAME_ID}")"
  raw_query="$(docker compose run --rm -T --entrypoint '' dynamodb-init \
    aws dynamodb query \
      --endpoint-url http://dynamodb:8000 \
      --table-name game_steps \
      --key-condition-expression 'game_id = :g' \
      --expression-attribute-values "${expression_values}" \
      --scan-index-forward 2>/dev/null || true)"
  printf '%s' "${raw_query}" >"${STEP_QUERY_RAW}"

  if python3 - "${STEP_QUERY_RAW}" "${PLAYER_B_ID}" "${DDB_MATCH_FILE}" <<'PY'
import json
import pathlib
import sys

raw = pathlib.Path(sys.argv[1]).read_text()
player_b_id = sys.argv[2]
match_file = pathlib.Path(sys.argv[3])
first_brace = raw.find("{")
if first_brace < 0:
    raise SystemExit(1)

payload = json.loads(raw[first_brace:])
items = payload.get("Items", [])
for item in items:
    command_type = item.get("command_type", {}).get("S", "")
    player_id = item.get("player_id", {}).get("S", "")
    result_status = item.get("result_status", {}).get("S", "")
    speak_text = item.get("speak_text", {}).get("S", "")
    turn_no = item.get("turn_no", {}).get("N", "")
    command_id = item.get("command_id", {}).get("S", "")
    if (
        command_type == "speak"
        and player_id == player_b_id
        and result_status == "APPLIED"
        and speak_text.startswith("bot fail:")
        and "deepagents invoke failed" in speak_text
    ):
        match_file.write_text(
            json.dumps(
                {
                    "turn_no": turn_no,
                    "command_id": command_id,
                    "player_id": player_id,
                    "speak_text": speak_text,
                },
                ensure_ascii=True,
            )
        )
        raise SystemExit(0)

raise SystemExit(1)
PY
  then
    ddb_matched=1
    break
  fi

  sleep "${DDB_POLL_INTERVAL_SECONDS}"
done

if [[ "${ddb_matched}" -ne 1 ]]; then
  echo "E2E_FAIL: watcher received SPEAK but DynamoDB did not contain expected speak record in time"
  echo "latest_dynamodb_query:"
  cat "${STEP_QUERY_RAW}" || true
  exit 1
fi

echo "E2E_PASS"
echo "websocket_match=$(cat "${WS_MATCH_FILE}")"
echo "dynamodb_match=$(cat "${DDB_MATCH_FILE}")"
echo "game_id=${GAME_ID}"
echo "player_b_id=${PLAYER_B_ID}"
echo "output_topic=${OUTPUT_TOPIC}"
