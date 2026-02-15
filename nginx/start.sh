#!/bin/sh
set -e

# Generate frontend config from environment variables
COWBOY_SERVER="${COWBOY_SERVER:-}"
if [ -n "$COWBOY_SERVER" ]; then
  # Ensure the value has a scheme
  case "$COWBOY_SERVER" in
    http://*|https://*) SERVER_URL="$COWBOY_SERVER" ;;
    *) SERVER_URL="http://$COWBOY_SERVER" ;;
  esac
  echo "window.COWBOY_SERVER = \"$SERVER_URL\";" > /usr/share/nginx/html/config.js
else
  echo "window.COWBOY_SERVER = \"\";" > /usr/share/nginx/html/config.js
fi

echo "=== Cowboy Nginx Reverse Proxy ==="
echo "Listening on port 80 (mapped to host port 8000)"
echo "COWBOY_SERVER=${COWBOY_SERVER:-(not set, using same-origin)}"
echo "Routes:"
echo "  /v2/games/{id}/commands  → web-service:8082"
echo "  /v2/games/{id}/stream    → game-watcher:8083 (WebSocket)"
echo "  /v2/games/{id}/snapshot  → game-watcher:8083"
echo "  /v2/*                    → game-manager:8081"
echo "  /*                       → static frontend files"
echo "==================================="

exec nginx -g 'daemon off;'
