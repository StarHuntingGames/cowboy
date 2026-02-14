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

const canvas = document.getElementById("gameCanvas");
const ctx = canvas.getContext("2d");

const statusLine = document.getElementById("statusLine");
const roundLine = document.getElementById("roundLine");
const gameIdLine = document.getElementById("gameIdLine");
const playersList = document.getElementById("playersList");
const logList = document.getElementById("logList");

const startButton = document.getElementById("startButton");
const newButton = document.getElementById("newButton");
const connectButton = document.getElementById("connectButton");
const gameIdInput = document.getElementById("gameIdInput");

const commandGrid = document.getElementById("commandGrid");
const numPlayersSelect = document.getElementById("numPlayersSelect");
let commandControls = {};

const MAP_ROWS = 11;
const MAP_COLS = 11;

// Double-saber weapon sprite.
const doubleSaberImg = new Image();
doubleSaberImg.src = "assets/double-saber.png";
const DEFAULT_HP = 10;
const DEFAULT_TIMEOUT_SECONDS = 120;
const LASER_BEAM_DURATION_MS = 1000;
const SWEEP_BEAM_DELAY_MS = 120;
const SWEEP_GLOW_DURATION_MS = 900;
const HIT_FLASH_DURATION_MS = 700;
const HIT_SHAKE_DURATION_MS = 260;
const HIT_SHAKE_AMPLITUDE = 4.5;
const WATCHER_RECONNECT_MS = 1200;
const WATCHER_HEALTH_CHECK_MS = 2000;
const WATCHER_STALE_MS_MIN = 25000;
const FINISH_ANIMATION_DURATION_MS = 2600;
const TIMEOUT_ANIMATION_DURATION_MS = 1500;
const SPEAK_ANIMATION_DURATION_MS = 10000;

const ALL_PLAYER_NAMES = ["A", "B", "C", "D"];
const DEFAULT_NUM_PLAYERS = 2;

function getNumPlayers() {
  return state.numPlayers || DEFAULT_NUM_PLAYERS;
}

function getPlayerOrder() {
  return ALL_PLAYER_NAMES.slice(0, getNumPlayers());
}

const DIRECTION = {
  up: { dr: -1, dc: 0 },
  left: { dr: 0, dc: -1 },
  down: { dr: 1, dc: 0 },
  right: { dr: 0, dc: 1 },
};

const OPPOSITE = {
  up: "down",
  down: "up",
  left: "right",
  right: "left",
};

const PERPENDICULAR = {
  up: ["left", "right"],
  down: ["left", "right"],
  left: ["up", "down"],
  right: ["up", "down"],
};

const COLORS = {
  A: "#c94833",
  B: "#2a61b8",
  C: "#27864f",
  D: "#986c1f",
};

const SIDES = {
  A: "Up",
  B: "Left",
  C: "Down",
  D: "Right",
};

const BACKEND = (() => {
  const config = window.COWBOY_BACKEND || {};
  return {
    manager: config.manager || "",
    web: config.web || "",
    watcher: config.watcher || "",
  };
})();

const KNIGHT_SPRITE_URLS = {
  idle: "https://img.itch.zone/aW1nLzE3NzY1MjYxLmdpZg%3D%3D/original/bPmjnC.gif",
  run: "https://img.itch.zone/aW1nLzE3Nzc0MjM5LmdpZg%3D%3D/original/0C5J0V.gif",
  attack: "https://img.itch.zone/aW1nLzE3Nzc0MjUzLmdpZg%3D%3D/original/z7%2FgDs.gif",
  roll: "https://img.itch.zone/aW1nLzE3Nzc0MjYxLmdpZg%3D%3D/original/3xUf8y.gif",
};

const KNIGHT_SPRITES = Object.fromEntries(
  Object.entries(KNIGHT_SPRITE_URLS).map(([key, url]) => {
    const img = new Image();
    img.src = url;
    return [key, img];
  })
);

const TEMPLATE = [
  [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0],
  [0, -1, 0, 0, 0, 1, 0, 0, 0, -1, 0],
  [2, 0, 1, 0, -1, 0, -1, 0, 1, 0, 2],
  [0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0],
  [0, 1, -1, 2, 0, 0, 0, 2, -1, 1, 0],
  [0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0],
  [2, 0, 1, 0, -1, 0, -1, 0, 1, 0, 2],
  [0, -1, 0, 0, 0, 1, 0, 0, 0, -1, 0],
  [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0],
  [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];

const state = {
  phase: "idle",
  numPlayers: DEFAULT_NUM_PLAYERS,
  map: [],
  players: [],
  currentTurnIndex: 0,
  round: 1,
  turnNo: 1,
  logs: [],
  laserBeams: [],
  sweepGlows: [],
  saberFx: [],
  hitFlashes: [],
  hitShakes: [],
  timeoutFx: [],
  speakFx: [],
  finishFx: null,
  laserAnimationFrame: null,
  pendingCommand: false,
  lastSnapshotAt: 0,
  lastLocalShotCommandId: null,
  reconnectTimer: null,
  watcherHealthTimer: null,
  lastFinishAnnounceKey: null,
  backend: {
    connected: false,
    gameId: null,
    status: "IDLE",
    timeoutSeconds: DEFAULT_TIMEOUT_SECONDS,
    watcherSocket: null,
    watcherLastEventAt: 0,
  },
};

let audioCtx = null;

function displayName(id) {
  const localPlayer = getPlayerById(id);
  const name = localPlayer?.name || id;
  const side = SIDES[name];
  return side ? `Player ${name} (${side})` : `Player ${name}`;
}

function cloneMapFromTemplate() {
  return TEMPLATE.map((row) =>
    row.map((cell) => {
      if (cell === 0) {
        return { type: "empty" };
      }
      return { type: "block", strength: cell };
    })
  );
}

function createPlayers(numPlayers) {
  const n = numPlayers || getNumPlayers();
  const midRow = Math.floor(MAP_ROWS / 2);
  const midCol = Math.floor(MAP_COLS / 2);

  const all = [
    { name: "A", id: "local-a", row: 0, col: midCol, hp: DEFAULT_HP, shield: "up", aim: "up", spriteMode: "idle", alive: true },
    { name: "B", id: "local-b", row: midRow, col: 0, hp: DEFAULT_HP, shield: "left", aim: "left", spriteMode: "idle", alive: true },
    { name: "C", id: "local-c", row: MAP_ROWS - 1, col: midCol, hp: DEFAULT_HP, shield: "down", aim: "down", spriteMode: "idle", alive: true },
    { name: "D", id: "local-d", row: midRow, col: MAP_COLS - 1, hp: DEFAULT_HP, shield: "right", aim: "right", spriteMode: "idle", alive: true },
  ];

  return all.slice(0, n);
}

function mapRows() {
  return state.map.length || MAP_ROWS;
}

function mapCols() {
  return state.map[0]?.length || MAP_COLS;
}

function toLocalMap(mapData) {
  if (!mapData || !Array.isArray(mapData.cells) || mapData.cells.length === 0) {
    return cloneMapFromTemplate();
  }

  return mapData.cells.map((row) =>
    row.map((cell) => {
      if (cell === 0) {
        return { type: "empty" };
      }
      return { type: "block", strength: cell };
    })
  );
}

function toLocalPlayers(players, previousPlayers = []) {
  if (!Array.isArray(players) || players.length === 0) {
    return createPlayers();
  }

  const previousById = new Map((previousPlayers || []).map((player) => [player.id, player]));
  const playerOrder = ALL_PLAYER_NAMES;
  const byName = new Map(
    players.map((player, index) => {
      const fallbackName = playerOrder[index] || "A";
      const name = typeof player.player_name === "string" && player.player_name ? player.player_name : fallbackName;
      return [
        name,
      {
        name,
        id: player.player_id,
        row: player.row,
        col: player.col,
        hp: player.hp,
        shield: player.shield,
        aim: previousById.get(player.player_id)?.aim || player.shield,
        spriteMode: "idle",
        alive: Boolean(player.alive),
      },
    ];
    })
  );

  return playerOrder.map((name) => byName.get(name)).filter(Boolean);
}

function turnIndexForPlayer(playerId) {
  const player = state.players.find((entry) => entry.id === playerId);
  const order = getPlayerOrder();
  const index = player ? order.indexOf(player.name) : -1;
  return index >= 0 ? index : 0;
}

function nextCommandId() {
  if (window.crypto && typeof window.crypto.randomUUID === "function") {
    return window.crypto.randomUUID();
  }
  return `cmd-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

async function requestJson(url, init) {
  let response;
  try {
    response = await fetch(url, init);
  } catch (error) {
    throw new Error(`Cannot reach backend at ${url}. ${error.message}`);
  }

  const bodyText = await response.text();
  let payload = null;
  if (bodyText) {
    try {
      payload = JSON.parse(bodyText);
    } catch (_error) {
      payload = bodyText;
    }
  }

  if (!response.ok) {
    const errorMessage =
      payload && typeof payload === "object" && payload.error
        ? payload.error
        : `${response.status} ${response.statusText}`;
    throw new Error(errorMessage);
  }

  return payload;
}

function resetVisualState() {
  state.laserBeams = [];
  state.hitFlashes = [];
  state.hitShakes = [];
  state.timeoutFx = [];
  state.speakFx = [];
  state.finishFx = null;
  stopLaserAnimationLoop();
}

async function createBackendGame(numPlayers) {
  return requestJson(`${BACKEND.manager}/v2/games`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      turn_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
      num_players: numPlayers || getNumPlayers(),
    }),
  });
}

async function startBackendGame(gameId) {
  return requestJson(`${BACKEND.manager}/v2/games/${encodeURIComponent(gameId)}/start`, {
    method: "POST",
  });
}

async function getBackendGame(gameId) {
  return requestJson(`${BACKEND.manager}/v2/games/${encodeURIComponent(gameId)}`, {
    method: "GET",
  });
}

async function getWatcherSnapshot(gameId, fromTurnNo = null) {
  const query = fromTurnNo === null ? "" : `?from_turn_no=${encodeURIComponent(fromTurnNo)}`;
  return requestJson(
    `${BACKEND.watcher}/v2/games/${encodeURIComponent(gameId)}/snapshot${query}`,
    { method: "GET" }
  );
}

async function submitBackendCommand(gameId, payload) {
  return requestJson(`${BACKEND.web}/v2/games/${encodeURIComponent(gameId)}/commands`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
}

function cloneLocalMap(map) {
  return map.map((row) =>
    row.map((tile) => {
      if (tile.type === "empty") {
        return { type: "empty" };
      }
      return { type: "block", strength: tile.strength };
    })
  );
}

function cloneLocalPlayers(players) {
  return players.map((player) => ({ ...player }));
}

function localSnapshot() {
  const activePlayer = getActivePlayer();
  return {
    map: cloneLocalMap(state.map),
    players: cloneLocalPlayers(state.players),
    turnNo: state.turnNo,
    roundNo: state.round,
    currentPlayerId: activePlayer ? activePlayer.id : null,
    status: state.backend.status,
  };
}

function toWebSocketUrl(baseUrl, gameId, fromTurnNo) {
  const parsed = new URL(baseUrl || location.origin);
  parsed.protocol = parsed.protocol === "https:" ? "wss:" : "ws:";
  parsed.pathname = `/v2/games/${encodeURIComponent(gameId)}/stream`;
  parsed.search = `from_turn_no=${encodeURIComponent(fromTurnNo)}`;
  return parsed.toString();
}

function closeWatcherStream() {
  if (state.reconnectTimer) {
    clearTimeout(state.reconnectTimer);
    state.reconnectTimer = null;
  }
  if (state.watcherHealthTimer) {
    clearInterval(state.watcherHealthTimer);
    state.watcherHealthTimer = null;
  }

  const socket = state.backend.watcherSocket;
  if (socket) {
    state.backend.watcherSocket = null;
    socket.onopen = null;
    socket.onmessage = null;
    socket.onclose = null;
    socket.onerror = null;
    socket.close();
  }
  state.backend.watcherLastEventAt = 0;
}

function scheduleWatcherReconnect(gameId, reason) {
  if (!state.backend.gameId || state.phase === "idle" || state.phase === "finished") {
    return;
  }

  if (!state.reconnectTimer) {
    if (reason) {
      pushLog(`Watcher stream ${reason}; reconnecting...`);
    }
    state.reconnectTimer = setTimeout(() => {
      state.reconnectTimer = null;
      const fromTurnNo = Math.max(0, state.turnNo);
      refreshSnapshot(gameId, fromTurnNo).catch(() => {});
      connectWatcherStream(gameId, fromTurnNo);
    }, WATCHER_RECONNECT_MS);
  }
}

function startWatcherHealthTimer(gameId, socket) {
  if (state.watcherHealthTimer) {
    clearInterval(state.watcherHealthTimer);
    state.watcherHealthTimer = null;
  }

  state.watcherHealthTimer = setInterval(() => {
    if (state.backend.watcherSocket !== socket) {
      return;
    }
    if (!state.backend.gameId || state.phase !== "playing") {
      return;
    }

    const lastEventAt = state.backend.watcherLastEventAt || state.lastSnapshotAt || 0;
    if (!lastEventAt) {
      return;
    }

    const timeoutMs = Math.max(
      WATCHER_STALE_MS_MIN,
      (state.backend.timeoutSeconds || DEFAULT_TIMEOUT_SECONDS) * 3000
    );
    if (Date.now() - lastEventAt < timeoutMs) {
      return;
    }

    state.backend.watcherSocket = null;
    socket.onopen = null;
    socket.onmessage = null;
    socket.onclose = null;
    socket.onerror = null;
    try {
      socket.close();
    } catch (_error) {}
    scheduleWatcherReconnect(gameId, "stale");
  }, WATCHER_HEALTH_CHECK_MS);
}

function buildActionButtons(playerName) {
  const ACTIONS = [
    { key: "J", action: "move",   label: "Move" },
    { key: "K", action: "shoot",  label: "Shoot" },
    { key: "L", action: "shield", label: "Shield" },
    { key: "↵", action: "speak",  label: "Speak" },
  ];

  const container = document.createElement("div");
  container.className = "action-btns";

  const buttons = {};
  let _value = "move";
  let _disabled = false;
  const _changeListeners = [];

  for (const { key, action, label } of ACTIONS) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "action-btn";
    btn.innerHTML = key ? `<span class="action-key">${key}</span> ${label}` : label;
    btn.dataset.action = action;
    btn.setAttribute("aria-label", key ? `${key}: ${label}` : label);
    buttons[action] = btn;

    btn.addEventListener("click", () => {
      if (_disabled) return;
      _value = action;
      updateActive();
      for (const fn of _changeListeners) fn();
      if (state.phase === "playing") {
        const activePlayer = getActivePlayer();
        if (activePlayer && activePlayer.name === playerName) {
          void executeActiveCommand();
        }
      }
    });
  }

  function updateActive() {
    for (const [a, b] of Object.entries(buttons)) {
      b.classList.toggle("active", a === _value);
      b.disabled = _disabled;
    }
  }

  for (const { action } of ACTIONS) {
    container.appendChild(buttons[action]);
  }

  updateActive();

  const proxy = {
    get value() { return _value; },
    set value(v) { _value = v; updateActive(); },
    get disabled() { return _disabled; },
    set disabled(v) { _disabled = !!v; updateActive(); },
    classList: container.classList,
    element: container,
    focus() { buttons[_value]?.focus(); },
    addEventListener(type, fn) {
      if (type === "change") _changeListeners.push(fn);
    },
  };

  return { container, proxy };
}

function buildDpad(playerName) {
  const ARROWS = { up: "\u25B2", left: "\u25C0", down: "\u25BC", right: "\u25B6" };
  const container = document.createElement("div");
  container.className = "dpad";

  const buttons = {};
  let _value = "up";
  let _disabled = false;

  for (const dir of ["up", "left", "down", "right"]) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `dpad-btn dpad-${dir}`;
    btn.textContent = ARROWS[dir];
    btn.dataset.dir = dir;
    btn.setAttribute("aria-label", dir);
    buttons[dir] = btn;

    btn.addEventListener("click", () => {
      if (_disabled) return;
      _value = dir;
      updateDpadActive();
    });
  }

  function updateDpadActive() {
    for (const [d, b] of Object.entries(buttons)) {
      b.classList.toggle("active", d === _value);
      b.disabled = _disabled;
    }
  }

  container.appendChild(buttons.up);
  const midRow = document.createElement("div");
  midRow.className = "dpad-mid";
  midRow.appendChild(buttons.left);
  midRow.appendChild(buttons.right);
  container.appendChild(midRow);
  container.appendChild(buttons.down);

  updateDpadActive();

  // Proxy object that behaves like a <select> for existing code.
  const proxy = {
    get value() { return _value; },
    set value(v) { _value = v; updateDpadActive(); },
    get disabled() { return _disabled; },
    set disabled(v) { _disabled = !!v; updateDpadActive(); },
    classList: container.classList,
    element: container,
    focus() { buttons[_value]?.focus(); },
  };

  return { container, proxy };
}

function buildCommandControls() {
  commandGrid.innerHTML = "";
  commandControls = {};

  // Only build controls for Player A (the human player). Bots are AI-controlled.
  const order = ["A"];
  for (const playerName of order) {
    const side = SIDES[playerName] || playerName;
    const lowerSide = side.toLowerCase();

    const wrapper = document.createElement("div");
    wrapper.className = "command-row";

    // D-pad first.
    const { container: dpadEl, proxy: dirProxy } = buildDpad(playerName);
    wrapper.appendChild(dpadEl);

    // Action buttons below.
    const { container: actionEl, proxy: actionProxy } = buildActionButtons(playerName);
    wrapper.appendChild(actionEl);

    // Speak text input (always visible, enabled when Speak is active).
    const speakInput = document.createElement("input");
    speakInput.id = `cmd-${lowerSide}-speak`;
    speakInput.dataset.player = playerName;
    speakInput.type = "text";
    speakInput.maxLength = 140;
    speakInput.placeholder = "Speak text";
    speakInput.setAttribute("aria-label", `Player ${playerName} speak text`);
    wrapper.appendChild(speakInput);

    commandGrid.appendChild(wrapper);

    commandControls[playerName] = {
      action: actionProxy,
      direction: dirProxy,
      speakText: speakInput,
    };

    actionProxy.addEventListener("change", () => {
      updateCommandControlMode(commandControls[playerName]);
      render();
    });

    updateCommandControlMode(commandControls[playerName]);
  }
}

function clearCommandInputs() {
  for (const control of Object.values(commandControls)) {
    control.action.value = "move";
    control.direction.value = "up";
    control.speakText.value = "";
    updateCommandControlMode(control);
  }
}

function updateCommandControlMode(control) {
  const isSpeak = control.action.value === "speak";
  control.direction.disabled = isSpeak || control.action.disabled;
}

function ensureAudioContext() {
  const AudioContextRef = window.AudioContext || window.webkitAudioContext;
  if (!AudioContextRef) {
    return null;
  }

  if (!audioCtx) {
    audioCtx = new AudioContextRef();
  }

  if (audioCtx.state === "suspended") {
    audioCtx.resume().catch(() => {});
  }

  return audioCtx;
}

function playHitSound(kind) {
  const ctxAudio = ensureAudioContext();
  if (!ctxAudio) {
    return;
  }

  const now = ctxAudio.currentTime;
  const gain = ctxAudio.createGain();
  const osc = ctxAudio.createOscillator();

  let startFreq = 260;
  let endFreq = 140;
  let peakGain = 0.08;
  let type = "square";

  if (kind === "player") {
    startFreq = 520;
    endFreq = 230;
    peakGain = 0.1;
    type = "sawtooth";
  } else if (kind === "shield") {
    startFreq = 390;
    endFreq = 260;
    peakGain = 0.09;
    type = "triangle";
  }

  osc.type = type;
  osc.frequency.setValueAtTime(startFreq, now);
  osc.frequency.exponentialRampToValueAtTime(endFreq, now + 0.14);

  gain.gain.setValueAtTime(0.0001, now);
  gain.gain.exponentialRampToValueAtTime(peakGain, now + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.17);

  osc.connect(gain);
  gain.connect(ctxAudio.destination);

  osc.start(now);
  osc.stop(now + 0.18);
}

function applyBackendSnapshot(rawSnapshot, source = "snapshot") {
  if (!rawSnapshot || !rawSnapshot.state) {
    throw new Error("Invalid snapshot payload");
  }

  const previous = localSnapshot();
  const previousPlayers = cloneLocalPlayers(state.players);

  // Detect num_players from the snapshot's player count
  if (rawSnapshot.state.players && rawSnapshot.state.players.length > 0) {
    const snapshotPlayerCount = rawSnapshot.state.players.length;
    if (snapshotPlayerCount !== state.numPlayers) {
      state.numPlayers = snapshotPlayerCount;
      numPlayersSelect.value = String(snapshotPlayerCount);
      buildCommandControls();
    }
  }

  state.map = toLocalMap(rawSnapshot.state.map);
  state.players = toLocalPlayers(rawSnapshot.state.players, previousPlayers);
  state.currentTurnIndex = turnIndexForPlayer(rawSnapshot.current_player_id);
  state.round = rawSnapshot.round_no || previous.roundNo || 1;
  state.turnNo = rawSnapshot.turn_no || previous.turnNo || 1;
  state.backend.status = rawSnapshot.status || state.backend.status;

  // Use server-provided turn_started_at for accurate countdown on reconnect.
  // Only reset to Date.now() when the turn actually advances (no server timestamp).
  if (rawSnapshot.turn_started_at) {
    state.lastSnapshotAt = new Date(rawSnapshot.turn_started_at).getTime();
  } else if (state.turnNo !== previous.turnNo || !state.lastSnapshotAt) {
    state.lastSnapshotAt = Date.now();
  }

  if (state.backend.status === "FINISHED") {
    state.phase = "finished";
  } else if (state.backend.status === "RUNNING") {
    state.phase = "playing";
  }

  const winner = winnerFromPlayers(state.players);
  const newlyFinished = previous.status !== "FINISHED" && state.backend.status === "FINISHED";

  return {
    source,
    previous,
    winner,
    newlyFinished,
    turnAdvanced: state.turnNo !== previous.turnNo || (getActivePlayer()?.id || null) !== previous.currentPlayerId,
  };
}

async function refreshSnapshot(gameId, fromTurnNo = null) {
  const snapshot = await getWatcherSnapshot(gameId, fromTurnNo);
  return applyBackendSnapshot(snapshot, "snapshot");
}

function connectWatcherStream(gameId, fromTurnNo = 0) {
  closeWatcherStream();

  let url;
  try {
    url = toWebSocketUrl(BACKEND.watcher, gameId, fromTurnNo);
  } catch (_error) {
    pushLog("Watcher URL is invalid.");
    return;
  }

  const socket = new WebSocket(url);
  state.backend.watcherSocket = socket;

  socket.onopen = () => {
    state.backend.watcherLastEventAt = Date.now();
    startWatcherHealthTimer(gameId, socket);
    pushLog(`Watcher stream connected for game ${gameId.slice(0, 8)}.`);
    render();
  };

  socket.onmessage = (event) => {
    state.backend.watcherLastEventAt = Date.now();
    let payload;
    try {
      payload = JSON.parse(event.data);
    } catch (_error) {
      return;
    }

    if (payload.event_type === "HEARTBEAT" || payload.event_type === "CONNECTED") {
      return;
    }

    if (payload.event_type === "ERROR") {
      pushLog(`Watcher error: ${payload.error || "unknown error"}`);
      render();
      return;
    }

    if (payload.event_type === "TIMEOUT") {
      const timedOutPlayerId = payload.player_id || null;
      startTimeoutAnimation(
        timedOutPlayerId,
        payload.turn_no || state.turnNo,
        payload.round_no || state.round
      );

      if (timedOutPlayerId) {
        pushLog(
          `${displayName(timedOutPlayerId)} timeout on turn ${payload.turn_no || state.turnNo}.`
        );
      } else {
        pushLog(`Timeout event on turn ${payload.turn_no || state.turnNo}.`);
      }

      if (!payload.snapshot) {
        render();
        return;
      }
    }

    if (payload.event_type === "SPEAK") {
      const speakerId = payload.player_id || "?";
      const spoken = typeof payload.speak_text === "string" ? payload.speak_text : "";
      if (spoken.trim().length > 0) {
        pushLog(`${displayName(speakerId)} speak: ${spoken}`);
      } else {
        pushLog(`${displayName(speakerId)} speak.`);
      }
      if (payload.player_id && spoken.trim().length > 0) {
        startSpeakAnimation(payload.player_id, spoken);
      }
      if (!payload.snapshot) {
        render();
        return;
      }
    }

    if (payload.event_type === "SHOOT") {
      const shooterId = payload.player_id || null;
      const shootDirection = payload.direction || null;
      const commandId = payload.command_id || null;

      // Skip if we already animated this shot locally (human player's own shot).
      if (commandId && commandId === state.lastLocalShotCommandId) {
        state.lastLocalShotCommandId = null;
      } else if (shooterId && shootDirection) {
        // Capture pre-shot state for animation before applying the snapshot.
        const beforeSnapshot = localSnapshot();
        playServerShotAnimation(beforeSnapshot, shooterId, shootDirection);
      }

      if (!payload.snapshot) {
        render();
        return;
      }
    }

    if (payload.event_type === "GAME_FINISHED" && !payload.snapshot) {
      const winner = winnerFromPlayers(state.players);
      state.backend.status = "FINISHED";
      state.phase = "finished";
      if (winner) {
        announceWinnerIfNeeded({ winner, newlyFinished: true }, null, true);
        startFinishCelebration(winner.id, true);
      }
      closeWatcherStream();
      render();
      return;
    }

    if (!payload.snapshot) {
      return;
    }

    try {
      const result = applyBackendSnapshot(payload.snapshot, "stream");
      if (payload.event_type === "GAME_STARTED") {
        pushLog("Game started event received.");
      } else if (payload.event_type === "GAME_FINISHED") {
        announceWinnerIfNeeded(result, null, true);
        if (result.winner) {
          startFinishCelebration(result.winner.id, true);
        }
      } else if (result.turnAdvanced) {
        const activePlayer = getActivePlayer();
        pushLog(
          `Turn synced from stream: ${displayName(activePlayer ? activePlayer.id : "?")} is active.`
        );
      }
      if (payload.event_type !== "GAME_FINISHED") {
        announceWinnerIfNeeded(result);
        if (result.newlyFinished && result.winner) {
          startFinishCelebration(result.winner.id);
        }
      }

      if (state.phase === "finished") {
        closeWatcherStream();
      }

      render();
      focusActiveCommandInput();
    } catch (error) {
      pushLog(`Stream snapshot parse failed: ${error.message}`);
      render();
    }
  };

  socket.onclose = () => {
    if (state.backend.watcherSocket === socket) {
      state.backend.watcherSocket = null;
    }

    if (!state.backend.gameId || state.phase === "idle" || state.phase === "finished") {
      return;
    }
    scheduleWatcherReconnect(gameId, "closed");
  };

  socket.onerror = () => {
    if (state.backend.watcherSocket === socket) {
      state.backend.watcherSocket = null;
    }
    scheduleWatcherReconnect(gameId, "error");
  };
}

function pushLog(message) {
  state.logs.unshift(message);
  state.logs = state.logs.slice(0, 14);
}

function phaseFromBackendStatus(status) {
  if (status === "RUNNING") {
    return "playing";
  }
  if (status === "FINISHED") {
    return "finished";
  }
  return "idle";
}

function winnerFromPlayers(players) {
  const alive = (players || []).filter((player) => player.alive);
  return alive.length === 1 ? alive[0] : null;
}

async function createNewGame() {
  if (state.pendingCommand || state.phase === "loading") {
    return;
  }

  state.phase = "loading";
  state.pendingCommand = true;
  state.logs = [];
  resetVisualState();
  closeWatcherStream();

  const selectedNumPlayers = parseInt(numPlayersSelect.value, 10) || DEFAULT_NUM_PLAYERS;
  state.numPlayers = Math.max(1, Math.min(4, selectedNumPlayers));
  buildCommandControls();
  clearCommandInputs();
  render();

  try {
    const created = await createBackendGame(state.numPlayers);
    state.backend.gameId = created.game_id;
    state.backend.status = created.status || "CREATED";
    state.backend.timeoutSeconds = created.turn_timeout_seconds || DEFAULT_TIMEOUT_SECONDS;
    state.turnNo = created.turn_no || 1;
    state.round = created.round_no || 1;
    state.phase = phaseFromBackendStatus(state.backend.status);
    state.map = cloneMapFromTemplate();
    state.players = createPlayers(state.numPlayers);
    if (Array.isArray(created.players)) {
      const idByName = new Map(
        created.players
          .filter((player) => player && typeof player.player_name === "string" && typeof player.player_id === "string")
          .map((player) => [player.player_name, player.player_id])
      );
      state.players = state.players.map((player) => ({
        ...player,
        id: idByName.get(player.name) || player.id,
      }));
    }
    state.currentTurnIndex = turnIndexForPlayer(created.current_player_id || "");
    gameIdInput.value = created.game_id;

    state.backend.connected = true;
    await refreshSnapshot(created.game_id);
    pushLog(`New game ${created.game_id.slice(0, 8)} created.`);
    pushLog("Press Start to begin this game.");
  } catch (error) {
    closeWatcherStream();
    state.phase = "idle";
    state.backend.connected = false;
    state.backend.gameId = null;
    state.backend.status = "ERROR";
    pushLog(`Backend start failed: ${error.message}`);
    pushLog("Check backend services and retry Start.");
  } finally {
    state.pendingCommand = false;
    render();
    focusActiveCommandInput();
  }
}

async function startCurrentGame() {
  if (state.pendingCommand || state.phase === "loading") {
    return;
  }

  const requestedGameId = (state.backend.gameId || gameIdInput.value || "").trim();
  if (!requestedGameId) {
    pushLog("Create a game with New, or paste a Game ID, before Start.");
    render();
    return;
  }

  state.pendingCommand = true;
  state.phase = "loading";
  render();

  try {
    const game = await getBackendGame(requestedGameId);

    state.backend.connected = true;
    state.backend.gameId = game.game_id;
    state.backend.status = game.status || "CREATED";
    state.backend.timeoutSeconds = game.turn_timeout_seconds || DEFAULT_TIMEOUT_SECONDS;
    gameIdInput.value = game.game_id;

    if (state.backend.status === "FINISHED") {
      pushLog(`Game ${game.game_id.slice(0, 8)} is already finished.`);
    } else if (state.backend.status === "RUNNING") {
      pushLog(`Game ${game.game_id.slice(0, 8)} is already running.`);
    } else {
      await startBackendGame(game.game_id);
      pushLog(`Game ${game.game_id.slice(0, 8)} started via backend.`);
      pushLog(`Turn order: ${getPlayerOrder().join(" -> ")}.`);
    }

    const sync = await refreshSnapshot(game.game_id);
    state.phase = phaseFromBackendStatus(state.backend.status);
    if (state.backend.status === "FINISHED" && sync.winner) {
      announceWinnerIfNeeded(sync, null, true);
      startFinishCelebration(sync.winner.id, true);
    } else {
      announceWinnerIfNeeded(sync);
    }

    if (state.phase === "playing") {
      connectWatcherStream(game.game_id, Math.max(0, state.turnNo));
    } else {
      closeWatcherStream();
    }
  } catch (error) {
    state.phase = phaseFromBackendStatus(state.backend.status);
    pushLog(`Start failed: ${error.message}`);
  } finally {
    state.pendingCommand = false;
    render();
    focusActiveCommandInput();
  }
}

async function connectToExistingGame() {
  if (state.pendingCommand || state.phase === "loading") {
    return;
  }

  const requestedGameId = gameIdInput.value.trim();
  if (!requestedGameId) {
    pushLog("Enter a Game ID to connect.");
    render();
    return;
  }

  state.pendingCommand = true;
  state.phase = "loading";
  render();

  try {
    const game = await getBackendGame(requestedGameId);
    closeWatcherStream();
    resetVisualState();

    state.backend.connected = true;
    state.backend.gameId = game.game_id;
    state.backend.status = game.status || "CREATED";
    state.backend.timeoutSeconds = game.turn_timeout_seconds || DEFAULT_TIMEOUT_SECONDS;

    const sync = await refreshSnapshot(game.game_id);
    state.phase = phaseFromBackendStatus(state.backend.status);
    gameIdInput.value = game.game_id;
    if (state.backend.status === "FINISHED" && sync.winner) {
      announceWinnerIfNeeded(sync, null, true);
      startFinishCelebration(sync.winner.id, true);
    } else {
      announceWinnerIfNeeded(sync);
    }

    if (state.phase === "playing") {
      connectWatcherStream(game.game_id, Math.max(0, state.turnNo));
    }

    pushLog(`Connected to game ${game.game_id.slice(0, 8)}.`);
    if (state.backend.status !== "RUNNING") {
      pushLog(`Game status is ${state.backend.status}.`);
    }
  } catch (error) {
    state.phase = phaseFromBackendStatus(state.backend.status);
    pushLog(`Connect failed: ${error.message}`);
  } finally {
    state.pendingCommand = false;
    render();
    focusActiveCommandInput();
  }
}

function getPlayerById(id) {
  return state.players.find((p) => p.id === id);
}

function getActivePlayer() {
  const order = getPlayerOrder();
  const activeName = order[state.currentTurnIndex];
  return state.players.find((player) => player.name === activeName);
}

function alivePlayers() {
  return state.players.filter((p) => p.alive);
}

function getCellSize() {
  const cols = mapCols();
  const rows = mapRows();
  return {
    tileW: canvas.width / cols,
    tileH: canvas.height / rows,
  };
}

function getCellCenter(row, col) {
  const { tileW, tileH } = getCellSize();
  return {
    x: col * tileW + tileW / 2,
    y: row * tileH + tileH / 2,
  };
}

function getPlayerRenderMetrics(player) {
  const { tileW, tileH } = getCellSize();
  const centerX = player.col * tileW + tileW / 2;
  const centerY = player.row * tileH + tileH / 2;
  const radius = Math.min(tileW, tileH) * 0.28;
  const knightY = centerY + Math.min(tileH, tileW) * 0.08;
  return { centerX, centerY, knightY, radius };
}

function colorWithAlpha(hex, alpha) {
  const safeHex = hex.replace("#", "");
  const r = parseInt(safeHex.slice(0, 2), 16);
  const g = parseInt(safeHex.slice(2, 4), 16);
  const b = parseInt(safeHex.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function edgePointForDirection(startX, startY, direction) {
  if (direction === "up") {
    return { x: startX, y: 2 };
  }
  if (direction === "down") {
    return { x: startX, y: canvas.height - 2 };
  }
  if (direction === "left") {
    return { x: 2, y: startY };
  }
  return { x: canvas.width - 2, y: startY };
}

function getGunMuzzlePoint(player, direction) {
  const metrics = getPlayerRenderMetrics(player);
  const gunSize = metrics.radius * 0.95;
  const sideShift = direction === player.shield ? 0.72 : 0;
  const gunBase = attachmentPosition(
    metrics.centerX,
    metrics.knightY,
    direction,
    metrics.radius * 1.62,
    metrics.radius,
    sideShift
  );
  const dir = directionVector(direction);
  return {
    x: gunBase.x + dir.dc * gunSize * 1.7,
    y: gunBase.y + dir.dr * gunSize * 1.7,
  };
}

function addLaserBeam(player, direction, endPoint) {
  const startPoint = getGunMuzzlePoint(player, direction);
  state.laserBeams.push({
    startX: startPoint.x,
    startY: startPoint.y,
    endX: endPoint.x,
    endY: endPoint.y,
    color: glowColor(player.name),
    startedAt: performance.now(),
    duration: LASER_BEAM_DURATION_MS,
  });
}

/** Add a laser beam between two arbitrary pixel coordinates.
 *  Optional: delay (ms before beam appears), beamWidth multiplier. */
function addLaserBeamRaw(sx, sy, ex, ey, color, delay, beamWidth) {
  state.laserBeams.push({
    startX: sx,
    startY: sy,
    endX: ex,
    endY: ey,
    color,
    startedAt: performance.now(),
    duration: LASER_BEAM_DURATION_MS,
    delay: delay || 0,
    beamWidth: beamWidth || 1,
  });
}

function addHitFlash(row, col) {
  state.hitFlashes.push({
    row,
    col,
    startedAt: performance.now(),
    duration: HIT_FLASH_DURATION_MS,
  });
}

function addHitShake(row, col, amplitude = HIT_SHAKE_AMPLITUDE) {
  state.hitShakes.push({
    row,
    col,
    amplitude,
    startedAt: performance.now(),
    duration: HIT_SHAKE_DURATION_MS,
  });
}

function randomFinishParticle() {
  const colors = ["#ff5f8f", "#ffd27a", "#62d2ff", "#78f47f", "#ffffff"];
  return {
    x: Math.random(),
    y: Math.random() * 0.4,
    vx: (Math.random() - 0.5) * 0.7,
    vy: 0.7 + Math.random() * 0.7,
    size: 3 + Math.random() * 6,
    rotation: Math.random() * Math.PI * 2,
    spin: (Math.random() - 0.5) * 5.4,
    color: colors[Math.floor(Math.random() * colors.length)],
  };
}

function startFinishCelebration(winnerId, force = false) {
  if (!winnerId) {
    return;
  }

  const key = `${state.backend.gameId || "no-game"}:${state.turnNo}:${winnerId}`;
  if (!force && state.finishFx && state.finishFx.key === key) {
    return;
  }

  state.finishFx = {
    key,
    winnerId,
    startedAt: performance.now(),
    duration: FINISH_ANIMATION_DURATION_MS,
    particles: Array.from({ length: 56 }, () => randomFinishParticle()),
  };

  ensureLaserAnimationLoop();
}

function startTimeoutAnimation(playerId, turnNo, roundNo) {
  const startedAt = performance.now();
  state.timeoutFx.push({
    playerId: playerId || null,
    turnNo,
    roundNo,
    startedAt,
    duration: TIMEOUT_ANIMATION_DURATION_MS,
  });
  state.timeoutFx = state.timeoutFx.slice(-6);
  ensureLaserAnimationLoop();
}

function startSpeakAnimation(playerId, speakText) {
  if (!playerId || !speakText || !speakText.trim()) {
    return;
  }

  state.speakFx.push({
    playerId,
    text: speakText.trim().slice(0, 140),
    startedAt: performance.now(),
    duration: SPEAK_ANIMATION_DURATION_MS,
  });
  state.speakFx = state.speakFx.slice(-6);
  ensureLaserAnimationLoop();
}

function pruneLaserBeams(now) {
  state.laserBeams = state.laserBeams.filter((beam) => now - beam.startedAt < beam.duration + (beam.delay || 0));
}

function pruneHitFlashes(now) {
  state.hitFlashes = state.hitFlashes.filter((flash) => now - flash.startedAt < flash.duration);
}

function pruneHitShakes(now) {
  state.hitShakes = state.hitShakes.filter((shake) => now - shake.startedAt < shake.duration);
}

function pruneTimeoutFx(now) {
  state.timeoutFx = state.timeoutFx.filter((fx) => now - fx.startedAt < fx.duration);
}

function pruneSpeakFx(now) {
  state.speakFx = state.speakFx.filter((fx) => now - fx.startedAt < fx.duration);
}

function pruneFinishFx(now) {
  if (!state.finishFx) {
    return;
  }
  if (now - state.finishFx.startedAt >= state.finishFx.duration) {
    state.finishFx = null;
  }
}

function stopLaserAnimationLoop() {
  if (state.laserAnimationFrame !== null) {
    cancelAnimationFrame(state.laserAnimationFrame);
    state.laserAnimationFrame = null;
  }
}

function ensureLaserAnimationLoop() {
  if (state.laserAnimationFrame !== null) {
    return;
  }

  const tick = () => {
    const now = performance.now();
    pruneLaserBeams(now);
    pruneSweepGlows(now);
    pruneSaberFx(now);
    pruneHitFlashes(now);
    pruneHitShakes(now);
    pruneTimeoutFx(now);
    pruneSpeakFx(now);
    pruneFinishFx(now);
    render();

    if (
      state.laserBeams.length > 0 ||
      state.sweepGlows.length > 0 ||
      state.saberFx.length > 0 ||
      state.hitFlashes.length > 0 ||
      state.hitShakes.length > 0 ||
      state.timeoutFx.length > 0 ||
      state.speakFx.length > 0 ||
      state.finishFx
    ) {
      state.laserAnimationFrame = requestAnimationFrame(tick);
      return;
    }

    state.laserAnimationFrame = null;
  };

  state.laserAnimationFrame = requestAnimationFrame(tick);
}

function inBoundsSnapshot(snapshot, row, col) {
  return row >= 0 && row < snapshot.map.length && col >= 0 && col < snapshot.map[0].length;
}

function blockTileSnapshot(snapshot, row, col) {
  if (!inBoundsSnapshot(snapshot, row, col)) {
    return null;
  }
  const tile = snapshot.map[row][col];
  return tile && tile.type === "block" ? tile : null;
}

function playerAtSnapshot(snapshot, row, col) {
  return snapshot.players.find((player) => player.alive && player.row === row && player.col === col) || null;
}

/**
 * Sweep a laser from (startRow, startCol) in sweepDir until hitting a wall,
 * player, or map edge.  Returns { kind, row, col, direction, blockedByShield, cells }.
 * `cells` is every cell the beam passes through (for glow animation).
 */
function sweepLaserSnapshot(snapshot, startRow, startCol, sweepDir) {
  const d = DIRECTION[sweepDir];
  let row = startRow + d.dr;
  let col = startCol + d.dc;
  const cells = [];

  while (inBoundsSnapshot(snapshot, row, col)) {
    cells.push({ row, col });

    if (blockTileSnapshot(snapshot, row, col)) {
      return { kind: "block", row, col, direction: sweepDir, cells };
    }

    const target = playerAtSnapshot(snapshot, row, col);
    if (target) {
      return {
        kind: "player",
        row,
        col,
        direction: sweepDir,
        blockedByShield: target.shield === OPPOSITE[sweepDir],
        cells,
      };
    }

    row += d.dr;
    col += d.dc;
  }

  return { kind: "miss", direction: sweepDir, cells };
}

/**
 * New laser-sweep shoot: the laser enters the adjacent cell in the shoot
 * direction, then sweeps both perpendicular directions from that entry cell.
 * Returns { fired, shooter, entryRow, entryCol, sweeps[] }.
 */
function simulateShotOutcome(snapshotBefore, shooterId, direction) {
  const shooter = snapshotBefore.players.find((player) => player.id === shooterId && player.alive);
  if (!shooter) {
    return { fired: false };
  }

  if (direction === shooter.shield) {
    return { fired: false };
  }

  const delta = DIRECTION[direction];
  const entryRow = shooter.row + delta.dr;
  const entryCol = shooter.col + delta.dc;

  // Entry cell must be in bounds.
  if (!inBoundsSnapshot(snapshotBefore, entryRow, entryCol)) {
    return { fired: false };
  }

  // Entry cell must be empty (no wall, no player).
  if (blockTileSnapshot(snapshotBefore, entryRow, entryCol)) {
    return { fired: false };
  }
  if (playerAtSnapshot(snapshotBefore, entryRow, entryCol)) {
    return { fired: false };
  }

  const perps = PERPENDICULAR[direction];
  const sweep1 = sweepLaserSnapshot(snapshotBefore, entryRow, entryCol, perps[0]);
  const sweep2 = sweepLaserSnapshot(snapshotBefore, entryRow, entryCol, perps[1]);

  return {
    fired: true,
    shooter,
    entryRow,
    entryCol,
    sweeps: [sweep1, sweep2],
  };
}

function playServerShotAnimation(snapshotBefore, shooterId, direction) {
  const outcome = simulateShotOutcome(snapshotBefore, shooterId, direction);
  if (!outcome.fired) {
    return;
  }

  const shooter = { ...outcome.shooter, aim: direction };
  const entryPoint = getCellCenter(outcome.entryRow, outcome.entryCol);
  const color = glowColor(shooter.name);
  const t = performance.now();

  // 0. Move the saber to the entry cell for the duration of the shot animation.
  //    Orient it perpendicular to the shoot direction (the sweep axis).
  const perpDir = PERPENDICULAR[direction][0]; // e.g. "up" for shooting right
  state.saberFx.push({
    playerId: shooter.name,
    entryRow: outcome.entryRow,
    entryCol: outcome.entryCol,
    perpDirection: perpDir,
    startedAt: t,
    duration: SWEEP_BEAM_DELAY_MS + SWEEP_GLOW_DURATION_MS + 200,
  });

  // 1. Short beam from gun muzzle → entry cell (instant).
  addLaserBeam(shooter, direction, entryPoint);

  // 2. Glow the entry cell itself.
  state.sweepGlows.push({
    row: outcome.entryRow,
    col: outcome.entryCol,
    color,
    startedAt: t,
    delay: 0,
    duration: SWEEP_GLOW_DURATION_MS,
  });

  // 3. Perpendicular sweep beams — delayed, wider, with cell-by-cell glow trail.
  for (const sweep of outcome.sweeps) {
    let endPoint;
    if (sweep.kind === "block" || sweep.kind === "player") {
      endPoint = getCellCenter(sweep.row, sweep.col);
    } else {
      endPoint = edgePointForDirection(entryPoint.x, entryPoint.y, sweep.direction);
    }

    // Wide perpendicular beam, delayed so it expands after the initial shot lands.
    addLaserBeamRaw(
      entryPoint.x, entryPoint.y,
      endPoint.x, endPoint.y,
      color,
      SWEEP_BEAM_DELAY_MS,
      3.5,
    );

    // Cell-by-cell glow trail with staggered delays for a "scanning" effect.
    const cellCount = sweep.cells ? sweep.cells.length : 0;
    for (let i = 0; i < cellCount; i++) {
      const cell = sweep.cells[i];
      state.sweepGlows.push({
        row: cell.row,
        col: cell.col,
        color,
        startedAt: t,
        delay: SWEEP_BEAM_DELAY_MS + i * 35,
        duration: SWEEP_GLOW_DURATION_MS,
      });
    }

    // Hit effects on the cell where the sweep stops.
    if (sweep.kind === "block") {
      addHitFlash(sweep.row, sweep.col);
      addHitShake(sweep.row, sweep.col);
      playHitSound("block");
    } else if (sweep.kind === "player") {
      addHitFlash(sweep.row, sweep.col);
      if (!sweep.blockedByShield) {
        addHitShake(sweep.row, sweep.col);
        playHitSound("player");
      }
    }
  }

  ensureLaserAnimationLoop();
}

function announceWinnerIfNeeded(result, killerPlayerId = null, force = false) {
  if (!result || !result.winner) {
    return;
  }
  if (!force && !result.newlyFinished) {
    return;
  }

  const announceKey = `${state.backend.gameId || "no-game"}:${state.turnNo}`;
  if (state.lastFinishAnnounceKey === announceKey) {
    return;
  }
  state.lastFinishAnnounceKey = announceKey;

  const winnerName = displayName(result.winner.id);
  if (killerPlayerId && killerPlayerId === result.winner.id) {
    pushLog(`${winnerName} eliminated the last opponent.`);
  } else {
    pushLog("Last opponent eliminated.");
  }
  pushLog(`Congratulations ${winnerName} win the game!`);
}

async function executeActiveCommand() {
  if (state.phase !== "playing") {
    return;
  }

  if (state.pendingCommand) {
    return;
  }

  const activePlayer = getActivePlayer();
  if (!activePlayer || !activePlayer.alive) {
    return;
  }

  const control = commandControls[activePlayer.name];
  if (!control) {
    return;
  }
  const action = control.action.value;
  const direction = action === "speak" ? null : control.direction.value;
  const speakText = control.speakText.value.trim();
  const beforeSnapshot = localSnapshot();

  if (!state.backend.gameId) {
    pushLog("No backend game id. Press Start.");
    render();
    return;
  }

  state.pendingCommand = true;
  render();

  try {
    if (action === "speak" && !speakText) {
      pushLog(`${displayName(activePlayer.id)} speak text cannot be empty.`);
      return;
    }

    const payload = {
      command_id: nextCommandId(),
      player_id: activePlayer.id,
      command_type: action,
      direction,
      speak_text: action === "speak" ? speakText : null,
      turn_no: state.turnNo,
      client_sent_at: new Date().toISOString(),
    };

    const submitResult = await submitBackendCommand(state.backend.gameId, payload);
    if (!submitResult || submitResult.accepted !== true) {
      pushLog(`${displayName(activePlayer.id)} command was not accepted by backend.`);
    } else {
      pushLog(`${displayName(activePlayer.id)} command accepted. Waiting for stream sync.`);
      if (action === "shoot" && direction !== activePlayer.shield) {
        state.lastLocalShotCommandId = payload.command_id;
        playServerShotAnimation(beforeSnapshot, activePlayer.id, direction);
      }
    }

    render();
    focusActiveCommandInput();
  } catch (error) {
    pushLog(`Command failed: ${error.message}`);
    render();
  } finally {
    state.pendingCommand = false;
    control.action.value = "move";
    updateCommandControlMode(control);
    render();
  }
}

function drawBoard(now) {
  const width = canvas.width;
  const height = canvas.height;
  const cols = mapCols();
  const rows = mapRows();
  const tileW = width / cols;
  const tileH = height / rows;

  ctx.clearRect(0, 0, width, height);

  for (let r = 0; r < rows; r += 1) {
    for (let c = 0; c < cols; c += 1) {
      const x = c * tileW;
      const y = r * tileH;
      const tile = state.map[r]?.[c] || { type: "empty" };

      ctx.fillStyle = "#f2d3a2";
      ctx.fillRect(x, y, tileW, tileH);

      if (tile.type === "block") {
        const shake = getCellShakeOffset(r, c, now);
        if (tile.strength === -1) {
          ctx.fillStyle = "#4f3f33";
        } else if (tile.strength === 1) {
          ctx.fillStyle = "#9f5f34";
        } else {
          ctx.fillStyle = "#7d4a2a";
        }
        ctx.fillRect(x + 4 + shake.dx, y + 4 + shake.dy, tileW - 8, tileH - 8);

        ctx.fillStyle = "#ffe8c8";
        ctx.font = '700 18px "Cabin", sans-serif';
        const value = tile.strength === -1 ? "∞" : String(tile.strength);
        ctx.fillText(value, x + tileW / 2 - 6 + shake.dx, y + tileH / 2 + 6 + shake.dy);
      }

      ctx.strokeStyle = "rgba(84, 46, 24, 0.28)";
      ctx.strokeRect(x, y, tileW, tileH);
    }
  }
}

function drawLaserBeams(now) {
  if (state.laserBeams.length === 0) {
    return;
  }

  ctx.save();
  ctx.lineCap = "round";

  for (const beam of state.laserBeams) {
    const delay = beam.delay || 0;
    const elapsed = now - beam.startedAt - delay;
    if (elapsed < 0) continue; // not visible yet

    const progress = Math.min(1, elapsed / beam.duration);
    const pulse = 0.5 + 0.5 * Math.sin(elapsed * 0.045);
    const alpha = Math.max(0, 1 - progress);
    const bw = beam.beamWidth || 1;
    const baseWidth = 3 + (1 - progress) * 4;
    const width = baseWidth * bw;

    // Grow-in: beam extends from start → end very quickly (first 60ms).
    const growT = Math.min(1, elapsed / 60);
    const curEndX = beam.startX + (beam.endX - beam.startX) * growT;
    const curEndY = beam.startY + (beam.endY - beam.startY) * growT;

    // Outer glow for wide sweep beams.
    if (bw > 1.5) {
      ctx.strokeStyle = colorWithAlpha(beam.color, 0.25 * alpha);
      ctx.lineWidth = width * 2.5;
      ctx.beginPath();
      ctx.moveTo(beam.startX, beam.startY);
      ctx.lineTo(curEndX, curEndY);
      ctx.stroke();
    }

    // Main coloured beam.
    ctx.strokeStyle = colorWithAlpha(beam.color, 0.75 * alpha + 0.15 * pulse);
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(beam.startX, beam.startY);
    ctx.lineTo(curEndX, curEndY);
    ctx.stroke();

    // White hot core.
    ctx.strokeStyle = colorWithAlpha("#ffffff", 0.5 * alpha);
    ctx.lineWidth = Math.max(1.5, width * 0.4);
    ctx.beginPath();
    ctx.moveTo(beam.startX, beam.startY);
    ctx.lineTo(curEndX, curEndY);
    ctx.stroke();
  }

  ctx.restore();
}

function pruneSweepGlows(now) {
  state.sweepGlows = state.sweepGlows.filter((g) => now - g.startedAt < g.duration + (g.delay || 0));
}

function pruneSaberFx(now) {
  state.saberFx = state.saberFx.filter((fx) => now - fx.startedAt < fx.duration);
}

/** Draw translucent colored rectangles over every cell in the laser sweep path. */
function drawSweepGlows(now) {
  if (state.sweepGlows.length === 0) {
    return;
  }

  const { tileW, tileH } = getCellSize();
  ctx.save();

  for (const g of state.sweepGlows) {
    const delay = g.delay || 0;
    const elapsed = now - g.startedAt - delay;
    if (elapsed < 0) continue;

    const progress = Math.min(1, elapsed / g.duration);
    // Fade in quickly, fade out slowly.
    const fadeIn = Math.min(1, elapsed / 80);
    const fadeOut = Math.max(0, 1 - progress);
    const alpha = fadeIn * fadeOut * 0.45;
    const pulse = 0.8 + 0.2 * Math.sin(elapsed * 0.06);

    const x = g.col * tileW;
    const y = g.row * tileH;
    const inset = 1;

    // Colored cell fill.
    ctx.fillStyle = colorWithAlpha(g.color, alpha * pulse);
    ctx.fillRect(x + inset, y + inset, tileW - inset * 2, tileH - inset * 2);

    // Bright border.
    ctx.strokeStyle = colorWithAlpha(g.color, Math.min(1, alpha * pulse * 2));
    ctx.lineWidth = 2.5;
    ctx.strokeRect(x + inset, y + inset, tileW - inset * 2, tileH - inset * 2);
  }

  ctx.restore();
}

function drawHitFlashes(now) {
  if (state.hitFlashes.length === 0) {
    return;
  }

  const { tileW, tileH } = getCellSize();
  ctx.save();

  for (const flash of state.hitFlashes) {
    const elapsed = now - flash.startedAt;
    const progress = Math.min(1, elapsed / flash.duration);
    const pulse = 0.45 + 0.55 * Math.abs(Math.sin(elapsed * 0.03));
    const alpha = Math.max(0.1, (1 - progress) * pulse);
    const lineW = 2 + (1 - progress) * 2;
    const x = flash.col * tileW + 1.5;
    const y = flash.row * tileH + 1.5;
    const w = tileW - 3;
    const h = tileH - 3;

    ctx.strokeStyle = `rgba(235, 38, 38, ${alpha})`;
    ctx.lineWidth = lineW;
    ctx.strokeRect(x, y, w, h);
  }

  ctx.restore();
}

function drawRoundedRectPath(x, y, width, height, radius) {
  const r = Math.max(0, Math.min(radius, width / 2, height / 2));
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + width - r, y);
  ctx.quadraticCurveTo(x + width, y, x + width, y + r);
  ctx.lineTo(x + width, y + height - r);
  ctx.quadraticCurveTo(x + width, y + height, x + width - r, y + height);
  ctx.lineTo(x + r, y + height);
  ctx.quadraticCurveTo(x, y + height, x, y + height - r);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
  ctx.closePath();
}

function drawSpeakAnimation(now) {
  if (state.speakFx.length === 0) {
    return;
  }

  const active = state.speakFx[state.speakFx.length - 1];
  const target = getPlayerById(active.playerId);
  if (!target) {
    return;
  }

  const elapsed = now - active.startedAt;
  const progress = Math.min(1, elapsed / active.duration);
  const fadeIn = Math.min(1, progress / 0.12);
  const fadeOut = Math.min(1, (1 - progress) / 0.22);
  const alpha = Math.max(0, Math.min(fadeIn, fadeOut));
  const bob = Math.sin(elapsed * 0.012) * 3;

  const center = getCellCenter(target.row, target.col);
  const bubbleText = `${displayName(active.playerId)}: ${active.text}`;
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.font = '700 15px "Cabin", sans-serif';
  const textWidth = ctx.measureText(bubbleText).width;
  const bubbleWidth = Math.min(canvas.width * 0.82, Math.max(180, textWidth + 28));
  const bubbleHeight = 38;
  const x = Math.max(8, Math.min(canvas.width - bubbleWidth - 8, center.x - bubbleWidth / 2));
  const y = Math.max(8, center.y - 70 - bubbleHeight + bob);

  drawRoundedRectPath(x, y, bubbleWidth, bubbleHeight, 12);
  ctx.fillStyle = "rgba(255, 248, 232, 0.96)";
  ctx.fill();
  ctx.strokeStyle = "rgba(78, 43, 22, 0.9)";
  ctx.lineWidth = 2;
  ctx.stroke();

  const tailBaseX = Math.max(x + 18, Math.min(x + bubbleWidth - 18, center.x));
  const tailTipX = center.x;
  const tailTipY = center.y - 30 + bob;
  ctx.beginPath();
  ctx.moveTo(tailBaseX - 8, y + bubbleHeight);
  ctx.lineTo(tailBaseX + 8, y + bubbleHeight);
  ctx.lineTo(tailTipX, tailTipY);
  ctx.closePath();
  ctx.fillStyle = "rgba(255, 248, 232, 0.96)";
  ctx.fill();
  ctx.strokeStyle = "rgba(78, 43, 22, 0.9)";
  ctx.stroke();

  ctx.fillStyle = "rgba(45, 24, 13, 0.98)";
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  ctx.fillText(bubbleText, x + 14, y + bubbleHeight / 2);
  ctx.restore();
}

function drawTimeoutAnimation(now) {
  if (state.timeoutFx.length === 0) {
    return;
  }

  const active = state.timeoutFx[state.timeoutFx.length - 1];
  const elapsed = now - active.startedAt;
  const progress = Math.min(1, elapsed / active.duration);
  const alpha = Math.max(0, (1 - progress) * 0.95);
  const pulse = 0.5 + 0.5 * Math.sin(elapsed * 0.03);

  ctx.save();
  ctx.fillStyle = `rgba(227, 72, 40, ${0.18 * alpha})`;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const line = active.playerId
    ? `${displayName(active.playerId)} TIMEOUT`
    : `TURN ${active.turnNo} TIMEOUT`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = `rgba(255, 247, 223, ${alpha})`;
  ctx.strokeStyle = `rgba(84, 18, 10, ${alpha})`;
  ctx.lineWidth = 4;
  ctx.font = '700 30px "Cabin", sans-serif';
  ctx.strokeText(line, canvas.width / 2, canvas.height * 0.14);
  ctx.fillText(line, canvas.width / 2, canvas.height * 0.14);
  ctx.restore();

  const target = active.playerId ? getPlayerById(active.playerId) : null;
  if (!target) {
    return;
  }

  const center = getCellCenter(target.row, target.col);
  const maxRadius = Math.min(canvas.width, canvas.height) * 0.2;
  const radius = 12 + progress * maxRadius;

  ctx.save();
  ctx.strokeStyle = `rgba(255, 220, 140, ${0.85 * alpha})`;
  ctx.lineWidth = 4 + 2 * pulse;
  ctx.beginPath();
  ctx.arc(center.x, center.y, radius, 0, Math.PI * 2);
  ctx.stroke();

  ctx.strokeStyle = `rgba(255, 98, 64, ${0.8 * alpha})`;
  ctx.lineWidth = 2.5;
  ctx.beginPath();
  ctx.arc(center.x, center.y, radius * 0.68, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

function drawFinishCelebration(now) {
  if (!state.finishFx) {
    return;
  }

  const fx = state.finishFx;
  const elapsed = now - fx.startedAt;
  const progress = Math.min(1, elapsed / fx.duration);
  const fadeIn = Math.min(1, progress / 0.22);
  const fadeOut = Math.min(1, (1 - progress) / 0.32);
  const alpha = Math.min(fadeIn, fadeOut);

  ctx.save();

  ctx.fillStyle = `rgba(32, 12, 8, ${0.38 * alpha})`;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const winnerName = displayName(fx.winnerId);
  const pulse = 1 + Math.sin(elapsed * 0.012) * 0.03;
  ctx.translate(canvas.width / 2, canvas.height * 0.26);
  ctx.scale(pulse, pulse);
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = `rgba(255, 247, 223, ${alpha})`;
  ctx.strokeStyle = `rgba(86, 35, 16, ${alpha})`;
  ctx.lineWidth = 5;
  ctx.font = '700 36px "Cabin", sans-serif';
  const line = `Congratulations ${winnerName} win the game!`;
  ctx.strokeText(line, 0, 0);
  ctx.fillText(line, 0, 0);
  ctx.restore();

  ctx.save();
  for (const particle of fx.particles) {
    const t = elapsed / 1000;
    const px = ((particle.x + particle.vx * t) % 1 + 1) % 1;
    const py = (particle.y + particle.vy * t) % 1;
    const x = px * canvas.width;
    const y = py * canvas.height;
    const rotate = particle.rotation + particle.spin * t;
    const size = particle.size;

    ctx.translate(x, y);
    ctx.rotate(rotate);
    ctx.fillStyle = particle.color;
    ctx.globalAlpha = alpha;
    ctx.fillRect(-size * 0.5, -size * 0.5, size, size * 0.72);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
  }
  ctx.restore();
}

function getCellShakeOffset(row, col, now) {
  if (state.hitShakes.length === 0) {
    return { dx: 0, dy: 0 };
  }

  let dx = 0;
  let dy = 0;
  for (const shake of state.hitShakes) {
    if (shake.row !== row || shake.col !== col) {
      continue;
    }

    const elapsed = now - shake.startedAt;
    const progress = Math.min(1, elapsed / shake.duration);
    const strength = shake.amplitude * (1 - progress);
    const phase = elapsed * 0.18;
    dx += Math.sin(phase + row * 1.3 + col * 0.7) * strength;
    dy += Math.cos(phase * 0.9 + row * 0.5 + col * 1.1) * strength * 0.7;
  }

  return { dx, dy };
}

function directionAngle(direction) {
  if (direction === "up") return -Math.PI / 2;
  if (direction === "down") return Math.PI / 2;
  if (direction === "left") return Math.PI;
  return 0;
}

function directionVector(direction) {
  return DIRECTION[direction] || DIRECTION.right;
}

function attachmentPosition(x, y, direction, baseOffset, size, sideShift) {
  const dir = directionVector(direction);
  const px = -dir.dr;
  const py = dir.dc;
  return {
    x: x + dir.dc * baseOffset + px * sideShift * size,
    y: y + dir.dr * baseOffset + py * sideShift * size,
  };
}

const GLOW_COLORS = {
  A: "#ff5f8f",
  B: "#62d2ff",
  C: "#78f47f",
  D: "#ffd27a",
};

function glowColor(playerName) {
  return GLOW_COLORS[playerName] || "#ffd27a";
}

function drawKnightShield(x, y, direction, size, playerId) {
  const glow = glowColor(playerId);
  const cellHalf = size * 1.785;
  const margin = 2.5;
  const shieldThickness = Math.max(2, size * 0.13);
  const shieldLength = (cellHalf * 2 - margin * 2) * 0.95;
  const edgeX = cellHalf - margin - shieldThickness;

  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(directionAngle(direction));

  ctx.fillStyle = "rgba(10, 16, 34, 0.85)";
  ctx.strokeStyle = "#9eb6d7";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.rect(edgeX, -shieldLength / 2, shieldThickness, shieldLength);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();

  ctx.strokeStyle = glow;
  ctx.lineWidth = 1.2;
  ctx.beginPath();
  ctx.moveTo(edgeX + shieldThickness * 0.5, -shieldLength * 0.35);
  ctx.lineTo(edgeX + shieldThickness * 0.5, -shieldLength * 0.12);
  ctx.moveTo(edgeX + shieldThickness * 0.5, shieldLength * 0.12);
  ctx.lineTo(edgeX + shieldThickness * 0.5, shieldLength * 0.35);
  ctx.stroke();
  ctx.restore();
}

function drawLaserGun(x, y, direction, size, playerId, shieldDirection, centered) {
  let gx, gy;
  if (centered) {
    // When centered, draw the saber at (x, y) directly (e.g. entry cell center).
    gx = x;
    gy = y;
  } else {
    // Normally, offset from the player body via attachmentPosition.
    const sideShift = direction === shieldDirection ? 0.72 : 0;
    const pos = attachmentPosition(x, y, direction, size * 1.62, size, sideShift);
    gx = pos.x;
    gy = pos.y;
  }
  const glow = glowColor(playerId);
  const s = size * 0.95;

  ctx.save();
  ctx.translate(gx, gy);
  ctx.rotate(directionAngle(direction));
  ctx.lineCap = "round";

  // --- Forward blade (aim direction) ---
  // Outer glow.
  ctx.strokeStyle = colorWithAlpha("#ff2222", 0.25);
  ctx.lineWidth = s * 0.42;
  ctx.beginPath();
  ctx.moveTo(s * 0.22, 0);
  ctx.lineTo(s * 1.7, 0);
  ctx.stroke();
  // Main red blade.
  ctx.strokeStyle = "#e83030";
  ctx.lineWidth = s * 0.18;
  ctx.beginPath();
  ctx.moveTo(s * 0.22, 0);
  ctx.lineTo(s * 1.7, 0);
  ctx.stroke();
  // White-hot core.
  ctx.strokeStyle = "rgba(255,255,255,0.85)";
  ctx.lineWidth = s * 0.06;
  ctx.beginPath();
  ctx.moveTo(s * 0.24, 0);
  ctx.lineTo(s * 1.65, 0);
  ctx.stroke();

  // --- Backward blade (opposite direction) ---
  ctx.strokeStyle = colorWithAlpha("#ff2222", 0.25);
  ctx.lineWidth = s * 0.42;
  ctx.beginPath();
  ctx.moveTo(-s * 0.22, 0);
  ctx.lineTo(-s * 1.5, 0);
  ctx.stroke();
  ctx.strokeStyle = "#e83030";
  ctx.lineWidth = s * 0.18;
  ctx.beginPath();
  ctx.moveTo(-s * 0.22, 0);
  ctx.lineTo(-s * 1.5, 0);
  ctx.stroke();
  ctx.strokeStyle = "rgba(255,255,255,0.85)";
  ctx.lineWidth = s * 0.06;
  ctx.beginPath();
  ctx.moveTo(-s * 0.24, 0);
  ctx.lineTo(-s * 1.45, 0);
  ctx.stroke();

  // --- Center grip ---
  // Outer cylinder.
  ctx.fillStyle = "#6e6e7a";
  const gripW = s * 0.44;
  const gripH = s * 0.22;
  ctx.fillRect(-gripW / 2, -gripH / 2, gripW, gripH);
  // Dark band in the middle.
  ctx.fillStyle = "#2a2a32";
  ctx.fillRect(-s * 0.05, -gripH / 2 - 0.5, s * 0.1, gripH + 1);
  // Ridges.
  ctx.fillStyle = "#8a8a96";
  ctx.fillRect(-gripW / 2 + 1, -gripH / 2 + 1, 2, gripH - 2);
  ctx.fillRect(gripW / 2 - 3, -gripH / 2 + 1, 2, gripH - 2);

  ctx.restore();
}

function drawHpBadge(x, y, hp, playerId) {
  const label = `${playerId}  HP ${hp}`;
  ctx.save();
  ctx.font = '700 11px "Cabin", sans-serif';
  ctx.fillStyle = "#2e1a12";
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  ctx.fillText(label, x, y);
  ctx.restore();
}

function drawKnightSprite(player, x, y, radius) {
  const sprite = KNIGHT_SPRITES[player.spriteMode] || KNIGHT_SPRITES.idle;
  const facing = player.aim || "right";
  const width = radius * 2.35;
  const height = radius * 2.35;

  ctx.save();
  ctx.translate(x, y);
  if (facing === "left") {
    ctx.scale(-1, 1);
  }

  if (sprite && sprite.complete && sprite.naturalWidth > 0) {
    ctx.drawImage(sprite, -width / 2, -height * 0.82, width, height);
  } else {
    ctx.fillStyle = "#1f2433";
    ctx.beginPath();
    ctx.arc(0, -radius * 0.15, radius * 0.55, 0, Math.PI * 2);
    ctx.fill();
  }

  ctx.restore();
}

function drawPlayer(player, isActive, now) {
  if (!player.alive) {
    return;
  }

  const { tileW, tileH } = getCellSize();
  const shake = getCellShakeOffset(player.row, player.col, now);
  const x = player.col * tileW + tileW / 2 + shake.dx;
  const y = player.row * tileH + tileH / 2 + shake.dy;
  const knightY = y + Math.min(tileH, tileW) * 0.08;
  const radius = Math.min(tileW, tileH) * 0.28;

  if (isActive) {
    ctx.strokeStyle = "#e13333";
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.arc(x, knightY, radius + 9, 0, Math.PI * 2);
    ctx.stroke();
  }

  // Draw attachments first so the knight body remains readable.
  drawKnightShield(x, y, player.shield, radius, player.name);

  // If there is an active saber effect (shot animation), draw the saber at the
  // entry cell oriented perpendicular to the shot direction, instead of at the player.
  const activeSaber = state.saberFx.find(
    (fx) => fx.playerId === player.name && now - fx.startedAt < fx.duration
  );
  if (activeSaber) {
    const entryCenter = getCellCenter(activeSaber.entryRow, activeSaber.entryCol);
    const entryShake = getCellShakeOffset(activeSaber.entryRow, activeSaber.entryCol, now);
    const ex = entryCenter.x + entryShake.dx;
    const ey = entryCenter.y + entryShake.dy;
    drawLaserGun(ex, ey, activeSaber.perpDirection, radius, player.name, player.shield, true);
  } else {
    drawLaserGun(x, knightY, player.aim || "right", radius, player.name, player.shield);
  }

  drawKnightSprite(player, x, knightY, radius);

  drawHpBadge(x, y + radius * 1.22 - 6, player.hp, player.name);
}

function updateStatusPanel() {
  const activePlayer = getActivePlayer();

  if (state.phase === "idle") {
    if (state.backend.gameId) {
      if (state.backend.status === "CREATED" || state.backend.status === "IDLE") {
        statusLine.textContent = "Game is created. Press Start.";
      } else {
        statusLine.textContent = `Connected game is ${state.backend.status}.`;
      }
    } else {
      statusLine.textContent = "Press New.";
    }
  } else if (state.phase === "loading") {
    statusLine.textContent = "Loading game state from backend...";
  } else if (state.phase === "finished") {
    const winner = alivePlayers()[0];
    statusLine.textContent = winner
      ? `Congratulations ${displayName(winner.id)} win the game!`
      : "Match finished.";
  } else if (activePlayer) {
    const game = state.backend.gameId ? `Game ${state.backend.gameId.slice(0, 8)} | ` : "";
    statusLine.textContent = `${game}Active: ${displayName(activePlayer.id)}. Choose command.`;
  }

  const status = state.backend.status || "UNKNOWN";
  const elapsedSeconds = state.lastSnapshotAt
    ? (Date.now() - state.lastSnapshotAt) / 1000
    : 0;
  const timeoutLeft = Math.max(0, Math.ceil(state.backend.timeoutSeconds - elapsedSeconds));
  roundLine.textContent = `Round: ${state.round} | Turn: ${state.turnNo} | Status: ${status} | Timeout: ${timeoutLeft}s`;
  gameIdLine.textContent = `Game ID: ${state.backend.gameId || "-"}`;

  playersList.innerHTML = "";
  for (const player of state.players) {
    const li = document.createElement("li");
    li.className = player.alive ? "" : "dead";
    li.textContent = `${displayName(player.id)} | HP ${player.hp} | Shield ${player.shield} | ${player.alive ? "Alive" : "Dead"}`;
    playersList.appendChild(li);
  }

  logList.innerHTML = "";
  for (const entry of state.logs) {
    const li = document.createElement("li");
    li.textContent = entry;
    logList.appendChild(li);
  }
}

function focusActiveCommandInput() {
  if (state.phase !== "playing") {
    return;
  }

  const activePlayer = getActivePlayer();
  if (!activePlayer) {
    return;
  }

  const control = commandControls[activePlayer.name];
  if (!control || control.action.disabled) {
    return;
  }

  control.action.focus();
}

function updateControls() {
  const playable = state.phase === "playing";
  const activePlayer = getActivePlayer();
  startButton.disabled = state.pendingCommand;
  newButton.disabled = state.pendingCommand;
  connectButton.disabled = state.pendingCommand;
  gameIdInput.disabled = state.pendingCommand;

  for (const playerName of Object.keys(commandControls)) {
    const control = commandControls[playerName];
    if (!control) continue;
    const player = state.players.find((entry) => entry.name === playerName);
    const isMyTurn = activePlayer && activePlayer.name === playerName;
    const enabled = playable && player && player.alive && !state.pendingCommand && isMyTurn;
    control.action.disabled = !enabled;
    updateCommandControlMode(control);
    const isActive = enabled && activePlayer && activePlayer.name === playerName;
    control.action.classList.toggle("active-input", isActive);
    control.direction.classList.toggle("active-input", isActive && control.action.value !== "speak");
    control.speakText.classList.toggle("active-input", isActive && control.action.value === "speak");
  }
}

function render() {
  const now = performance.now();
  pruneLaserBeams(now);
  pruneSweepGlows(now);
  pruneHitFlashes(now);
  pruneHitShakes(now);
  pruneTimeoutFx(now);
  pruneSpeakFx(now);
  pruneFinishFx(now);
  drawBoard(now);
  drawSweepGlows(now);
  const activePlayer = getActivePlayer();
  for (const player of state.players) {
    drawPlayer(player, activePlayer && player.id === activePlayer.id, now);
  }
  drawLaserBeams(now);
  drawHitFlashes(now);
  drawSpeakAnimation(now);
  drawTimeoutAnimation(now);
  drawFinishCelebration(now);

  updateStatusPanel();
  updateControls();
}

startButton.addEventListener("click", () => {
  void startCurrentGame();
});
newButton.addEventListener("click", () => {
  void createNewGame();
});
connectButton.addEventListener("click", () => {
  void connectToExistingGame();
});
gameIdInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    void connectToExistingGame();
  }
});

// Command control event listeners are set up in buildCommandControls()

window.addEventListener("keydown", (event) => {
  const ARROW_TO_DIR = {
    ArrowUp: "up",
    ArrowLeft: "left",
    ArrowDown: "down",
    ArrowRight: "right",
    w: "up",
    a: "left",
    s: "down",
    d: "right",
  };

  const dir = ARROW_TO_DIR[event.key] || ARROW_TO_DIR[event.key.toLowerCase()];
  if (dir) {
    // Don't hijack arrows when typing in a text input.
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
      return;
    }
    event.preventDefault();
    const control = commandControls["A"];
    if (control && !control.direction.disabled) {
      control.direction.value = dir;
    }
    return;
  }

  const KEY_TO_ACTION = { j: "move", k: "shoot", l: "shield" };
  const action = KEY_TO_ACTION[event.key.toLowerCase()];
  if (action) {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
      return;
    }
    event.preventDefault();
    const control = commandControls["A"];
    if (control && !control.action.disabled) {
      control.action.value = action;
      updateCommandControlMode(control);
      render();
      if (action === "speak") {
        control.speakText.focus();
      } else if (state.phase === "playing") {
        const activePlayer = getActivePlayer();
        if (activePlayer && activePlayer.name === "A") {
          void executeActiveCommand();
        }
      }
    }
    return;
  }

  if (event.key === "Enter") {
    event.preventDefault();
    const control = commandControls["A"];
    if (control && !control.action.disabled && state.phase === "playing") {
      control.action.value = "speak";
      updateCommandControlMode(control);
      const activePlayer = getActivePlayer();
      if (activePlayer && activePlayer.name === "A") {
        void executeActiveCommand();
      }
    }
    return;
  }

});

window.addEventListener("beforeunload", () => {
  closeWatcherStream();
});

setInterval(() => {
  if (state.phase === "playing") {
    render();
  }
}, 400);

state.numPlayers = parseInt(numPlayersSelect.value, 10) || DEFAULT_NUM_PLAYERS;
buildCommandControls();
state.map = cloneMapFromTemplate();
state.players = createPlayers();
clearCommandInputs();
pushLog("Ready. Press New to create a game, then Start.");
render();

numPlayersSelect.addEventListener("change", () => {
  if (state.phase !== "idle" && state.phase !== "finished") return;
  state.numPlayers = parseInt(numPlayersSelect.value, 10) || DEFAULT_NUM_PLAYERS;
  buildCommandControls();
  state.players = createPlayers();
  clearCommandInputs();
  render();
});

const initialGameId = new URLSearchParams(window.location.search).get("gameId");
if (initialGameId) {
  gameIdInput.value = initialGameId;
  pushLog(`Auto-connect requested for game ${initialGameId.slice(0, 8)}.`);
  render();
  void connectToExistingGame();
}
