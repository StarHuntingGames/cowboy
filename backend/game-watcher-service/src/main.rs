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

use std::{net::SocketAddr, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use cowboy_common::{
    CommandType, Direction, GameInstanceResponse, GameStatus, PlayerId, ResultStatus,
    SnapshotResponse, StepEvent, StepEventType,
};
use lambda_http::run as lambda_run;
use rdkafka::{
    Message,
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{broadcast, mpsc},
    time::{MissedTickBehavior, interval},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    manager_base_url: String,
    watch_events_tx: broadcast::Sender<WatcherBroadcastEvent>,
}

#[derive(Debug, Clone)]
enum WatcherBroadcastEvent {
    Timeout(TimeoutBroadcastEvent),
    Speak(SpeakBroadcastEvent),
    Shoot(ShootBroadcastEvent),
    GameFinished(GameFinishedBroadcastEvent),
}

#[derive(Debug, Clone)]
struct TimeoutBroadcastEvent {
    game_id: String,
    step_seq: u64,
    turn_no: u64,
    round_no: u64,
    player_id: Option<PlayerId>,
    result_status: ResultStatus,
    created_at: chrono::DateTime<Utc>,
    snapshot: Option<SnapshotResponse>,
}

#[derive(Debug, Clone)]
struct GameFinishedBroadcastEvent {
    game_id: String,
    step_seq: u64,
    turn_no: u64,
    round_no: u64,
    created_at: chrono::DateTime<Utc>,
    snapshot: Option<SnapshotResponse>,
}

#[derive(Debug, Clone)]
struct SpeakBroadcastEvent {
    game_id: String,
    step_seq: u64,
    turn_no: u64,
    round_no: u64,
    player_id: Option<PlayerId>,
    speak_text: String,
    created_at: chrono::DateTime<Utc>,
    snapshot: Option<SnapshotResponse>,
}

#[derive(Debug, Clone)]
struct ShootBroadcastEvent {
    game_id: String,
    step_seq: u64,
    turn_no: u64,
    round_no: u64,
    player_id: Option<PlayerId>,
    direction: Option<Direction>,
    command_id: String,
    created_at: chrono::DateTime<Utc>,
    snapshot: Option<SnapshotResponse>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "game_watcher_service=debug,tower_http=info".to_string()),
        )
        .init();

    let (watch_events_tx, _) = broadcast::channel(512);
    let state = AppState {
        client: reqwest::Client::new(),
        manager_base_url: std::env::var("GAME_MANAGER_BASE_URL")
            .ok()
            .unwrap_or_else(|| "http://game-manager-service:8081".to_string()),
        watch_events_tx,
    };

    let app = build_router(state.clone());
    let lambda_mode = std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok();

    if !lambda_mode {
        let kafka_state = state.clone();
        tokio::spawn(async move {
            run_output_consumer(kafka_state).await;
        });
    }

    if lambda_mode {
        info!("AWS Lambda runtime detected; running game-watcher-service in lambda mode");
        lambda_run(app)
            .await
            .map_err(|e| anyhow::Error::msg(format!("lambda runtime error: {e}")))?;
        return Ok(());
    }

    let bind_addr = parse_bind_addr("WATCHER_SERVICE_BIND", "0.0.0.0:8083")?;
    info!(%bind_addr, "game-watcher-service listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v2/games/{game_id}/snapshot", get(snapshot_handler))
        .route("/v2/games/{game_id}/stream", get(stream_handler))
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
    Json(serde_json::json!({"ok": true, "service": "game-watcher-service"}))
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    from_turn_no: Option<u64>,
}

async fn snapshot_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    let snapshot = fetch_snapshot(&state, &game_id).await?;

    if let Some(from_turn_no) = query.from_turn_no {
        info!(
            game_id = %game_id,
            from_turn_no,
            snapshot_turn_no = snapshot.turn_no,
            "snapshot requested"
        );
    }

    Ok(Json(snapshot))
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    from_turn_no: Option<u64>,
}

async fn stream_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
    Path(game_id): Path<String>,
    Query(query): Query<StreamQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        handle_socket(socket, state, game_id, query.from_turn_no.unwrap_or(0))
    })
}

async fn handle_socket(
    mut socket: axum::extract::ws::WebSocket,
    state: AppState,
    game_id: String,
    from_turn_no: u64,
) {
    let connected = serde_json::json!({
        "event_type": "CONNECTED",
        "game_id": game_id,
        "from_turn_no": from_turn_no,
        "connected_at": Utc::now(),
        "message": "watch stream connected"
    })
    .to_string();

    if send_ws_event(&mut socket, &game_id, "CONNECTED", connected, None)
        .await
        .is_err()
    {
        return;
    }

    let mut watch_events_rx = state.watch_events_tx.subscribe();
    let mut last_sent_turn_no = from_turn_no;
    let mut last_status: Option<GameStatus> = None;
    let mut sent_initial = false;

    let mut ticker = interval(Duration::from_millis(800));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match fetch_snapshot(&state, &game_id).await {
                    Ok(snapshot) => {
                        let status_changed = match last_status {
                            Some(previous) => previous != snapshot.status,
                            None => true,
                        };
                        let turn_advanced = snapshot.turn_no > last_sent_turn_no;
                        let should_send = !sent_initial || turn_advanced || status_changed;

                        if should_send {
                            let event_type = if status_changed && last_status.is_some() {
                                if snapshot.status == GameStatus::Running {
                                    "GAME_STARTED"
                                } else if snapshot.status == GameStatus::Finished {
                                    "GAME_FINISHED"
                                } else {
                                    "SNAPSHOT"
                                }
                            } else {
                                "SNAPSHOT"
                            };

                            let event = serde_json::json!({
                                "event_type": event_type,
                                "game_id": game_id,
                                "snapshot": &snapshot,
                                "emitted_at": Utc::now()
                            })
                            .to_string();

                            if send_ws_event(
                                &mut socket,
                                &game_id,
                                event_type,
                                event,
                                Some(&snapshot),
                            )
                                .await
                                .is_err()
                            {
                                break;
                            }

                            sent_initial = true;
                        }

                        last_sent_turn_no = last_sent_turn_no.max(snapshot.turn_no);
                        last_status = Some(snapshot.status);
                    }
                    Err(error) => {
                        let payload = serde_json::json!({
                            "event_type": "ERROR",
                            "game_id": game_id,
                            "error": error.message,
                            "at": Utc::now()
                        })
                        .to_string();

                        if send_ws_event(&mut socket, &game_id, "ERROR", payload, None)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            event = watch_events_rx.recv() => {
                match event {
                    Ok(WatcherBroadcastEvent::Timeout(timeout)) => {
                        if timeout.game_id != game_id {
                            continue;
                        }

                        if let Some(snapshot) = timeout.snapshot.as_ref() {
                            last_sent_turn_no = last_sent_turn_no.max(snapshot.turn_no);
                            last_status = Some(snapshot.status);
                            sent_initial = true;
                        }

                        let payload = serde_json::json!({
                            "event_type": "TIMEOUT",
                            "game_id": timeout.game_id.as_str(),
                            "step_seq": timeout.step_seq,
                            "turn_no": timeout.turn_no,
                            "round_no": timeout.round_no,
                            "player_id": timeout.player_id,
                            "result_status": timeout.result_status,
                            "timeout_at": timeout.created_at,
                            "snapshot": timeout.snapshot.clone(),
                            "emitted_at": Utc::now(),
                        })
                        .to_string();

                        if send_ws_event(
                            &mut socket,
                            &game_id,
                            "TIMEOUT",
                            payload,
                            timeout.snapshot.as_ref(),
                        )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(WatcherBroadcastEvent::GameFinished(finished)) => {
                        if finished.game_id != game_id {
                            continue;
                        }

                        if let Some(snapshot) = finished.snapshot.as_ref() {
                            last_sent_turn_no = last_sent_turn_no.max(snapshot.turn_no);
                            last_status = Some(snapshot.status);
                            sent_initial = true;
                        }

                        let payload = serde_json::json!({
                            "event_type": "GAME_FINISHED",
                            "game_id": finished.game_id.as_str(),
                            "step_seq": finished.step_seq,
                            "turn_no": finished.turn_no,
                            "round_no": finished.round_no,
                            "finished_at": finished.created_at,
                            "snapshot": finished.snapshot.clone(),
                            "emitted_at": Utc::now(),
                        })
                        .to_string();

                        if send_ws_event(
                            &mut socket,
                            &game_id,
                            "GAME_FINISHED",
                            payload,
                            finished.snapshot.as_ref(),
                        )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(WatcherBroadcastEvent::Speak(speak)) => {
                        if speak.game_id != game_id {
                            continue;
                        }

                        if let Some(snapshot) = speak.snapshot.as_ref() {
                            last_sent_turn_no = last_sent_turn_no.max(snapshot.turn_no);
                            last_status = Some(snapshot.status);
                            sent_initial = true;
                        }

                        let payload = serde_json::json!({
                            "event_type": "SPEAK",
                            "game_id": speak.game_id.as_str(),
                            "step_seq": speak.step_seq,
                            "turn_no": speak.turn_no,
                            "round_no": speak.round_no,
                            "player_id": speak.player_id,
                            "speak_text": speak.speak_text.as_str(),
                            "spoke_at": speak.created_at,
                            "snapshot": speak.snapshot.clone(),
                            "emitted_at": Utc::now(),
                        })
                        .to_string();

                        if send_ws_event(
                            &mut socket,
                            &game_id,
                            "SPEAK",
                            payload,
                            speak.snapshot.as_ref(),
                        )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(WatcherBroadcastEvent::Shoot(shoot)) => {
                        if shoot.game_id != game_id {
                            continue;
                        }

                        if let Some(snapshot) = shoot.snapshot.as_ref() {
                            last_sent_turn_no = last_sent_turn_no.max(snapshot.turn_no);
                            last_status = Some(snapshot.status);
                            sent_initial = true;
                        }

                        let payload = serde_json::json!({
                            "event_type": "SHOOT",
                            "game_id": shoot.game_id.as_str(),
                            "step_seq": shoot.step_seq,
                            "turn_no": shoot.turn_no,
                            "round_no": shoot.round_no,
                            "player_id": shoot.player_id,
                            "direction": shoot.direction,
                            "command_id": shoot.command_id.as_str(),
                            "shot_at": shoot.created_at,
                            "snapshot": shoot.snapshot.clone(),
                            "emitted_at": Utc::now(),
                        })
                        .to_string();

                        if send_ws_event(
                            &mut socket,
                            &game_id,
                            "SHOOT",
                            payload,
                            shoot.snapshot.as_ref(),
                        )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(game_id = %game_id, skipped, "watcher stream lagged timeout events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }
}

fn to_json_log<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| format!("json_encode_error:{error}"))
}

fn log_ws_push(
    event_type: &str,
    game_id: &str,
    payload: &str,
    snapshot: Option<&SnapshotResponse>,
) {
    if let Some(snapshot) = snapshot {
        let snapshot_json = to_json_log(snapshot);
        let state_json = to_json_log(&snapshot.state);
        let map_json = to_json_log(&snapshot.state.map);
        info!(
            event_type = event_type,
            game_id = game_id,
            snapshot_turn_no = snapshot.turn_no,
            snapshot_last_step_seq = snapshot.last_step_seq,
            snapshot_status = ?snapshot.status,
            snapshot_json = %snapshot_json,
            state_json = %state_json,
            map_json = %map_json,
            websocket_payload = %payload,
            "pushing websocket event to frontend"
        );
        return;
    }

    info!(
        event_type = event_type,
        game_id = game_id,
        websocket_payload = %payload,
        "pushing websocket event to frontend"
    );
}

async fn send_ws_event(
    socket: &mut axum::extract::ws::WebSocket,
    game_id: &str,
    event_type: &str,
    payload: String,
    snapshot: Option<&SnapshotResponse>,
) -> Result<(), ()> {
    log_ws_push(event_type, game_id, &payload, snapshot);
    socket
        .send(axum::extract::ws::Message::Text(payload.into()))
        .await
        .map_err(|error| {
            warn!(
                event_type = event_type,
                game_id = game_id,
                error = ?error,
                "failed to push websocket event to frontend"
            );
        })
}

async fn run_output_consumer(state: AppState) {
    let bootstrap_servers = std::env::var("KAFKA_BOOTSTRAP_SERVERS")
        .ok()
        .unwrap_or_else(|| "kafka:9092".to_string());
    let output_topic_prefix = std::env::var("GAME_OUTPUT_TOPIC_PREFIX")
        .ok()
        .unwrap_or_else(|| "game.output".to_string());
    let group_id = std::env::var("WATCHER_OUTPUT_CONSUMER_GROUP_ID")
        .ok()
        .unwrap_or_else(|| "game-watcher-output-v1".to_string());

    let topic_pattern = format!("^{}\\..*\\.v1$", output_topic_prefix.replace('.', "\\."));
    let (step_tx, mut step_rx) = mpsc::channel::<StepEvent>(128);
    let reader_bootstrap_servers = bootstrap_servers.clone();
    let reader_group_id = group_id.clone();
    let reader_topic_pattern = topic_pattern.clone();
    tokio::spawn(async move {
        if let Err(error) = consume_output_steps(
            reader_bootstrap_servers,
            reader_topic_pattern,
            reader_group_id,
            step_tx,
        )
        .await
        {
            warn!(error = %error, "watcher output consumer task exited");
        }
    });

    while let Some(step) = step_rx.recv().await {
        let snapshot = match fetch_snapshot(&state, &step.game_id).await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                warn!(
                    game_id = %step.game_id,
                    message = %error.message,
                    "output kafka event had no fresh snapshot"
                );
                None
            }
        };

        if is_timeout_step(&step) {
            let timeout_event = TimeoutBroadcastEvent {
                game_id: step.game_id.clone(),
                step_seq: step.step_seq,
                turn_no: step.turn_no,
                round_no: step.round_no,
                player_id: step
                    .command
                    .as_ref()
                    .and_then(|command| command.player_id.clone()),
                result_status: step.result_status,
                created_at: step.created_at,
                snapshot: snapshot.clone(),
            };

            if state.watch_events_tx.receiver_count() > 0
                && let Err(error) = state
                    .watch_events_tx
                    .send(WatcherBroadcastEvent::Timeout(timeout_event))
            {
                warn!(
                    ?error,
                    "failed to fan out timeout event to websocket subscribers"
                );
            }
        }

        if is_game_finished_step(&step) {
            let finished_event = GameFinishedBroadcastEvent {
                game_id: step.game_id.clone(),
                step_seq: step.step_seq,
                turn_no: step.turn_no,
                round_no: step.round_no,
                created_at: step.created_at,
                snapshot: snapshot.clone(),
            };

            if state.watch_events_tx.receiver_count() > 0
                && let Err(error) = state
                    .watch_events_tx
                    .send(WatcherBroadcastEvent::GameFinished(finished_event))
            {
                warn!(
                    ?error,
                    "failed to fan out game-finished event to websocket subscribers"
                );
            }
        }

        if is_speak_step(&step) {
            let speak_text = step
                .command
                .as_ref()
                .and_then(|command| command.speak_text.clone())
                .unwrap_or_default();
            if speak_text.trim().is_empty() {
                continue;
            }

            let speak_event = SpeakBroadcastEvent {
                game_id: step.game_id.clone(),
                step_seq: step.step_seq,
                turn_no: step.turn_no,
                round_no: step.round_no,
                player_id: step
                    .command
                    .as_ref()
                    .and_then(|command| command.player_id.clone()),
                speak_text,
                created_at: step.created_at,
                snapshot: snapshot.clone(),
            };

            if state.watch_events_tx.receiver_count() > 0
                && let Err(error) = state
                    .watch_events_tx
                    .send(WatcherBroadcastEvent::Speak(speak_event))
            {
                warn!(
                    ?error,
                    "failed to fan out speak event to websocket subscribers"
                );
            }
        }

        if is_shoot_step(&step) {
            let shoot_event = ShootBroadcastEvent {
                game_id: step.game_id.clone(),
                step_seq: step.step_seq,
                turn_no: step.turn_no,
                round_no: step.round_no,
                player_id: step
                    .command
                    .as_ref()
                    .and_then(|command| command.player_id.clone()),
                direction: step.command.as_ref().and_then(|command| command.direction),
                command_id: step
                    .command
                    .as_ref()
                    .map(|command| command.command_id.clone())
                    .unwrap_or_default(),
                created_at: step.created_at,
                snapshot,
            };

            if state.watch_events_tx.receiver_count() > 0
                && let Err(error) = state
                    .watch_events_tx
                    .send(WatcherBroadcastEvent::Shoot(shoot_event))
            {
                warn!(
                    ?error,
                    "failed to fan out shoot event to websocket subscribers"
                );
            }
        }
    }
}

async fn consume_output_steps(
    bootstrap_servers: String,
    topic_pattern: String,
    group_id: String,
    step_tx: mpsc::Sender<StepEvent>,
) -> anyhow::Result<()> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &bootstrap_servers)
        .set("group.id", &group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("topic.metadata.refresh.interval.ms", "1000")
        .set("topic.metadata.refresh.fast.interval.ms", "250")
        .create()
        .context("failed to create watcher output consumer")?;
    consumer
        .subscribe(&[&topic_pattern])
        .context("failed to subscribe watcher output topic pattern")?;
    info!(
        bootstrap_servers = %bootstrap_servers,
        topic_pattern = %topic_pattern,
        group_id = %group_id,
        "watcher output kafka consumer started"
    );

    loop {
        let message = match consumer.recv().await {
            Ok(message) => message,
            Err(error) => {
                warn!(?error, "watcher output consumer recv error");
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
        };

        let payload = match message.payload() {
            Some(payload) => payload,
            None => {
                if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(?error, "watcher failed to commit empty payload");
                }
                continue;
            }
        };

        let step = match serde_json::from_slice::<StepEvent>(payload) {
            Ok(step) => step,
            Err(error) => {
                warn!(?error, "failed to parse output kafka step payload");
                if let Err(commit_err) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(?commit_err, "watcher failed to commit invalid payload");
                }
                continue;
            }
        };

        if (is_timeout_step(&step)
            || is_game_finished_step(&step)
            || is_speak_step(&step)
            || is_shoot_step(&step))
            && step_tx.send(step).await.is_err()
        {
            return Ok(());
        }

        if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
            warn!(?error, "watcher failed to commit consumed message");
        }
    }
}

fn is_timeout_step(step: &StepEvent) -> bool {
    step.event_type == StepEventType::TimeoutApplied
        || (step
            .command
            .as_ref()
            .map(|command| command.command_type == CommandType::Timeout)
            .unwrap_or(false)
            && step.result_status == ResultStatus::TimeoutApplied)
}

fn is_game_finished_step(step: &StepEvent) -> bool {
    step.event_type == StepEventType::GameFinished
}

fn is_speak_step(step: &StepEvent) -> bool {
    step.event_type == StepEventType::StepApplied
        && step.result_status == ResultStatus::Applied
        && step
            .command
            .as_ref()
            .map(|command| command.command_type == CommandType::Speak)
            .unwrap_or(false)
}

fn is_shoot_step(step: &StepEvent) -> bool {
    step.event_type == StepEventType::StepApplied
        && step.result_status == ResultStatus::Applied
        && step
            .command
            .as_ref()
            .map(|command| command.command_type == CommandType::Shoot)
            .unwrap_or(false)
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
}

async fn fetch_snapshot(state: &AppState, game_id: &str) -> Result<SnapshotResponse, ApiError> {
    let url = format!("{}/v2/games/{}", state.manager_base_url, game_id);

    let response = state
        .client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("manager request failed: {e}")))?;

    let status = response.status();

    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found(format!("game {} not found", game_id)));
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "manager returned {}: {}",
            status, body
        )));
    }

    let game = response
        .json::<GameInstanceResponse>()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("invalid manager response: {e}")))?;

    Ok(to_snapshot(game))
}

fn to_snapshot(game: GameInstanceResponse) -> SnapshotResponse {
    SnapshotResponse {
        game_id: game.game_id,
        status: game.status,
        turn_no: game.turn_no,
        round_no: game.round_no,
        current_player_id: game.current_player_id,
        state: game.state,
        // V2 turn-only cursor assumption.
        last_step_seq: game.turn_no,
        turn_started_at: game.turn_started_at,
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

#[cfg(test)]
mod tests {
    use super::*;
    use cowboy_common::{
        CommandEnvelope, CommandSource, GameStateSnapshot, MapSource, default_map,
        initial_players,
    };

    fn timeout_step(event_type: StepEventType, command_type: Option<CommandType>) -> StepEvent {
        let command = command_type.map(|kind| CommandEnvelope {
            command_id: "cmd-1".to_string(),
            source: CommandSource::Timer,
            game_id: "game-1".to_string(),
            player_id: Some("Up".to_string()),
            command_type: kind,
            direction: None,
            speak_text: None,
            turn_no: 4,
            sent_at: Utc::now(),
        });

        StepEvent {
            game_id: "game-1".to_string(),
            step_seq: 8,
            turn_no: 4,
            round_no: 2,
            event_type,
            result_status: ResultStatus::TimeoutApplied,
            command,
            state_after: GameStateSnapshot {
                map: default_map(),
                players: initial_players(11, 11, 10, 4),
            },
            created_at: Utc::now(),
        }
    }

    #[test]
    fn timeout_detection_matches_timeout_applied_event_type() {
        let step = timeout_step(StepEventType::TimeoutApplied, Some(CommandType::Move));
        assert!(is_timeout_step(&step));
    }

    #[test]
    fn timeout_detection_matches_timeout_command_type() {
        let step = timeout_step(StepEventType::StepApplied, Some(CommandType::Timeout));
        assert!(is_timeout_step(&step));
    }

    #[test]
    fn timeout_detection_ignores_non_timeout_steps() {
        let step = timeout_step(StepEventType::StepApplied, Some(CommandType::Move));
        assert!(!is_timeout_step(&step));
    }

    #[test]
    fn speak_detection_matches_applied_speak_step() {
        let mut step = timeout_step(StepEventType::StepApplied, Some(CommandType::Speak));
        step.result_status = ResultStatus::Applied;
        if let Some(command) = step.command.as_mut() {
            command.speak_text = Some("hello".to_string());
        }
        assert!(is_speak_step(&step));
    }

    #[test]
    fn speak_detection_ignores_non_applied_speak_step() {
        let step = timeout_step(StepEventType::StepApplied, Some(CommandType::Speak));
        assert!(!is_speak_step(&step));
    }

    #[test]
    fn game_finished_detection_matches_event_type() {
        let step = timeout_step(StepEventType::GameFinished, Some(CommandType::Move));
        assert!(is_game_finished_step(&step));
    }

    #[test]
    fn shoot_detection_matches_applied_shoot_step() {
        let mut step = timeout_step(StepEventType::StepApplied, Some(CommandType::Shoot));
        step.result_status = ResultStatus::Applied;
        assert!(is_shoot_step(&step));
    }

    #[test]
    fn shoot_detection_ignores_non_applied_shoot_step() {
        let step = timeout_step(StepEventType::StepApplied, Some(CommandType::Shoot));
        assert!(!is_shoot_step(&step));
    }

    #[test]
    fn shoot_detection_ignores_non_shoot_commands() {
        let mut step = timeout_step(StepEventType::StepApplied, Some(CommandType::Move));
        step.result_status = ResultStatus::Applied;
        assert!(!is_shoot_step(&step));
    }

    #[test]
    fn to_snapshot_uses_turn_number_as_cursor() {
        let now = Utc::now();
        let game = GameInstanceResponse {
            game_id: "game-1".to_string(),
            status: GameStatus::Running,
            map_source: MapSource::Default,
            turn_timeout_seconds: 10,
            turn_no: 7,
            round_no: 2,
            current_player_id: "Right".to_string(),
            created_at: now,
            started_at: Some(now),
            turn_started_at: Some(now),
            input_topic: Some("game.commands.game-1.v1".to_string()),
            output_topic: Some("game.output.game-1.v1".to_string()),
            state: GameStateSnapshot {
                map: default_map(),
                players: initial_players(11, 11, 10, 4),
            },
        };

        let snapshot = to_snapshot(game);
        assert_eq!(snapshot.turn_no, 7);
        assert_eq!(snapshot.last_step_seq, 7);
        assert_eq!(snapshot.status, GameStatus::Running);
    }

    #[tokio::test]
    async fn health_reports_service_name() {
        let payload = health().await.0;
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["service"], "game-watcher-service");
    }
}
