#!/bin/sh
set -e

echo "=== Cowboy Nginx Reverse Proxy ==="
echo "Listening on port 80 (mapped to host port 8000)"
echo "Routes:"
echo "  /v2/games/{id}/commands  → web-service:8082"
echo "  /v2/games/{id}/stream    → game-watcher:8083 (WebSocket)"
echo "  /v2/games/{id}/snapshot  → game-watcher:8083"
echo "  /v2/*                    → game-manager:8081"
echo "  /*                       → static frontend files"
echo "==================================="

exec nginx -g 'daemon off;'
