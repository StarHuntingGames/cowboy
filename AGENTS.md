# Repository Guidelines

## Project Structure & Module Organization
`backend/` is a Rust workspace with service crates (`game-manager-service`, `web-service`, `game-service`, `timer-service`, `game-watcher-service`, `bot-manager-service`, `bot-service`) plus shared models in `cowboy-common/`.  
`backend/bot-service/python/` contains the Python player agent; `backend/bot-service/python/tests/` and `backend/bot-service-test/` contain Python tests.  
`frontend/` is a static HTML/JS app (`index.html`, `game.js`) with assets in `frontend/assets/`.  
`conf/` stores runtime YAML config, `scripts/` holds E2E and utility scripts, `mcp/` contains the MCP server package, and `docs/` tracks architecture/rules/test cases.

## Build, Test, and Development Commands
- `make up`: build and start the full Docker Compose stack.
- `make down` / `make clean`: stop services (clean also removes volumes).
- `make logs` / `make ps`: inspect running services and logs.
- `make backend-fmt`: run `cargo fmt` across `backend/`.
- `make backend-check`: run `cargo check` for Rust services.
- `cargo test --manifest-path backend/Cargo.toml`: run Rust tests.
- `.venv/bin/pytest backend/bot-service/python/tests/`: run Python bot-agent tests.
- `make e2e-llm-failure-speak` (or other `make e2e-*` targets): run integration/E2E checks.

## Coding Style & Naming Conventions
Rust uses workspace defaults (edition 2024) and should always be formatted with `cargo fmt`; follow idiomatic naming (`snake_case` functions/modules, `PascalCase` types).  
Python follows PEP 8 with 4-space indentation, type hints, and `snake_case` identifiers.  
Frontend JavaScript in `frontend/game.js` uses 2-space indentation and `camelCase` names.  
Preserve existing license headers in source files when editing.

## Testing Guidelines
Keep Rust unit tests near implementation using `#[cfg(test)]` and `#[tokio::test]` where async behavior is involved.  
Name Python tests `test_*.py`; place unit/integration tests under `backend/bot-service/python/tests/` and cross-service scenarios under `backend/bot-service-test/`.  
Before opening a PR, run relevant Rust + Python tests and at least one affected E2E script if behavior crosses service boundaries.

## Commit & Pull Request Guidelines
Recent history favors short imperative commit subjects (for example, `add mcp service`, `Update README.md`). Keep subjects concise and scoped to one change.  
PRs should include:
- what changed and why,
- services/configs touched (`conf/*.yaml`, `.env` expectations),
- validation evidence (command list + outcomes),
- UI screenshots or short recordings for frontend-visible changes.

## Security & Configuration Tips
Never commit secrets. Keep API keys in `.env` (`BOT_LLM_API_KEY`, `LANGSMITH_API_KEY`) and use `${...}` placeholders in config YAML files.
