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

use chrono::{DateTime, Utc};
use rand::Rng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_TURN_TIMEOUT_SECONDS: u64 = 120;
pub const DEFAULT_PLAYER_HP: i32 = 10;
pub const DEFAULT_NUM_PLAYERS: u8 = 2;
pub const MAX_NUM_PLAYERS: u8 = 4;
pub const MIN_NUM_PLAYERS: u8 = 1;

/// All possible player names in turn order.
pub const ALL_PLAYER_NAMES: [PlayerName; 4] = [
    PlayerName::A,
    PlayerName::B,
    PlayerName::C,
    PlayerName::D,
];

pub type PlayerId = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PlayerName {
    A,
    B,
    C,
    D,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Left,
    Down,
    Right,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandType {
    Move,
    Shield,
    Shoot,
    Speak,
    Timeout,
    GameStarted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    User,
    Bot,
    Timer,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GameStatus {
    Created,
    Running,
    Finished,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MapSource {
    Custom,
    Default,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResultStatus {
    Applied,
    TimeoutApplied,
    IgnoredTimeout,
    InvalidCommand,
    InvalidTurn,
    DuplicateCommand,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepEventType {
    GameStarted,
    StepApplied,
    TimeoutApplied,
    GameFinished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapData {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub player_name: PlayerName,
    pub player_id: PlayerId,
    pub hp: i32,
    pub row: usize,
    pub col: usize,
    pub shield: Direction,
    pub alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameStateSnapshot {
    pub map: MapData,
    pub players: Vec<PlayerState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGameRequest {
    pub turn_timeout_seconds: Option<u64>,
    pub map: Option<MapData>,
    #[serde(default)]
    pub bot_players: Option<Vec<PlayerName>>,
    /// Number of players in this game (1-4, default 2).
    #[serde(default)]
    pub num_players: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGameResponse {
    pub game_id: String,
    pub status: GameStatus,
    pub map_source: MapSource,
    pub turn_no: u64,
    pub round_no: u64,
    pub current_player_id: PlayerId,
    pub players: Vec<PlayerIdentity>,
    pub turn_timeout_seconds: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartGameResponse {
    pub game_id: String,
    pub status: GameStatus,
    pub started: bool,
    pub reason: Option<String>,
    pub turn_no: u64,
    pub round_no: u64,
    pub current_player_id: PlayerId,
    pub started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInstanceResponse {
    pub game_id: String,
    pub status: GameStatus,
    pub map_source: MapSource,
    pub turn_timeout_seconds: u64,
    pub turn_no: u64,
    pub round_no: u64,
    pub current_player_id: PlayerId,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    /// When the current turn began (used by frontend for accurate countdown).
    #[serde(default)]
    pub turn_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub input_topic: Option<String>,
    #[serde(default)]
    pub output_topic: Option<String>,
    pub state: GameStateSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub game_id: String,
    pub status: GameStatus,
    pub turn_no: u64,
    pub round_no: u64,
    pub current_player_id: PlayerId,
    pub state: GameStateSnapshot,
    pub last_step_seq: u64,
    /// When the current turn began (used by frontend for accurate countdown).
    #[serde(default)]
    pub turn_started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerIdentity {
    pub player_name: PlayerName,
    pub player_id: PlayerId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitCommandRequest {
    pub command_id: String,
    pub player_id: PlayerId,
    pub command_type: CommandType,
    pub direction: Option<Direction>,
    #[serde(default)]
    pub speak_text: Option<String>,
    pub turn_no: u64,
    pub client_sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitCommandResponse {
    pub accepted: bool,
    pub command_id: String,
    pub queued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub command_id: String,
    pub source: CommandSource,
    pub game_id: String,
    pub player_id: Option<PlayerId>,
    pub command_type: CommandType,
    pub direction: Option<Direction>,
    #[serde(default)]
    pub speak_text: Option<String>,
    pub turn_no: u64,
    pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepEvent {
    pub game_id: String,
    pub step_seq: u64,
    pub turn_no: u64,
    pub round_no: u64,
    pub event_type: StepEventType,
    pub result_status: ResultStatus,
    pub command: Option<CommandEnvelope>,
    pub state_after: GameStateSnapshot,
    pub created_at: DateTime<Utc>,
}

/// Create the initial set of players for a game.
///
/// `num_players` must be 1–4.  Players are assigned in order A, B, C, D and
/// placed on the edges of the grid (top, left, bottom, right respectively).
pub fn initial_players(rows: usize, cols: usize, hp: i32, num_players: u8) -> Vec<PlayerState> {
    let mid_row = rows / 2;
    let mid_col = cols / 2;
    let n = (num_players.max(MIN_NUM_PLAYERS).min(MAX_NUM_PLAYERS)) as usize;

    let all = vec![
        PlayerState {
            player_name: PlayerName::A,
            player_id: Uuid::new_v4().to_string(),
            hp,
            row: 0,
            col: mid_col,
            shield: Direction::Up,
            alive: true,
        },
        PlayerState {
            player_name: PlayerName::B,
            player_id: Uuid::new_v4().to_string(),
            hp,
            row: mid_row,
            col: 0,
            shield: Direction::Left,
            alive: true,
        },
        PlayerState {
            player_name: PlayerName::C,
            player_id: Uuid::new_v4().to_string(),
            hp,
            row: rows.saturating_sub(1),
            col: mid_col,
            shield: Direction::Down,
            alive: true,
        },
        PlayerState {
            player_name: PlayerName::D,
            player_id: Uuid::new_v4().to_string(),
            hp,
            row: mid_row,
            col: cols.saturating_sub(1),
            shield: Direction::Right,
            alive: true,
        },
    ];

    all.into_iter().take(n).collect()
}

pub fn generate_default_map(rows: usize, cols: usize, num_players: u8) -> MapData {
    let mut rng = rand::rng();
    let mut cells = vec![vec![0_i32; cols]; rows];

    for row in &mut cells {
        for cell in row {
            let roll: u8 = rng.random_range(0..100);
            *cell = if roll < 70 {
                0
            } else if roll < 86 {
                1
            } else if roll < 96 {
                2
            } else {
                -1
            };
        }
    }

    let mid_row = rows / 2;
    let mid_col = cols / 2;
    let n = (num_players.max(MIN_NUM_PLAYERS).min(MAX_NUM_PLAYERS)) as usize;
    let all_safe_positions = [
        (0, mid_col),
        (mid_row, 0),
        (rows.saturating_sub(1), mid_col),
        (mid_row, cols.saturating_sub(1)),
    ];

    for &(r, c) in all_safe_positions.iter().take(n) {
        if r < rows && c < cols {
            cells[r][c] = 0;
        }
    }

    MapData { rows, cols, cells }
}

pub fn default_map() -> MapData {
    MapData {
        rows: 11,
        cols: 11,
        cells: vec![
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0],
            vec![0, -1, 0, 0, 0, 1, 0, 0, 0, -1, 0],
            vec![2, 0, 1, 0, -1, 0, -1, 0, 1, 0, 2],
            vec![0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0],
            vec![0, 1, -1, 2, 0, 0, 0, 2, -1, 1, 0],
            vec![0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0],
            vec![2, 0, 1, 0, -1, 0, -1, 0, 1, 0, 2],
            vec![0, -1, 0, 0, 0, 1, 0, 0, 0, -1, 0],
            vec![0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0],
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ],
    }
}

/// Replace `${VAR_NAME}` patterns in a string with values from environment variables.
/// Unknown or unset variables are replaced with an empty string.
pub fn expand_env_vars(input: &str) -> String {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    re.replace_all(input, |caps: &regex::Captures| {
        let var_name = &caps[1];
        std::env::var(var_name).unwrap_or_default()
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn initial_players_start_on_side_centers_4_players() {
        let players = initial_players(11, 11, DEFAULT_PLAYER_HP, 4);
        assert_eq!(players.len(), 4);

        let a = players
            .iter()
            .find(|p| p.player_name == PlayerName::A)
            .unwrap();
        assert_eq!((a.row, a.col, a.shield), (0, 5, Direction::Up));
        assert_eq!(a.hp, DEFAULT_PLAYER_HP);

        let b = players
            .iter()
            .find(|p| p.player_name == PlayerName::B)
            .unwrap();
        assert_eq!((b.row, b.col, b.shield), (5, 0, Direction::Left));

        let c = players
            .iter()
            .find(|p| p.player_name == PlayerName::C)
            .unwrap();
        assert_eq!((c.row, c.col, c.shield), (10, 5, Direction::Down));

        let d = players
            .iter()
            .find(|p| p.player_name == PlayerName::D)
            .unwrap();
        assert_eq!((d.row, d.col, d.shield), (5, 10, Direction::Right));

        let unique_tiles: HashSet<(usize, usize)> =
            players.iter().map(|p| (p.row, p.col)).collect();
        assert_eq!(unique_tiles.len(), 4);

        let unique_ids: HashSet<String> = players.iter().map(|p| p.player_id.clone()).collect();
        assert_eq!(unique_ids.len(), 4);
    }

    #[test]
    fn initial_players_default_2_players() {
        let players = initial_players(11, 11, DEFAULT_PLAYER_HP, DEFAULT_NUM_PLAYERS);
        assert_eq!(players.len(), 2);
        assert_eq!(players[0].player_name, PlayerName::A);
        assert_eq!(players[1].player_name, PlayerName::B);
    }

    #[test]
    fn initial_players_3_players() {
        let players = initial_players(11, 11, DEFAULT_PLAYER_HP, 3);
        assert_eq!(players.len(), 3);
        assert_eq!(players[0].player_name, PlayerName::A);
        assert_eq!(players[1].player_name, PlayerName::B);
        assert_eq!(players[2].player_name, PlayerName::C);
    }

    #[test]
    fn initial_players_1_player() {
        let players = initial_players(11, 11, DEFAULT_PLAYER_HP, 1);
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].player_name, PlayerName::A);
    }

    #[test]
    fn generate_default_map_keeps_spawn_positions_empty() {
        let map = generate_default_map(11, 11, 4);
        assert_eq!(map.cells[0][5], 0);
        assert_eq!(map.cells[5][0], 0);
        assert_eq!(map.cells[10][5], 0);
        assert_eq!(map.cells[5][10], 0);
    }

    #[test]
    fn generate_default_map_2_players_keeps_2_spawns_empty() {
        let map = generate_default_map(11, 11, 2);
        assert_eq!(map.cells[0][5], 0);
        assert_eq!(map.cells[5][0], 0);
    }

    #[test]
    fn generate_default_map_only_uses_supported_block_values() {
        let map = generate_default_map(31, 31, 4);
        for row in &map.cells {
            for value in row {
                assert!([-1, 0, 1, 2].contains(value));
            }
        }
    }

    #[test]
    fn built_in_default_map_has_valid_size_and_safe_spawns() {
        let map = default_map();
        assert_eq!(map.rows, 11);
        assert_eq!(map.cols, 11);
        assert_eq!(map.cells.len(), map.rows);
        assert_eq!(map.cells[0].len(), map.cols);
        assert_eq!(map.cells[0][5], 0);
        assert_eq!(map.cells[5][0], 0);
        assert_eq!(map.cells[10][5], 0);
        assert_eq!(map.cells[5][10], 0);
    }
}
