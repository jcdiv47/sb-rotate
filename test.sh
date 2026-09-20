#!/usr/bin/env bash
set -euo pipefail

readonly HOST="dmit-malibu"
readonly REMOTE_BINARY="/root/sb-rotate"
readonly SERVER_CONFIG="/etc/sing-box/config.json"
readonly CLIENTS_DIR="/etc/sing-box/clients/"
readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly LOCAL_BINARY="$SCRIPT_DIR/target/release/sb-rotate"

cd "$SCRIPT_DIR"
cargo build --locked --release

scp "$LOCAL_BINARY" "$HOST:${REMOTE_BINARY}.new"
# ssh "$HOST" \
#   "install -m 0755 '${REMOTE_BINARY}.new' '$REMOTE_BINARY' && \
#    rm -f '${REMOTE_BINARY}.new' && \
#    '$REMOTE_BINARY' rotate \
#      --server '$SERVER_CONFIG' \
#      --clients '$CLIENTS_DIR' \
#      --type hysteria2"
ssh "$HOST" \
  "install -m 0755 '${REMOTE_BINARY}.new' '$REMOTE_BINARY' && \
   rm -f '${REMOTE_BINARY}.new'
