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

COMPOSE := docker compose
COMPOSE_FILE := docker-compose.yml

.PHONY: all up down restart build logs ps clean init restart-bot restart-bot-manager backend-fmt backend-check e2e-llm-failure-speak e2e-llm-failure-speak-ci e2e-verify-bot-config-wiring e2e-llm-connection-test live-llm-decide integration-live-llm-output integration-update-timeout

all: up

up:
	$(COMPOSE) -f $(COMPOSE_FILE) up --build -d

down:
	$(COMPOSE) -f $(COMPOSE_FILE) down --remove-orphans

clean.sh:
	echo ~/clean.sh

restart: down clean.sh up

build:
	$(COMPOSE) -f $(COMPOSE_FILE) build

logs:
	$(COMPOSE) -f $(COMPOSE_FILE) logs -f --tail=200

ps:
	$(COMPOSE) -f $(COMPOSE_FILE) ps

clean:
	$(COMPOSE) -f $(COMPOSE_FILE) down -v --remove-orphans

restart-bot:
	$(COMPOSE) -f $(COMPOSE_FILE) up --build --no-deps -d bot-service

restart-bot-manager:
	$(COMPOSE) -f $(COMPOSE_FILE) restart bot-manager-service

init:
	$(COMPOSE) -f $(COMPOSE_FILE) up -d zookeeper kafka dynamodb
	$(COMPOSE) -f $(COMPOSE_FILE) up --abort-on-container-exit kafka-init dynamodb-init

backend-fmt:
	cargo fmt --all --manifest-path backend/Cargo.toml

backend-check:
	cargo check --manifest-path backend/Cargo.toml

e2e-llm-failure-speak:
	./scripts/e2e_llm_failure_speak.sh

e2e-llm-failure-speak-ci:
	WS_TIMEOUT_SECONDS=45 DDB_WAIT_SECONDS=12 DDB_POLL_INTERVAL_SECONDS=1 ./scripts/e2e_llm_failure_speak.sh

e2e-verify-bot-config-wiring:
	./scripts/e2e_verify_bot_config_wiring.sh

e2e-llm-connection-test:
	./scripts/e2e_llm_connection_test.sh

live-llm-decide:
	./scripts/live_llm_decide.sh

integration-live-llm-output:
	./.venv/bin/pytest backend/bot-service/python/tests/test_player_agent_integration_live_output.py

integration-update-timeout:
	./.venv/bin/pytest backend/bot-service/python/tests/test_player_agent_update_timeout.py
