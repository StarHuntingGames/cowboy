#!/bin/sh
# Generate config.js from COWBOY_SERVER environment variable.
# Used by Vercel build and local development.
COWBOY_SERVER="${COWBOY_SERVER:-}"
if [ -n "$COWBOY_SERVER" ]; then
  case "$COWBOY_SERVER" in
    http://*|https://*) SERVER_URL="$COWBOY_SERVER" ;;
    *) SERVER_URL="http://$COWBOY_SERVER" ;;
  esac
else
  SERVER_URL=""
fi
cat > "$(dirname "$0")/config.js" <<EOF
window.COWBOY_SERVER = "$SERVER_URL";
EOF
echo "Generated config.js with COWBOY_SERVER=$SERVER_URL"
