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

use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use cowboy_common::{
    CommandEnvelope, CommandSource, CommandType, SubmitCommandRequest, SubmitCommandResponse,
};
use lambda_http::run as lambda_run;
use rdkafka::{
    config::ClientConfig,
    producer::{FutureProducer, FutureRecord},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    publisher: Arc<dyn CommandPublisher>,
}

#[async_trait]
trait CommandPublisher: Send + Sync {
    async fn publish(&self, command: &CommandEnvelope) -> anyhow::Result<()>;
}

#[derive(Clone)]
struct KafkaCommandPublisher {
    producer: FutureProducer,
    input_topic_prefix: String,
}

impl KafkaCommandPublisher {
    fn from_env() -> anyhow::Result<Self> {
        let bootstrap_servers = std::env::var("KAFKA_BOOTSTRAP_SERVERS")
            .ok()
            .unwrap_or_else(|| "kafka:9092".to_string());
        let producer = ClientConfig::new()
            .set("bootstrap.servers", bootstrap_servers)
            .set("message.timeout.ms", "5000")
            .create()
            .context("failed to create Kafka command producer")?;
        let input_topic_prefix = std::env::var("GAME_INPUT_TOPIC_PREFIX")
            .ok()
            .unwrap_or_else(|| "game.commands".to_string());
        Ok(Self {
            producer,
            input_topic_prefix,
        })
    }

    fn topic_for_game(&self, game_id: &str) -> String {
        format!("{}.{}.v1", self.input_topic_prefix, game_id)
    }
}

#[async_trait]
impl CommandPublisher for KafkaCommandPublisher {
    async fn publish(&self, command: &CommandEnvelope) -> anyhow::Result<()> {
        let topic = self.topic_for_game(&command.game_id);
        let payload = serde_json::to_string(command).context("failed to encode command")?;
        self.producer
            .send(
                FutureRecord::to(&topic)
                    .key(&command.command_id)
                    .payload(&payload),
                std::time::Duration::from_secs(5),
            )
            .await
            .map_err(|(error, _)| anyhow::anyhow!("Kafka publish failed: {error:?}"))?;

        info!(
            game_id = %command.game_id,
            command_id = %command.command_id,
            command_type = ?command.command_type,
            topic = %topic,
            "command published to Kafka input topic"
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "web_service=debug,tower_http=info".to_string()),
        )
        .init();

    let state = AppState {
        publisher: Arc::new(KafkaCommandPublisher::from_env()?),
    };

    let app = build_router(state);

    if std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok() {
        info!("AWS Lambda runtime detected; running web-service in lambda mode");
        lambda_run(app)
            .await
            .map_err(|e| anyhow::Error::msg(format!("lambda runtime error: {e}")))?;
        return Ok(());
    }

    let bind_addr = parse_bind_addr("WEB_SERVICE_BIND", "0.0.0.0:8082")?;
    info!(%bind_addr, "web-service listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v2/games/{game_id}/commands", post(submit_command_handler))
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
    Json(serde_json::json!({"ok": true, "service": "web-service"}))
}

async fn submit_command_handler(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(request): Json<SubmitCommandRequest>,
) -> Result<Json<SubmitCommandResponse>, ApiError> {
    validate_user_command(&request)?;

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

    state
        .publisher
        .publish(&command)
        .await
        .map_err(|e| ApiError::internal(format!("failed to publish command: {e}")))?;

    Ok(Json(SubmitCommandResponse {
        accepted: true,
        command_id: command.command_id,
        queued_at: Utc::now(),
    }))
}

fn validate_user_command(request: &SubmitCommandRequest) -> Result<(), ApiError> {
    if request.command_id.trim().is_empty() {
        return Err(ApiError::bad_request("command_id is required"));
    }

    if matches!(
        request.command_type,
        CommandType::Timeout | CommandType::GameStarted
    ) {
        return Err(ApiError::bad_request(
            "command_type timeout/game_started is reserved for system services",
        ));
    }

    if matches!(
        request.command_type,
        CommandType::Move | CommandType::Shield | CommandType::Shoot
    ) && request.direction.is_none()
    {
        return Err(ApiError::bad_request(
            "direction is required for move/shield/shoot commands",
        ));
    }

    if request.command_type == CommandType::Speak
        && request
            .speak_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .is_none()
    {
        return Err(ApiError::bad_request(
            "speak_text is required for speak commands",
        ));
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use cowboy_common::{Direction};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingPublisher {
        published: Mutex<Vec<CommandEnvelope>>,
        fail: bool,
    }

    #[async_trait]
    impl CommandPublisher for RecordingPublisher {
        async fn publish(&self, command: &CommandEnvelope) -> anyhow::Result<()> {
            if self.fail {
                return Err(anyhow::anyhow!("forced publish error"));
            }
            self.published.lock().unwrap().push(command.clone());
            Ok(())
        }
    }

    fn make_request(
        command_type: CommandType,
        direction: Option<Direction>,
    ) -> SubmitCommandRequest {
        SubmitCommandRequest {
            command_id: "cmd-1".to_string(),
            player_id: "Up".to_string(),
            command_type,
            direction,
            speak_text: None,
            turn_no: 1,
            client_sent_at: Utc::now(),
        }
    }

    #[test]
    fn validate_user_command_rejects_reserved_types() {
        let timeout_req = make_request(CommandType::Timeout, None);
        let started_req = make_request(CommandType::GameStarted, None);

        let timeout_err = validate_user_command(&timeout_req).unwrap_err();
        let started_err = validate_user_command(&started_req).unwrap_err();

        assert_eq!(timeout_err.status, StatusCode::BAD_REQUEST);
        assert_eq!(started_err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_user_command_requires_direction_for_non_timeout_commands() {
        let move_req = make_request(CommandType::Move, None);
        let shield_req = make_request(CommandType::Shield, None);
        let shoot_req = make_request(CommandType::Shoot, None);

        assert!(validate_user_command(&move_req).is_err());
        assert!(validate_user_command(&shield_req).is_err());
        assert!(validate_user_command(&shoot_req).is_err());
    }

    #[test]
    fn validate_user_command_accepts_valid_move_request() {
        let req = make_request(CommandType::Move, Some(Direction::Left));
        assert!(validate_user_command(&req).is_ok());
    }

    #[test]
    fn validate_user_command_requires_speak_text_for_speak() {
        let req = make_request(CommandType::Speak, None);
        assert!(validate_user_command(&req).is_err());
    }

    #[test]
    fn validate_user_command_accepts_valid_speak_request() {
        let mut req = make_request(CommandType::Speak, None);
        req.speak_text = Some("hello cowboy".to_string());
        assert!(validate_user_command(&req).is_ok());
    }

    #[tokio::test]
    async fn submit_command_handler_publishes_envelope() {
        let publisher = Arc::new(RecordingPublisher::default());
        let state = AppState {
            publisher: publisher.clone(),
        };
        let req = make_request(CommandType::Shoot, Some(Direction::Right));

        let response = submit_command_handler(
            State(state),
            Path("game-123".to_string()),
            Json(req.clone()),
        )
        .await
        .unwrap()
        .0;

        assert!(response.accepted);
        assert_eq!(response.command_id, req.command_id);

        let published = publisher.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        let command = &published[0];
        assert_eq!(command.game_id, "game-123");
        assert_eq!(command.command_type, CommandType::Shoot);
        assert_eq!(command.direction, Some(Direction::Right));
        assert_eq!(command.source, CommandSource::User);
    }

    #[tokio::test]
    async fn submit_command_handler_returns_internal_error_on_publish_failure() {
        let publisher = Arc::new(RecordingPublisher {
            published: Mutex::new(vec![]),
            fail: true,
        });
        let state = AppState { publisher };
        let req = make_request(CommandType::Move, Some(Direction::Down));

        let err = submit_command_handler(State(state), Path("game-123".to_string()), Json(req))
            .await
            .unwrap_err();

        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("failed to publish command"));
    }
}
