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
    CommandType, GameInstanceResponse, GameStatus, SnapshotResponse, StepEvent, StepEventType,
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
struct WatcherBroadcastEvent {
    game_id: String,
    ws_event_type: String,
    ws_payload: String,
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
                    Ok(ev) => {
                        if ev.game_id != game_id {
                            continue;
                        }

                        if let Some(snapshot) = ev.snapshot.as_ref() {
                            last_sent_turn_no = last_sent_turn_no.max(snapshot.turn_no);
                            last_status = Some(snapshot.status);
                            sent_initial = true;
                        }

                        if send_ws_event(
                            &mut socket,
                            &game_id,
                            &ev.ws_event_type,
                            ev.ws_payload,
                            ev.snapshot.as_ref(),
                        )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(game_id = %game_id, skipped, "watcher stream lagged broadcast events");
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

        let ws_event_type = step_ws_event_type(&step);
        let ws_payload = build_step_ws_payload(&step, &snapshot, ws_event_type);

        if state.watch_events_tx.receiver_count() > 0
            && let Err(error) = state.watch_events_tx.send(WatcherBroadcastEvent {
                game_id: step.game_id,
                ws_event_type: ws_event_type.to_string(),
                ws_payload,
                snapshot,
            })
        {
            warn!(
                ?error,
                "failed to fan out step event to websocket subscribers"
            );
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

        if step_tx.send(step).await.is_err() {
            return Ok(());
        }

        if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
            warn!(?error, "watcher failed to commit consumed message");
        }
    }
}

fn step_ws_event_type(step: &StepEvent) -> &'static str {
    match step.event_type {
        StepEventType::GameStarted => "GAME_STARTED",
        StepEventType::GameFinished => "GAME_FINISHED",
        StepEventType::TimeoutApplied => "TIMEOUT",
        StepEventType::StepApplied => {
            if let Some(cmd) = step.command.as_ref() {
                match cmd.command_type {
                    CommandType::Move => "MOVE",
                    CommandType::Shoot => "SHOOT",
                    CommandType::Shield => "SHIELD",
                    CommandType::Speak => "SPEAK",
                    CommandType::Timeout => "TIMEOUT",
                    CommandType::GameStarted => "GAME_STARTED",
                }
            } else {
                "STEP_APPLIED"
            }
        }
    }
}

fn build_step_ws_payload(
    step: &StepEvent,
    snapshot: &Option<SnapshotResponse>,
    ws_event_type: &str,
) -> String {
    let mut payload = serde_json::json!({
        "event_type": ws_event_type,
        "game_id": step.game_id,
        "step_seq": step.step_seq,
        "turn_no": step.turn_no,
        "round_no": step.round_no,
        "result_status": step.result_status,
        "created_at": step.created_at,
        "snapshot": snapshot,
        "emitted_at": Utc::now(),
    });

    if let Some(cmd) = step.command.as_ref() {
        let obj = payload.as_object_mut().unwrap();
        obj.insert("player_id".into(), serde_json::json!(cmd.player_id));
        obj.insert("command_type".into(), serde_json::json!(cmd.command_type));
        if let Some(dir) = cmd.direction {
            obj.insert("direction".into(), serde_json::json!(dir));
        }
        if let Some(text) = &cmd.speak_text {
            obj.insert("speak_text".into(), serde_json::json!(text));
        }
        obj.insert("command_id".into(), serde_json::json!(cmd.command_id));
    }

    payload.to_string()
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
        CommandEnvelope, CommandSource, GameStateSnapshot, MapSource, ResultStatus, default_map,
        initial_players,
    };

    fn make_step(event_type: StepEventType, command_type: Option<CommandType>) -> StepEvent {
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
            result_status: ResultStatus::Applied,
            command,
            state_after: GameStateSnapshot {
                map: default_map(),
                players: initial_players(11, 11, 10, 4),
            },
            created_at: Utc::now(),
        }
    }

    #[test]
    fn ws_event_type_game_started() {
        let step = make_step(StepEventType::GameStarted, None);
        assert_eq!(step_ws_event_type(&step), "GAME_STARTED");
    }

    #[test]
    fn ws_event_type_game_finished() {
        let step = make_step(StepEventType::GameFinished, None);
        assert_eq!(step_ws_event_type(&step), "GAME_FINISHED");
    }

    #[test]
    fn ws_event_type_timeout_applied() {
        let step = make_step(StepEventType::TimeoutApplied, Some(CommandType::Timeout));
        assert_eq!(step_ws_event_type(&step), "TIMEOUT");
    }

    #[test]
    fn ws_event_type_move() {
        let step = make_step(StepEventType::StepApplied, Some(CommandType::Move));
        assert_eq!(step_ws_event_type(&step), "MOVE");
    }

    #[test]
    fn ws_event_type_shoot() {
        let step = make_step(StepEventType::StepApplied, Some(CommandType::Shoot));
        assert_eq!(step_ws_event_type(&step), "SHOOT");
    }

    #[test]
    fn ws_event_type_shield() {
        let step = make_step(StepEventType::StepApplied, Some(CommandType::Shield));
        assert_eq!(step_ws_event_type(&step), "SHIELD");
    }

    #[test]
    fn ws_event_type_speak() {
        let step = make_step(StepEventType::StepApplied, Some(CommandType::Speak));
        assert_eq!(step_ws_event_type(&step), "SPEAK");
    }

    #[test]
    fn ws_event_type_step_applied_no_command() {
        let step = make_step(StepEventType::StepApplied, None);
        assert_eq!(step_ws_event_type(&step), "STEP_APPLIED");
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
