#!/usr/bin/env bash
# Start the local Cassie dev stack (Cassandra + cassie-api + cassie-ai)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [ ! -f .env ]; then
  echo "ERROR: .env file not found in $SCRIPT_DIR"
  echo "Create it with: LLM_API_KEY=sk-ant-..."
  exit 1
fi

echo "==> Building and starting Cassie stack..."
docker compose up -d --build

echo ""
echo "==> Waiting for cassie-api to be healthy..."
for i in $(seq 1 30); do
  if curl -sf http://localhost:8080/ready > /dev/null 2>&1; then
    echo "    cassie-api is ready"
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "    ERROR: cassie-api did not become healthy in time"
    docker compose logs cassie
    exit 1
  fi
  sleep 3
done

echo ""
echo "==> Waiting for cassie-ai to be healthy..."
for i in $(seq 1 20); do
  if curl -sf http://localhost:8081/health > /dev/null 2>&1; then
    echo "    cassie-ai is ready"
    break
  fi
  if [ "$i" -eq 20 ]; then
    echo "    ERROR: cassie-ai did not become healthy in time"
    docker compose logs cassie-ai
    exit 1
  fi
  sleep 3
done

echo ""
echo "==> Stack is up!"
echo "    cassie-api  ->  http://localhost:8080"
echo "    cassie-ai   ->  http://localhost:8081"
echo "    cassandra   ->  localhost:9042"
echo ""
echo "    Query:  curl -s -X POST http://localhost:8081/query \\"
echo "              -H 'Content-Type: application/json' \\"
echo "              -d '{\"user_id\":\"dev\",\"question\":\"your question here\"}'"
echo ""
echo "    Ingest: cd ai-context-pipeline/data-pipeline && \\"
echo "            python ingest_local.py --dir D:/dev/runpod --cassie-url http://localhost:8080 --user-id dev"
