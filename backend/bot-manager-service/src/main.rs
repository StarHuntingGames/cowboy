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
    GameInstanceResponse, GameStatus, PlayerId, PlayerName, StepEvent, StepEventType,
    expand_env_vars,
};
use rdkafka::{
    Message,
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    assignments: Arc<Mutex<HashMap<String, GameAssignment>>>,
    game_topic_workers: Arc<Mutex<HashMap<String, GameTopicWorker>>>,
    client: reqwest::Client,
    manager_base_url: String,
    bot_service_base_urls: Vec<String>,
    bots_per_instance_capacity: usize,
    llm_profiles: LlmProfilesConfig,
    bot_state_store: Option<BotStateStore>,
    bootstrap_servers: String,
    output_topic_prefix: String,
    consumer_group_id: String,
    default_game_guide_version: String,
}

struct GameTopicWorker {
    output_topic: String,
    stop_tx: Option<oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct BotStateStore {
    client: DynamoClient,
    table_name: String,
}

#[derive(Debug, Clone)]
struct GameAssignment {
    game_id: String,
    humans: HashMap<PlayerId, PlayerName>,
    bindings: HashMap<PlayerId, BotBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BotBinding {
    player_name: PlayerName,
    player_id: PlayerId,
    bot_id: String,
    bot_service_base_url: String,
    status: String,
    game_guide_version: String,
}

#[derive(Debug, Deserialize)]
struct DefaultAssignmentRequest {
    #[serde(default)]
    apply_immediately: Option<bool>,
    #[serde(default)]
    game_guide_version: Option<String>,
    #[serde(default)]
    force_recreate: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BulkAssignmentRequest {
    human_player_ids: Vec<PlayerId>,
    bot_player_ids: Vec<PlayerId>,
    #[serde(default)]
    game_guide_version: Option<String>,
    #[serde(default)]
    force_recreate: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BindBotRequest {
    player_id: PlayerId,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    create_bot_if_missing: Option<bool>,
    #[serde(default)]
    game_guide_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StopBotsRequest {
    #[allow(dead_code)]
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct AssignmentResponse {
    game_id: String,
    humans: Vec<HumanAssignment>,
    bindings: Vec<BotBinding>,
}

#[derive(Debug, Serialize)]
struct HumanAssignment {
    player_name: PlayerName,
    player_id: PlayerId,
}

#[derive(Debug, Serialize)]
struct DefaultAssignmentResult {
    assigned: bool,
    game_id: String,
    humans: Vec<HumanAssignment>,
    bindings: Vec<BotBinding>,
}

#[derive(Debug, Serialize)]
struct BindResponse {
    bound: bool,
    game_id: String,
    player_id: PlayerId,
    bot_id: String,
    bot_service_base_url: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct StopBotsResponse {
    stopped: bool,
    game_id: String,
    destroyed_bot_count: usize,
}

#[derive(Debug, Serialize)]
struct BotCreateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    bot_id: Option<String>,
    game_id: String,
    player_name: PlayerName,
    player_id: PlayerId,
    input_topic: String,
    output_topic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_output_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BotCreateResponse {
    bot_id: String,
}

#[derive(Debug, Serialize)]
struct BotEventUpdateRequest {
    step: StepEvent,
}

#[derive(Debug, Deserialize)]
struct BotEventUpdateResponse {
    #[allow(dead_code)]
    accepted: bool,
    #[allow(dead_code)]
    bot_id: String,
}

#[derive(Debug, Serialize)]
struct TeachGameRequest {
    game_guide_version: String,
    rules_markdown: String,
    command_schema: serde_json::Value,
    examples: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
struct LlmProfilesConfig {
    default: Option<LlmProfile>,
    players: HashMap<PlayerName, LlmProfile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LlmProfile {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    output_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LlmProfilesConfigFile {
    #[serde(default)]
    default: LlmProfile,
    #[serde(default)]
    players: HashMap<String, LlmProfile>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "bot_manager_service=debug,tower_http=info".to_string()),
        )
        .init();

    let bot_state_store = load_bot_state_store().await;
    let state = AppState {
        assignments: Arc::new(Mutex::new(HashMap::new())),
        game_topic_workers: Arc::new(Mutex::new(HashMap::new())),
        client: reqwest::Client::new(),
        manager_base_url: std::env::var("GAME_MANAGER_BASE_URL")
            .ok()
            .unwrap_or_else(|| "http://game-manager-service:8081".to_string()),
        bot_service_base_urls: parse_bot_service_base_urls(),
        bots_per_instance_capacity: parse_instance_capacity(),
        llm_profiles: load_llm_profiles_config(),
        bot_state_store,
        bootstrap_servers: std::env::var("KAFKA_BOOTSTRAP_SERVERS")
            .ok()
            .unwrap_or_else(|| "kafka:9092".to_string()),
        output_topic_prefix: std::env::var("GAME_OUTPUT_TOPIC_PREFIX")
            .ok()
            .unwrap_or_else(|| "game.output".to_string()),
        consumer_group_id: std::env::var("BOT_MANAGER_CONSUMER_GROUP_ID")
            .ok()
            .unwrap_or_else(|| "bot-manager-v3".to_string()),
        default_game_guide_version: std::env::var("BOT_GAME_GUIDE_VERSION")
            .ok()
            .unwrap_or_else(|| "v1".to_string()),
    };
    info!(
        bot_service_base_urls = ?state.bot_service_base_urls,
        bots_per_instance_capacity = state.bots_per_instance_capacity,
        llm_default_configured = state.llm_profiles.default.is_some(),
        llm_players_configured = state.llm_profiles.players.len(),
        bot_state_store_enabled = state.bot_state_store.is_some(),
        "bot-manager loaded bot-service instance config"
    );

    let kafka_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = run_output_consumer(kafka_state).await {
            warn!(error = %error, "bot-manager output consumer stopped");
        }
    });

    let app = build_router(state);
    let bind_addr = parse_bind_addr("BOT_MANAGER_BIND", "0.0.0.0:8090")?;
    info!(%bind_addr, "bot-manager-service listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/internal/v3/games/{game_id}/assignments/default",
            post(default_assignment_handler),
        )
        .route(
            "/internal/v3/games/{game_id}/assignments",
            post(assignments_handler).get(get_assignments_handler),
        )
        .route(
            "/internal/v3/games/{game_id}/bindings",
            post(bind_bot_handler),
        )
        .route(
            "/internal/v3/games/{game_id}/bots/stop",
            post(stop_bots_handler),
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

fn parse_bot_service_base_urls() -> Vec<String> {
    if let Ok(raw) = std::env::var("BOT_SERVICE_BASE_URLS") {
        let values: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect();
        if !values.is_empty() {
            return values;
        }
    }

    vec![
        std::env::var("BOT_SERVICE_BASE_URL")
            .ok()
            .unwrap_or_else(|| "http://bot-service:8091".to_string()),
    ]
}

fn parse_instance_capacity() -> usize {
    std::env::var("BOTS_PER_INSTANCE_CAPACITY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(2)
}

async fn load_bot_state_store() -> Option<BotStateStore> {
    if std::env::var("DYNAMODB_ENDPOINT").is_err() && std::env::var("AWS_REGION").is_err() {
        return None;
    }

    let mut loader = aws_config::defaults(BehaviorVersion::latest());
    if let Ok(endpoint) = std::env::var("DYNAMODB_ENDPOINT") {
        loader = loader.endpoint_url(endpoint);
    }
    let config = loader.load().await;
    let table_name = std::env::var("BOT_STATE_TABLE")
        .ok()
        .unwrap_or_else(|| "bot_players".to_string());

    info!(table_name = %table_name, "bot-manager DynamoDB state store enabled");
    Some(BotStateStore {
        client: DynamoClient::new(&config),
        table_name,
    })
}

fn load_llm_profiles_config() -> LlmProfilesConfig {
    let Some(path) = std::env::var("BOT_MANAGER_LLM_CONFIG_PATH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return LlmProfilesConfig::default();
    };

    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to read bot-manager llm config path");
            return LlmProfilesConfig::default();
        }
    };

    let expanded = expand_env_vars(&raw);
    let parsed = match serde_yaml::from_str::<LlmProfilesConfigFile>(&expanded) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to parse bot-manager llm config yaml");
            return LlmProfilesConfig::default();
        }
    };

    let default = parsed.default.normalized();
    let mut players: HashMap<PlayerName, LlmProfile> = HashMap::new();
    for (name, profile) in parsed.players {
        let Some(player_name) = parse_player_name(&name) else {
            warn!(player_name = %name, "ignoring unknown player key in llm config");
            continue;
        };

        if let Some(profile) = profile.normalized() {
            players.insert(player_name, profile);
        }
    }

    let config = LlmProfilesConfig { default, players };
    info!(
        path = %path,
        default_configured = config.default.is_some(),
        player_config_count = config.players.len(),
        "loaded bot-manager llm profile config"
    );
    config
}

fn parse_player_name(value: &str) -> Option<PlayerName> {
    match value.trim().to_ascii_uppercase().as_str() {
        "A" => Some(PlayerName::A),
        "B" => Some(PlayerName::B),
        "C" => Some(PlayerName::C),
        "D" => Some(PlayerName::D),
        _ => None,
    }
}

impl LlmProfile {
    fn normalized(self) -> Option<Self> {
        let base_url = normalize_optional_string(self.base_url);
        let model = normalize_optional_string(self.model);
        let api_key = normalize_optional_string(self.api_key);
        let output_mode = normalize_optional_string(self.output_mode);

        if base_url.is_none() && model.is_none() && api_key.is_none() && output_mode.is_none() {
            None
        } else {
            Some(Self {
                base_url,
                model,
                api_key,
                output_mode,
            })
        }
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
}

fn resolve_llm_profile(config: &LlmProfilesConfig, player_name: PlayerName) -> Option<LlmProfile> {
    let default = config.default.clone();
    let player = config.players.get(&player_name).cloned();

    let merged = LlmProfile {
        base_url: player
            .as_ref()
            .and_then(|value| value.base_url.clone())
            .or_else(|| default.as_ref().and_then(|value| value.base_url.clone())),
        model: player
            .as_ref()
            .and_then(|value| value.model.clone())
            .or_else(|| default.as_ref().and_then(|value| value.model.clone())),
        api_key: player
            .as_ref()
            .and_then(|value| value.api_key.clone())
            .or_else(|| default.as_ref().and_then(|value| value.api_key.clone())),
        output_mode: player
            .as_ref()
            .and_then(|value| value.output_mode.clone())
            .or_else(|| default.as_ref().and_then(|value| value.output_mode.clone())),
    };

    merged.normalized()
}

fn player_name_value(player_name: PlayerName) -> &'static str {
    match player_name {
        PlayerName::A => "A",
        PlayerName::B => "B",
        PlayerName::C => "C",
        PlayerName::D => "D",
    }
}

fn game_status_value(status: GameStatus) -> &'static str {
    match status {
        GameStatus::Created => "CREATED",
        GameStatus::Running => "RUNNING",
        GameStatus::Finished => "FINISHED",
    }
}

async fn upsert_bot_state_record(
    state: &AppState,
    payload: &BotCreateRequest,
    bot_id: &str,
    bot_service_base_url: &str,
    game_guide_version: &str,
    bot_status: &str,
    player_state: &str,
    game_status: GameStatus,
) -> Result<(), ApiError> {
    let Some(store) = state.bot_state_store.as_ref() else {
        return Ok(());
    };

    let now = Utc::now().to_rfc3339();
    let mut item = HashMap::new();
    item.insert(
        "game_id".to_string(),
        AttributeValue::S(payload.game_id.clone()),
    );
    item.insert(
        "player_id".to_string(),
        AttributeValue::S(payload.player_id.clone()),
    );
    item.insert("bot_id".to_string(), AttributeValue::S(bot_id.to_string()));
    item.insert(
        "player_name".to_string(),
        AttributeValue::S(player_name_value(payload.player_name).to_string()),
    );
    item.insert(
        "bot_service_base_url".to_string(),
        AttributeValue::S(bot_service_base_url.to_string()),
    );
    item.insert(
        "game_guide_version".to_string(),
        AttributeValue::S(game_guide_version.to_string()),
    );
    item.insert(
        "bot_status".to_string(),
        AttributeValue::S(bot_status.to_string()),
    );
    item.insert(
        "player_state".to_string(),
        AttributeValue::S(player_state.to_string()),
    );
    item.insert(
        "game_state".to_string(),
        AttributeValue::S(game_status_value(game_status).to_string()),
    );
    item.insert("created_at".to_string(), AttributeValue::S(now.clone()));
    item.insert("updated_at".to_string(), AttributeValue::S(now));

    match payload.llm_model.as_ref() {
        Some(model) => {
            item.insert("model".to_string(), AttributeValue::S(model.clone()));
        }
        None => {
            item.insert("model".to_string(), AttributeValue::Null(true));
        }
    }
    match payload.llm_base_url.as_ref() {
        Some(base_url) => {
            item.insert("base_url".to_string(), AttributeValue::S(base_url.clone()));
        }
        None => {
            item.insert("base_url".to_string(), AttributeValue::Null(true));
        }
    }
    match payload.llm_api_key.as_ref() {
        Some(api_key) => {
            item.insert("api_key".to_string(), AttributeValue::S(api_key.clone()));
        }
        None => {
            item.insert("api_key".to_string(), AttributeValue::Null(true));
        }
    }

    store
        .client
        .put_item()
        .table_name(&store.table_name)
        .set_item(Some(item))
        .send()
        .await
        .map_err(|error| {
            ApiError::bad_gateway(format!("failed to persist bot state record: {error}"))
        })?;

    Ok(())
}

async fn update_bot_state_record(
    state: &AppState,
    game_id: &str,
    player_id: &str,
    bot_status: &str,
    player_state: &str,
    game_status: GameStatus,
) -> Result<(), ApiError> {
    let Some(store) = state.bot_state_store.as_ref() else {
        return Ok(());
    };

    let now = Utc::now().to_rfc3339();
    store
        .client
        .update_item()
        .table_name(&store.table_name)
        .key("game_id", AttributeValue::S(game_id.to_string()))
        .key("player_id", AttributeValue::S(player_id.to_string()))
        .update_expression("SET bot_status = :bot_status, player_state = :player_state, game_state = :game_state, updated_at = :updated_at")
        .expression_attribute_values(":bot_status", AttributeValue::S(bot_status.to_string()))
        .expression_attribute_values(":player_state", AttributeValue::S(player_state.to_string()))
        .expression_attribute_values(
            ":game_state",
            AttributeValue::S(game_status_value(game_status).to_string()),
        )
        .expression_attribute_values(":updated_at", AttributeValue::S(now))
        .send()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("failed to update bot state record: {error}")))?;

    Ok(())
}

async fn update_game_state_record(
    state: &AppState,
    game_id: &str,
    player_id: &str,
    game_status: GameStatus,
) -> Result<(), ApiError> {
    let Some(store) = state.bot_state_store.as_ref() else {
        return Ok(());
    };

    let now = Utc::now().to_rfc3339();
    store
        .client
        .update_item()
        .table_name(&store.table_name)
        .key("game_id", AttributeValue::S(game_id.to_string()))
        .key("player_id", AttributeValue::S(player_id.to_string()))
        .update_expression("SET game_state = :game_state, updated_at = :updated_at")
        .expression_attribute_values(
            ":game_state",
            AttributeValue::S(game_status_value(game_status).to_string()),
        )
        .expression_attribute_values(":updated_at", AttributeValue::S(now))
        .send()
        .await
        .map_err(|error| {
            ApiError::bad_gateway(format!("failed to update game state record: {error}"))
        })?;

    Ok(())
}

async fn update_assignment_game_state(
    state: &AppState,
    assignment: &GameAssignment,
    game_status: GameStatus,
) {
    for binding in assignment.bindings.values() {
        if let Err(error) =
            update_game_state_record(state, &assignment.game_id, &binding.player_id, game_status)
                .await
        {
            warn!(
                game_id = %assignment.game_id,
                player_id = %binding.player_id,
                error = %error.message,
                "failed to update bot table game_state"
            );
        }
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true, "service": "bot-manager-service"}))
}

async fn default_assignment_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<DefaultAssignmentRequest>,
) -> Result<Json<DefaultAssignmentResult>, ApiError> {
    let apply_immediately = request.apply_immediately.unwrap_or(true);
    let guide_version = request
        .game_guide_version
        .unwrap_or_else(|| state.default_game_guide_version.clone());
    let force_recreate = request.force_recreate.unwrap_or(false);

    let assignment = assign_default_for_game(
        &state,
        &game_id,
        apply_immediately,
        &guide_version,
        force_recreate,
    )
    .await?;

    let response = assignment_to_response(&assignment);
    Ok(Json(DefaultAssignmentResult {
        assigned: true,
        game_id,
        humans: response.humans,
        bindings: response.bindings,
    }))
}

async fn assignments_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<BulkAssignmentRequest>,
) -> Result<Json<DefaultAssignmentResult>, ApiError> {
    let game = fetch_game(&state, &game_id).await?;
    let guide_version = request
        .game_guide_version
        .unwrap_or_else(|| state.default_game_guide_version.clone());
    let force_recreate = request.force_recreate.unwrap_or(false);

    let humans: HashSet<String> = request.human_player_ids.into_iter().collect();
    let bots: HashSet<String> = request.bot_player_ids.into_iter().collect();

    let assignment = assign_players_for_game(
        &state,
        &game,
        humans,
        bots,
        &guide_version,
        true,
        force_recreate,
    )
    .await?;

    let response = assignment_to_response(&assignment);
    Ok(Json(DefaultAssignmentResult {
        assigned: true,
        game_id,
        humans: response.humans,
        bindings: response.bindings,
    }))
}

async fn bind_bot_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<BindBotRequest>,
) -> Result<Json<BindResponse>, ApiError> {
    let game = fetch_game(&state, &game_id).await?;
    let player = game
        .state
        .players
        .iter()
        .find(|player| player.player_id == request.player_id)
        .ok_or_else(|| ApiError::bad_request("player_id does not belong to game"))?;

    let create_bot_if_missing = request.create_bot_if_missing.unwrap_or(true);
    let guide_version = request
        .game_guide_version
        .unwrap_or_else(|| state.default_game_guide_version.clone());

    let existing_assignment = {
        let assignments = state.assignments.lock().await;
        assignments.get(&game_id).cloned()
    };

    let existing_binding = existing_assignment
        .as_ref()
        .and_then(|assignment| assignment.bindings.get(&request.player_id).cloned());

    let desired_bot_id = request.bot_id.clone().or_else(|| {
        existing_binding
            .as_ref()
            .map(|binding| binding.bot_id.clone())
    });

    if desired_bot_id.is_none() && !create_bot_if_missing {
        return Err(ApiError::bad_request(
            "bot_id is required when create_bot_if_missing=false",
        ));
    }

    let binding = ensure_binding(
        &state,
        &game,
        player.player_name,
        &player.player_id,
        desired_bot_id,
        &guide_version,
        false,
        &HashMap::new(),
    )
    .await?;

    let old_binding_to_delete = {
        let mut assignments = state.assignments.lock().await;
        let assignment = assignments
            .entry(game_id.clone())
            .or_insert_with(|| GameAssignment {
                game_id: game_id.clone(),
                humans: game
                    .state
                    .players
                    .iter()
                    .map(|entry| (entry.player_id.clone(), entry.player_name))
                    .collect(),
                bindings: HashMap::new(),
            });

        let old_binding_to_delete =
            assignment
                .bindings
                .remove(&binding.player_id)
                .filter(|previous| {
                    previous.bot_id != binding.bot_id
                        || previous.bot_service_base_url != binding.bot_service_base_url
                });

        assignment.humans.remove(&binding.player_id);
        assignment
            .bindings
            .insert(binding.player_id.clone(), binding.clone());
        old_binding_to_delete
    };

    if let Some(old_binding) = old_binding_to_delete {
        if let Err(error) = update_bot_state_record(
            &state,
            &game.game_id,
            &old_binding.player_id,
            "STOPPED",
            "BOT_UNASSIGNED",
            game.status,
        )
        .await
        {
            warn!(
                game_id = %game.game_id,
                player_id = %old_binding.player_id,
                error = %error.message,
                "failed to update previous bot binding state before delete"
            );
        }
        let _ = delete_bot(
            &state,
            &old_binding.bot_service_base_url,
            &old_binding.bot_id,
        )
        .await;
    }

    if game.status == GameStatus::Running
        && let Some(output_topic) = game.output_topic.as_deref()
        && let Err(error) = ensure_game_topic_worker(&state, &game.game_id, output_topic).await
    {
        warn!(
            game_id = %game.game_id,
            output_topic = %output_topic,
            error = %error,
            "failed to ensure per-game output consumer after bind"
        );
    }

    Ok(Json(BindResponse {
        bound: true,
        game_id,
        player_id: binding.player_id,
        bot_id: binding.bot_id,
        bot_service_base_url: binding.bot_service_base_url,
        status: binding.status,
    }))
}

async fn get_assignments_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignments = state.assignments.lock().await;
    let assignment = assignments
        .get(&game_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("no assignments for game {}", game_id)))?;
    Ok(Json(assignment_to_response(&assignment)))
}

async fn stop_bots_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(_request): Json<StopBotsRequest>,
) -> Result<Json<StopBotsResponse>, ApiError> {
    let game_status = fetch_game(&state, &game_id)
        .await
        .ok()
        .map(|game| game.status);
    let destroyed = stop_bots_for_game(&state, &game_id, game_status, true).await;
    Ok(Json(StopBotsResponse {
        stopped: true,
        game_id,
        destroyed_bot_count: destroyed,
    }))
}

async fn assign_default_for_game(
    state: &AppState,
    game_id: &str,
    apply_immediately: bool,
    guide_version: &str,
    force_recreate: bool,
) -> Result<GameAssignment, ApiError> {
    let game = fetch_game(state, game_id).await?;

    let mut humans = HashSet::new();
    let mut bots = HashSet::new();

    for player in &game.state.players {
        if player.player_name == PlayerName::A {
            humans.insert(player.player_id.clone());
        } else {
            bots.insert(player.player_id.clone());
        }
    }

    assign_players_for_game(
        state,
        &game,
        humans,
        bots,
        guide_version,
        apply_immediately,
        force_recreate,
    )
    .await
}

async fn assign_players_for_game(
    state: &AppState,
    game: &GameInstanceResponse,
    humans: HashSet<PlayerId>,
    bots: HashSet<PlayerId>,
    guide_version: &str,
    apply_immediately: bool,
    force_recreate: bool,
) -> Result<GameAssignment, ApiError> {
    if humans.intersection(&bots).next().is_some() {
        return Err(ApiError::bad_request(
            "human_player_ids and bot_player_ids must not overlap",
        ));
    }

    let players_by_id: HashMap<PlayerId, PlayerName> = game
        .state
        .players
        .iter()
        .map(|player| (player.player_id.clone(), player.player_name))
        .collect();

    for player_id in humans.iter().chain(bots.iter()) {
        if !players_by_id.contains_key(player_id) {
            return Err(ApiError::bad_request(format!(
                "player_id {} does not belong to game",
                player_id
            )));
        }
    }

    let existing_assignment = {
        let assignments = state.assignments.lock().await;
        assignments.get(&game.game_id).cloned()
    };

    let mut next_bindings: HashMap<PlayerId, BotBinding> = HashMap::new();
    for player_id in &bots {
        let player_name = *players_by_id
            .get(player_id)
            .ok_or_else(|| ApiError::bad_request("bot player id not found"))?;

        let existing_bot_id = existing_assignment
            .as_ref()
            .and_then(|assignment| assignment.bindings.get(player_id))
            .map(|binding| binding.bot_id.clone());

        if apply_immediately {
            let binding = ensure_binding(
                state,
                game,
                player_name,
                player_id,
                existing_bot_id,
                guide_version,
                force_recreate,
                &next_bindings,
            )
            .await?;
            next_bindings.insert(player_id.clone(), binding);
        } else if let Some(existing) = existing_assignment
            .as_ref()
            .and_then(|assignment| assignment.bindings.get(player_id))
            .cloned()
        {
            next_bindings.insert(player_id.clone(), existing);
        }
    }

    if let Some(previous) = existing_assignment {
        for (player_id, binding) in previous.bindings {
            if !bots.contains(&player_id) {
                if let Err(error) = update_bot_state_record(
                    state,
                    &game.game_id,
                    &binding.player_id,
                    "STOPPED",
                    "BOT_UNASSIGNED",
                    game.status,
                )
                .await
                {
                    warn!(
                        game_id = %game.game_id,
                        player_id = %binding.player_id,
                        error = %error.message,
                        "failed to update unassigned bot state before delete"
                    );
                }
                let _ = delete_bot(state, &binding.bot_service_base_url, &binding.bot_id).await;
            }
        }
    }

    let next_humans: HashMap<PlayerId, PlayerName> = humans
        .iter()
        .filter_map(|id| players_by_id.get(id).map(|name| (id.clone(), *name)))
        .collect();

    let assignment = GameAssignment {
        game_id: game.game_id.clone(),
        humans: next_humans,
        bindings: next_bindings,
    };

    {
        let mut assignments = state.assignments.lock().await;
        assignments.insert(game.game_id.clone(), assignment.clone());
    }

    if game.status == GameStatus::Running
        && let Some(output_topic) = game.output_topic.as_deref()
        && let Err(error) = ensure_game_topic_worker(state, &game.game_id, output_topic).await
    {
        warn!(
            game_id = %game.game_id,
            output_topic = %output_topic,
            error = %error,
            "failed to ensure per-game output consumer after assignment"
        );
    }

    Ok(assignment)
}

async fn ensure_binding(
    state: &AppState,
    game: &GameInstanceResponse,
    player_name: PlayerName,
    player_id: &str,
    desired_bot_id: Option<String>,
    guide_version: &str,
    force_recreate: bool,
    pending_bindings: &HashMap<PlayerId, BotBinding>,
) -> Result<BotBinding, ApiError> {
    let input_topic = game
        .input_topic
        .clone()
        .ok_or_else(|| ApiError::bad_gateway("game has no input_topic"))?;
    let output_topic = game
        .output_topic
        .clone()
        .ok_or_else(|| ApiError::bad_gateway("game has no output_topic"))?;

    let maybe_existing = {
        let assignments = state.assignments.lock().await;
        assignments
            .get(&game.game_id)
            .and_then(|assignment| assignment.bindings.get(player_id).cloned())
    };

    if let Some(existing) = maybe_existing.clone() {
        if !force_recreate {
            return Ok(existing);
        }
        if let Err(error) = update_bot_state_record(
            state,
            &game.game_id,
            &existing.player_id,
            "STOPPED",
            "BOT_REPLACED",
            game.status,
        )
        .await
        {
            warn!(
                game_id = %game.game_id,
                player_id = %existing.player_id,
                error = %error.message,
                "failed to update existing bot state before force recreate"
            );
        }
        let _ = delete_bot(state, &existing.bot_service_base_url, &existing.bot_id).await;
    }

    let preferred_instance_url = maybe_existing
        .as_ref()
        .map(|binding| binding.bot_service_base_url.as_str());
    let bot_service_base_url =
        select_bot_service_base_url(state, preferred_instance_url, pending_bindings).await?;

    let llm_profile = resolve_llm_profile(&state.llm_profiles, player_name);
    let create_payload = BotCreateRequest {
        bot_id: desired_bot_id.clone(),
        game_id: game.game_id.clone(),
        player_name,
        player_id: player_id.to_string(),
        input_topic,
        output_topic,
        llm_base_url: llm_profile
            .as_ref()
            .and_then(|profile| profile.base_url.clone()),
        llm_model: llm_profile
            .as_ref()
            .and_then(|profile| profile.model.clone()),
        llm_api_key: llm_profile
            .as_ref()
            .and_then(|profile| profile.api_key.clone()),
        llm_output_mode: llm_profile
            .as_ref()
            .and_then(|profile| profile.output_mode.clone()),
    };

    let bot_id = match create_bot(state, &bot_service_base_url, &create_payload).await {
        Ok(response) => response.bot_id,
        Err(error) => {
            if let Some(id) = desired_bot_id.clone() {
                // Existing bot id may already exist; treat that as attach and continue to teach.
                warn!(bot_id = %id, error = %error.message, "bot create failed; attempting to continue with existing bot id");
                id
            } else {
                return Err(error);
            }
        }
    };

    upsert_bot_state_record(
        state,
        &create_payload,
        &bot_id,
        &bot_service_base_url,
        guide_version,
        "CREATED",
        "BOT_ASSIGNED",
        game.status,
    )
    .await?;

    teach_game(state, &bot_service_base_url, &bot_id, guide_version).await?;

    update_bot_state_record(
        state,
        &game.game_id,
        player_id,
        "READY",
        "BOT_READY",
        game.status,
    )
    .await?;

    Ok(BotBinding {
        player_name,
        player_id: player_id.to_string(),
        bot_id,
        bot_service_base_url,
        status: "READY".to_string(),
        game_guide_version: guide_version.to_string(),
    })
}

async fn create_bot(
    state: &AppState,
    bot_service_base_url: &str,
    payload: &BotCreateRequest,
) -> Result<BotCreateResponse, ApiError> {
    let url = format!("{}/internal/v3/bots", bot_service_base_url);
    let response = state
        .client
        .post(url)
        .json(payload)
        .send()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("bot create request failed: {error}")))?;

    if response.status() == StatusCode::CONFLICT {
        let bot_id = payload
            .bot_id
            .clone()
            .ok_or_else(|| ApiError::bad_gateway("bot create conflict without bot_id"))?;
        return Ok(BotCreateResponse { bot_id });
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "bot create returned {}: {}",
            status, body
        )));
    }

    response
        .json::<BotCreateResponse>()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("invalid bot create response: {error}")))
}

async fn teach_game(
    state: &AppState,
    bot_service_base_url: &str,
    bot_id: &str,
    guide_version: &str,
) -> Result<(), ApiError> {
    let url = format!(
        "{}/internal/v3/bots/{}/teach-game",
        bot_service_base_url, bot_id
    );
    let payload = TeachGameRequest {
        game_guide_version: guide_version.to_string(),
        rules_markdown: default_rules_markdown(),
        command_schema: serde_json::json!({
            "allowed": ["move", "shoot", "shield", "speak"],
            "direction_required_for": ["move", "shoot", "shield"],
            "speak_text_required_for": ["speak"]
        }),
        examples: vec![
            serde_json::json!({"command_type":"move","direction":"up"}),
            serde_json::json!({"command_type":"speak","speak_text":"Watch this turn."}),
        ],
    };

    let response = state
        .client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("teach-game request failed: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "teach-game returned {}: {}",
            status, body
        )));
    }

    Ok(())
}

async fn update_bot_from_step_event(
    state: &AppState,
    binding: &BotBinding,
    step: &StepEvent,
) -> Result<(), ApiError> {
    let url = format!(
        "{}/internal/v3/bots/{}/update",
        binding.bot_service_base_url, binding.bot_id
    );
    let payload = BotEventUpdateRequest { step: step.clone() };
    let response = state
        .client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("bot update request failed: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "bot update returned {}: {}",
            status, body
        )));
    }

    let _ = response
        .json::<BotEventUpdateResponse>()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("invalid bot update response: {error}")))?;
    Ok(())
}

async fn delete_bot(
    state: &AppState,
    bot_service_base_url: &str,
    bot_id: &str,
) -> Result<(), ApiError> {
    let url = format!("{}/internal/v3/bots/{}", bot_service_base_url, bot_id);
    let response =
        state.client.delete(url).send().await.map_err(|error| {
            ApiError::bad_gateway(format!("bot delete request failed: {error}"))
        })?;

    if !response.status().is_success() && response.status() != StatusCode::NOT_FOUND {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        return Err(ApiError::bad_gateway(format!(
            "bot delete returned {}: {}",
            status, body
        )));
    }

    Ok(())
}

async fn fetch_game(state: &AppState, game_id: &str) -> Result<GameInstanceResponse, ApiError> {
    let url = format!("{}/v2/games/{}", state.manager_base_url, game_id);
    let response = state
        .client
        .get(url)
        .send()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("manager request failed: {error}")))?;

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

    response
        .json::<GameInstanceResponse>()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("invalid manager response: {error}")))
}

fn assignment_to_response(assignment: &GameAssignment) -> AssignmentResponse {
    let mut humans: Vec<HumanAssignment> = assignment
        .humans
        .iter()
        .map(|(player_id, player_name)| HumanAssignment {
            player_name: *player_name,
            player_id: player_id.clone(),
        })
        .collect();
    humans.sort_by_key(|entry| player_sort_key(entry.player_name));

    let mut bindings: Vec<BotBinding> = assignment.bindings.values().cloned().collect();
    bindings.sort_by_key(|entry| player_sort_key(entry.player_name));

    AssignmentResponse {
        game_id: assignment.game_id.clone(),
        humans,
        bindings,
    }
}

fn player_sort_key(player_name: PlayerName) -> u8 {
    match player_name {
        PlayerName::A => 0,
        PlayerName::B => 1,
        PlayerName::C => 2,
        PlayerName::D => 3,
    }
}

fn default_rules_markdown() -> String {
    "Cowboy game rules: one command per turn; valid commands are move, shoot, shield, speak. Timeouts advance turn. Late commands are ignored by game service but recorded.".to_string()
}

async fn select_bot_service_base_url(
    state: &AppState,
    preferred_base_url: Option<&str>,
    pending_bindings: &HashMap<PlayerId, BotBinding>,
) -> Result<String, ApiError> {
    if state.bot_service_base_urls.is_empty() {
        return Err(ApiError::bad_gateway(
            "no bot-service instance configured (BOT_SERVICE_BASE_URLS)",
        ));
    }

    let mut loads: HashMap<String, usize> = state
        .bot_service_base_urls
        .iter()
        .cloned()
        .map(|url| (url, 0usize))
        .collect();

    {
        let assignments = state.assignments.lock().await;
        for assignment in assignments.values() {
            for binding in assignment.bindings.values() {
                *loads
                    .entry(binding.bot_service_base_url.clone())
                    .or_insert(0usize) += 1;
            }
        }
    }

    for binding in pending_bindings.values() {
        *loads
            .entry(binding.bot_service_base_url.clone())
            .or_insert(0usize) += 1;
    }

    if let Some(preferred) = preferred_base_url {
        if !preferred.trim().is_empty() {
            loads.entry(preferred.to_string()).or_insert(0usize);
            return Ok(preferred.to_string());
        }
    }

    let mut ranked: Vec<(String, usize)> = loads.into_iter().collect();
    ranked.sort_by_key(|entry| entry.1);

    if let Some((url, _)) = ranked
        .iter()
        .find(|(_, load)| *load < state.bots_per_instance_capacity)
    {
        return Ok(url.clone());
    }

    let Some((fallback_url, fallback_load)) = ranked.first().cloned() else {
        return Err(ApiError::bad_gateway(
            "no bot-service instance available for assignment",
        ));
    };

    warn!(
        bot_service_base_url = %fallback_url,
        load = fallback_load,
        capacity = state.bots_per_instance_capacity,
        "all bot-service instances are at configured capacity; assigning to least-loaded instance"
    );
    Ok(fallback_url)
}

async fn run_output_consumer(state: AppState) -> anyhow::Result<()> {
    let control_group_id = format!("{}-control", state.consumer_group_id);
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &state.bootstrap_servers)
        .set("group.id", &control_group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("topic.metadata.refresh.interval.ms", "1000")
        .create()
        .context("failed to create bot-manager control Kafka consumer")?;

    let pattern = format!(
        "^{}\\..*\\.v1$",
        state.output_topic_prefix.replace('.', "\\.")
    );
    consumer
        .subscribe(&[&pattern])
        .context("failed to subscribe output topic pattern")?;

    info!(
        pattern = %pattern,
        group_id = %control_group_id,
        "bot-manager control consumer subscribed to output topics"
    );

    loop {
        let message = match consumer.recv().await {
            Ok(message) => message,
            Err(error) => {
                warn!(?error, "bot-manager kafka recv error");
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
        };
        let output_topic = message.topic().to_string();

        let payload = match message.payload() {
            Some(payload) => payload,
            None => {
                let _ = consumer.commit_message(&message, CommitMode::Async);
                continue;
            }
        };

        let step = match serde_json::from_slice::<StepEvent>(payload) {
            Ok(step) => step,
            Err(error) => {
                warn!(?error, "bot-manager failed to parse step event");
                let _ = consumer.commit_message(&message, CommitMode::Async);
                continue;
            }
        };

        if step.event_type == StepEventType::GameStarted {
            if let Err(error) = on_game_started(&state, &step.game_id).await {
                warn!(game_id = %step.game_id, error = %error.message, "bot-manager failed to reconcile GAME_STARTED");
            }
            if let Err(error) =
                ensure_game_topic_worker(&state, &step.game_id, &output_topic).await
            {
                warn!(
                    game_id = %step.game_id,
                    output_topic = %output_topic,
                    error = %error,
                    "bot-manager failed to start per-game output consumer"
                );
            }
        }

        if step.event_type == StepEventType::GameFinished {
            let destroyed = stop_bots_for_game(
                &state,
                &step.game_id,
                Some(GameStatus::Finished),
                true,
            )
            .await;
            info!(
                game_id = %step.game_id,
                destroyed_bot_count = destroyed,
                "bot-manager handled GAME_FINISHED in control consumer"
            );
        }

        if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
            warn!(?error, "bot-manager failed to commit message");
        }
    }
}

async fn ensure_game_topic_worker(
    state: &AppState,
    game_id: &str,
    output_topic: &str,
) -> anyhow::Result<()> {
    let mut workers = state.game_topic_workers.lock().await;
    if let Some(existing) = workers.get(game_id)
        && existing.output_topic == output_topic
    {
        return Ok(());
    }

    if let Some(mut old_worker) = workers.remove(game_id) {
        if let Some(stop_tx) = old_worker.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        old_worker.join.abort();
    }

    let game_id_owned = game_id.to_string();
    let output_topic_owned = output_topic.to_string();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let worker_state = state.clone();
    let worker_game_id = game_id_owned.clone();
    let worker_output_topic = output_topic_owned.clone();
    let join = tokio::spawn(async move {
        if let Err(error) = run_game_topic_worker(
            worker_state,
            worker_game_id.clone(),
            worker_output_topic.clone(),
            stop_rx,
        )
        .await
        {
            warn!(
                game_id = %worker_game_id,
                output_topic = %worker_output_topic,
                error = %error,
                "bot-manager per-game output consumer stopped with error"
            );
        }
    });

    workers.insert(
        game_id_owned.clone(),
        GameTopicWorker {
            output_topic: output_topic_owned,
            stop_tx: Some(stop_tx),
            join,
        },
    );

    info!(
        game_id = %game_id_owned,
        output_topic = %output_topic,
        "bot-manager started per-game output consumer"
    );
    Ok(())
}

async fn stop_game_topic_worker(state: &AppState, game_id: &str) {
    let maybe_worker = {
        let mut workers = state.game_topic_workers.lock().await;
        workers.remove(game_id)
    };

    if let Some(mut worker) = maybe_worker {
        if let Some(stop_tx) = worker.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        worker.join.abort();
        info!(game_id = %game_id, "bot-manager stopped per-game output consumer");
    }
}

async fn run_game_topic_worker(
    state: AppState,
    game_id: String,
    output_topic: String,
    mut stop_rx: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let worker_group_id = format!("{}-{}", state.consumer_group_id, game_id);
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &state.bootstrap_servers)
        .set("group.id", &worker_group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .create()
        .context("failed to create bot-manager per-game Kafka consumer")?;
    consumer
        .subscribe(&[&output_topic])
        .context("failed to subscribe per-game output topic")?;

    info!(
        game_id = %game_id,
        output_topic = %output_topic,
        group_id = %worker_group_id,
        "bot-manager per-game consumer subscribed"
    );

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                break;
            }
            message = consumer.recv() => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        warn!(game_id = %game_id, output_topic = %output_topic, ?error, "bot-manager per-game kafka recv error");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                };

                let payload = match message.payload() {
                    Some(payload) => payload,
                    None => {
                        let _ = consumer.commit_message(&message, CommitMode::Async);
                        continue;
                    }
                };

                let step = match serde_json::from_slice::<StepEvent>(payload) {
                    Ok(step) => step,
                    Err(error) => {
                        warn!(game_id = %game_id, output_topic = %output_topic, ?error, "bot-manager per-game failed to parse step event");
                        let _ = consumer.commit_message(&message, CommitMode::Async);
                        continue;
                    }
                };

                if step.game_id != game_id {
                    let _ = consumer.commit_message(&message, CommitMode::Async);
                    continue;
                }

                forward_step_updates_for_game(&state, &game_id, &step).await;

                if step.event_type == StepEventType::GameFinished {
                    let destroyed = stop_bots_for_game(
                        &state,
                        &game_id,
                        Some(GameStatus::Finished),
                        false,
                    )
                    .await;
                    info!(
                        game_id = %game_id,
                        output_topic = %output_topic,
                        destroyed_bot_count = destroyed,
                        "bot-manager handled GAME_FINISHED in per-game consumer"
                    );
                    let _ = consumer.commit_message(&message, CommitMode::Async);
                    break;
                }

                if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(game_id = %game_id, output_topic = %output_topic, ?error, "bot-manager per-game failed to commit message");
                }
            }
        }
    }

    {
        let mut workers = state.game_topic_workers.lock().await;
        workers.remove(&game_id);
    }
    info!(
        game_id = %game_id,
        output_topic = %output_topic,
        "bot-manager per-game output consumer stopped"
    );
    Ok(())
}

async fn forward_step_updates_for_game(state: &AppState, game_id: &str, step: &StepEvent) {
    let assignment = {
        let assignments = state.assignments.lock().await;
        assignments.get(game_id).cloned()
    };
    let Some(assignment) = assignment else {
        return;
    };

    for binding in assignment.bindings.values() {
        if let Err(error) = update_bot_from_step_event(state, binding, step).await {
            warn!(
                game_id = %assignment.game_id,
                bot_id = %binding.bot_id,
                player_id = %binding.player_id,
                step_seq = step.step_seq,
                step_event_type = ?step.event_type,
                error = %error.message,
                "failed to forward step update to bot-service"
            );
        }
    }
}

async fn on_game_started(state: &AppState, game_id: &str) -> Result<(), ApiError> {
    let maybe_assignment = {
        let assignments = state.assignments.lock().await;
        assignments.get(game_id).cloned()
    };

    if let Some(existing_assignment) = maybe_assignment {
        update_assignment_game_state(state, &existing_assignment, GameStatus::Running).await;
        return Ok(());
    }

    let assignment = assign_default_for_game(
        state,
        game_id,
        true,
        &state.default_game_guide_version,
        false,
    )
    .await?;

    info!(
        game_id = %assignment.game_id,
        bots = assignment.bindings.len(),
        humans = assignment.humans.len(),
        "bot-manager auto-assigned default bots on game start"
    );
    update_assignment_game_state(state, &assignment, GameStatus::Running).await;
    Ok(())
}

async fn stop_bots_for_game(
    state: &AppState,
    game_id: &str,
    game_status: Option<GameStatus>,
    stop_topic_worker: bool,
) -> usize {
    if stop_topic_worker {
        stop_game_topic_worker(state, game_id).await;
    }

    let assignment = {
        let mut assignments = state.assignments.lock().await;
        assignments.remove(game_id)
    };

    let Some(assignment) = assignment else {
        return 0;
    };

    let mut destroyed = 0usize;
    let resolved_game_status = game_status.unwrap_or(GameStatus::Finished);
    for binding in assignment.bindings.values() {
        if let Err(error) = update_bot_state_record(
            state,
            &assignment.game_id,
            &binding.player_id,
            "STOPPED",
            "BOT_STOPPED",
            resolved_game_status,
        )
        .await
        {
            warn!(
                game_id = %assignment.game_id,
                player_id = %binding.player_id,
                error = %error.message,
                "failed to update bot table state before delete"
            );
        }

        match delete_bot(state, &binding.bot_service_base_url, &binding.bot_id).await {
            Ok(()) => destroyed += 1,
            Err(error) => {
                warn!(bot_id = %binding.bot_id, error = %error.message, "failed to delete bot while stopping game")
            }
        }
    }

    destroyed
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        warn!(status = %self.status, message = %self.message, "bot-manager request failed");
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}
