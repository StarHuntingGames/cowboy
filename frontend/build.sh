#!/bin/sh
# Generate config.js from environment variables.
# Used by Vercel build and local development.
COWBOY_SERVER="${COWBOY_SERVER:-}"
if [ -n "$COWBOY_SERVER" ]; then
  case "$COWBOY_SERVER" in
    http://*|https://*) SERVER_URL="$COWBOY_SERVER" ;;
    *) SERVER_URL="https://$COWBOY_SERVER" ;;
  esac
else
  SERVER_URL=""
fi

# COWBOY_SHOW_BOTS: set to "true" to show the bot player options in the UI.
COWBOY_SHOW_BOTS="${COWBOY_SHOW_BOTS:-false}"

cat > "$(dirname "$0")/config.js" <<EOF
window.COWBOY_SERVER = "$SERVER_URL";
window.COWBOY_SHOW_BOTS = $( [ "$COWBOY_SHOW_BOTS" = "true" ] && echo "true" || echo "false" );
EOF
echo "Generated config.js with COWBOY_SERVER=$SERVER_URL COWBOY_SHOW_BOTS=$COWBOY_SHOW_BOTS"
