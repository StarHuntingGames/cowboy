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
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use chrono::Utc;
use cowboy_common::{
    CommandEnvelope, CommandSource, CommandType, GameInstanceResponse, GameStatus, ResultStatus,
    StepEvent, StepEventType,
};
use rdkafka::{
    Message,
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
    producer::{FutureProducer, FutureRecord},
};
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    producer: FutureProducer,
    client: reqwest::Client,
    manager_base_url: String,
    bootstrap_servers: String,
    input_topic_prefix: String,
    output_topic_prefix: String,
    consumer_group_id: String,
    default_timeout_seconds: u64,
    timers: Arc<Mutex<HashMap<String, TimerEntry>>>,
}

#[derive(Debug, Clone)]
struct TimerEntry {
    generation: u64,
    turn_no: u64,
    scheduled_at: Instant,
}

impl AppState {
    fn from_env() -> anyhow::Result<Self> {
        let bootstrap_servers = std::env::var("KAFKA_BOOTSTRAP_SERVERS")
            .ok()
            .unwrap_or_else(|| "kafka:9092".to_string());
        let producer = ClientConfig::new()
            .set("bootstrap.servers", &bootstrap_servers)
            .set("message.timeout.ms", "5000")
            .create()
            .context("failed to create timer-service producer")?;
        Ok(Self {
            producer,
            client: reqwest::Client::new(),
            manager_base_url: std::env::var("GAME_MANAGER_BASE_URL")
                .ok()
                .unwrap_or_else(|| "http://game-manager-service:8081".to_string()),
            bootstrap_servers,
            input_topic_prefix: std::env::var("GAME_INPUT_TOPIC_PREFIX")
                .ok()
                .unwrap_or_else(|| "game.commands".to_string()),
            output_topic_prefix: std::env::var("GAME_OUTPUT_TOPIC_PREFIX")
                .ok()
                .unwrap_or_else(|| "game.output".to_string()),
            consumer_group_id: std::env::var("TIMER_CONSUMER_GROUP_ID")
                .ok()
                .unwrap_or_else(|| "timer-service-v1".to_string()),
            default_timeout_seconds: std::env::var("TURN_TIMEOUT_SECONDS_DEFAULT")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(120)
                .max(1),
            timers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn output_topic_pattern(&self) -> String {
        format!(
            "^{}\\..*\\.v1$",
            self.output_topic_prefix.replace('.', "\\.")
        )
    }

    fn input_topic_for_game(&self, game_id: &str) -> String {
        format!("{}.{}.v1", self.input_topic_prefix, game_id)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "timer_service=debug".to_string()),
        )
        .init();

    let state = AppState::from_env()?;
    let runner_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = run_step_consumer(runner_state).await {
            warn!(error = %error, "timer consumer stopped");
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("timer-service shutting down");
    Ok(())
}

async fn run_step_consumer(state: AppState) -> anyhow::Result<()> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &state.bootstrap_servers)
        .set("group.id", &state.consumer_group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("topic.metadata.refresh.interval.ms", "1000")
        .set("topic.metadata.refresh.fast.interval.ms", "250")
        .create()
        .context("failed to create timer-service consumer")?;

    let pattern = state.output_topic_pattern();
    consumer
        .subscribe(&[&pattern])
        .context("failed to subscribe timer-service output topics")?;
    info!(pattern = %pattern, "timer-service subscribed to output topics");

    loop {
        let message = match consumer.recv().await {
            Ok(message) => message,
            Err(error) => {
                warn!(?error, "timer-service kafka receive error");
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
        };

        let payload = match message.payload() {
            Some(payload) => payload,
            None => {
                if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(?error, "timer-service failed to commit empty payload");
                }
                continue;
            }
        };

        let step = match serde_json::from_slice::<StepEvent>(payload) {
            Ok(step) => step,
            Err(error) => {
                warn!(?error, "timer-service failed to parse step payload");
                if let Err(commit_err) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(
                        ?commit_err,
                        "timer-service commit failed for invalid payload"
                    );
                }
                continue;
            }
        };

        handle_step_event(&state, step).await;

        if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
            warn!(?error, "timer-service failed to commit consumed step");
        }
    }
}

async fn handle_step_event(state: &AppState, step: StepEvent) {
    if step.event_type == StepEventType::GameFinished {
        let mut timers = state.timers.lock().await;
        timers.remove(&step.game_id);
        info!(game_id = %step.game_id, "timer cancelled on game finish");
        return;
    }

    if !should_reset_timer(&step) {
        return;
    }

    let game = match fetch_game(state, &step.game_id).await {
        Ok(game) => game,
        Err(error) => {
            warn!(game_id = %step.game_id, error = %error, "timer-service failed to fetch game after step");
            return;
        }
    };

    if game.status != GameStatus::Running {
        let mut timers = state.timers.lock().await;
        timers.remove(&game.game_id);
        return;
    }

    let timeout_seconds = if game.turn_timeout_seconds == 0 {
        state.default_timeout_seconds.max(1)
    } else {
        game.turn_timeout_seconds.max(1)
    };
    let generation = {
        let mut timers = state.timers.lock().await;
        let next_generation = timers
            .get(&game.game_id)
            .map(|entry| entry.generation + 1)
            .unwrap_or(1);
        timers.insert(
            game.game_id.clone(),
            TimerEntry {
                generation: next_generation,
                turn_no: game.turn_no,
                scheduled_at: Instant::now(),
            },
        );
        next_generation
    };

    let runner = state.clone();
    let game_id = game.game_id.clone();
    let turn_no = game.turn_no;
    info!(
        game_id = %game_id,
        turn_no,
        timeout_seconds,
        "timer scheduled for turn"
    );
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_seconds)).await;
        if let Err(error) = fire_timeout_if_still_valid(&runner, game_id, turn_no, generation).await
        {
            warn!(error = %error, "timer timeout publish failed");
        }
    });
}

fn should_reset_timer(step: &StepEvent) -> bool {
    if step.event_type == StepEventType::GameStarted {
        return true;
    }

    matches!(
        step.result_status,
        ResultStatus::Applied | ResultStatus::TimeoutApplied
    )
}

async fn fire_timeout_if_still_valid(
    state: &AppState,
    game_id: String,
    expected_turn_no: u64,
    expected_generation: u64,
) -> anyhow::Result<()> {
    {
        let timers = state.timers.lock().await;
        let Some(entry) = timers.get(&game_id) else {
            return Ok(());
        };
        if entry.generation != expected_generation || entry.turn_no != expected_turn_no {
            return Ok(());
        }
        let _ = entry.scheduled_at.elapsed();
    }

    let game = fetch_game(state, &game_id).await?;
    if game.status != GameStatus::Running || game.turn_no != expected_turn_no {
        return Ok(());
    }

    let command = CommandEnvelope {
        command_id: format!(
            "timeout-{}-{}-{}",
            game_id,
            expected_turn_no,
            Utc::now().timestamp_millis()
        ),
        source: CommandSource::Timer,
        game_id: game_id.clone(),
        player_id: Some(game.current_player_id.clone()),
        command_type: CommandType::Timeout,
        direction: None,
        speak_text: None,
        turn_no: expected_turn_no,
        sent_at: Utc::now(),
    };
    let topic = state.input_topic_for_game(&game_id);
    let payload = serde_json::to_string(&command).context("failed to encode timeout command")?;
    state
        .producer
        .send(
            FutureRecord::to(&topic)
                .key(&command.command_id)
                .payload(&payload),
            Duration::from_secs(5),
        )
        .await
        .map_err(|(error, _)| anyhow::anyhow!("Kafka timeout publish failed: {error:?}"))?;

    info!(
        game_id = %game_id,
        turn_no = expected_turn_no,
        topic = %topic,
        "published timeout command to input topic"
    );
    Ok(())
}

async fn fetch_game(state: &AppState, game_id: &str) -> anyhow::Result<GameInstanceResponse> {
    let url = format!("{}/v2/games/{}", state.manager_base_url, game_id);
    let response = state
        .client
        .get(url)
        .send()
        .await
        .context("failed to fetch game from manager")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "".to_string());
        anyhow::bail!("manager returned {} for game {}: {}", status, game_id, body);
    }
    response
        .json::<GameInstanceResponse>()
        .await
        .context("invalid manager game payload")
}
