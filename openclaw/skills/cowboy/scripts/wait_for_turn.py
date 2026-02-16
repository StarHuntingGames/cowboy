#!/usr/bin/env python3
# Cowboy Skill — WebSocket Turn Waiter
# Copyright (C) 2026 StarHuntingGames
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

"""WebSocket-based turn waiter for the Cowboy game.

Supports three modes:

1. Single-shot (default):
   python3 wait_for_turn.py <server_url> <game_id> <player_id> [timeout]
   Waits for one turn, prints snapshot JSON to stdout, exits.

2. Loop mode (--loop):
   python3 wait_for_turn.py --loop <server_url> <game_id> <player_id>
   Persistent background process. Writes turn snapshots to files in
   /tmp/cowboy_<game_id>/. Use --wait-for to read them.

3. Wait-for mode (--wait-for):
   python3 wait_for_turn.py --wait-for <game_id> <turn_counter> [timeout]
   Blocks until my_turn_<N>.json or game_over.json appears, prints it.

Falls back to HTTP polling when the ``websockets`` package is not installed.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import time
from pathlib import Path
from urllib.parse import urlparse
from urllib.request import urlopen


def _output_dir(game_id: str) -> Path:
    """Return the output directory for a game."""
    return Path(f"/tmp/cowboy_{game_id}")


# ---------------------------------------------------------------------------
# Wait-for mode: block until a turn file or game_over file appears
# ---------------------------------------------------------------------------

def wait_for_file(game_id: str, n: int, timeout: float = 300) -> None:
    """Block until my_turn_N.json or game_over.json exists, then print it."""
    d = _output_dir(game_id)
    turn_file = d / f"my_turn_{n}.json"
    game_over = d / "game_over.json"
    pid_file = d / "pid"
    deadline = time.time() + timeout

    while time.time() < deadline:
        if game_over.exists():
            print(game_over.read_text())
            return
        if turn_file.exists():
            print(turn_file.read_text())
            return
        # Check if the loop process is still alive
        if pid_file.exists():
            try:
                pid = int(pid_file.read_text().strip())
                os.kill(pid, 0)
            except (OSError, ValueError):
                # Process died — check files one more time
                if game_over.exists():
                    print(game_over.read_text())
                    return
                if turn_file.exists():
                    print(turn_file.read_text())
                    return
                print(json.dumps({
                    "error": "listener_died",
                    "message": "The WebSocket listener process has exited",
                }))
                return
        time.sleep(0.3)

    print(json.dumps({"error": "timeout", "timeout_seconds": timeout}))


# ---------------------------------------------------------------------------
# HTTP polling fallback (no external dependencies)
# ---------------------------------------------------------------------------

def http_poll(
    server_url: str,
    game_id: str,
    player_id: str,
    timeout: float = 120,
) -> None:
    """Poll the snapshot endpoint until it is our turn or the game ends."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            url = f"{server_url}/v2/games/{game_id}/snapshot?from_turn_no=1"
            with urlopen(url, timeout=5) as resp:
                snap = json.loads(resp.read())

            status = snap.get("status", "UNKNOWN")
            current = snap.get("current_player_id", "")

            if status == "FINISHED":
                print(json.dumps(snap))
                return
            if status == "RUNNING" and current == player_id:
                print(json.dumps(snap))
                return
        except Exception as exc:
            print(f"poll error: {exc}", file=sys.stderr)

        time.sleep(2)

    print(json.dumps({"error": "timeout", "timeout_seconds": timeout}))


def http_poll_loop(
    server_url: str,
    game_id: str,
    player_id: str,
    output_dir: Path,
) -> None:
    """Persistent polling loop that writes turn files."""
    counter = 0
    last_turn_no = -1

    while True:
        try:
            url = f"{server_url}/v2/games/{game_id}/snapshot?from_turn_no=1"
            with urlopen(url, timeout=5) as resp:
                snap = json.loads(resp.read())

            status = snap.get("status", "UNKNOWN")
            current = snap.get("current_player_id", "")
            turn_no = snap.get("turn_no", -1)

            if status == "FINISHED":
                (output_dir / "game_over.json").write_text(json.dumps(snap))
                print(f"Game finished. Wrote game_over.json", file=sys.stderr)
                return

            if (
                status == "RUNNING"
                and current == player_id
                and turn_no != last_turn_no
            ):
                counter += 1
                last_turn_no = turn_no
                (output_dir / f"my_turn_{counter}.json").write_text(
                    json.dumps(snap)
                )
                print(
                    f"Turn {turn_no} -> my_turn_{counter}.json",
                    file=sys.stderr,
                )
        except Exception as exc:
            print(f"poll error: {exc}", file=sys.stderr)

        time.sleep(2)


# ---------------------------------------------------------------------------
# WebSocket listener (preferred — instant turn detection)
# ---------------------------------------------------------------------------

async def ws_wait(
    server_url: str,
    game_id: str,
    player_id: str,
    timeout: float = 120,
) -> None:
    """Listen on the WebSocket stream until our turn or game end."""
    import websockets  # type: ignore[import-untyped]

    parsed = urlparse(server_url)
    scheme = "wss" if parsed.scheme == "https" else "ws"
    host = parsed.hostname or "localhost"
    port = parsed.port or (443 if scheme == "wss" else 80)
    ws_url = f"{scheme}://{host}:{port}/v2/games/{game_id}/stream?from_turn_no=1"

    deadline = asyncio.get_event_loop().time() + timeout

    while asyncio.get_event_loop().time() < deadline:
        try:
            async with websockets.connect(ws_url) as ws:
                print("ws: connected", file=sys.stderr)

                while asyncio.get_event_loop().time() < deadline:
                    remaining = deadline - asyncio.get_event_loop().time()
                    if remaining <= 0:
                        break

                    try:
                        raw = await asyncio.wait_for(
                            ws.recv(), timeout=min(remaining, 30)
                        )
                    except asyncio.TimeoutError:
                        continue

                    try:
                        msg = json.loads(raw)
                    except (json.JSONDecodeError, TypeError):
                        continue

                    event_type = msg.get("event_type", "")

                    # Game finished — always exit
                    if event_type == "GAME_FINISHED":
                        snapshot = msg.get("snapshot", msg)
                        snapshot["status"] = "FINISHED"
                        print(json.dumps(snapshot))
                        return

                    # Extract the nested snapshot from the event
                    snapshot = msg.get("snapshot")
                    if not snapshot:
                        continue

                    status = snapshot.get("status", "")
                    current = snapshot.get("current_player_id", "")

                    if status == "FINISHED":
                        print(json.dumps(snapshot))
                        return
                    if status == "RUNNING" and current == player_id:
                        print(json.dumps(snapshot))
                        return

        except asyncio.CancelledError:
            break
        except Exception as exc:
            print(f"ws error: {exc}, reconnecting...", file=sys.stderr)
            remaining = deadline - asyncio.get_event_loop().time()
            if remaining <= 0:
                break
            await asyncio.sleep(min(1, remaining))

    print(json.dumps({"error": "timeout", "timeout_seconds": timeout}))


async def ws_loop(
    server_url: str,
    game_id: str,
    player_id: str,
    output_dir: Path,
) -> None:
    """Persistent WebSocket loop that writes turn files."""
    import websockets  # type: ignore[import-untyped]

    parsed = urlparse(server_url)
    scheme = "wss" if parsed.scheme == "https" else "ws"
    host = parsed.hostname or "localhost"
    port = parsed.port or (443 if scheme == "wss" else 80)
    ws_url = f"{scheme}://{host}:{port}/v2/games/{game_id}/stream?from_turn_no=1"

    counter = 0

    while True:
        try:
            async with websockets.connect(ws_url) as ws:
                print("ws: connected (loop mode)", file=sys.stderr)

                while True:
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=30)
                    except asyncio.TimeoutError:
                        continue

                    try:
                        msg = json.loads(raw)
                    except (json.JSONDecodeError, TypeError):
                        continue

                    event_type = msg.get("event_type", "")

                    if event_type == "GAME_FINISHED":
                        snapshot = msg.get("snapshot", msg)
                        snapshot["status"] = "FINISHED"
                        (output_dir / "game_over.json").write_text(
                            json.dumps(snapshot)
                        )
                        print("Game finished. Wrote game_over.json", file=sys.stderr)
                        return

                    snapshot = msg.get("snapshot")
                    if not snapshot:
                        continue

                    status = snapshot.get("status", "")
                    current = snapshot.get("current_player_id", "")

                    if status == "FINISHED":
                        (output_dir / "game_over.json").write_text(
                            json.dumps(snapshot)
                        )
                        print("Game finished. Wrote game_over.json", file=sys.stderr)
                        return

                    if status == "RUNNING" and current == player_id:
                        counter += 1
                        (output_dir / f"my_turn_{counter}.json").write_text(
                            json.dumps(snapshot)
                        )
                        print(
                            f"Turn {snapshot.get('turn_no', '?')} -> "
                            f"my_turn_{counter}.json",
                            file=sys.stderr,
                        )

        except asyncio.CancelledError:
            break
        except Exception as exc:
            print(f"ws error: {exc}, reconnecting in 1s...", file=sys.stderr)
            await asyncio.sleep(1)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    if len(sys.argv) < 2:
        print(
            f"Usage: {sys.argv[0]} [--loop|--wait-for] <args...>",
            file=sys.stderr,
        )
        sys.exit(1)

    # --wait-for mode: block until a turn file appears
    if sys.argv[1] == "--wait-for":
        if len(sys.argv) < 4:
            print(
                f"Usage: {sys.argv[0]} --wait-for <game_id> <turn_counter> [timeout]",
                file=sys.stderr,
            )
            sys.exit(1)
        game_id = sys.argv[2]
        turn_counter = int(sys.argv[3])
        timeout = float(sys.argv[4]) if len(sys.argv) > 4 else 300
        wait_for_file(game_id, turn_counter, timeout)
        return

    # --loop mode: persistent background process
    if sys.argv[1] == "--loop":
        if len(sys.argv) < 5:
            print(
                f"Usage: {sys.argv[0]} --loop <server_url> <game_id> <player_id>",
                file=sys.stderr,
            )
            sys.exit(1)
        server_url = sys.argv[2].rstrip("/")
        game_id = sys.argv[3]
        player_id = sys.argv[4]
        output_dir = _output_dir(game_id)
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / "pid").write_text(str(os.getpid()))
        print(
            f"Loop mode: output_dir={output_dir}, pid={os.getpid()}",
            file=sys.stderr,
        )
        try:
            import websockets  # noqa: F401
            asyncio.run(ws_loop(server_url, game_id, player_id, output_dir))
        except ImportError:
            print("websockets not installed, using HTTP polling", file=sys.stderr)
            http_poll_loop(server_url, game_id, player_id, output_dir)
        return

    # Single-shot mode (default): wait for one turn
    if len(sys.argv) < 4:
        print(
            f"Usage: {sys.argv[0]} <server_url> <game_id> <player_id> [timeout]",
            file=sys.stderr,
        )
        sys.exit(1)

    server_url = sys.argv[1].rstrip("/")
    game_id = sys.argv[2]
    player_id = sys.argv[3]
    timeout = float(sys.argv[4]) if len(sys.argv) > 4 else 120

    try:
        import websockets  # noqa: F401
        asyncio.run(ws_wait(server_url, game_id, player_id, timeout))
    except ImportError:
        print("websockets not installed, using HTTP polling", file=sys.stderr)
        http_poll(server_url, game_id, player_id, timeout)


if __name__ == "__main__":
    main()
