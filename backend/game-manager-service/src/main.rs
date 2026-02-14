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
    sync::Arc,
};

use anyhow::Context;
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use cowboy_common::{
    CommandType, CreateGameRequest, CreateGameResponse, DEFAULT_NUM_PLAYERS, DEFAULT_PLAYER_HP,
    DEFAULT_TURN_TIMEOUT_SECONDS, Direction, GameInstanceResponse, GameStateSnapshot, GameStatus,
    MAX_NUM_PLAYERS, MIN_NUM_PLAYERS, MapData, MapSource, PlayerId, PlayerIdentity, PlayerName,
    ResultStatus, StartGameResponse, StepEvent, StepEventType, SubmitCommandRequest, default_map,
    generate_default_map, initial_players,
};
use lambda_http::run as lambda_run;
use rdkafka::{
    admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
    client::DefaultClientContext,
    config::ClientConfig,
    producer::{FutureProducer, FutureRecord},
    types::RDKafkaErrorCode,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    store: Arc<RwLock<InMemoryStore>>,
    topic_provisioner: Arc<dyn TopicProvisioner>,
    step_event_publisher: Arc<dyn StepEventPublisher>,
    bot_assigner: Arc<dyn BotAssigner>,
}

#[derive(Default)]
struct InMemoryStore {
    default_map: Option<MapData>,
    games: HashMap<String, GameInstance>,
}

#[derive(Clone)]
struct GameInstance {
    game_id: String,
    status: GameStatus,
    map_source: MapSource,
    turn_timeout_seconds: u64,
    turn_no: u64,
    round_no: u64,
    current_player_id: PlayerId,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    /// When the current turn began (set on game start and each turn advance).
    turn_started_at: Option<DateTime<Utc>>,
    state: GameStateSnapshot,
    last_step_seq: u64,
    input_topic: String,
    output_topic: String,
}

#[derive(Debug, Clone)]
struct GameTopics {
    input_topic: String,
    output_topic: String,
}

#[async_trait]
trait TopicProvisioner: Send + Sync {
    async fn ensure_game_topics(&self, game_id: &str) -> anyhow::Result<GameTopics>;
    async fn delete_game_topics(&self, game_topics: &GameTopics) -> anyhow::Result<()>;
}

#[async_trait]
trait StepEventPublisher: Send + Sync {
    async fn publish_step_event(&self, topic: &str, event: &StepEvent) -> anyhow::Result<()>;
}

#[async_trait]
trait BotAssigner: Send + Sync {
    async fn assign_for_new_game(
        &self,
        game: &GameInstance,
        requested_bot_players: Option<Vec<PlayerName>>,
    ) -> anyhow::Result<()>;
}

#[derive(Clone)]
struct BotManagerAssigner {
    client: reqwest::Client,
    base_url: String,
}

impl BotManagerAssigner {
    fn from_env() -> Self {
        let base_url = std::env::var("BOT_MANAGER_BASE_URL")
            .ok()
            .unwrap_or_else(|| "http://bot-manager-service:8090".to_string());

        Self {
            client: reqwest::Client::new(),
            base_url,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }

    async fn post_json(&self, url: String, payload: serde_json::Value) -> anyhow::Result<()> {
        let response = self
            .client
            .post(url.clone())
            .json(&payload)
            .send()
            .await
            .context("failed to call bot-manager")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            anyhow::bail!("bot-manager returned {status}: {body}");
        }

        Ok(())
    }

    fn dedupe_players(players: Vec<PlayerName>) -> Vec<PlayerName> {
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for player in players {
            if seen.insert(player) {
                deduped.push(player);
            }
        }
        deduped
    }
}

#[async_trait]
impl BotAssigner for BotManagerAssigner {
    async fn assign_for_new_game(
        &self,
        game: &GameInstance,
        requested_bot_players: Option<Vec<PlayerName>>,
    ) -> anyhow::Result<()> {
        if requested_bot_players.is_none() {
            let url = self.endpoint(&format!(
                "internal/v3/games/{}/assignments/default",
                game.game_id
            ));
            let payload = serde_json::json!({
                "apply_immediately": true,
                "force_recreate": true
            });
            return self.post_json(url, payload).await;
        }

        let bot_players = Self::dedupe_players(requested_bot_players.unwrap_or_default());
        let bot_names: HashSet<PlayerName> = bot_players.iter().copied().collect();
        let players_by_name: HashMap<PlayerName, PlayerId> = game
            .state
            .players
            .iter()
            .map(|player| (player.player_name, player.player_id.clone()))
            .collect();

        let mut bot_player_ids = Vec::with_capacity(bot_players.len());
        for player_name in bot_players {
            let player_id = players_by_name.get(&player_name).ok_or_else(|| {
                anyhow::anyhow!(
                    "requested bot player {player_name:?} is not present in game {}",
                    game.game_id
                )
            })?;
            bot_player_ids.push(player_id.clone());
        }

        let human_player_ids: Vec<PlayerId> = game
            .state
            .players
            .iter()
            .filter(|player| !bot_names.contains(&player.player_name))
            .map(|player| player.player_id.clone())
            .collect();

        let url = self.endpoint(&format!("internal/v3/games/{}/assignments", game.game_id));
        let payload = serde_json::json!({
            "human_player_ids": human_player_ids,
            "bot_player_ids": bot_player_ids,
            "force_recreate": true
        });

        self.post_json(url, payload).await
    }
}

#[derive(Debug, Clone)]
struct KafkaTopicProvisioner {
    bootstrap_servers: Vec<String>,
    input_topic_prefix: String,
    output_topic_prefix: String,
}

impl KafkaTopicProvisioner {
    fn from_env() -> Self {
        let bootstrap_servers = std::env::var("KAFKA_BOOTSTRAP_SERVERS")
            .ok()
            .and_then(|value| {
                let hosts: Vec<String> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|host| !host.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                if hosts.is_empty() { None } else { Some(hosts) }
            })
            .unwrap_or_else(|| vec!["kafka:9092".to_string()]);

        Self {
            bootstrap_servers,
            input_topic_prefix: std::env::var("GAME_INPUT_TOPIC_PREFIX")
                .ok()
                .unwrap_or_else(|| "game.commands".to_string()),
            output_topic_prefix: std::env::var("GAME_OUTPUT_TOPIC_PREFIX")
                .ok()
                .unwrap_or_else(|| "game.output".to_string()),
        }
    }

    fn game_topics(&self, game_id: &str) -> GameTopics {
        GameTopics {
            input_topic: format!("{}.{}.v1", self.input_topic_prefix, game_id),
            output_topic: format!("{}.{}.v1", self.output_topic_prefix, game_id),
        }
    }

    fn admin_client(&self) -> anyhow::Result<AdminClient<DefaultClientContext>> {
        let bootstrap_servers = self.bootstrap_servers.join(",");
        ClientConfig::new()
            .set("bootstrap.servers", &bootstrap_servers)
            .create()
            .context("failed to create Kafka admin client")
    }
}

#[derive(Clone)]
struct KafkaStepEventPublisher {
    producer: FutureProducer,
}

impl KafkaStepEventPublisher {
    fn from_env() -> anyhow::Result<Self> {
        let bootstrap_servers = std::env::var("KAFKA_BOOTSTRAP_SERVERS")
            .ok()
            .unwrap_or_else(|| "kafka:9092".to_string());
        let producer = ClientConfig::new()
            .set("bootstrap.servers", bootstrap_servers)
            .set("message.timeout.ms", "5000")
            .create()
            .context("failed to create Kafka step-event producer")?;
        Ok(Self { producer })
    }
}

#[async_trait]
impl StepEventPublisher for KafkaStepEventPublisher {
    async fn publish_step_event(&self, topic: &str, event: &StepEvent) -> anyhow::Result<()> {
        let payload = serde_json::to_string(event).context("failed to encode step event")?;
        self.producer
            .send(
                FutureRecord::to(topic)
                    .key(&event.game_id)
                    .payload(&payload),
                std::time::Duration::from_secs(5),
            )
            .await
            .map_err(|(error, _)| anyhow::anyhow!("Kafka publish failed: {error:?}"))?;
        Ok(())
    }
}

#[async_trait]
impl TopicProvisioner for KafkaTopicProvisioner {
    async fn ensure_game_topics(&self, game_id: &str) -> anyhow::Result<GameTopics> {
        let game_topics = self.game_topics(game_id);
        let admin_client = self.admin_client()?;

        let topics = vec![
            NewTopic::new(&game_topics.input_topic, 1, TopicReplication::Fixed(1)),
            NewTopic::new(&game_topics.output_topic, 1, TopicReplication::Fixed(1)),
        ];
        let results = admin_client
            .create_topics(topics.iter(), &AdminOptions::new())
            .await
            .context("failed to send Kafka topic creation request")?;

        for result in results {
            match result {
                Ok(topic_name) => {
                    info!(topic = %topic_name, "created per-game Kafka topic");
                }
                Err((topic_name, RDKafkaErrorCode::TopicAlreadyExists)) => {
                    info!(topic = %topic_name, "per-game Kafka topic already exists");
                }
                Err((topic_name, error_code)) => {
                    return Err(anyhow::anyhow!(
                        "failed to create Kafka topic {topic_name}: {error_code:?}"
                    ));
                }
            }
        }

        Ok(game_topics)
    }

    async fn delete_game_topics(&self, game_topics: &GameTopics) -> anyhow::Result<()> {
        let admin_client = self.admin_client()?;

        let results = admin_client
            .delete_topics(
                &[
                    game_topics.input_topic.as_str(),
                    game_topics.output_topic.as_str(),
                ],
                &AdminOptions::new(),
            )
            .await
            .context("failed to send Kafka topic deletion request")?;

        for result in results {
            match result {
                Ok(topic_name) => {
                    info!(topic = %topic_name, "deleted per-game Kafka topic");
                }
                Err((topic_name, RDKafkaErrorCode::UnknownTopicOrPartition)) => {
                    info!(topic = %topic_name, "per-game Kafka topic already absent");
                }
                Err((topic_name, error_code)) => {
                    return Err(anyhow::anyhow!(
                        "failed to delete Kafka topic {topic_name}: {error_code:?}"
                    ));
                }
            }
        }

        Ok(())
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FinishGameResponse {
    finished: bool,
    reason: Option<String>,
    status: GameStatus,
    winner_player_id: Option<PlayerId>,
    turn_no: u64,
    round_no: u64,
    current_player_id: PlayerId,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "game_manager_service=debug,tower_http=info".to_string()),
        )
        .init();

    let mut store = InMemoryStore::default();
    if let Some(map) = load_default_map_config() {
        info!(rows = map.rows, cols = map.cols, "loaded default map from YAML config");
        store.default_map = Some(map);
    }

    let state = AppState {
        store: Arc::new(RwLock::new(store)),
        topic_provisioner: Arc::new(KafkaTopicProvisioner::from_env()),
        step_event_publisher: Arc::new(KafkaStepEventPublisher::from_env()?),
        bot_assigner: Arc::new(BotManagerAssigner::from_env()),
    };

    let app = build_router(state);

    if std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok() {
        info!("AWS Lambda runtime detected; running game-manager-service in lambda mode");
        lambda_run(app)
            .await
            .map_err(|e| anyhow::Error::msg(format!("lambda runtime error: {e}")))?;
        return Ok(());
    }

    let bind_addr = parse_bind_addr("GAME_MANAGER_BIND", "0.0.0.0:8081")?;
    info!(%bind_addr, "game-manager-service listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_default_map_config() -> Option<MapData> {
    let path = std::env::var("DEFAULT_MAP_CONFIG_PATH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;

    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to read default map config file");
            return None;
        }
    };

    if raw.trim().is_empty() {
        warn!(path = %path, "default map config file is empty");
        return None;
    }

    match serde_yaml::from_str::<MapData>(&raw) {
        Ok(map) => Some(map),
        Err(error) => {
            warn!(path = %path, error = %error, "failed to parse default map config yaml");
            None
        }
    }
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v2/maps/default", get(get_default_map_handler))
        .route("/v2/games", post(create_game_handler))
        .route("/v2/games/{game_id}", get(get_game_handler))
        .route("/v2/games/{game_id}/start", post(start_game_handler))
        .route(
            "/internal/v2/games/{game_id}/commands/apply",
            post(apply_command_handler),
        )
        .route(
            "/internal/v2/games/{game_id}/finish",
            post(finish_game_handler),
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
    Json(serde_json::json!({"ok": true, "service": "game-manager-service"}))
}

async fn get_default_map_handler(State(state): State<AppState>) -> Result<Json<MapData>, ApiError> {
    let mut store = state.store.write().await;
    if store.default_map.is_none() {
        store.default_map = Some(default_map());
    }

    let map = store
        .default_map
        .clone()
        .ok_or_else(|| ApiError::internal("default map unavailable"))?;
    Ok(Json(map))
}

async fn create_game_handler(
    State(state): State<AppState>,
    Json(request): Json<CreateGameRequest>,
) -> Result<Json<CreateGameResponse>, ApiError> {
    let CreateGameRequest {
        turn_timeout_seconds,
        map,
        bot_players,
        num_players,
    } = request;

    let timeout = turn_timeout_seconds
        .unwrap_or(DEFAULT_TURN_TIMEOUT_SECONDS)
        .max(1);
    let num_players = num_players
        .unwrap_or(DEFAULT_NUM_PLAYERS)
        .max(MIN_NUM_PLAYERS)
        .min(MAX_NUM_PLAYERS);

    let game_id = Uuid::new_v4().to_string();
    let game_topics = state
        .topic_provisioner
        .ensure_game_topics(&game_id)
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "failed to provision Kafka topics for game {game_id}: {error:#}"
            ))
        })?;

    let game = {
        let mut store = state.store.write().await;

        let (map_source, map) = if let Some(map) = map {
            (MapSource::Custom, map)
        } else {
            let selected = if let Some(existing) = store.default_map.clone() {
                existing
            } else {
                let generated = generate_default_map(11, 11, num_players);
                store.default_map = Some(generated.clone());
                generated
            };
            (MapSource::Default, selected)
        };

        let created_at = Utc::now();

        let state_snapshot = GameStateSnapshot {
            players: initial_players(map.rows, map.cols, DEFAULT_PLAYER_HP, num_players),
            map,
        };

        let game = GameInstance {
            game_id: game_id.clone(),
            status: GameStatus::Created,
            map_source,
            turn_timeout_seconds: timeout,
            turn_no: 1,
            round_no: 1,
            current_player_id: state_snapshot
                .players
                .first()
                .map(|player| player.player_id.clone())
                .ok_or_else(|| ApiError::internal("no players in game"))?,
            created_at,
            started_at: None,
            turn_started_at: None,
            state: state_snapshot,
            last_step_seq: 0,
            input_topic: game_topics.input_topic.clone(),
            output_topic: game_topics.output_topic.clone(),
        };

        info!(
            game_id = %game.game_id,
            input_topic = %game.input_topic,
            output_topic = %game.output_topic,
            "provisioned per-game Kafka topics"
        );

        store.games.insert(game_id.clone(), game.clone());
        game
    };

    if let Err(error) = state
        .bot_assigner
        .assign_for_new_game(&game, bot_players)
        .await
    {
        {
            let mut store = state.store.write().await;
            store.games.remove(&game_id);
        }

        if let Err(cleanup_error) = state
            .topic_provisioner
            .delete_game_topics(&game_topics)
            .await
        {
            warn!(
                game_id = %game_id,
                input_topic = %game_topics.input_topic,
                output_topic = %game_topics.output_topic,
                error = %cleanup_error,
                "failed to rollback topics after bot assignment error"
            );
        }

        return Err(ApiError::bad_gateway(format!(
            "failed to assign bots for game {game_id}: {error:#}"
        )));
    }

    Ok(Json(CreateGameResponse {
        game_id,
        status: game.status,
        map_source: game.map_source,
        turn_no: game.turn_no,
        round_no: game.round_no,
        current_player_id: game.current_player_id.clone(),
        players: game
            .state
            .players
            .iter()
            .map(|player| PlayerIdentity {
                player_name: player.player_name,
                player_id: player.player_id.clone(),
            })
            .collect(),
        turn_timeout_seconds: game.turn_timeout_seconds,
        created_at: game.created_at,
    }))
}

async fn get_game_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
) -> Result<Json<GameInstanceResponse>, ApiError> {
    let store = state.store.read().await;
    let game = store
        .games
        .get(&game_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("game {} not found", game_id)))?;

    Ok(Json(GameInstanceResponse {
        game_id: game.game_id,
        status: game.status,
        map_source: game.map_source,
        turn_timeout_seconds: game.turn_timeout_seconds,
        turn_no: game.turn_no,
        round_no: game.round_no,
        current_player_id: game.current_player_id.clone(),
        created_at: game.created_at,
        started_at: game.started_at,
        turn_started_at: game.turn_started_at,
        input_topic: Some(game.input_topic),
        output_topic: Some(game.output_topic),
        state: game.state,
    }))
}

async fn start_game_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
) -> Result<Json<StartGameResponse>, ApiError> {
    let (response, output_topic, started_event) = {
        let mut store = state.store.write().await;
        let game = store
            .games
            .get_mut(&game_id)
            .ok_or_else(|| ApiError::not_found(format!("game {} not found", game_id)))?;

        if game.status == GameStatus::Running {
            return Ok(Json(StartGameResponse {
                game_id: game.game_id.clone(),
                status: game.status,
                started: false,
                reason: Some("ALREADY_RUNNING".to_string()),
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
                started_at: game.started_at,
            }));
        }

        if game.status == GameStatus::Finished {
            return Ok(Json(StartGameResponse {
                game_id: game.game_id.clone(),
                status: game.status,
                started: false,
                reason: Some("GAME_FINISHED".to_string()),
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
                started_at: game.started_at,
            }));
        }

        let now = Utc::now();
        game.status = GameStatus::Running;
        game.started_at = Some(now);
        game.turn_started_at = Some(now);
        game.last_step_seq += 1;

        let started_event = StepEvent {
            game_id: game.game_id.clone(),
            step_seq: game.last_step_seq,
            turn_no: game.turn_no,
            round_no: game.round_no,
            event_type: StepEventType::GameStarted,
            result_status: ResultStatus::Applied,
            command: None,
            state_after: game.state.clone(),
            created_at: now,
        };

        (
            StartGameResponse {
                game_id: game.game_id.clone(),
                status: game.status,
                started: true,
                reason: None,
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
                started_at: game.started_at,
            },
            game.output_topic.clone(),
            started_event,
        )
    };

    state
        .step_event_publisher
        .publish_step_event(&output_topic, &started_event)
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "failed to publish GAME_STARTED event for game {game_id}: {error:#}"
            ))
        })?;

    info!(
        game_id = %started_event.game_id,
        step_seq = started_event.step_seq,
        output_topic = %output_topic,
        "published GAME_STARTED event"
    );

    Ok(Json(response))
}

async fn apply_command_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<SubmitCommandRequest>,
) -> Result<Json<ApplyCommandResponse>, ApiError> {
    let mut store = state.store.write().await;
    let game = store
        .games
        .get_mut(&game_id)
        .ok_or_else(|| ApiError::not_found(format!("game {} not found", game_id)))?;

    let mut response = ApplyCommandResponse {
        accepted: false,
        applied: false,
        reason: None,
        turn_no: game.turn_no,
        round_no: game.round_no,
        current_player_id: game.current_player_id.clone(),
        status: game.status,
    };

    if game.status != GameStatus::Running {
        response.reason = Some("GAME_NOT_RUNNING".to_string());
        return Ok(Json(response));
    }

    if request.player_id != game.current_player_id {
        response.reason = Some("INVALID_TURN_PLAYER".to_string());
        return Ok(Json(response));
    }

    if request.turn_no != game.turn_no {
        response.reason = Some("STALE_TURN_NO".to_string());
        return Ok(Json(response));
    }

    let player_idx = game
        .state
        .players
        .iter()
        .position(|p| p.player_id == request.player_id)
        .ok_or_else(|| ApiError::internal("player not found in state"))?;

    if !game.state.players[player_idx].alive {
        response.reason = Some("PLAYER_DEAD".to_string());
        return Ok(Json(response));
    }

    let direction = request.direction;
    let (applied, consume_turn, reason) = match request.command_type {
        CommandType::Move => match direction {
            Some(dir) => apply_move(game, player_idx, dir),
            None => (false, false, Some("MISSING_DIRECTION".to_string())),
        },
        CommandType::Shield => match direction {
            Some(dir) => {
                game.state.players[player_idx].shield = dir;
                (true, true, None)
            }
            None => (false, false, Some("MISSING_DIRECTION".to_string())),
        },
        CommandType::Shoot => match direction {
            Some(dir) => apply_shoot(game, player_idx, dir),
            None => (false, false, Some("MISSING_DIRECTION".to_string())),
        },
        CommandType::Speak => {
            let has_text = request
                .speak_text
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .is_some();
            if has_text {
                (true, true, None)
            } else {
                (false, false, Some("MISSING_SPEAK_TEXT".to_string()))
            }
        }
        CommandType::Timeout => (true, true, None),
        CommandType::GameStarted => (false, false, Some("RESERVED_COMMAND_TYPE".to_string())),
    };

    response.accepted = true;
    response.applied = applied;
    response.reason = reason;

    if consume_turn {
        advance_turn(game);
        game.last_step_seq += 1;
    }

    response.turn_no = game.turn_no;
    response.round_no = game.round_no;
    response.current_player_id = game.current_player_id.clone();
    response.status = game.status;

    Ok(Json(response))
}

async fn finish_game_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<FinishGameRequest>,
) -> Result<Json<FinishGameResponse>, ApiError> {
    let (response, game_topics, finished_event) = {
        let mut store = state.store.write().await;
        let game = store
            .games
            .get_mut(&game_id)
            .ok_or_else(|| ApiError::not_found(format!("game {} not found", game_id)))?;

        if let Some(expected_turn_no) = request.expected_turn_no
            && game.turn_no != expected_turn_no
        {
            return Ok(Json(FinishGameResponse {
                finished: false,
                reason: Some("STALE_TURN_NO".to_string()),
                status: game.status,
                winner_player_id: winner_player_id(game),
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
            }));
        }

        if game.status == GameStatus::Finished {
            return Ok(Json(FinishGameResponse {
                finished: false,
                reason: Some("ALREADY_FINISHED".to_string()),
                status: game.status,
                winner_player_id: winner_player_id(game),
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
            }));
        }

        if alive_player_count(game) != 1 {
            return Ok(Json(FinishGameResponse {
                finished: false,
                reason: Some("NOT_LAST_PLAYER_LEFT".to_string()),
                status: game.status,
                winner_player_id: winner_player_id(game),
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
            }));
        }

        game.status = GameStatus::Finished;

        (
            FinishGameResponse {
                finished: true,
                reason: None,
                status: game.status,
                winner_player_id: winner_player_id(game),
                turn_no: game.turn_no,
                round_no: game.round_no,
                current_player_id: game.current_player_id.clone(),
            },
            GameTopics {
                input_topic: game.input_topic.clone(),
                output_topic: game.output_topic.clone(),
            },
            StepEvent {
                game_id: game.game_id.clone(),
                step_seq: game.last_step_seq.saturating_add(1),
                turn_no: game.turn_no,
                round_no: game.round_no,
                event_type: StepEventType::GameFinished,
                result_status: ResultStatus::Applied,
                command: None,
                state_after: game.state.clone(),
                created_at: Utc::now(),
            },
        )
    };

    if let Err(error) = state
        .step_event_publisher
        .publish_step_event(&game_topics.output_topic, &finished_event)
        .await
    {
        warn!(
            game_id = %finished_event.game_id,
            output_topic = %game_topics.output_topic,
            error = %error,
            "failed to publish GAME_FINISHED event"
        );
    } else {
        info!(
            game_id = %finished_event.game_id,
            output_topic = %game_topics.output_topic,
            "published GAME_FINISHED event"
        );
    }

    if let Err(error) = state
        .topic_provisioner
        .delete_game_topics(&game_topics)
        .await
    {
        warn!(
            input_topic = %game_topics.input_topic,
            output_topic = %game_topics.output_topic,
            error = %error,
            "failed to delete per-game Kafka topics after game finish"
        );
    } else {
        info!(
            input_topic = %game_topics.input_topic,
            output_topic = %game_topics.output_topic,
            "deleted per-game Kafka topics after game finish"
        );
    }

    Ok(Json(response))
}

fn apply_move(
    game: &mut GameInstance,
    player_idx: usize,
    direction: Direction,
) -> (bool, bool, Option<String>) {
    let (dr, dc) = delta(direction);
    let next_row = game.state.players[player_idx].row as i32 + dr;
    let next_col = game.state.players[player_idx].col as i32 + dc;

    if !in_bounds(&game.state.map, next_row, next_col) {
        return (false, false, Some("MOVE_OUT_OF_BOUNDS".to_string()));
    }

    let nr = next_row as usize;
    let nc = next_col as usize;

    if game.state.map.cells[nr][nc] != 0 {
        return (false, false, Some("MOVE_BLOCKED_BY_BLOCK".to_string()));
    }

    if player_at(game, nr, nc).is_some() {
        return (false, false, Some("MOVE_BLOCKED_BY_PLAYER".to_string()));
    }

    game.state.players[player_idx].row = nr;
    game.state.players[player_idx].col = nc;
    (true, true, None)
}

fn apply_shoot(
    game: &mut GameInstance,
    player_idx: usize,
    direction: Direction,
) -> (bool, bool, Option<String>) {
    let (shooter_row, shooter_col, shooter_shield) = {
        let shooter = &game.state.players[player_idx];
        (shooter.row, shooter.col, shooter.shield)
    };

    // Cannot shoot through own shield.
    if direction == shooter_shield {
        return (
            false,
            false,
            Some("CANNOT_SHOOT_THROUGH_OWN_SHIELD".to_string()),
        );
    }

    // The laser enters the adjacent cell in the shoot direction.
    let (dr, dc) = delta(direction);
    let entry_row = shooter_row as i32 + dr;
    let entry_col = shooter_col as i32 + dc;

    // Entry cell must be in bounds.
    if !in_bounds(&game.state.map, entry_row, entry_col) {
        return (
            false,
            false,
            Some("SHOOT_BLOCKED_BY_EDGE".to_string()),
        );
    }

    let er = entry_row as usize;
    let ec = entry_col as usize;

    // Entry cell must be empty — no wall, no player.
    if game.state.map.cells[er][ec] != 0 {
        return (
            false,
            false,
            Some("SHOOT_BLOCKED_BY_BLOCK".to_string()),
        );
    }
    if player_at(game, er, ec).is_some() {
        return (
            false,
            false,
            Some("SHOOT_BLOCKED_BY_PLAYER".to_string()),
        );
    }

    // From the entry cell, sweep a laser in both perpendicular directions.
    let (perp1, perp2) = perpendicular_directions(direction);
    sweep_laser(game, er, ec, perp1);
    sweep_laser(game, er, ec, perp2);

    (true, true, None)
}

/// Returns the two directions perpendicular to the given direction.
fn perpendicular_directions(direction: Direction) -> (Direction, Direction) {
    match direction {
        Direction::Up | Direction::Down => (Direction::Left, Direction::Right),
        Direction::Left | Direction::Right => (Direction::Up, Direction::Down),
    }
}

/// Sweep a laser beam from (start_row, start_col) in the given direction,
/// damaging the first wall or player it hits, then stopping.
fn sweep_laser(
    game: &mut GameInstance,
    start_row: usize,
    start_col: usize,
    direction: Direction,
) {
    let (dr, dc) = delta(direction);
    let mut row = start_row as i32 + dr;
    let mut col = start_col as i32 + dc;

    while in_bounds(&game.state.map, row, col) {
        let r = row as usize;
        let c = col as usize;

        // Hit a wall — damage it if destructible, then stop.
        let block = game.state.map.cells[r][c];
        if block != 0 {
            if block > 0 {
                let next = block - 1;
                game.state.map.cells[r][c] = if next <= 0 { 0 } else { next };
            }
            return;
        }

        // Hit a player — check shield, apply damage, then stop.
        if let Some(target_idx) = player_at(game, r, c) {
            let incoming = opposite(direction);
            let target = &mut game.state.players[target_idx];
            if target.shield != incoming {
                target.hp = (target.hp - 1).max(0);
                if target.hp == 0 {
                    target.alive = false;
                }
            }
            return;
        }

        row += dr;
        col += dc;
    }
}

fn player_at(game: &GameInstance, row: usize, col: usize) -> Option<usize> {
    game.state
        .players
        .iter()
        .position(|p| p.alive && p.row == row && p.col == col)
}

fn alive_player_count(game: &GameInstance) -> usize {
    game.state.players.iter().filter(|p| p.alive).count()
}

fn winner_player_id(game: &GameInstance) -> Option<PlayerId> {
    game.state
        .players
        .iter()
        .find(|p| p.alive)
        .map(|p| p.player_id.clone())
}

fn in_bounds(map: &MapData, row: i32, col: i32) -> bool {
    row >= 0 && col >= 0 && (row as usize) < map.rows && (col as usize) < map.cols
}

fn delta(direction: Direction) -> (i32, i32) {
    match direction {
        Direction::Up => (-1, 0),
        Direction::Left => (0, -1),
        Direction::Down => (1, 0),
        Direction::Right => (0, 1),
    }
}

fn opposite(direction: Direction) -> Direction {
    match direction {
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
    }
}

fn advance_turn(game: &mut GameInstance) {
    let player_count = game.state.players.len();
    if player_count == 0 {
        return;
    }

    let Some(current_index) = game
        .state
        .players
        .iter()
        .position(|player| player.player_id == game.current_player_id)
    else {
        return;
    };

    let mut next_index = current_index;
    for _ in 0..player_count {
        next_index = (next_index + 1) % player_count;
        let next_player = &game.state.players[next_index];
        if next_player.alive {
            if next_index <= current_index {
                game.round_no += 1;
            }
            game.current_player_id = next_player.player_id.clone();
            game.turn_no += 1;
            game.turn_started_at = Some(Utc::now());
            return;
        }
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

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
    use axum::extract::{Path, State};
    use std::sync::Mutex;

    struct NoopTopicProvisioner;

    #[async_trait]
    impl TopicProvisioner for NoopTopicProvisioner {
        async fn ensure_game_topics(&self, game_id: &str) -> anyhow::Result<GameTopics> {
            Ok(GameTopics {
                input_topic: format!("test.commands.{game_id}.v1"),
                output_topic: format!("test.output.{game_id}.v1"),
            })
        }

        async fn delete_game_topics(&self, _game_topics: &GameTopics) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopStepEventPublisher;

    #[async_trait]
    impl StepEventPublisher for NoopStepEventPublisher {
        async fn publish_step_event(&self, _topic: &str, _event: &StepEvent) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct NoopBotAssigner;

    #[async_trait]
    impl BotAssigner for NoopBotAssigner {
        async fn assign_for_new_game(
            &self,
            _game: &GameInstance,
            _requested_bot_players: Option<Vec<PlayerName>>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingStepEventPublisher {
        published: Mutex<Vec<(String, StepEvent)>>,
    }

    #[async_trait]
    impl StepEventPublisher for RecordingStepEventPublisher {
        async fn publish_step_event(&self, topic: &str, event: &StepEvent) -> anyhow::Result<()> {
            self.published
                .lock()
                .unwrap()
                .push((topic.to_string(), event.clone()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingTopicProvisioner {
        game_ids: Mutex<Vec<String>>,
        deleted_topics: Mutex<Vec<GameTopics>>,
    }

    #[async_trait]
    impl TopicProvisioner for RecordingTopicProvisioner {
        async fn ensure_game_topics(&self, game_id: &str) -> anyhow::Result<GameTopics> {
            self.game_ids.lock().unwrap().push(game_id.to_string());
            Ok(GameTopics {
                input_topic: format!("record.commands.{game_id}.v1"),
                output_topic: format!("record.output.{game_id}.v1"),
            })
        }

        async fn delete_game_topics(&self, game_topics: &GameTopics) -> anyhow::Result<()> {
            self.deleted_topics
                .lock()
                .unwrap()
                .push(game_topics.clone());
            Ok(())
        }
    }

    fn app_state() -> AppState {
        AppState {
            store: Arc::new(RwLock::new(InMemoryStore::default())),
            topic_provisioner: Arc::new(NoopTopicProvisioner),
            step_event_publisher: Arc::new(NoopStepEventPublisher),
            bot_assigner: Arc::new(NoopBotAssigner),
        }
    }

    fn pid(response: &CreateGameResponse, name: PlayerName) -> PlayerId {
        response
            .players
            .iter()
            .find(|p| p.player_name == name)
            .unwrap_or_else(|| panic!("player {:?} not found in response", name))
            .player_id
            .clone()
    }

    fn custom_map(rows: usize, cols: usize) -> MapData {
        MapData {
            rows,
            cols,
            cells: vec![vec![0; cols]; rows],
        }
    }

    #[tokio::test]
    async fn create_game_without_map_uses_default_map() {
        let state = app_state();
        let response = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: None,
                bot_players: None,
                num_players: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(response.status, GameStatus::Created);
        assert_eq!(response.map_source, MapSource::Default);
        assert_eq!(response.turn_no, 1);
        assert_eq!(response.round_no, 1);
        assert_eq!(response.current_player_id, pid(&response, PlayerName::A));
        assert_eq!(response.turn_timeout_seconds, DEFAULT_TURN_TIMEOUT_SECONDS);
        assert_eq!(response.players.len(), 2);

        let game = get_game_handler(State(state), Path(response.game_id.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(game.state.map.rows, 11);
        assert_eq!(game.state.map.cols, 11);
        assert_eq!(game.state.players.len(), 2);
    }

    #[tokio::test]
    async fn create_game_provisions_per_game_topics() {
        let recorder = Arc::new(RecordingTopicProvisioner::default());
        let state = AppState {
            store: Arc::new(RwLock::new(InMemoryStore::default())),
            topic_provisioner: recorder.clone(),
            step_event_publisher: Arc::new(NoopStepEventPublisher),
            bot_assigner: Arc::new(NoopBotAssigner),
        };

        let response = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let game_ids = recorder.game_ids.lock().unwrap();
        assert_eq!(game_ids.len(), 1);
        assert_eq!(game_ids[0], response.game_id);

        let store = state.store.read().await;
        let game = store.games.get(&response.game_id).unwrap();
        assert_eq!(
            game.input_topic,
            format!("record.commands.{}.v1", response.game_id)
        );
        assert_eq!(
            game.output_topic,
            format!("record.output.{}.v1", response.game_id)
        );
    }

    #[tokio::test]
    async fn create_game_with_custom_map_uses_custom_source() {
        let state = app_state();
        let response = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: Some(15),
                map: Some(custom_map(5, 7)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(response.map_source, MapSource::Custom);
        assert_eq!(response.turn_timeout_seconds, 15);

        let game = get_game_handler(State(state), Path(response.game_id.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(game.state.map.rows, 5);
        assert_eq!(game.state.map.cols, 7);
    }

    #[tokio::test]
    async fn start_game_is_idempotent_for_running_game() {
        let state = app_state();
        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: None,
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let first = start_game_handler(State(state.clone()), Path(created.game_id.clone()))
            .await
            .unwrap()
            .0;
        assert!(first.started);
        assert_eq!(first.status, GameStatus::Running);
        assert!(first.started_at.is_some());

        let second = start_game_handler(State(state), Path(created.game_id))
            .await
            .unwrap()
            .0;
        assert!(!second.started);
        assert_eq!(second.reason.as_deref(), Some("ALREADY_RUNNING"));
    }

    #[tokio::test]
    async fn start_game_publishes_game_started_event_to_output_topic() {
        let publisher = Arc::new(RecordingStepEventPublisher::default());
        let state = AppState {
            store: Arc::new(RwLock::new(InMemoryStore::default())),
            topic_provisioner: Arc::new(NoopTopicProvisioner),
            step_event_publisher: publisher.clone(),
            bot_assigner: Arc::new(NoopBotAssigner),
        };

        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let started = start_game_handler(State(state), Path(created.game_id.clone()))
            .await
            .unwrap()
            .0;
        assert!(started.started);

        let published = publisher.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(
            published[0].0,
            format!("test.output.{}.v1", created.game_id)
        );
        assert_eq!(published[0].1.event_type, StepEventType::GameStarted);
        assert_eq!(published[0].1.turn_no, 1);
    }

    #[tokio::test]
    async fn get_game_returns_not_found_for_unknown_id() {
        let state = app_state();
        let err = get_game_handler(State(state), Path("missing-game".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_default_map_returns_stable_map() {
        let state = app_state();
        let first = get_default_map_handler(State(state.clone()))
            .await
            .unwrap()
            .0;
        let second = get_default_map_handler(State(state)).await.unwrap().0;

        assert_eq!(first.rows, second.rows);
        assert_eq!(first.cols, second.cols);
        assert_eq!(first.cells, second.cells);
    }

    #[tokio::test]
    async fn shoot_toward_own_shield_is_rejected_without_turn_advance() {
        let state = app_state();
        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let _ = start_game_handler(State(state.clone()), Path(created.game_id.clone()))
            .await
            .unwrap();

        let player_a = pid(&created, PlayerName::A);
        let response = apply_command_handler(
            State(state),
            Path(created.game_id),
            Json(SubmitCommandRequest {
                command_id: "cmd-own-shield".to_string(),
                player_id: player_a.clone(),
                command_type: CommandType::Shoot,
                direction: Some(Direction::Up),
                speak_text: None,
                turn_no: 1,
                client_sent_at: Utc::now(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(response.accepted);
        assert!(!response.applied);
        assert_eq!(
            response.reason.as_deref(),
            Some("CANNOT_SHOOT_THROUGH_OWN_SHIELD")
        );
        assert_eq!(response.turn_no, 1);
        assert_eq!(response.current_player_id, player_a);
    }

    #[tokio::test]
    async fn shoot_hits_player_and_advances_turn() {
        let state = app_state();
        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let _ = start_game_handler(State(state.clone()), Path(created.game_id.clone()))
            .await
            .unwrap();

        let player_a = pid(&created, PlayerName::A);
        let player_b = pid(&created, PlayerName::B);
        let player_c = pid(&created, PlayerName::C);
        let response = apply_command_handler(
            State(state.clone()),
            Path(created.game_id.clone()),
            Json(SubmitCommandRequest {
                command_id: "cmd-shoot-down".to_string(),
                player_id: player_a,
                command_type: CommandType::Shoot,
                direction: Some(Direction::Down),
                speak_text: None,
                turn_no: 1,
                client_sent_at: Utc::now(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(response.accepted);
        assert!(response.applied);
        assert_eq!(response.turn_no, 2);
        assert_eq!(response.round_no, 1);
        assert_eq!(response.current_player_id, player_b);

        let game = get_game_handler(State(state), Path(created.game_id))
            .await
            .unwrap()
            .0;
        let down = game
            .state
            .players
            .iter()
            .find(|p| p.player_id == player_c)
            .expect("down player must exist");
        assert_eq!(down.hp, DEFAULT_PLAYER_HP - 1);
        assert!(down.alive);
    }

    #[tokio::test]
    async fn speak_advances_turn_without_state_damage() {
        let state = app_state();
        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let _ = start_game_handler(State(state.clone()), Path(created.game_id.clone()))
            .await
            .unwrap();

        let player_a = pid(&created, PlayerName::A);
        let player_b = pid(&created, PlayerName::B);
        let player_c = pid(&created, PlayerName::C);
        let response = apply_command_handler(
            State(state.clone()),
            Path(created.game_id.clone()),
            Json(SubmitCommandRequest {
                command_id: "cmd-speak".to_string(),
                player_id: player_a,
                command_type: CommandType::Speak,
                direction: None,
                speak_text: Some("hello".to_string()),
                turn_no: 1,
                client_sent_at: Utc::now(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(response.accepted);
        assert!(response.applied);
        assert_eq!(response.turn_no, 2);
        assert_eq!(response.current_player_id, player_b);

        let game = get_game_handler(State(state), Path(created.game_id))
            .await
            .unwrap()
            .0;
        let down = game
            .state
            .players
            .iter()
            .find(|p| p.player_id == player_c)
            .expect("down player must exist");
        assert_eq!(down.hp, DEFAULT_PLAYER_HP);
    }

    #[tokio::test]
    async fn speak_without_text_is_rejected_without_turn_advance() {
        let state = app_state();
        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let _ = start_game_handler(State(state.clone()), Path(created.game_id.clone()))
            .await
            .unwrap();

        let player_a = pid(&created, PlayerName::A);
        let response = apply_command_handler(
            State(state),
            Path(created.game_id),
            Json(SubmitCommandRequest {
                command_id: "cmd-speak-empty".to_string(),
                player_id: player_a.clone(),
                command_type: CommandType::Speak,
                direction: None,
                speak_text: Some("   ".to_string()),
                turn_no: 1,
                client_sent_at: Utc::now(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(response.accepted);
        assert!(!response.applied);
        assert_eq!(response.reason.as_deref(), Some("MISSING_SPEAK_TEXT"));
        assert_eq!(response.turn_no, 1);
        assert_eq!(response.current_player_id, player_a);
    }

    #[tokio::test]
    async fn finish_game_marks_status_when_one_player_left() {
        let recorder = Arc::new(RecordingTopicProvisioner::default());
        let step_publisher = Arc::new(RecordingStepEventPublisher::default());
        let state = AppState {
            store: Arc::new(RwLock::new(InMemoryStore::default())),
            topic_provisioner: recorder.clone(),
            step_event_publisher: step_publisher.clone(),
            bot_assigner: Arc::new(NoopBotAssigner),
        };
        let created = create_game_handler(
            State(state.clone()),
            Json(CreateGameRequest {
                turn_timeout_seconds: None,
                map: Some(custom_map(5, 5)),
                bot_players: None,
                num_players: Some(4),
            }),
        )
        .await
        .unwrap()
        .0;

        let _ = start_game_handler(State(state.clone()), Path(created.game_id.clone()))
            .await
            .unwrap();
        let game_id = created.game_id.clone();
        let player_a = pid(&created, PlayerName::A);

        {
            let mut store = state.store.write().await;
            let game = store.games.get_mut(&created.game_id).unwrap();
            for player in &mut game.state.players {
                if player.player_id != player_a {
                    player.alive = false;
                    player.hp = 0;
                }
            }
        }

        let finished = finish_game_handler(
            State(state),
            Path(game_id.clone()),
            Json(FinishGameRequest {
                expected_turn_no: Some(1),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(finished.finished);
        assert_eq!(finished.status, GameStatus::Finished);
        assert_eq!(finished.winner_player_id, Some(player_a));

        let deleted_topics = recorder.deleted_topics.lock().unwrap();
        assert_eq!(deleted_topics.len(), 1);
        assert_eq!(
            deleted_topics[0].input_topic,
            format!("record.commands.{}.v1", game_id)
        );
        assert_eq!(
            deleted_topics[0].output_topic,
            format!("record.output.{}.v1", game_id)
        );

        let published = step_publisher.published.lock().unwrap();
        assert_eq!(published.len(), 2);
        assert_eq!(published[1].0, format!("record.output.{}.v1", game_id));
        assert_eq!(published[1].1.event_type, StepEventType::GameFinished);
    }
}
