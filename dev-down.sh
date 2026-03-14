#!/usr/bin/env bash
# Stop the local Cassie dev stack
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Stopping Cassie stack..."
docker compose down

echo ""
echo "==> Stack stopped. Cassandra data volume is preserved."
echo "    To also wipe the database: docker compose down -v"
