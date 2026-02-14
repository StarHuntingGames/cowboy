# Cow Boy V2 Task List

Last updated: 2026-02-09

Status legend:
- [ ] Todo
- [-] In Progress
- [x] Done

## 1) Planning and Contracts
- [x] Finalize V2 architecture and service responsibilities
- [x] Finalize API contracts for all services
- [x] Finalize Kafka command/step message contracts
- [x] Finalize DynamoDB table design
- [ ] Review V2 design doc for final consistency before coding

## 2) Frontend (keep current design + new state UI)
- [x] Add game state bar (status, game id, turn, round, current player)
- [-] Add turn + timeout panel (countdown + progress + timeout flash)
- [x] Wire Start button to V2 start-game API
- [x] Wire command submit to Web Service API
- [x] Add snapshot bootstrap and websocket sync flow
- [x] Add reconnect/resync behavior
- [x] Show finish winner message/animation from websocket and snapshot state

## 3) Backend - Game Manager Service (Rust)
- [x] Bootstrap Rust service project structure
- [x] Implement create game API (with custom map)
- [x] Implement create game API (without map -> default map flow)
- [-] Implement default map generation and persistence
- [x] Implement start game API
- [-] Emit game started event for watcher broadcast path
- [x] Add internal finish endpoint for game-service-controlled finish transition
- [x] Add AWS Lambda runtime compatibility (`aws-lambda-rust-runtime`)

## 4) Backend - Web Service (Rust)
- [x] Bootstrap Rust service project structure
- [x] Implement submit command API
- [-] Validate request schema and publish to Kafka
- [ ] Add idempotency handling for command_id (if required by final contract)
- [x] Add AWS Lambda runtime compatibility (`aws-lambda-rust-runtime`)

## 5) Backend - Game Service (Rust)
- [x] Bootstrap Rust service project structure
- [x] Add HTTP command processing endpoint for current local integration
- [x] Mark game as `FINISHED` when one alive player remains
- [ ] Consume commands from Kafka
- [ ] Load current state and validate command
- [ ] Apply game rules and produce state transition
- [ ] Persist one step record per consumed command to DynamoDB
- [ ] Publish step event to Kafka
- [ ] Handle timeout and ignored-timeout command behavior

## 6) Backend - Timer Service (Rust)
- [x] Bootstrap Rust service project structure
- [ ] Start/reset per-turn timer based on applied step events
- [ ] Publish timeout command when timer expires
- [ ] Ensure old timers are cancelled on turn change

## 7) Backend - Game Watcher Service (Rust)
- [x] Bootstrap Rust service project structure
- [x] Implement latest snapshot API
- [x] Implement websocket stream API (`from_turn_no`)
- [-] Stream valid applied events to clients
- [x] Broadcast game started lifecycle event
- [x] Broadcast game finished lifecycle event
- [x] Add AWS Lambda runtime compatibility (`aws-lambda-rust-runtime`)

## 8) Infrastructure and Storage
- [-] Create DynamoDB table: `default_maps`
- [-] Create DynamoDB table: `game_instances`
- [-] Create DynamoDB table: `game_steps`
- [ ] Add needed indexes (for command dedupe/querying)
- [-] Configure Kafka topics and retention

## 9) Integration and Testing
- [x] Add first-pass unit tests for shared models and API validation paths
- [x] Create consolidated test case document for manual + API verification
- [x] End-to-end test: create -> start -> play turns
- [x] End-to-end test: finish transition + websocket `GAME_FINISHED` event
- [ ] End-to-end test: timeout flow and ignored late command
- [ ] End-to-end test: watcher snapshot + stream sync
- [ ] End-to-end test: reconnect from `from_turn_no`
- [ ] Load/perf sanity checks for command throughput

## 10) Delivery
- [x] Update docs for local run/deploy
- [x] Add docker-compose stack for one-command local startup
- [x] Add Makefile shortcuts for common setup/run commands
- [ ] Prepare V2 release checklist
- [ ] Final acceptance test with frontend

## Task Update Policy
- Update this file whenever a task status changes.
- Move tasks from `[ ]` to `[-]` when started.
- Move tasks from `[-]` to `[x]` when completed.
- Add new tasks immediately when scope changes.
