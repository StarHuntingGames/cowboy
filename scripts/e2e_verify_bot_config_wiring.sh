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

TMP_DIR="$(mktemp -d)"
GAME_ID=""

cleanup() {
  if [[ -n "${GAME_ID}" ]]; then
    curl -sS -X POST \
      -H "content-type: application/json" \
      "${BOT_MANAGER_BASE_URL}/internal/v3/games/${GAME_ID}/bots/stop" \
      -d '{"reason":"E2E_VERIFY_CONFIG_WIRING_CLEANUP"}' >/dev/null || true
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

docker compose up -d bot-manager-service >/dev/null

wait_http_ok "${MANAGER_BASE_URL}/health"
wait_http_ok "${BOT_MANAGER_BASE_URL}/health"
wait_http_ok "${BOT_SERVICE_BASE_URL}/health"

python3 - <<'PY' >"${TMP_DIR}/expected_bot_manager_llm.json"
import json
import pathlib
import yaml

cfg = yaml.safe_load(pathlib.Path("bot-manager-llm.yaml").read_text()) or {}
default = cfg.get("default") or {}
players = cfg.get("players") or {}

def normalize(value):
    if value is None:
        return None
    text = str(value).strip()
    return text if text else None

resolved = {}
for player_name in ("B", "C", "D"):
    override = players.get(player_name) or {}
    resolved[player_name] = {
        "base_url": normalize(override.get("base_url")) or normalize(default.get("base_url")),
        "model": normalize(override.get("model")) or normalize(default.get("model")),
        "api_key": normalize(override.get("api_key")) or normalize(default.get("api_key")),
    }

print(json.dumps(resolved, ensure_ascii=True))
PY

python3 - <<'PY' >"${TMP_DIR}/expected_langsmith_env.json"
import json
import pathlib
import yaml

cfg = yaml.safe_load(pathlib.Path("bot-service-langsmith.yaml").read_text()) or {}
enabled = bool(cfg.get("enabled", True))
env = {
    "LANGSMITH_TRACING": str(enabled).lower(),
    "LANGCHAIN_TRACING_V2": str(enabled).lower(),
}

def add(key, value):
    text = "" if value is None else str(value).strip()
    if text:
        env[key] = text

add("LANGSMITH_API_KEY", cfg.get("api_key"))
add("LANGSMITH_ENDPOINT", cfg.get("endpoint"))
add("LANGSMITH_PROJECT", cfg.get("project"))
add("LANGSMITH_WORKSPACE_ID", cfg.get("workspace_id"))

extra_env = cfg.get("extra_env") or {}
if isinstance(extra_env, dict):
    for key, value in extra_env.items():
        key_text = str(key).strip()
        value_text = str(value).strip()
        if key_text and value_text:
            env[key_text] = value_text

print(json.dumps(env, ensure_ascii=True))
PY

create_game_response="$(
  curl -fsS \
    -X POST \
    -H "content-type: application/json" \
    "${MANAGER_BASE_URL}/v2/games" \
    -d '{"turn_timeout_seconds":10}'
)"
GAME_ID="$(printf '%s' "${create_game_response}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["game_id"])')"

assignments_response=""
for _ in $(seq 1 45); do
  response="$(curl -sS "${BOT_MANAGER_BASE_URL}/internal/v3/games/${GAME_ID}/assignments" || true)"
  if [[ -n "${response}" ]]; then
    count="$(printf '%s' "${response}" | python3 -c 'import json,sys
try:
    payload=json.load(sys.stdin)
    print(len(payload.get("bindings", [])))
except Exception:
    print(0)')"
    if [[ "${count}" -ge 3 ]]; then
      assignments_response="${response}"
      break
    fi
  fi
  sleep 1
done

if [[ -z "${assignments_response}" ]]; then
  echo "E2E_FAIL: bot assignments not ready for game ${GAME_ID}"
  exit 1
fi

printf '%s' "${assignments_response}" >"${TMP_DIR}/assignments.json"

python3 - "${BOT_SERVICE_BASE_URL}" "${TMP_DIR}/assignments.json" "${TMP_DIR}/expected_bot_manager_llm.json" <<'PY'
import json
import sys
import urllib.request

bot_service_base_url = sys.argv[1].rstrip("/")
assignments = json.load(open(sys.argv[2]))
expected = json.load(open(sys.argv[3]))

errors = []
for binding in assignments.get("bindings", []):
    player_name = binding.get("player_name")
    bot_id = binding.get("bot_id")
    if player_name not in expected:
        continue
    req = urllib.request.Request(f"{bot_service_base_url}/internal/v3/bots/{bot_id}", method="GET")
    with urllib.request.urlopen(req, timeout=15) as resp:
        info = json.load(resp)
    exp = expected[player_name]
    actual_base = info.get("llm_base_url")
    actual_model = info.get("llm_model")
    if (actual_base or None) != exp.get("base_url"):
        errors.append(f"player {player_name} bot {bot_id} llm_base_url mismatch expected={exp.get('base_url')} actual={actual_base}")
    if (actual_model or None) != exp.get("model"):
        errors.append(f"player {player_name} bot {bot_id} llm_model mismatch expected={exp.get('model')} actual={actual_model}")

if errors:
    print("E2E_FAIL: bot-service bot info does not match bot-manager-llm.yaml")
    for entry in errors:
        print(entry)
    raise SystemExit(1)

print("bot_info_check=PASS")
PY

expression_values="$(printf '{\":g\":{\"S\":\"%s\"}}' "${GAME_ID}")"
ddb_raw="$(
  docker compose run --rm -T --entrypoint '' dynamodb-init \
    aws dynamodb query \
      --endpoint-url http://dynamodb:8000 \
      --table-name bot_players \
      --key-condition-expression 'game_id = :g' \
      --expression-attribute-values "${expression_values}" \
      --scan-index-forward 2>&1 || true
)"
printf '%s' "${ddb_raw}" >"${TMP_DIR}/ddb_bot_players_raw.txt"

python3 - "${TMP_DIR}/ddb_bot_players_raw.txt" "${TMP_DIR}/expected_bot_manager_llm.json" <<'PY'
import json
import pathlib
import sys

raw = pathlib.Path(sys.argv[1]).read_text()
expected = json.load(open(sys.argv[2]))
first = raw.find("{")
if first < 0:
    print("E2E_FAIL: DynamoDB query returned no JSON payload")
    raise SystemExit(1)

payload = json.loads(raw[first:])
items = payload.get("Items", [])
if len(items) < 3:
    print(f"E2E_FAIL: expected at least 3 bot_players records, got {len(items)}")
    raise SystemExit(1)

errors = []
seen = set()
for item in items:
    player_name = item.get("player_name", {}).get("S", "")
    if player_name not in expected:
        continue
    seen.add(player_name)
    exp = expected[player_name]
    actual_base = item.get("base_url", {}).get("S")
    actual_model = item.get("model", {}).get("S")
    actual_key = item.get("api_key", {}).get("S")
    if (actual_base or None) != exp.get("base_url"):
        errors.append(f"DDB player {player_name} base_url mismatch")
    if (actual_model or None) != exp.get("model"):
        errors.append(f"DDB player {player_name} model mismatch")
    if (actual_key or None) != exp.get("api_key"):
        errors.append(f"DDB player {player_name} api_key mismatch")

missing = sorted(set(expected.keys()) - seen)
if missing:
    errors.append(f"DDB missing expected players: {','.join(missing)}")

if errors:
    print("E2E_FAIL: bot_players table does not match bot-manager-llm.yaml")
    for entry in errors:
        print(entry)
    raise SystemExit(1)

print("bot_players_table_check=PASS")
PY

for _ in $(seq 1 40); do
  count="$(docker compose exec -T bot-service sh -lc 'pgrep -f player_agent.py | wc -l' 2>/dev/null | tr -d ' ')"
  if [[ "${count}" -ge 3 ]]; then
    break
  fi
  sleep 1
done

agent_env_json="$(
  docker compose exec -T bot-service python3 - <<'PY'
import json
import os
from pathlib import Path

def read_text(path: str) -> str:
    return Path(path).read_text(errors="replace")

result = []
for pid_name in sorted(os.listdir("/proc")):
    if not pid_name.isdigit():
        continue
    cmdline_path = f"/proc/{pid_name}/cmdline"
    environ_path = f"/proc/{pid_name}/environ"
    try:
        cmdline = read_text(cmdline_path).replace("\x00", " ").strip()
    except Exception:
        continue
    if "player_agent.py" not in cmdline:
        continue
    try:
        environ_raw = read_text(environ_path)
    except Exception:
        continue
    env_map = {}
    for entry in environ_raw.split("\x00"):
        if "=" not in entry:
            continue
        key, value = entry.split("=", 1)
        env_map[key] = value
    result.append(
        {
            "pid": int(pid_name),
            "cmdline": cmdline,
            "env": env_map,
        }
    )

print(json.dumps(result, ensure_ascii=True))
PY
)"
printf '%s' "${agent_env_json}" >"${TMP_DIR}/agent_env.json"

python3 - "${TMP_DIR}/expected_langsmith_env.json" "${TMP_DIR}/agent_env.json" <<'PY'
import json
import sys

expected = json.load(open(sys.argv[1]))
agents = json.load(open(sys.argv[2]))

if len(agents) < 1:
    print("E2E_FAIL: no running player_agent.py process found in bot-service")
    raise SystemExit(1)

errors = []
for agent in agents:
    env = agent.get("env", {})
    for key, expected_value in expected.items():
        actual_value = env.get(key)
        if actual_value != expected_value:
            errors.append(
                f"pid {agent.get('pid')} missing/mismatch env {key} expected={expected_value!r} actual={actual_value!r}"
            )

if errors:
    print("E2E_FAIL: player-agent env does not match bot-service-langsmith.yaml")
    for entry in errors:
        print(entry)
    raise SystemExit(1)

print("player_agent_langsmith_env_check=PASS")
PY

echo "E2E_PASS"
echo "game_id=${GAME_ID}"
echo "verified_files=bot-manager-llm.yaml,bot-service-langsmith.yaml"
