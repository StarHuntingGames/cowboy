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
    collections::HashMap,
    net::{SocketAddr, TcpListener as StdTcpListener},
    path::Path as FsPath,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
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
    PlayerId, PlayerName, ResultStatus, StepEvent, StepEventType, expand_env_vars,
};
use rdkafka::{
    Message,
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
    producer::{FutureProducer, FutureRecord},
};
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    bots: Arc<Mutex<HashMap<String, BotRecord>>>,
    producer: FutureProducer,
    manager_base_url: String,
    bootstrap_servers: String,
    consumer_group_prefix: String,
    python_bin: String,
    agent_script_path: String,
    python_requirements_path: Option<String>,
    auto_install_python_requirements: bool,
    agent_timeout_ms: u64,
    agent_update_timeout_ms: u64,
    mock_kafka: bool,
    deepagents_enabled: bool,
    python_requirements_status: Arc<Mutex<Option<Result<(), String>>>>,
    langsmith: Option<LangSmithConfig>,
    prompt_config: Option<AgentPromptConfig>,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct LangSmithConfig {
    env_vars: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct AgentPromptConfig {
    system_prompt: String,
    user_prompt_template: String,
    custom_system_prompt: Option<String>,
    custom_user_prompt: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LangSmithConfigFile {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    extra_env: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AgentPromptConfigFile {
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default, alias = "user_prompt_template")]
    user_prompt: Option<String>,
    #[serde(default)]
    custom_system_prompt: Option<String>,
    #[serde(default)]
    custom_user_prompt: Option<String>,
}

struct BotRecord {
    config: BotConfig,
    status: BotLifecycleStatus,
    game_guide_version: Option<String>,
    worker: Option<BotWorkerHandle>,
}

struct BotWorkerHandle {
    stop_tx: Option<oneshot::Sender<()>>,
    update_tx: mpsc::UnboundedSender<StepEvent>,
    join: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct BotConfig {
    bot_id: String,
    game_id: String,
    player_name: PlayerName,
    player_id: PlayerId,
    input_topic: String,
    output_topic: String,
    llm_base_url: Option<String>,
    llm_model: Option<String>,
    llm_api_key: Option<String>,
    llm_output_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum BotLifecycleStatus {
    Created,
    Ready,
}

#[derive(Debug, Deserialize)]
struct CreateBotRequest {
    #[serde(default)]
    bot_id: Option<String>,
    game_id: String,
    player_name: PlayerName,
    player_id: PlayerId,
    input_topic: String,
    output_topic: String,
    #[serde(default)]
    llm_base_url: Option<String>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    llm_api_key: Option<String>,
    #[serde(default)]
    llm_output_mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateBotResponse {
    bot_id: String,
    status: BotLifecycleStatus,
}

#[derive(Debug, Deserialize)]
struct TeachGameRequest {
    game_guide_version: String,
    #[serde(default)]
    rules_markdown: Option<String>,
    #[serde(default)]
    command_schema: Option<serde_json::Value>,
    #[serde(default)]
    examples: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct TeachGameResponse {
    bot_id: String,
    status: BotLifecycleStatus,
    game_guide_version: String,
}

#[derive(Debug, Serialize)]
struct DeleteBotResponse {
    deleted: bool,
    bot_id: String,
}

#[derive(Debug, Serialize)]
struct BotInfoResponse {
    bot_id: String,
    game_id: String,
    player_name: PlayerName,
    player_id: PlayerId,
    status: BotLifecycleStatus,
    game_guide_version: Option<String>,
    llm_base_url: Option<String>,
    llm_model: Option<String>,
    llm_output_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BotUpdateRequest {
    step: StepEvent,
}

#[derive(Debug, Serialize)]
struct BotUpdateResponse {
    accepted: bool,
    bot_id: String,
}

#[derive(Debug, Deserialize)]
struct AgentDecisionResponse {
    command_type: CommandType,
    #[serde(default)]
    direction: Option<Direction>,
    #[serde(default)]
    speak_text: Option<String>,
    #[serde(default)]
    decision_source: Option<String>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    llm_system: Option<String>,
    #[serde(default)]
    llm_input: Option<String>,
    #[serde(default)]
    llm_output: Option<String>,
    #[serde(default)]
    llm_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum CommandSelectionSource {
    PythonAgent,
    LlmFailureSpeak,
    RustFallback,
}

impl CommandSelectionSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::PythonAgent => "python_agent",
            Self::LlmFailureSpeak => "llm_failure_speak",
            Self::RustFallback => "rust_fallback",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DecisionValidationError {
    UnsupportedCommandType,
    MissingSpeakText,
    MissingDirection,
}

impl DecisionValidationError {
    fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedCommandType => "unsupported_command_type",
            Self::MissingSpeakText => "missing_speak_text",
            Self::MissingDirection => "missing_direction",
        }
    }
}

#[derive(Debug, Serialize)]
struct PlayerAgentInitRequest {
    bot_id: String,
    game_id: String,
    player_name: PlayerName,
    player_id: PlayerId,
    llm_base_url: Option<String>,
    llm_model: Option<String>,
    llm_api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct PlayerAgentDecideRequest<'a> {
    force_speak: bool,
    game: &'a GameInstanceResponse,
}

#[derive(Debug, Serialize)]
struct PlayerAgentUpdateRequest<'a> {
    game: &'a GameInstanceResponse,
    step_event_type: StepEventType,
    step_seq: u64,
    step_turn_no: u64,
    step_round_no: u64,
    #[serde(default)]
    command: Option<&'a CommandEnvelope>,
    is_bot_turn: bool,
}

#[derive(Debug, Deserialize)]
struct PlayerAgentEnvelopeResponse {
    ok: bool,
    #[serde(default)]
    decision: Option<AgentDecisionResponse>,
    #[serde(default)]
    update: Option<AgentUpdateResponse>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentUpdateResponse {
    #[serde(default)]
    update_source: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    memory_size: Option<usize>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    llm_system: Option<String>,
    #[serde(default)]
    llm_input: Option<String>,
    #[serde(default)]
    llm_output: Option<String>,
    #[serde(default)]
    llm_error: Option<String>,
}

struct PythonPlayerAgent {
    bot_id: String,
    game_id: String,
    base_url: String,
    client: reqwest::Client,
    timeout_ms: u64,
    child: Child,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "bot_service=debug,tower_http=info".to_string()),
        )
        .init();

    let bootstrap_servers = std::env::var("KAFKA_BOOTSTRAP_SERVERS")
        .ok()
        .unwrap_or_else(|| "kafka:9092".to_string());
    let producer = ClientConfig::new()
        .set("bootstrap.servers", &bootstrap_servers)
        .set("message.timeout.ms", "5000")
        .create()
        .context("failed to create bot-service producer")?;
    let deepagents_enabled = std::env::var("BOT_AGENT_USE_DEEPAGENTS")
        .ok()
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true);
    let auto_install_python_requirements =
        parse_env_bool("BOT_AGENT_AUTO_INSTALL_REQUIREMENTS", true);
    let mut python_bin = std::env::var("BOT_AGENT_PYTHON_BIN")
        .ok()
        .unwrap_or_else(|| "python3".to_string());
    let agent_script_path = std::env::var("BOT_AGENT_SCRIPT_PATH")
        .ok()
        .unwrap_or_else(|| "/app/bot-agent/player_agent.py".to_string());
    let langsmith = load_langsmith_config();
    let prompt_config_path = resolve_prompt_config_path(&agent_script_path);
    let prompt_config = load_prompt_config(prompt_config_path.as_deref());
    let python_requirements_path =
        normalize_optional_string(std::env::var("BOT_AGENT_REQUIREMENTS_PATH").ok())
            .or_else(|| derive_requirements_path(&agent_script_path));
    if deepagents_enabled {
        if let Some(venv_python_bin) = discover_workspace_venv_python(&agent_script_path) {
            if venv_python_bin != python_bin {
                let configured_ready = probe_python_bot_agent_dependencies(&python_bin)
                    .await
                    .unwrap_or(false);
                if !configured_ready {
                    if let Ok(true) = probe_python_bot_agent_dependencies(&venv_python_bin).await {
                        warn!(
                            configured_python_bin = %python_bin,
                            fallback_python_bin = %venv_python_bin,
                            "configured python is missing bot-agent dependencies; falling back to workspace venv python"
                        );
                        python_bin = venv_python_bin;
                    }
                }
            }
        }
    }

    let state = AppState {
        bots: Arc::new(Mutex::new(HashMap::new())),
        producer,
        manager_base_url: std::env::var("GAME_MANAGER_BASE_URL")
            .ok()
            .unwrap_or_else(|| "http://game-manager-service:8081".to_string()),
        bootstrap_servers,
        consumer_group_prefix: std::env::var("BOT_SERVICE_CONSUMER_GROUP_PREFIX")
            .ok()
            .unwrap_or_else(|| "bot-service".to_string()),
        python_bin,
        agent_script_path,
        python_requirements_path,
        auto_install_python_requirements,
        agent_timeout_ms: std::env::var("BOT_AGENT_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120000),
        agent_update_timeout_ms: std::env::var("BOT_AGENT_UPDATE_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120000),
        mock_kafka: parse_env_bool("BOT_SERVICE_MOCK_KAFKA", false),
        deepagents_enabled,
        python_requirements_status: Arc::new(Mutex::new(None)),
        langsmith,
        prompt_config,
        client: reqwest::Client::new(),
    };
    if state.deepagents_enabled {
        ensure_python_requirements_ready(&state)
            .await
            .context("failed to prepare bot-service python runtime")?;
    }

    let app = build_router(state);
    let bind_addr = parse_bind_addr("BOT_SERVICE_BIND", "0.0.0.0:8091")?;
    info!(%bind_addr, "bot-service listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/internal/v3/bots", post(create_bot_handler))
        .route(
            "/internal/v3/bots/{bot_id}",
            get(get_bot_handler).delete(delete_bot_handler),
        )
        .route(
            "/internal/v3/bots/{bot_id}/teach-game",
            post(teach_game_handler),
        )
        .route(
            "/internal/v3/bots/{bot_id}/update",
            post(update_bot_handler),
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

fn load_langsmith_config() -> Option<LangSmithConfig> {
    let Some(path) = std::env::var("BOT_AGENT_LANGSMITH_CONFIG_PATH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return None;
    };

    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to read bot-service langsmith config path");
            return None;
        }
    };

    let expanded = expand_env_vars(&raw);
    let parsed = match serde_yaml::from_str::<LangSmithConfigFile>(&expanded) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to parse bot-service langsmith config yaml");
            return None;
        }
    };

    let enabled = parsed.enabled.unwrap_or(true);
    let mut env_vars = vec![
        ("LANGSMITH_TRACING".to_string(), enabled.to_string()),
        ("LANGCHAIN_TRACING_V2".to_string(), enabled.to_string()),
    ];

    if let Some(api_key) = normalize_optional_string(parsed.api_key) {
        env_vars.push(("LANGSMITH_API_KEY".to_string(), api_key));
    }
    if let Some(endpoint) = normalize_optional_string(parsed.endpoint) {
        env_vars.push(("LANGSMITH_ENDPOINT".to_string(), endpoint));
    }
    if let Some(project) = normalize_optional_string(parsed.project) {
        env_vars.push(("LANGSMITH_PROJECT".to_string(), project));
    }
    if let Some(workspace_id) = normalize_optional_string(parsed.workspace_id) {
        env_vars.push(("LANGSMITH_WORKSPACE_ID".to_string(), workspace_id));
    }

    for (key, value) in parsed.extra_env {
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if !key.is_empty() && !value.is_empty() {
            env_vars.push((key, value));
        }
    }

    info!(
        path = %path,
        enabled,
        env_var_count = env_vars.len(),
        "loaded bot-service langsmith config"
    );
    Some(LangSmithConfig { env_vars })
}

fn resolve_prompt_config_path(agent_script_path: &str) -> Option<String> {
    if let Some(explicit_path) =
        normalize_optional_string(std::env::var("BOT_AGENT_PROMPTS_CONFIG_PATH").ok())
    {
        return Some(explicit_path);
    }

    if let Some(path) = discover_workspace_file(agent_script_path, "bot-service-prompts.yaml") {
        return Some(path);
    }

    let container_default = "/app/config/bot-service-prompts.yaml";
    if FsPath::new(container_default).is_file() {
        return Some(container_default.to_string());
    }

    None
}

fn discover_workspace_file(agent_script_path: &str, file_name: &str) -> Option<String> {
    let script_path = FsPath::new(agent_script_path);
    for ancestor in script_path.ancestors() {
        let candidate = ancestor.join(file_name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn load_prompt_config(path: Option<&str>) -> Option<AgentPromptConfig> {
    let Some(path) = path.map(str::trim).filter(|value| !value.is_empty()) else {
        return None;
    };

    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to read bot-service prompt config path");
            return None;
        }
    };

    let parsed = match serde_yaml::from_str::<AgentPromptConfigFile>(&raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn!(path = %path, error = %error, "failed to parse bot-service prompt config yaml");
            return None;
        }
    };

    let Some(system_prompt) = normalize_optional_string(parsed.system_prompt) else {
        warn!(path = %path, "bot-service prompt config missing system_prompt; ignoring file");
        return None;
    };
    let Some(user_prompt_template) = normalize_optional_string(parsed.user_prompt) else {
        warn!(path = %path, "bot-service prompt config missing user_prompt; ignoring file");
        return None;
    };

    let custom_system_prompt = normalize_optional_string(parsed.custom_system_prompt);
    let custom_user_prompt = normalize_optional_string(parsed.custom_user_prompt);

    info!(
        path = %path,
        system_prompt_len = system_prompt.chars().count(),
        user_prompt_len = user_prompt_template.chars().count(),
        custom_system_prompt_len = custom_system_prompt.as_deref().map(|s| s.chars().count()).unwrap_or(0),
        custom_user_prompt_len = custom_user_prompt.as_deref().map(|s| s.chars().count()).unwrap_or(0),
        "loaded bot-service prompt config"
    );
    Some(AgentPromptConfig {
        system_prompt,
        user_prompt_template,
        custom_system_prompt,
        custom_user_prompt,
    })
}

fn allocate_local_agent_port() -> anyhow::Result<u16> {
    let listener = StdTcpListener::bind("127.0.0.1:0")
        .context("failed to allocate local port for python player-agent")?;
    let port = listener
        .local_addr()
        .context("failed to read allocated local player-agent port")?
        .port();
    Ok(port)
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
}

fn parse_env_bool(var_name: &str, default: bool) -> bool {
    std::env::var(var_name)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                default
            } else {
                !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
            }
        })
        .unwrap_or(default)
}

fn derive_requirements_path(agent_script_path: &str) -> Option<String> {
    let script_path = FsPath::new(agent_script_path);
    script_path.parent().map(|parent| {
        parent
            .join("requirements.txt")
            .to_string_lossy()
            .to_string()
    })
}

fn discover_workspace_venv_python(agent_script_path: &str) -> Option<String> {
    let script_path = FsPath::new(agent_script_path);
    for ancestor in script_path.ancestors() {
        let candidate = ancestor.join(".venv").join("bin").join("python");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

async fn probe_python_bot_agent_dependencies(python_bin: &str) -> anyhow::Result<bool> {
    let output = Command::new(python_bin)
        .arg("-c")
        .arg("import deepagents,fastapi,uvicorn,langchain,langchain_openai,langchain_anthropic")
        .output()
        .await
        .context("failed to probe python bot-agent dependencies")?;
    if output.status.success() {
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    warn!(
        python_bin = %python_bin,
        probe_stdout = %truncate_chars_with_ellipsis(stdout.trim(), 500),
        probe_stderr = %truncate_chars_with_ellipsis(stderr.trim(), 500),
        "python bot-agent dependency probe failed"
    );
    Ok(false)
}

async fn run_pip_install(
    python_bin: &str,
    requirements_path: &str,
    break_system_packages: bool,
) -> anyhow::Result<()> {
    let mut command = Command::new(python_bin);
    command
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("--disable-pip-version-check")
        .arg("--no-input");
    if break_system_packages {
        command.arg("--break-system-packages");
    }
    command.arg("-r").arg(requirements_path);

    let output = command
        .output()
        .await
        .context("failed to execute pip install for bot-agent requirements")?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::bail!(
        "pip install failed status={} break_system_packages={} stdout='{}' stderr='{}'",
        output.status,
        break_system_packages,
        truncate_chars_with_ellipsis(stdout.trim(), 700),
        truncate_chars_with_ellipsis(stderr.trim(), 700),
    )
}

async fn ensure_python_requirements_ready_uncached(
    python_bin: &str,
    requirements_path: &str,
) -> anyhow::Result<()> {
    if !FsPath::new(requirements_path).is_file() {
        anyhow::bail!(
            "bot-agent requirements file not found: {} (set BOT_AGENT_REQUIREMENTS_PATH to override)",
            requirements_path
        );
    }

    if probe_python_bot_agent_dependencies(python_bin).await? {
        info!(
            python_bin = %python_bin,
            requirements_path = %requirements_path,
            "python bot-agent dependencies already installed"
        );
        return Ok(());
    }

    info!(
        python_bin = %python_bin,
        requirements_path = %requirements_path,
        "installing python bot-agent requirements"
    );

    if let Err(primary_error) = run_pip_install(python_bin, requirements_path, false).await {
        warn!(
            python_bin = %python_bin,
            requirements_path = %requirements_path,
            error = %primary_error,
            "standard pip install failed; retrying with --break-system-packages"
        );
        run_pip_install(python_bin, requirements_path, true)
            .await
            .with_context(|| {
                format!(
                    "failed to install bot-agent requirements after retry; primary error: {:#}",
                    primary_error
                )
            })?;
    }

    if !probe_python_bot_agent_dependencies(python_bin).await? {
        anyhow::bail!("bot-agent dependency probe still failing after pip install");
    }

    info!(
        python_bin = %python_bin,
        requirements_path = %requirements_path,
        "python bot-agent requirements are installed"
    );
    Ok(())
}

async fn ensure_python_requirements_ready(state: &AppState) -> anyhow::Result<()> {
    if !state.auto_install_python_requirements {
        info!(
            "BOT_AGENT_AUTO_INSTALL_REQUIREMENTS is disabled; skipping python dependency preflight"
        );
        return Ok(());
    }

    let requirements_path = state.python_requirements_path.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "unable to resolve bot-agent requirements path from BOT_AGENT_SCRIPT_PATH='{}'",
            state.agent_script_path
        )
    })?;

    let mut status_guard = state.python_requirements_status.lock().await;
    if let Some(status) = status_guard.as_ref() {
        return status.clone().map_err(anyhow::Error::msg);
    }

    let resolved = ensure_python_requirements_ready_uncached(&state.python_bin, requirements_path)
        .await
        .map_err(|error| format!("{:#}", error));
    *status_guard = Some(resolved.clone());

    resolved.map_err(anyhow::Error::msg)
}

fn truncate_log_field(value: Option<&str>, max_chars: usize) -> String {
    let Some(text) = value.map(str::trim).filter(|entry| !entry.is_empty()) else {
        return String::new();
    };

    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...[truncated]");
            return out;
        }
        out.push(ch);
    }
    out
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true, "service": "bot-service"}))
}

async fn create_bot_handler(
    State(state): State<AppState>,
    Json(request): Json<CreateBotRequest>,
) -> Result<Json<CreateBotResponse>, ApiError> {
    if request.game_id.trim().is_empty()
        || request.player_id.trim().is_empty()
        || request.input_topic.trim().is_empty()
        || request.output_topic.trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "game_id, player_id, input_topic, and output_topic are required",
        ));
    }

    let bot_id = request
        .bot_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("bot-{}", Uuid::new_v4()));

    let mut bots = state.bots.lock().await;
    if bots.contains_key(&bot_id) {
        return Err(ApiError::conflict(format!("bot {} already exists", bot_id)));
    }

    let config = BotConfig {
        bot_id: bot_id.clone(),
        game_id: request.game_id,
        player_name: request.player_name,
        player_id: request.player_id,
        input_topic: request.input_topic,
        output_topic: request.output_topic,
        llm_base_url: normalize_optional_string(request.llm_base_url),
        llm_model: normalize_optional_string(request.llm_model),
        llm_api_key: normalize_optional_string(request.llm_api_key),
        llm_output_mode: normalize_optional_string(request.llm_output_mode),
    };

    bots.insert(
        bot_id.clone(),
        BotRecord {
            config,
            status: BotLifecycleStatus::Created,
            game_guide_version: None,
            worker: None,
        },
    );

    Ok(Json(CreateBotResponse {
        bot_id,
        status: BotLifecycleStatus::Created,
    }))
}

async fn get_bot_handler(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotInfoResponse>, ApiError> {
    let bots = state.bots.lock().await;
    let record = bots
        .get(&bot_id)
        .ok_or_else(|| ApiError::not_found(format!("bot {} not found", bot_id)))?;

    Ok(Json(BotInfoResponse {
        bot_id: bot_id.clone(),
        game_id: record.config.game_id.clone(),
        player_name: record.config.player_name,
        player_id: record.config.player_id.clone(),
        status: record.status,
        game_guide_version: record.game_guide_version.clone(),
        llm_base_url: record.config.llm_base_url.clone(),
        llm_model: record.config.llm_model.clone(),
        llm_output_mode: record.config.llm_output_mode.clone(),
    }))
}

async fn teach_game_handler(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    Json(request): Json<TeachGameRequest>,
) -> Result<Json<TeachGameResponse>, ApiError> {
    if request.game_guide_version.trim().is_empty() {
        return Err(ApiError::bad_request("game_guide_version is required"));
    }

    let (config, previous_worker) = {
        let mut bots = state.bots.lock().await;
        let record = bots
            .get_mut(&bot_id)
            .ok_or_else(|| ApiError::not_found(format!("bot {} not found", bot_id)))?;

        // Hold onto optional payload for future prompt templates.
        let _ = request.rules_markdown.as_deref();
        let _ = request.command_schema.as_ref();
        let _ = request.examples.as_ref();

        let previous_worker = record.worker.take();
        record.game_guide_version = Some(request.game_guide_version.clone());
        record.status = BotLifecycleStatus::Ready;
        (record.config.clone(), previous_worker)
    };

    if let Some(mut worker) = previous_worker {
        if let Some(stop_tx) = worker.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        worker.join.abort();
    }

    let worker = spawn_bot_worker(state.clone(), config.clone());

    {
        let mut bots = state.bots.lock().await;
        if let Some(record) = bots.get_mut(&bot_id) {
            record.worker = Some(worker);
            record.status = BotLifecycleStatus::Ready;
        }
    }

    Ok(Json(TeachGameResponse {
        bot_id,
        status: BotLifecycleStatus::Ready,
        game_guide_version: request.game_guide_version,
    }))
}

async fn update_bot_handler(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    Json(request): Json<BotUpdateRequest>,
) -> Result<Json<BotUpdateResponse>, ApiError> {
    let update_tx = {
        let bots = state.bots.lock().await;
        let record = bots
            .get(&bot_id)
            .ok_or_else(|| ApiError::not_found(format!("bot {} not found", bot_id)))?;
        let worker = record.worker.as_ref().ok_or_else(|| {
            ApiError::conflict(format!(
                "bot {} is not active; teach-game not started",
                bot_id
            ))
        })?;
        worker.update_tx.clone()
    };

    update_tx.send(request.step).map_err(|_| {
        ApiError::conflict(format!(
            "bot {} worker is unavailable; update channel closed",
            bot_id
        ))
    })?;

    Ok(Json(BotUpdateResponse {
        accepted: true,
        bot_id,
    }))
}

async fn delete_bot_handler(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
) -> Result<Json<DeleteBotResponse>, ApiError> {
    let maybe_worker = {
        let mut bots = state.bots.lock().await;
        bots.remove(&bot_id).and_then(|record| record.worker)
    };

    if let Some(mut worker) = maybe_worker {
        if let Some(stop_tx) = worker.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        worker.join.abort();
    }

    Ok(Json(DeleteBotResponse {
        deleted: true,
        bot_id,
    }))
}

impl PythonPlayerAgent {
    async fn start(state: &AppState, config: &BotConfig) -> anyhow::Result<Self> {
        let host = "127.0.0.1";
        let port = allocate_local_agent_port()?;
        let base_url = format!("http://{}:{}", host, port);

        let mut command = Command::new(&state.python_bin);
        command
            .arg(&state.agent_script_path)
            .arg("--host")
            .arg(host)
            .arg("--port")
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if let Some(langsmith) = &state.langsmith {
            for (key, value) in &langsmith.env_vars {
                command.env(key, value);
            }
        }
        if let Some(prompt_config) = &state.prompt_config {
            command.env("BOT_AGENT_SYSTEM_PROMPT", &prompt_config.system_prompt);
            command.env(
                "BOT_AGENT_USER_PROMPT_TEMPLATE",
                &prompt_config.user_prompt_template,
            );
            if let Some(custom_system) = &prompt_config.custom_system_prompt {
                command.env("BOT_AGENT_CUSTOM_SYSTEM_PROMPT", custom_system);
            }
            if let Some(custom_user) = &prompt_config.custom_user_prompt {
                command.env("BOT_AGENT_CUSTOM_USER_PROMPT", custom_user);
            }
        }
        let output_mode = config
            .llm_output_mode
            .as_deref()
            .unwrap_or("command_text");
        command.env("BOT_AGENT_OUTPUT_MODE", output_mode);
        command.env(
            "BOT_AGENT_UPDATE_TIMEOUT_MS",
            state.agent_update_timeout_ms.to_string(),
        );

        let child = command
            .spawn()
            .context("failed to spawn python player agent process")?;

        let mut agent = Self {
            bot_id: config.bot_id.clone(),
            game_id: config.game_id.clone(),
            base_url,
            client: state.client.clone(),
            timeout_ms: state.agent_timeout_ms,
            child,
        };
        agent.wait_until_ready().await?;

        let init = PlayerAgentInitRequest {
            bot_id: config.bot_id.clone(),
            game_id: config.game_id.clone(),
            player_name: config.player_name,
            player_id: config.player_id.clone(),
            llm_base_url: config.llm_base_url.clone(),
            llm_model: config.llm_model.clone(),
            llm_api_key: config.llm_api_key.clone(),
        };
        let response = agent
            .post_json("/init", &init)
            .await
            .context("player-agent init request failed")?;
        if !response.ok {
            let detail = response
                .error
                .unwrap_or_else(|| "unknown init error".to_string());
            anyhow::bail!("python player agent init rejected: {}", detail);
        }

        Ok(agent)
    }

    async fn decide(
        &mut self,
        game: &GameInstanceResponse,
        force_speak: bool,
    ) -> anyhow::Result<AgentDecisionResponse> {
        let request = PlayerAgentDecideRequest { force_speak, game };
        let response = self
            .post_json("/decide", &request)
            .await
            .context("player-agent decide request failed")?;
        if !response.ok {
            let detail = response
                .error
                .unwrap_or_else(|| "unknown decide error".to_string());
            anyhow::bail!("python player agent decide rejected: {}", detail);
        }

        response
            .decision
            .ok_or_else(|| anyhow::anyhow!("python player agent response missing decision"))
    }

    async fn update(
        &mut self,
        game: &GameInstanceResponse,
        step: &StepEvent,
        is_bot_turn: bool,
    ) -> anyhow::Result<AgentUpdateResponse> {
        let request = PlayerAgentUpdateRequest {
            game,
            step_event_type: step.event_type.clone(),
            step_seq: step.step_seq,
            step_turn_no: step.turn_no,
            step_round_no: step.round_no,
            command: step.command.as_ref(),
            is_bot_turn,
        };
        let response = self
            .post_json("/update", &request)
            .await
            .context("player-agent update request failed")?;
        if !response.ok {
            let detail = response
                .error
                .unwrap_or_else(|| "unknown update error".to_string());
            anyhow::bail!("python player agent update rejected: {}", detail);
        }

        response
            .update
            .ok_or_else(|| anyhow::anyhow!("python player agent response missing update"))
    }

    async fn shutdown(&mut self) {
        let _ = self.post_json("/shutdown", &serde_json::json!({})).await;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    async fn wait_until_ready(&mut self) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(self.timeout_ms.saturating_mul(2).max(1200));
        let health_url = format!("{}/health", self.base_url);

        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .context("failed to poll player-agent process status")?
            {
                anyhow::bail!(
                    "player-agent exited before ready for bot {} game {} with status {}",
                    self.bot_id,
                    self.game_id,
                    status
                );
            }

            if let Ok(response) = self
                .client
                .get(&health_url)
                .timeout(Duration::from_millis(350))
                .send()
                .await
            {
                if response.status().is_success() {
                    return Ok(());
                }
            }

            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "player-agent health check timed out for bot {} game {}",
                    self.bot_id,
                    self.game_id
                );
            }

            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }

    async fn post_json<T: Serialize>(
        &self,
        path: &str,
        payload: &T,
    ) -> anyhow::Result<PlayerAgentEnvelopeResponse> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let response = self
            .client
            .post(&url)
            .timeout(Duration::from_millis(self.timeout_ms))
            .json(payload)
            .send()
            .await
            .with_context(|| format!("failed to call player-agent endpoint {}", path))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read player-agent response body")?;
        if !status.is_success() {
            anyhow::bail!(
                "player-agent endpoint {} returned {}: {}",
                path,
                status,
                body
            );
        }

        serde_json::from_str::<PlayerAgentEnvelopeResponse>(&body)
            .context("failed to decode player-agent response")
    }
}

fn spawn_bot_worker(state: AppState, config: BotConfig) -> BotWorkerHandle {
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let (update_tx, update_rx) = mpsc::unbounded_channel::<StepEvent>();
    let join = tokio::spawn(async move {
        if let Err(error) = run_bot_worker(state, config.clone(), stop_rx, update_rx).await {
            warn!(
                bot_id = %config.bot_id,
                game_id = %config.game_id,
                error = %error,
                "bot worker stopped with error"
            );
        }
    });

    BotWorkerHandle {
        stop_tx: Some(stop_tx),
        update_tx,
        join,
    }
}

async fn run_bot_worker(
    state: AppState,
    config: BotConfig,
    mut stop_rx: oneshot::Receiver<()>,
    mut update_rx: mpsc::UnboundedReceiver<StepEvent>,
) -> anyhow::Result<()> {
    let consumer: Option<StreamConsumer> = if state.mock_kafka {
        None
    } else {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", &state.bootstrap_servers)
            .set(
                "group.id",
                format!("{}-{}", state.consumer_group_prefix, config.bot_id),
            )
            .set("enable.auto.commit", "false")
            .set("auto.offset.reset", "latest")
            .create()
            .context("failed to create bot worker Kafka consumer")?;

        consumer
            .subscribe(&[&config.output_topic])
            .context("failed to subscribe bot worker output topic")?;
        Some(consumer)
    };

    info!(
        bot_id = %config.bot_id,
        game_id = %config.game_id,
        output_topic = %config.output_topic,
        "bot worker started"
    );

    let mut last_acted_turn_no: u64 = 0;
    let mut has_spoken_once = false;
    let mut retry_count: u32 = 0;
    const MAX_RETRIES_PER_TURN: u32 = 2;
    let mut python_agent = if state.deepagents_enabled {
        match PythonPlayerAgent::start(&state, &config).await {
            Ok(agent) => Some(agent),
            Err(error) => {
                let error_detail = format!("{:#}", error);
                warn!(
                    bot_id = %config.bot_id,
                    game_id = %config.game_id,
                    error = %error_detail,
                    "failed to initialize python player-agent; fallback policy will be used"
                );
                None
            }
        }
    } else {
        None
    };

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                info!(bot_id = %config.bot_id, "bot worker received stop signal");
                break;
            }
            maybe_step = update_rx.recv() => {
                let Some(step) = maybe_step else {
                    continue;
                };

                if step.game_id != config.game_id {
                    continue;
                }
                if !matches!(step.event_type, StepEventType::GameStarted | StepEventType::StepApplied | StepEventType::TimeoutApplied) {
                    continue;
                }

                let game = match fetch_game(&state, &config.game_id).await {
                    Ok(game) => game,
                    Err(error) => {
                        warn!(
                            bot_id = %config.bot_id,
                            game_id = %config.game_id,
                            error = %error,
                            step_seq = step.step_seq,
                            step_event_type = ?step.event_type,
                            "bot worker failed to fetch game snapshot for external update"
                        );
                        continue;
                    }
                };

                if let Err(error) =
                    process_python_update_for_step(&state, &config, &game, &step, &mut python_agent)
                        .await
                {
                    warn!(
                        bot_id = %config.bot_id,
                        game_id = %config.game_id,
                        error = %error,
                        step_seq = step.step_seq,
                        step_event_type = ?step.event_type,
                        "bot worker failed processing external update step"
                    );
                }
            }
            message = async {
                if let Some(consumer) = &consumer {
                    consumer.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        warn!(bot_id = %config.bot_id, ?error, "bot worker kafka recv error");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                };

                let payload = match message.payload() {
                    Some(payload) => payload,
                    None => {
                        if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                        continue;
                    }
                };

                let step = match serde_json::from_slice::<StepEvent>(payload) {
                    Ok(step) => step,
                    Err(error) => {
                        warn!(bot_id = %config.bot_id, ?error, "bot worker invalid step payload");
                        if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                        continue;
                    }
                };

                if step.game_id != config.game_id {
                    if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                    continue;
                }

                if step.event_type == StepEventType::GameFinished {
                    info!(bot_id = %config.bot_id, game_id = %config.game_id, "game finished event observed by bot worker");
                    if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                    break;
                }

                if !matches!(step.event_type, StepEventType::GameStarted | StepEventType::StepApplied | StepEventType::TimeoutApplied) {
                    if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                    continue;
                }

                let game = match fetch_game(&state, &config.game_id).await {
                    Ok(game) => game,
                    Err(error) => {
                        warn!(bot_id = %config.bot_id, game_id = %config.game_id, error = %error, "bot worker failed to fetch game snapshot");
                        if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                        continue;
                    }
                };

                if game.status != GameStatus::Running {
                    if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                    continue;
                }

                let is_bot_turn = game.current_player_id == config.player_id;

                // If the step event shows our own command was rejected (InvalidCommand)
                // and the turn has NOT advanced, reset last_acted_turn_no so we retry
                // (up to MAX_RETRIES_PER_TURN times with fallback policy).
                let mut force_fallback_retry = false;
                if is_bot_turn
                    && game.turn_no == last_acted_turn_no
                    && step.result_status == ResultStatus::InvalidCommand
                {
                    if let Some(ref cmd) = step.command {
                        if cmd.player_id.as_deref() == Some(config.player_id.as_str()) {
                            if retry_count < MAX_RETRIES_PER_TURN {
                                retry_count += 1;
                                warn!(
                                    bot_id = %config.bot_id,
                                    game_id = %config.game_id,
                                    player_id = %config.player_id,
                                    turn_no = game.turn_no,
                                    retry_count = retry_count,
                                    max_retries = MAX_RETRIES_PER_TURN,
                                    rejected_command_type = ?cmd.command_type,
                                    rejected_direction = ?cmd.direction,
                                    "bot command rejected; retrying with fallback policy"
                                );
                                last_acted_turn_no = game.turn_no.saturating_sub(1);
                                force_fallback_retry = true;
                            } else {
                                warn!(
                                    bot_id = %config.bot_id,
                                    game_id = %config.game_id,
                                    player_id = %config.player_id,
                                    turn_no = game.turn_no,
                                    retry_count = retry_count,
                                    "bot command rejected; max retries reached, waiting for timeout"
                                );
                            }
                        }
                    }
                }

                // Reset retry counter when the turn advances.
                if game.turn_no > last_acted_turn_no && !force_fallback_retry {
                    retry_count = 0;
                }

                let should_decide = is_bot_turn && game.turn_no > last_acted_turn_no;

                if !should_decide {
                    if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
                    continue;
                }

                if python_agent.is_none() && state.deepagents_enabled {
                    python_agent = match PythonPlayerAgent::start(&state, &config).await {
                        Ok(agent) => Some(agent),
                        Err(error) => {
                            let error_detail = format!("{:#}", error);
                            warn!(
                                bot_id = %config.bot_id,
                                game_id = %config.game_id,
                                error = %error_detail,
                                "python player-agent restart failed; using fallback policy"
                            );
                            None
                        }
                    };
                }

                let force_speak = !has_spoken_once;
                let mut drop_python_agent = false;
                let mut llm_failure_message: Option<String> = None;
                let decision = if force_fallback_retry {
                    // On retry after rejection, skip LLM and use Rust fallback policy
                    // to avoid repeating the same invalid action.
                    info!(
                        bot_id = %config.bot_id,
                        game_id = %config.game_id,
                        turn_no = game.turn_no,
                        "using fallback policy for retry after rejected command"
                    );
                    None
                } else if let Some(agent) = python_agent.as_mut() {
                    match agent.decide(&game, force_speak).await {
                        Ok(decision) => Some(decision),
                        Err(error) => {
                            let error_detail = format!("{:#}", error);
                            let mut agent_exited = false;
                            match agent.child.try_wait() {
                                Ok(Some(status)) => {
                                    agent_exited = true;
                                    warn!(
                                        bot_id = %config.bot_id,
                                        game_id = %config.game_id,
                                        status = %status,
                                        "python player-agent process exited after decide failure"
                                    );
                                }
                                Ok(None) => {}
                                Err(wait_error) => {
                                    warn!(
                                        bot_id = %config.bot_id,
                                        game_id = %config.game_id,
                                        error = %wait_error,
                                        "failed to poll python player-agent process after decide failure"
                                    );
                                }
                            }
                            drop_python_agent = agent_exited;
                            warn!(
                                bot_id = %config.bot_id,
                                game_id = %config.game_id,
                                error = %error_detail,
                                agent_exited = agent_exited,
                                "python player-agent decide failed; using fallback policy for this turn"
                            );
                            llm_failure_message = Some(error_detail);
                            None
                        }
                    }
                } else {
                    None
                };

                if drop_python_agent {
                    if let Some(mut broken_agent) = python_agent.take() {
                        broken_agent.shutdown().await;
                    }
                }

                if let Some(agent_decision) = decision.as_ref() {
                    let llm_system_log =
                        truncate_log_field(agent_decision.llm_system.as_deref(), 1200);
                    let llm_input_log =
                        truncate_log_field(agent_decision.llm_input.as_deref(), 2400);
                    let llm_output_log =
                        truncate_log_field(agent_decision.llm_output.as_deref(), 2400);
                    info!(
                        bot_id = %config.bot_id,
                        game_id = %config.game_id,
                        player_id = %config.player_id,
                        turn_no = game.turn_no,
                        agent_decision_source = agent_decision
                            .decision_source
                            .as_deref()
                            .unwrap_or("unspecified"),
                        agent_command_type = ?agent_decision.command_type,
                        agent_llm_model = agent_decision.llm_model.as_deref().unwrap_or(""),
                        agent_llm_error = agent_decision.llm_error.as_deref().unwrap_or(""),
                        agent_llm_system = %llm_system_log,
                        agent_llm_input = %llm_input_log,
                        agent_llm_output = %llm_output_log,
                        "python player-agent decision received"
                    );
                } else {
                    info!(
                        bot_id = %config.bot_id,
                        game_id = %config.game_id,
                        player_id = %config.player_id,
                        turn_no = game.turn_no,
                        "python player-agent decision unavailable; using rust fallback policy"
                    );
                }

                let (command, selection_source) =
                    build_bot_command(
                        &config,
                        &game,
                        decision.as_ref(),
                        llm_failure_message.as_deref(),
                    );
                if let Err(error) = publish_command(&state, &config, &command).await {
                    warn!(bot_id = %config.bot_id, game_id = %config.game_id, error = %error, "bot worker failed to publish command");
                } else {
                    info!(
                        bot_id = %config.bot_id,
                        game_id = %config.game_id,
                        player_id = %config.player_id,
                        turn_no = game.turn_no,
                        selection_source = selection_source.as_str(),
                        command_type = ?command.command_type,
                        "bot command published"
                    );
                    last_acted_turn_no = game.turn_no;
                    if command.command_type == CommandType::Speak {
                        has_spoken_once = true;
                    }
                }

                if let Some(consumer) = &consumer { let _ = consumer.commit_message(&message, CommitMode::Async); }
            }
        }
    }

    if let Some(mut agent) = python_agent {
        agent.shutdown().await;
    }

    info!(bot_id = %config.bot_id, game_id = %config.game_id, "bot worker stopped");
    Ok(())
}

async fn process_python_update_for_step(
    state: &AppState,
    config: &BotConfig,
    game: &GameInstanceResponse,
    step: &StepEvent,
    python_agent: &mut Option<PythonPlayerAgent>,
) -> anyhow::Result<()> {
    if !state.deepagents_enabled {
        return Ok(());
    }

    if python_agent.is_none() {
        *python_agent = match PythonPlayerAgent::start(state, config).await {
            Ok(agent) => Some(agent),
            Err(error) => {
                let error_detail = format!("{:#}", error);
                warn!(
                    bot_id = %config.bot_id,
                    game_id = %config.game_id,
                    error = %error_detail,
                    "python player-agent restart failed before update call"
                );
                None
            }
        };
    }

    let mut drop_python_agent = false;
    if let Some(agent) = python_agent.as_mut() {
        let is_bot_turn = game.current_player_id == config.player_id;
        let update_start = std::time::Instant::now();
        match agent.update(game, step, is_bot_turn).await {
            Ok(update) => {
                let llm_system_log = truncate_log_field(update.llm_system.as_deref(), 1200);
                let llm_input_log = truncate_log_field(update.llm_input.as_deref(), 2400);
                let llm_output_log = truncate_log_field(update.llm_output.as_deref(), 2400);
                let summary_log = truncate_log_field(update.summary.as_deref(), 600);
                let elapsed_ms = update_start.elapsed().as_millis();
                info!(
                    bot_id = %config.bot_id,
                    game_id = %config.game_id,
                    player_id = %config.player_id,
                    turn_no = game.turn_no,
                    step_seq = step.step_seq,
                    step_event_type = ?step.event_type,
                    update_source = update
                        .update_source
                        .as_deref()
                        .unwrap_or("unspecified"),
                    update_memory_size = update.memory_size.unwrap_or_default(),
                    agent_llm_model = update.llm_model.as_deref().unwrap_or(""),
                    agent_llm_error = update.llm_error.as_deref().unwrap_or(""),
                    agent_llm_system = %llm_system_log,
                    agent_llm_input = %llm_input_log,
                    agent_llm_output = %llm_output_log,
                    update_summary = %summary_log,
                    update_elapsed_ms = elapsed_ms,
                    "python player-agent update processed"
                );
            }
            Err(error) => {
                let error_detail = format!("{:#}", error);
                let mut agent_exited = false;
                match agent.child.try_wait() {
                    Ok(Some(status)) => {
                        agent_exited = true;
                        warn!(
                            bot_id = %config.bot_id,
                            game_id = %config.game_id,
                            status = %status,
                            "python player-agent process exited after update failure"
                        );
                    }
                    Ok(None) => {}
                    Err(wait_error) => {
                        warn!(
                            bot_id = %config.bot_id,
                            game_id = %config.game_id,
                            error = %wait_error,
                            "failed to poll python player-agent process after update failure"
                        );
                    }
                }
                drop_python_agent = agent_exited;
                let elapsed_ms = update_start.elapsed().as_millis();
                warn!(
                    bot_id = %config.bot_id,
                    game_id = %config.game_id,
                    error = %error_detail,
                    agent_exited = agent_exited,
                    update_timeout_ms = state.agent_update_timeout_ms,
                    update_elapsed_ms = elapsed_ms,
                    "python player-agent update failed"
                );
            }
        }
    }

    if drop_python_agent {
        if let Some(mut broken_agent) = python_agent.take() {
            broken_agent.shutdown().await;
        }
    }

    Ok(())
}

fn build_bot_command(
    config: &BotConfig,
    game: &GameInstanceResponse,
    decision: Option<&AgentDecisionResponse>,
    llm_failure_message: Option<&str>,
) -> (CommandEnvelope, CommandSelectionSource) {
    if let Some(message) = llm_failure_message
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        return (
            build_llm_failure_speak_command(config, game, message),
            CommandSelectionSource::LlmFailureSpeak,
        );
    }

    if let Some(decision) = decision {
        if let Some(message) = llm_failure_message_from_decision(decision) {
            return (
                build_llm_failure_speak_command(config, game, message),
                CommandSelectionSource::LlmFailureSpeak,
            );
        }

        match command_from_decision(config, game, decision) {
            Ok(command) => return (command, CommandSelectionSource::PythonAgent),
            Err(reason) => {
                let mut failure_reason = format!("invalid decision: {}", reason.as_str());
                if let Some(source) = decision
                    .decision_source
                    .as_deref()
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                {
                    failure_reason.push_str("; source=");
                    failure_reason.push_str(source);
                }
                if let Some(error) = decision
                    .llm_error
                    .as_deref()
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                {
                    failure_reason.push_str("; llm_error=");
                    failure_reason.push_str(error);
                }
                warn!(
                    bot_id = %config.bot_id,
                    game_id = %config.game_id,
                    rejection_reason = reason.as_str(),
                    command_type = ?decision.command_type,
                    direction = ?decision.direction,
                    speak_text_len = decision.speak_text.as_deref().map(str::len).unwrap_or(0),
                    decision_source = decision.decision_source.as_deref().unwrap_or("unspecified"),
                    llm_model = decision.llm_model.as_deref().unwrap_or(""),
                    llm_error = decision.llm_error.as_deref().unwrap_or(""),
                    fallback_message = %failure_reason,
                    "player-agent decision was invalid; using fallback policy"
                );
                return (
                    build_fallback_bot_command(config, game, &failure_reason),
                    CommandSelectionSource::RustFallback,
                );
            }
        }
    }
    (
        build_fallback_bot_command(config, game, "fallback policy used"),
        CommandSelectionSource::RustFallback,
    )
}

fn llm_failure_message_from_decision(decision: &AgentDecisionResponse) -> Option<&str> {
    let source = decision
        .decision_source
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if source != "python_fallback" {
        return None;
    }

    decision
        .llm_error
        .as_deref()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
}

fn truncate_chars_with_ellipsis(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars - 3 {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn build_llm_failure_speak_text(message: &str) -> String {
    let normalized = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let cleaned = if normalized.is_empty() {
        "unknown error".to_string()
    } else {
        normalized
    };

    let prefix = "bot fail:";
    let max_total = 140usize;
    let max_message = max_total.saturating_sub(prefix.chars().count());
    let clipped = truncate_chars_with_ellipsis(&cleaned, max_message);
    format!("{prefix}{clipped}")
}

fn build_llm_failure_speak_command(
    config: &BotConfig,
    game: &GameInstanceResponse,
    message: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        command_id: format!(
            "bot-{}-{}-{}",
            config.bot_id,
            game.turn_no,
            Utc::now().timestamp_millis()
        ),
        source: CommandSource::Bot,
        game_id: config.game_id.clone(),
        player_id: Some(config.player_id.clone()),
        command_type: CommandType::Speak,
        direction: None,
        speak_text: Some(build_llm_failure_speak_text(message)),
        turn_no: game.turn_no,
        sent_at: Utc::now(),
    }
}

fn command_from_decision(
    config: &BotConfig,
    game: &GameInstanceResponse,
    decision: &AgentDecisionResponse,
) -> Result<CommandEnvelope, DecisionValidationError> {
    if !is_supported_bot_command(decision.command_type) {
        return Err(DecisionValidationError::UnsupportedCommandType);
    }

    let (direction, speak_text) = if decision.command_type == CommandType::Speak {
        let speak_text = decision
            .speak_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or(DecisionValidationError::MissingSpeakText)?
            .to_string();
        (None, Some(speak_text))
    } else {
        (
            Some(
                decision
                    .direction
                    .ok_or(DecisionValidationError::MissingDirection)?,
            ),
            None,
        )
    };

    Ok(CommandEnvelope {
        command_id: format!(
            "bot-{}-{}-{}",
            config.bot_id,
            game.turn_no,
            Utc::now().timestamp_millis()
        ),
        source: CommandSource::Bot,
        game_id: config.game_id.clone(),
        player_id: Some(config.player_id.clone()),
        command_type: decision.command_type,
        direction,
        speak_text,
        turn_no: game.turn_no,
        sent_at: Utc::now(),
    })
}

fn is_supported_bot_command(command_type: CommandType) -> bool {
    matches!(
        command_type,
        CommandType::Move | CommandType::Shoot | CommandType::Shield | CommandType::Speak
    )
}

fn build_fallback_bot_command(
    config: &BotConfig,
    game: &GameInstanceResponse,
    message: &str,
) -> CommandEnvelope {
    build_llm_failure_speak_command(config, game, message)
}

async fn publish_command(
    state: &AppState,
    config: &BotConfig,
    command: &CommandEnvelope,
) -> anyhow::Result<()> {
    if state.mock_kafka {
        info!(
            bot_id = %config.bot_id,
            game_id = %config.game_id,
            command_type = ?command.command_type,
            "mock kafka enabled; skipping publish"
        );
        return Ok(());
    }

    let payload = serde_json::to_string(command).context("failed to encode bot command")?;
    state
        .producer
        .send(
            FutureRecord::to(&config.input_topic)
                .key(&command.command_id)
                .payload(&payload),
            Duration::from_secs(5),
        )
        .await
        .map_err(|(error, _)| anyhow::anyhow!("kafka publish failed: {error:?}"))?;
    Ok(())
}

async fn fetch_game(state: &AppState, game_id: &str) -> anyhow::Result<GameInstanceResponse> {
    let url = format!("{}/v2/games/{}", state.manager_base_url, game_id);
    let response = state
        .client
        .get(url)
        .send()
        .await
        .context("failed to fetch game")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        anyhow::bail!("manager returned {}: {}", status, body);
    }

    response
        .json::<GameInstanceResponse>()
        .await
        .context("invalid manager game payload")
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

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        warn!(status = %self.status, message = %self.message, "bot-service request failed");
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}
