// Copyright (C) 2026 StarHuntingGames
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::Context;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{Client as DynamoClient, types::AttributeValue};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use cowboy_common::{
    CommandEnvelope, CommandSource, CommandType, Direction, GameInstanceResponse, GameStatus,
    PlayerId, ResultStatus, StepEvent, StepEventType, SubmitCommandRequest,
};
use rdkafka::{
    Message,
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
    producer::{FutureProducer, FutureRecord},
};
use serde::{Deserialize, Serialize};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    manager_base_url: String,
    kafka: KafkaSettings,
    producer: FutureProducer,
    dedupe: Arc<tokio::sync::Mutex<HashMap<String, HashSet<String>>>>,
    step_seq: Arc<AtomicU64>,
    step_store: Option<DynamoStepStore>,
    game_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

#[derive(Clone)]
struct KafkaSettings {
    input_topic_prefix: String,
    output_topic_prefix: String,
    bootstrap_servers: String,
    consumer_group_id: String,
}

#[derive(Clone)]
struct DynamoStepStore {
    client: DynamoClient,
    table_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyCommandResponse {
    accepted: bool,
    applied: bool,
    reason: Option<String>,
    turn_no: u64,
    round_no: u64,
    current_player_id: PlayerId,
    status: GameStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FinishGameRequest {
    expected_turn_no: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct FinishGameResponse {
    finished: bool,
    reason: Option<String>,
    status: GameStatus,
    winner_player_id: Option<PlayerId>,
    turn_no: u64,
    round_no: u64,
    current_player_id: PlayerId,
}

#[derive(Debug, Clone)]
struct ProcessedOutcome {
    accepted: bool,
    applied: bool,
    reason: Option<String>,
    game: GameInstanceResponse,
    #[allow(dead_code)]
    result_status: ResultStatus,
}

impl AppState {
    async fn from_env() -> anyhow::Result<Self> {
        let kafka = KafkaSettings {
            bootstrap_servers: std::env::var("KAFKA_BOOTSTRAP_SERVERS")
                .ok()
                .unwrap_or_else(|| "kafka:9092".to_string()),
            input_topic_prefix: std::env::var("GAME_INPUT_TOPIC_PREFIX")
                .ok()
                .unwrap_or_else(|| "game.commands".to_string()),
            output_topic_prefix: std::env::var("GAME_OUTPUT_TOPIC_PREFIX")
                .ok()
                .unwrap_or_else(|| "game.output".to_string()),
            consumer_group_id: std::env::var("GAME_SERVICE_CONSUMER_GROUP_ID")
                .ok()
                .unwrap_or_else(|| "game-service-v1".to_string()),
        };

        let producer = ClientConfig::new()
            .set("bootstrap.servers", &kafka.bootstrap_servers)
            .set("message.timeout.ms", "5000")
            .create()
            .context("failed to create Kafka producer in game-service")?;

        let step_store =
            if std::env::var("DYNAMODB_ENDPOINT").is_ok() || std::env::var("AWS_REGION").is_ok() {
                let mut loader = aws_config::defaults(BehaviorVersion::latest());
                if let Ok(endpoint) = std::env::var("DYNAMODB_ENDPOINT") {
                    loader = loader.endpoint_url(endpoint);
                }
                let config = loader.load().await;
                Some(DynamoStepStore {
                    client: DynamoClient::new(&config),
                    table_name: std::env::var("GAME_STEPS_TABLE")
                        .ok()
                        .unwrap_or_else(|| "game_steps".to_string()),
                })
            } else {
                None
            };

        Ok(Self {
            client: reqwest::Client::new(),
            manager_base_url: std::env::var("GAME_MANAGER_BASE_URL")
                .ok()
                .unwrap_or_else(|| "http://game-manager-service:8081".to_string()),
            kafka,
            producer,
            dedupe: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            step_seq: Arc::new(AtomicU64::new(
                Utc::now().timestamp_micros().unsigned_abs().max(1),
            )),
            step_store,
            game_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    async fn game_lock(&self, game_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.game_locks.lock().await;
        locks
            .entry(game_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn next_step_seq(&self) -> u64 {
        self.step_seq.fetch_add(1, Ordering::Relaxed)
    }

    fn input_topic_pattern(&self) -> String {
        format!(
            "^{}\\..*\\.v1$",
            self.kafka.input_topic_prefix.replace('.', "\\.")
        )
    }

    fn output_topic_for_game(&self, game_id: &str) -> String {
        format!("{}.{}.v1", self.kafka.output_topic_prefix, game_id)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "game_service=debug,tower_http=info".to_string()),
        )
        .init();

    let state = AppState::from_env().await?;

    let app = build_router(state.clone());
    let lambda_mode = std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok();
    if !lambda_mode {
        let consumer_state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = run_command_consumer(consumer_state).await {
                warn!(error = %error, "game-service command consumer stopped");
            }
        });
    }

    let bind_addr = parse_bind_addr("GAME_SERVICE_BIND", "0.0.0.0:8084")?;
    info!(%bind_addr, "game-service listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/internal/v2/games/{game_id}/commands/process",
            post(process_command_handler),
        )
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

fn parse_bind_addr(var_name: &str, default: &str) -> anyhow::Result<SocketAddr> {
    let value = std::env::var(var_name)
        .ok()
        .unwrap_or_else(|| default.to_string());
    value.parse().context(format!("invalid {var_name}"))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true, "service": "game-service"}))
}

async fn process_command_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<SubmitCommandRequest>,
) -> Result<Json<ApplyCommandResponse>, ApiError> {
    let command = CommandEnvelope {
        command_id: request.command_id.clone(),
        source: CommandSource::User,
        game_id,
        player_id: Some(request.player_id),
        command_type: request.command_type,
        direction: request.direction,
        speak_text: request.speak_text.clone(),
        turn_no: request.turn_no,
        sent_at: request.client_sent_at,
    };
    let lock = state.game_lock(&command.game_id).await;
    let _guard = lock.lock().await;
    let outcome = process_command(&state, command).await?;
    Ok(Json(ApplyCommandResponse {
        accepted: outcome.accepted,
        applied: outcome.applied,
        reason: outcome.reason,
        turn_no: outcome.game.turn_no,
        round_no: outcome.game.round_no,
        current_player_id: outcome.game.current_player_id.clone(),
        status: outcome.game.status,
    }))
}

async fn run_command_consumer(state: AppState) -> anyhow::Result<()> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &state.kafka.bootstrap_servers)
        .set("group.id", &state.kafka.consumer_group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("topic.metadata.refresh.interval.ms", "1000")
        .set("topic.metadata.refresh.fast.interval.ms", "250")
        .create()
        .context("failed to create Kafka consumer in game-service")?;

    let pattern = state.input_topic_pattern();
    consumer
        .subscribe(&[&pattern])
        .context("failed to subscribe to game command topics")?;
    info!(pattern = %pattern, "game-service Kafka consumer subscribed");

    loop {
        let message = match consumer.recv().await {
            Ok(message) => message,
            Err(error) => {
                warn!(?error, "game-service Kafka receive error");
                tokio::time::sleep(Duration::from_millis(400)).await;
                continue;
            }
        };

        let payload = match message.payload() {
            Some(payload) => payload,
            None => {
                warn!("received empty Kafka payload in game-service");
                if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(?error, "failed to commit empty message");
                }
                continue;
            }
        };

        let command = match serde_json::from_slice::<CommandEnvelope>(payload) {
            Ok(command) => command,
            Err(error) => {
                warn!(?error, "invalid command payload in Kafka");
                if let Err(commit_err) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(?commit_err, "failed to commit invalid payload message");
                }
                continue;
            }
        };

        info!(
            game_id = %command.game_id,
            command_id = %command.command_id,
            command_type = ?command.command_type,
            source = ?command.source,
            player_id = command.player_id.as_deref().unwrap_or("none"),
            turn_no = command.turn_no,
            direction = ?command.direction,
            offset = message.offset(),
            "game-service received command from Kafka"
        );

        let lock = state.game_lock(&command.game_id).await;
        let _guard = lock.lock().await;
        match process_command(&state, command).await {
            Ok(outcome) => {
                info!(
                    game_id = %outcome.game.game_id,
                    accepted = outcome.accepted,
                    applied = outcome.applied,
                    reason = outcome.reason.as_deref().unwrap_or("none"),
                    result_status = ?outcome.result_status,
                    current_turn_no = outcome.game.turn_no,
                    current_player_id = %outcome.game.current_player_id,
                    "game-service processed command"
                );
            }
            Err(error) => {
                warn!(?error, "game-service failed to process command");
            }
        }
        drop(_guard);

        if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
            warn!(?error, "failed to commit consumed command message");
        }
    }
}

async fn process_command(
    state: &AppState,
    command: CommandEnvelope,
) -> Result<ProcessedOutcome, ApiError> {
    if command.command_type == CommandType::GameStarted {
        let game = manager_get_game(state, &command.game_id).await?;
        let event = build_step_event(
            state,
            &game,
            command,
            StepEventType::StepApplied,
            ResultStatus::InvalidCommand,
        );
        publish_and_persist(state, event, Some("RESERVED_COMMAND_TYPE")).await;
        return Ok(ProcessedOutcome {
            accepted: false,
            applied: false,
            reason: Some("RESERVED_COMMAND_TYPE".to_string()),
            game,
            result_status: ResultStatus::InvalidCommand,
        });
    }

    if is_duplicate_command(state, &command.game_id, &command.command_id).await {
        let game = manager_get_game(state, &command.game_id).await?;
        let event = build_step_event(
            state,
            &game,
            command,
            StepEventType::StepApplied,
            ResultStatus::DuplicateCommand,
        );
        publish_and_persist(state, event, Some("DUPLICATE_COMMAND")).await;
        return Ok(ProcessedOutcome {
            accepted: false,
            applied: false,
            reason: Some("DUPLICATE_COMMAND".to_string()),
            game,
            result_status: ResultStatus::DuplicateCommand,
        });
    }

    let before = manager_get_game(state, &command.game_id).await?;
    if before.status != GameStatus::Running {
        let event = build_step_event(
            state,
            &before,
            command,
            StepEventType::StepApplied,
            ResultStatus::InvalidTurn,
        );
        publish_and_persist(state, event, Some("GAME_NOT_RUNNING")).await;
        return Ok(ProcessedOutcome {
            accepted: false,
            applied: false,
            reason: Some("GAME_NOT_RUNNING".to_string()),
            game: before,
            result_status: ResultStatus::InvalidTurn,
        });
    }

    if command.command_type == CommandType::Timeout {
        return process_timeout_command(state, command, before).await;
    }

    process_user_command(state, command, before).await
}

async fn process_user_command(
    state: &AppState,
    mut command: CommandEnvelope,
    before: GameInstanceResponse,
) -> Result<ProcessedOutcome, ApiError> {
    if command.turn_no < before.turn_no {
        let event = build_step_event(
            state,
            &before,
            command,
            StepEventType::StepApplied,
            ResultStatus::IgnoredTimeout,
        );
        publish_and_persist(state, event, Some("LATE_COMMAND_IGNORED")).await;
        return Ok(ProcessedOutcome {
            accepted: false,
            applied: false,
            reason: Some("LATE_COMMAND_IGNORED".to_string()),
            game: before,
            result_status: ResultStatus::IgnoredTimeout,
        });
    }

    let player_id = command
        .player_id
        .clone()
        .ok_or_else(|| ApiError::bad_request("user command missing player_id".to_string()))?;

    let request = SubmitCommandRequest {
        command_id: command.command_id.clone(),
        player_id: player_id.clone(),
        command_type: command.command_type,
        direction: command.direction,
        speak_text: command.speak_text.clone(),
        turn_no: command.turn_no,
        client_sent_at: command.sent_at,
    };

    let mut apply = manager_apply_command(state, &command.game_id, &request).await?;

    // If the command was invalid (not stale turn, wrong player, dead, or game not running),
    // convert it to a speak command so the turn always advances.
    if !apply.applied {
        let is_convertible = !matches!(
            apply.reason.as_deref(),
            Some("STALE_TURN_NO")
                | Some("INVALID_TURN_PLAYER")
                | Some("PLAYER_DEAD")
                | Some("GAME_NOT_RUNNING")
        );

        if is_convertible {
            let original_desc = format_command_description(&command);
            let speak_text = format!("invalid command: \"{original_desc}\"");

            warn!(
                game_id = %command.game_id,
                player_id = %player_id,
                original_command = %original_desc,
                rejection_reason = apply.reason.as_deref().unwrap_or("unknown"),
                converted_speak_text = %speak_text,
                "converting invalid command to speak"
            );

            let speak_request = SubmitCommandRequest {
                command_id: command.command_id.clone(),
                player_id: player_id.clone(),
                command_type: CommandType::Speak,
                direction: None,
                speak_text: Some(speak_text.clone()),
                turn_no: command.turn_no,
                client_sent_at: command.sent_at,
            };

            apply = manager_apply_command(state, &command.game_id, &speak_request).await?;

            // Update the command envelope to reflect the conversion
            command.command_type = CommandType::Speak;
            command.speak_text = Some(speak_text);
            command.direction = None;
        }
    }

    let mut after = manager_get_game(state, &command.game_id).await?;

    let (result_status, event_reason) = if apply.applied {
        (ResultStatus::Applied, None)
    } else {
        match apply.reason.as_deref() {
            Some("STALE_TURN_NO") => (ResultStatus::IgnoredTimeout, Some("STALE_TURN_NO")),
            Some("INVALID_TURN_PLAYER") | Some("PLAYER_DEAD") | Some("GAME_NOT_RUNNING") => {
                (ResultStatus::InvalidTurn, apply.reason.as_deref())
            }
            _ => (ResultStatus::InvalidCommand, apply.reason.as_deref()),
        }
    };

    let event = build_step_event(
        state,
        &after,
        command.clone(),
        StepEventType::StepApplied,
        result_status,
    );
    publish_and_persist(state, event, event_reason).await;

    if apply.applied {
        let alive_players = after.state.players.iter().filter(|p| p.alive).count();
        if after.status != GameStatus::Finished && alive_players == 1 {
            let finish = manager_finish_game(state, &after.game_id, after.turn_no).await?;
            if finish.finished {
                info!(
                    game_id = %after.game_id,
                    winner = ?finish.winner_player_id,
                    turn_no = finish.turn_no,
                    "game-service marked game as FINISHED"
                );
                after = manager_get_game(state, &after.game_id).await?;
            } else {
                warn!(
                    game_id = %after.game_id,
                    reason = ?finish.reason,
                    "game-service finish request did not transition game"
                );
            }
        }
    }

    Ok(ProcessedOutcome {
        accepted: apply.accepted,
        applied: apply.applied,
        reason: apply.reason,
        game: after,
        result_status,
    })
}

fn format_command_description(command: &CommandEnvelope) -> String {
    let cmd_type = match command.command_type {
        CommandType::Move => "move",
        CommandType::Shield => "shield",
        CommandType::Shoot => "shoot",
        CommandType::Speak => "speak",
        CommandType::Timeout => "timeout",
        CommandType::GameStarted => "game_started",
    };

    let dir = command.direction.map(|d| match d {
        Direction::Up => "up",
        Direction::Down => "down",
        Direction::Left => "left",
        Direction::Right => "right",
    });

    match dir {
        Some(d) => format!("{cmd_type} {d}"),
        None => cmd_type.to_string(),
    }
}

async fn process_timeout_command(
    state: &AppState,
    command: CommandEnvelope,
    before: GameInstanceResponse,
) -> Result<ProcessedOutcome, ApiError> {
    if command.turn_no < before.turn_no {
        let event = build_step_event(
            state,
            &before,
            command,
            StepEventType::StepApplied,
            ResultStatus::IgnoredTimeout,
        );
        publish_and_persist(state, event, Some("LATE_TIMEOUT_IGNORED")).await;
        return Ok(ProcessedOutcome {
            accepted: false,
            applied: false,
            reason: Some("LATE_TIMEOUT_IGNORED".to_string()),
            game: before,
            result_status: ResultStatus::IgnoredTimeout,
        });
    }

    let player_id = command
        .player_id
        .clone()
        .unwrap_or_else(|| before.current_player_id.clone());
    let request = SubmitCommandRequest {
        command_id: command.command_id.clone(),
        player_id,
        command_type: CommandType::Timeout,
        direction: None,
        speak_text: None,
        turn_no: command.turn_no,
        client_sent_at: command.sent_at,
    };

    let apply = manager_apply_command(state, &command.game_id, &request).await?;
    let after = manager_get_game(state, &command.game_id).await?;
    let (event_type, result_status, event_reason) = if apply.applied {
        (
            StepEventType::TimeoutApplied,
            ResultStatus::TimeoutApplied,
            None,
        )
    } else {
        match apply.reason.as_deref() {
            Some("STALE_TURN_NO") | Some("INVALID_TURN_PLAYER") => (
                StepEventType::StepApplied,
                ResultStatus::IgnoredTimeout,
                apply.reason.as_deref(),
            ),
            _ => (
                StepEventType::StepApplied,
                ResultStatus::InvalidTurn,
                apply.reason.as_deref(),
            ),
        }
    };

    let event = build_step_event(state, &after, command, event_type, result_status);
    publish_and_persist(state, event, event_reason).await;

    Ok(ProcessedOutcome {
        accepted: apply.accepted,
        applied: apply.applied,
        reason: apply.reason,
        game: after,
        result_status,
    })
}

fn build_step_event(
    state: &AppState,
    game: &GameInstanceResponse,
    command: CommandEnvelope,
    event_type: StepEventType,
    result_status: ResultStatus,
) -> StepEvent {
    StepEvent {
        game_id: game.game_id.clone(),
        step_seq: state.next_step_seq(),
        turn_no: game.turn_no,
        round_no: game.round_no,
        event_type,
        result_status,
        command: Some(command),
        state_after: game.state.clone(),
        created_at: Utc::now(),
    }
}

async fn publish_and_persist(state: &AppState, step: StepEvent, reason: Option<&str>) {
    let topic = state.output_topic_for_game(&step.game_id);
    if let Err(error) = publish_step_event(state, &topic, &step).await {
        warn!(game_id = %step.game_id, topic = %topic, error = %error, "failed to publish step event");
    }
    if let Some(store) = state.step_store.as_ref()
        && let Err(error) = persist_step_record(store, &step, reason).await
    {
        warn!(game_id = %step.game_id, error = %error, "failed to persist step record");
    }
}

async fn publish_step_event(state: &AppState, topic: &str, step: &StepEvent) -> anyhow::Result<()> {
    let payload = serde_json::to_string(step).context("failed to encode step event")?;
    state
        .producer
        .send(
            FutureRecord::to(topic).key(&step.game_id).payload(&payload),
            Duration::from_secs(5),
        )
        .await
        .map_err(|(error, _)| anyhow::anyhow!("Kafka publish failed: {error:?}"))?;
    Ok(())
}

async fn persist_step_record(
    store: &DynamoStepStore,
    step: &StepEvent,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    let command_id = step
        .command
        .as_ref()
        .map(|command| command.command_id.clone())
        .unwrap_or_else(|| format!("system-{}-{}", step.game_id, step.step_seq));
    let source = step
        .command
        .as_ref()
        .map(|command| serde_json::to_string(&command.source))
        .transpose()?
        .unwrap_or_else(|| "\"system\"".to_string());
    let command_type = step
        .command
        .as_ref()
        .map(|command| serde_json::to_string(&command.command_type))
        .transpose()?
        .unwrap_or_else(|| "\"game_started\"".to_string());
    let player_id = step
        .command
        .as_ref()
        .and_then(|command| command.player_id.clone())
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let direction = step
        .command
        .as_ref()
        .and_then(|command| command.direction)
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let speak_text = step
        .command
        .as_ref()
        .and_then(|command| command.speak_text.clone());

    let mut item = HashMap::new();
    item.insert(
        "game_id".to_string(),
        AttributeValue::S(step.game_id.clone()),
    );
    item.insert(
        "step_seq".to_string(),
        AttributeValue::N(step.step_seq.to_string()),
    );
    item.insert(
        "turn_no".to_string(),
        AttributeValue::N(step.turn_no.to_string()),
    );
    item.insert(
        "round_no".to_string(),
        AttributeValue::N(step.round_no.to_string()),
    );
    item.insert("command_id".to_string(), AttributeValue::S(command_id));
    item.insert(
        "source".to_string(),
        AttributeValue::S(source.trim_matches('"').to_string()),
    );
    item.insert(
        "command_type".to_string(),
        AttributeValue::S(command_type.trim_matches('"').to_string()),
    );
    item.insert(
        "event_type".to_string(),
        AttributeValue::S(
            serde_json::to_string(&step.event_type)?
                .trim_matches('"')
                .to_string(),
        ),
    );
    item.insert(
        "result_status".to_string(),
        AttributeValue::S(
            serde_json::to_string(&step.result_status)?
                .trim_matches('"')
                .to_string(),
        ),
    );
    item.insert(
        "state_after".to_string(),
        AttributeValue::S(serde_json::to_string(&step.state_after)?),
    );
    item.insert(
        "created_at".to_string(),
        AttributeValue::S(step.created_at.to_rfc3339()),
    );
    if let Some(value) = player_id {
        item.insert(
            "player_id".to_string(),
            AttributeValue::S(value.trim_matches('"').to_string()),
        );
    }
    if let Some(value) = direction {
        item.insert(
            "direction".to_string(),
            AttributeValue::S(value.trim_matches('"').to_string()),
        );
    }
    if let Some(value) = speak_text
        && !value.trim().is_empty()
    {
        item.insert("speak_text".to_string(), AttributeValue::S(value));
    }
    if let Some(value) = reason {
        item.insert(
            "result_reason".to_string(),
            AttributeValue::S(value.to_string()),
        );
    }

    store
        .client
        .put_item()
        .table_name(&store.table_name)
        .set_item(Some(item))
        .send()
        .await
        .context("failed to put item into game_steps table")?;
    Ok(())
}

async fn is_duplicate_command(state: &AppState, game_id: &str, command_id: &str) -> bool {
    let mut dedupe = state.dedupe.lock().await;
    let set = dedupe
        .entry(game_id.to_string())
        .or_insert_with(HashSet::new);
    !set.insert(command_id.to_string())
}

async fn manager_apply_command(
    state: &AppState,
    game_id: &str,
    request: &SubmitCommandRequest,
) -> Result<ApplyCommandResponse, ApiError> {
    let url = format!(
        "{}/internal/v2/games/{}/commands/apply",
        state.manager_base_url, game_id
    );

    let response = state
        .client
        .post(url)
        .json(request)
        .send()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("manager apply request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "manager apply returned {}: {}",
            status, body
        )));
    }

    response
        .json::<ApplyCommandResponse>()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("invalid manager apply response: {e}")))
}

async fn manager_get_game(
    state: &AppState,
    game_id: &str,
) -> Result<GameInstanceResponse, ApiError> {
    let url = format!("{}/v2/games/{}", state.manager_base_url, game_id);

    let response = state
        .client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("manager get game request failed: {e}")))?;

    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found(format!("game {} not found", game_id)));
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "manager get game returned {}: {}",
            status, body
        )));
    }

    response
        .json::<GameInstanceResponse>()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("invalid manager game response: {e}")))
}

async fn manager_finish_game(
    state: &AppState,
    game_id: &str,
    turn_no: u64,
) -> Result<FinishGameResponse, ApiError> {
    let url = format!(
        "{}/internal/v2/games/{}/finish",
        state.manager_base_url, game_id
    );

    let response = state
        .client
        .post(url)
        .json(&FinishGameRequest {
            expected_turn_no: Some(turn_no),
        })
        .send()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("manager finish request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "manager finish returned {}: {}",
            status, body
        )));
    }

    response
        .json::<FinishGameResponse>()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("invalid manager finish response: {e}")))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        warn!(status = %self.status, message = %self.message, "request failed");
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}
