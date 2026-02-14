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
BOT_MANAGER_BASE_URL="${BOT_MANAGER_BASE_URL:-http://localhost:8090}"
BOT_SERVICE_BASE_URL="${BOT_SERVICE_BASE_URL:-http://localhost:8091}"
WEB_SERVICE_BASE_URL="${WEB_SERVICE_BASE_URL:-http://localhost:8082}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-80}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-3}"

if [[ -z "${LLM_MOCK_CONTENT:-}" ]]; then
  LLM_MOCK_CONTENT='{"command_type":"move","direction":"up","speak_text":null} {"note":"extra"}'
fi

COMPOSE=(docker compose -f docker-compose.yml -f docker-compose.e2e-llm-mock.yaml)
TMP_DIR="$(mktemp -d)"
CONFIG_BACKUP_DIR="${TMP_DIR}/config_backup"
mkdir -p "${CONFIG_BACKUP_DIR}"

BOT_MANAGER_CONFIG_SRC="${1:-}"
BOT_SERVICE_LANGSMITH_SRC="${2:-}"

cleanup() {
  for dest in "${!CONFIG_BACKUPS[@]}"; do
    backup="${CONFIG_BACKUPS[$dest]}"
    if [[ -n "${backup}" && -f "${backup}" ]]; then
      cp "${backup}" "${dest}"
    else
      rm -f "${dest}"
    fi
  done
  rm -rf "${TMP_DIR}"
}

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

declare -A CONFIG_BACKUPS
apply_config() {
  local src="$1"
  local dest="$2"
  if [[ "${src}" == "${dest}" ]]; then
    return 0
  fi
  if [[ ! -f "${src}" ]]; then
    echo "E2E_FAIL: config file not found: ${src}"
    exit 1
  fi
  if [[ -f "${dest}" ]]; then
    local backup="${CONFIG_BACKUP_DIR}/$(basename "${dest}")"
    cp "${dest}" "${backup}"
    CONFIG_BACKUPS["${dest}"]="${backup}"
  else
    CONFIG_BACKUPS["${dest}"]=""
  fi
  cp "${src}" "${dest}"
}

trap cleanup EXIT

if [[ -z "${BOT_MANAGER_CONFIG_SRC}" ]]; then
  BOT_MANAGER_CONFIG_SRC="${TMP_DIR}/bot-manager-llm.yaml"
  cat <<'YAML' > "${BOT_MANAGER_CONFIG_SRC}"
default:
  base_url: "http://llm-mock:9999/v1"
  model: "openai:gpt-4o-mini"
  api_key: "e2e-mock-key"
players: {}
YAML
fi

if [[ -z "${BOT_SERVICE_LANGSMITH_SRC}" ]]; then
  BOT_SERVICE_LANGSMITH_SRC="${TMP_DIR}/bot-service-langsmith.yaml"
  cat <<'YAML' > "${BOT_SERVICE_LANGSMITH_SRC}"
enabled: false
api_key: ""
endpoint: ""
project: ""
workspace_id: ""
extra_env: {}
YAML
fi

apply_config "${BOT_MANAGER_CONFIG_SRC}" "bot-manager-llm.yaml"
apply_config "${BOT_SERVICE_LANGSMITH_SRC}" "bot-service-langsmith.yaml"

export LLM_MOCK_CONTENT
"${COMPOSE[@]}" up -d llm-mock bot-manager-service bot-service web-service game-service timer-service game-manager-service >/dev/null

wait_http_ok "${MANAGER_BASE_URL}/health"
wait_http_ok "${BOT_MANAGER_BASE_URL}/health"
wait_http_ok "${BOT_SERVICE_BASE_URL}/health"
wait_http_ok "${WEB_SERVICE_BASE_URL}/health"

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
player_a=next(player for player in game["state"]["players"] if player["player_name"]=="A")
player_b=next(player for player in game["state"]["players"] if player["player_name"]=="B")
print(player_a["player_id"], player_b["player_id"])')
EOF

BOT_ID=""
for _ in $(seq 1 45); do
  response="$(curl -sS "${BOT_MANAGER_BASE_URL}/internal/v3/games/${GAME_ID}/assignments" || true)"
  if [[ -n "${response}" ]]; then
    printf '%s' "${response}" >"${TMP_DIR}/assignments.json"
    BOT_ID="$(python3 - "${TMP_DIR}/assignments.json" <<'PY'
import json
import sys
payload = json.load(open(sys.argv[1]))
for binding in payload.get("bindings", []):
    if binding.get("player_name") == "B":
        print(binding.get("bot_id", ""))
        raise SystemExit(0)
print("")
PY
)"
    if [[ -n "${BOT_ID}" ]]; then
      break
    fi
  fi
  sleep 1
done

if [[ -z "${BOT_ID}" ]]; then
  echo "E2E_FAIL: bot-manager did not assign player B"
  exit 1
fi

for _ in $(seq 1 40); do
  status="$(curl -sS "${BOT_SERVICE_BASE_URL}/internal/v3/bots/${BOT_ID}" | python3 -c 'import json,sys
try:
    print(json.load(sys.stdin).get("status", ""))
except Exception:
    print("")')"
  if [[ "${status}" == "READY" ]]; then
    break
  fi
  sleep 1
done

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
  "command_id": "e2e-deepagents-open-${GAME_ID}",
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

error_message="deepagents output did not contain a valid JSON object"
matched=0
last_candidates="[]"

deadline_epoch="$(( $(date +%s) + MAX_WAIT_SECONDS ))"
while [[ "$(date +%s)" -lt "${deadline_epoch}" ]]; do
  expression_values="$(printf '{":g":{"S":"%s"}}' "${GAME_ID}")"
  raw_query="$(
    "${COMPOSE[@]}" run --rm -T --entrypoint '' dynamodb-init \
      aws dynamodb query \
        --endpoint-url http://dynamodb:8000 \
        --table-name game_steps \
        --key-condition-expression 'game_id = :g' \
        --expression-attribute-values "${expression_values}" \
        --scan-index-forward 2>/dev/null || true
  )"
  printf '%s' "${raw_query}" >"${TMP_DIR}/game_steps_raw.json"

  if python3 - "${TMP_DIR}/game_steps_raw.json" "${PLAYER_B_ID}" "${error_message}" "${TMP_DIR}/ddb_match.json" "${TMP_DIR}/ddb_candidates.json" <<'PY'
import json
import pathlib
import sys

raw = pathlib.Path(sys.argv[1]).read_text()
player_b_id = sys.argv[2]
error_message = sys.argv[3]
match_file = pathlib.Path(sys.argv[4])
candidates_file = pathlib.Path(sys.argv[5])

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

    if player_id != player_b_id:
        continue

    candidates.append(
        {
            "turn_no": turn_no,
            "result_status": result_status,
            "command_id": command_id,
            "command_type": command_type,
            "speak_text": speak_text,
        }
    )

    if command_type == "speak" and error_message in speak_text:
        candidates_file.write_text(json.dumps(candidates, ensure_ascii=True))
        print("E2E_FAIL: observed failure speak from deepagents output parser")
        raise SystemExit(2)

    if command_type in {"move", "shoot", "shield"}:
        match_file.write_text(
            json.dumps(
                {
                    "turn_no": turn_no,
                    "result_status": result_status,
                    "command_id": command_id,
                    "player_id": player_id,
                    "command_type": command_type,
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
    matched=1
    break
  else
    exit_code="$?"
    if [[ "${exit_code}" -eq 2 ]]; then
      if [[ -f "${TMP_DIR}/ddb_candidates.json" ]]; then
        last_candidates="$(cat "${TMP_DIR}/ddb_candidates.json")"
      fi
      echo "last_b_player_candidates=${last_candidates}"
      "${COMPOSE[@]}" logs --since=10m bot-service | tail -n 200 || true
      exit 1
    fi
  fi

  if [[ -f "${TMP_DIR}/ddb_candidates.json" ]]; then
    last_candidates="$(cat "${TMP_DIR}/ddb_candidates.json")"
  fi
  sleep "${POLL_INTERVAL_SECONDS}"
done

if [[ "${matched}" -ne 1 ]]; then
  echo "E2E_FAIL: did not observe a parsed command for player B"
  echo "game_id=${GAME_ID}"
  echo "player_b_id=${PLAYER_B_ID}"
  echo "last_b_player_candidates=${last_candidates}"
  "${COMPOSE[@]}" logs --since=10m bot-service | tail -n 200 || true
  exit 1
fi

echo "deepagents_json_output_parsing=PASS"
