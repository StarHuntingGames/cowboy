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
WEB_SERVICE_BASE_URL="${WEB_SERVICE_BASE_URL:-http://localhost:8082}"
BOT_SERVICE_BASE_URL="${BOT_SERVICE_BASE_URL:-http://localhost:8091}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-75}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-3}"
EXPECTED_SUBSTRING="${EXPECTED_SUBSTRING:-deepagents invoke failed: Connection error.}"
ALT_EXPECTED_SUBSTRING="${ALT_EXPECTED_SUBSTRING:-operation timed out}"
FALLBACK_EXPECTED_SUBSTRING="${FALLBACK_EXPECTED_SUBSTRING:-player-agent decide request failed}"

TMP_DIR="$(mktemp -d)"
GAME_ID=""

cleanup() {
  if [[ -n "${GAME_ID}" ]]; then
    curl -sS -X POST \
      -H "content-type: application/json" \
      "http://localhost:8090/internal/v3/games/${GAME_ID}/bots/stop" \
      -d '{"reason":"E2E_CONNECTION_TEST_CLEANUP"}' >/dev/null || true
  fi
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

wait_http_ok() {
  local url="$1"
  local retries="${2:-60}"
  local sleep_seconds="${3:-1}"
  local i
  for ((i = 1; i <= retries; i++)); do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${sleep_seconds}"
  done
  echo "E2E_FAIL: timeout waiting for ${url}"
  return 1
}

docker compose up -d bot-manager-service web-service game-service timer-service >/dev/null

wait_http_ok "${MANAGER_BASE_URL}/health"
wait_http_ok "${WEB_SERVICE_BASE_URL}/health"
wait_http_ok "${BOT_SERVICE_BASE_URL}/health"

create_game_response="$(
  curl -fsS \
    -X POST \
    -H "content-type: application/json" \
    "${MANAGER_BASE_URL}/v2/games" \
    -d '{"turn_timeout_seconds":30}'
)"
GAME_ID="$(printf '%s' "${create_game_response}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["game_id"])')"

game_response="$(curl -fsS "${MANAGER_BASE_URL}/v2/games/${GAME_ID}")"
read -r PLAYER_A_ID PLAYER_B_ID <<EOF
$(printf '%s' "${game_response}" | python3 -c 'import json,sys
game=json.load(sys.stdin)
pa=next(p for p in game["state"]["players"] if p["player_name"]=="A")
pb=next(p for p in game["state"]["players"] if p["player_name"]=="B")
print(pa["player_id"], pb["player_id"])')
EOF

start_response="$(curl -fsS -X POST "${MANAGER_BASE_URL}/v2/games/${GAME_ID}/start")"
read -r TURN_NO CURRENT_PLAYER_ID <<EOF
$(printf '%s' "${start_response}" | python3 -c 'import json,sys
payload=json.load(sys.stdin)
print(payload["turn_no"], payload["current_player_id"])')
EOF

if [[ "${CURRENT_PLAYER_ID}" != "${PLAYER_A_ID}" ]]; then
  echo "E2E_FAIL: expected player A to start first turn"
  echo "current_player_id=${CURRENT_PLAYER_ID}"
  echo "player_a_id=${PLAYER_A_ID}"
  exit 1
fi

client_sent_at="$(date -u +"%Y-%m-%dT%H:%M:%S.%3NZ")"
open_command_payload="$(cat <<EOF
{
  "command_id": "e2e-connection-open-${GAME_ID}",
  "player_id": "${PLAYER_A_ID}",
  "command_type": "speak",
  "direction": null,
  "speak_text": "connection-test-open",
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

deadline_epoch="$(( $(date +%s) + MAX_WAIT_SECONDS ))"
matched_dynamo=0
last_candidates="[]"
while [[ "$(date +%s)" -lt "${deadline_epoch}" ]]; do
  expression_values="$(printf '{\":g\":{\"S\":\"%s\"}}' "${GAME_ID}")"
  raw_query="$(
    docker compose run --rm -T --entrypoint '' dynamodb-init \
      aws dynamodb query \
        --endpoint-url http://dynamodb:8000 \
        --table-name game_steps \
        --key-condition-expression 'game_id = :g' \
        --expression-attribute-values "${expression_values}" \
        --scan-index-forward 2>/dev/null || true
  )"
  printf '%s' "${raw_query}" >"${TMP_DIR}/game_steps_raw.json"

  if python3 - "${TMP_DIR}/game_steps_raw.json" "${PLAYER_B_ID}" "${EXPECTED_SUBSTRING}" "${ALT_EXPECTED_SUBSTRING}" "${FALLBACK_EXPECTED_SUBSTRING}" "${TMP_DIR}/ddb_match.json" "${TMP_DIR}/ddb_candidates.json" <<'PY'
import json
import pathlib
import sys

raw = pathlib.Path(sys.argv[1]).read_text()
player_b_id = sys.argv[2]
expected = sys.argv[3]
alt_expected = sys.argv[4]
fallback_expected = sys.argv[5]
match_file = pathlib.Path(sys.argv[6])
candidates_file = pathlib.Path(sys.argv[7])

first = raw.find("{")
if first < 0:
    raise SystemExit(1)

payload = json.loads(raw[first:])
items = payload.get("Items", [])
candidates = []
for item in items:
    command_type = item.get("command_type", {}).get("S", "")
    player_id = item.get("player_id", {}).get("S", "")
    result_status = item.get("result_status", {}).get("S", "")
    speak_text = item.get("speak_text", {}).get("S", "")
    turn_no = item.get("turn_no", {}).get("N", "")
    command_id = item.get("command_id", {}).get("S", "")

    if command_type != "speak" or player_id != player_b_id:
        continue

    candidates.append(
        {
            "turn_no": turn_no,
            "result_status": result_status,
            "command_id": command_id,
            "speak_text": speak_text,
        }
    )

    is_match = (
        expected in speak_text
        or alt_expected in speak_text
        or fallback_expected in speak_text
    )
    if result_status == "APPLIED" and is_match:
        match_file.write_text(
            json.dumps(
                {
                    "turn_no": turn_no,
                    "result_status": result_status,
                    "command_id": command_id,
                    "player_id": player_b_id,
                    "speak_text": speak_text,
                },
                ensure_ascii=True,
            )
        )
        candidates_file.write_text(json.dumps(candidates, ensure_ascii=True))
        raise SystemExit(0)

candidates_file.write_text(json.dumps(candidates, ensure_ascii=True))
raise SystemExit(1)
PY
  then
    matched_dynamo=1
    break
  fi

  if [[ -f "${TMP_DIR}/ddb_candidates.json" ]]; then
    last_candidates="$(cat "${TMP_DIR}/ddb_candidates.json")"
  fi
  sleep "${POLL_INTERVAL_SECONDS}"
done

if [[ "${matched_dynamo}" -ne 1 ]]; then
  echo "E2E_FAIL: did not observe expected connection-error fallback speak in DynamoDB"
  echo "game_id=${GAME_ID}"
  echo "player_b_id=${PLAYER_B_ID}"
  echo "expected_substring=${EXPECTED_SUBSTRING}"
  echo "last_b_player_speak_candidates=${last_candidates}"
  docker compose logs --since=10m bot-service | tail -n 200 || true
  exit 1
fi

if ! docker compose logs --since=10m bot-service | grep -F "${GAME_ID}" | grep -E "${EXPECTED_SUBSTRING}|${ALT_EXPECTED_SUBSTRING}|${FALLBACK_EXPECTED_SUBSTRING}" >/dev/null; then
  echo "E2E_FAIL: DynamoDB matched but bot-service logs did not contain expected connection-failure text for this game"
  echo "game_id=${GAME_ID}"
  echo "ddb_match=$(cat "${TMP_DIR}/ddb_match.json")"
  docker compose logs --since=10m bot-service | tail -n 240 || true
  exit 1
fi

echo "E2E_PASS"
echo "game_id=${GAME_ID}"
echo "player_b_id=${PLAYER_B_ID}"
echo "ddb_match=$(cat "${TMP_DIR}/ddb_match.json")"
echo "verified_log_patterns=${EXPECTED_SUBSTRING}|${ALT_EXPECTED_SUBSTRING}|${FALLBACK_EXPECTED_SUBSTRING}"
