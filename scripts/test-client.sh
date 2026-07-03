#!/usr/bin/env bash
# ── scripts/test-client.sh ─────────────────────────────────────────────────────
# Drives the umans-proxy-rs server like a real client. Assumes the proxy is
# running on http://127.0.0.1:8084 and a mock upstream is enabled (or you're
# pointing at a real UMANS key).
#
# Usage:
#   ./scripts/test-client.sh                       # default test suite
#   ./scripts/test-client.sh --burst 20             # 20 concurrent non-stream
#   ./scripts/test-client.sh --stream               # exercise SSE streaming
#   ./scripts/test-client.sh --stream-burst 10      # 10 concurrent SSE streams
#   ./scripts/test-client.sh --anthropic            # exercise /v1/messages

set -euo pipefail

BASE="${BASE:-http://127.0.0.1:8084}"
KEY="${KEY:-mock-key}"
BURST=5
RUN_STREAM=0
RUN_ANTHROPIC=0
RUN_BURST=0
STREAM_BURST=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --burst) BURST="${2:-5}"; RUN_BURST=1; shift 2 ;;
    --stream) RUN_STREAM=1; shift ;;
    --stream-burst) STREAM_BURST="${2:-5}"; shift 2 ;;
    --anthropic) RUN_ANTHROPIC=1; shift ;;
    --key) KEY="$2"; shift 2 ;;
    --base) BASE="$2"; shift 2 ;;
    *) echo "unknown arg: $1"; exit 2 ;;
  esac
done

# Colors
G="\033[32m"; R="\033[31m"; Y="\033[33m"; D="\033[90m"; B="\033[1m"; X="\033[0m"

ok()   { printf "${G}✓${X} %s\n" "$1"; }
fail() { printf "${R}✗${X} %s\n" "$1"; FAIL=1; }
note() { printf "${D}  %s${X}\n" "$1"; }
hdr()  { printf "\n${B}── %s ──${X}\n" "$1"; }
FAIL=0

# ── healthz ──────────────────────────────────────────────────────────────────
hdr "Health check"
if STATUS=$(curl -s -o /tmp/h.json -w "%{http_code}" "$BASE/healthz"); then
  if [[ "$STATUS" == "200" ]]; then
    ok "GET /healthz → 200"
    note "$(jq -c '{ok, models_count, valid_tokens, total_tokens, runtime, port}' /tmp/h.json 2>/dev/null || cat /tmp/h.json | head -c 200)"
  else
    fail "GET /healthz → $STATUS"
  fi
fi

# ── /v1/models ───────────────────────────────────────────────────────────────
hdr "Models list"
STATUS=$(curl -s -o /tmp/m.json -w "%{http_code}" "$BASE/v1/models")
if [[ "$STATUS" == "200" ]]; then
  ok "GET /v1/models → 200"
  IDS=$(jq -r '.data[].id' /tmp/m.json 2>/dev/null || echo "(jq missing)")
  note "models: $(echo "$IDS" | tr '\n' ' ')"
else
  fail "GET /v1/models → $STATUS"
fi

# ── Non-streaming chat completion ─────────────────────────────────────────────
hdr "Chat completion (non-streaming)"
RESP=$(curl -s -X POST "$BASE/v1/chat/completions" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "umans-coder",
    "stream": false,
    "messages": [{"role": "user", "content": "Say hello in one word."}]
  }')
CONTENT=$(echo "$RESP" | jq -r '.choices[0].message.content // empty' 2>/dev/null)
if [[ -n "$CONTENT" ]]; then
  ok "POST /v1/chat/completions → $CONTENT"
else
  fail "POST /v1/chat/completions"
  note "$RESP" | head -c 300
fi

# ── Streaming chat completion ─────────────────────────────────────────────────
if [[ $RUN_STREAM -eq 1 ]]; then
  hdr "Chat completion (streaming SSE)"
  echo -n "  "
  curl -sN -X POST "$BASE/v1/chat/completions" \
    -H "Authorization: Bearer $KEY" \
    -H "Content-Type: application/json" \
    -d '{
      "model": "umans-coder",
      "stream": true,
      "messages": [{"role": "user", "content": "Stream test"}]
    }' | while IFS= read -r line; do
      if [[ "$line" =~ ^data:[[:space:]](.*) ]] && [[ "${BASH_REMATCH[1]}" != "[DONE]" ]]; then
        CHUNK=$(echo "${BASH_REMATCH[1]}" | jq -r '.choices[0].delta.content // empty' 2>/dev/null)
        [[ -n "$CHUNK" ]] && printf "%s" "$CHUNK"
      fi
    done
  echo ""
  ok "Streaming received"
fi

# ── Anthropic /v1/messages ──────────────────────────────────────────────────
if [[ $RUN_ANTHROPIC -eq 1 ]]; then
  hdr "Anthropic messages"
  STATUS=$(curl -s -o /tmp/a.json -w "%{http_code}" -X POST "$BASE/v1/messages" \
    -H "x-api-key: $KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "Content-Type: application/json" \
    -d '{
      "model": "umans-coder",
      "max_tokens": 50,
      "messages": [{"role": "user", "content": "Hi from anthropic client"}]
    }')
  if [[ "$STATUS" == "200" ]]; then
    ok "POST /v1/messages → 200"
    note "$(jq -c '{id, type, role, content: .content[0].text}' /tmp/a.json 2>/dev/null)"
  else
    fail "POST /v1/messages → $STATUS"
    note "$(cat /tmp/a.json | head -c 300)"
  fi
fi

# ── Burst test (concurrency gating) ──────────────────────────────────────────
if [[ $STREAM_BURST -gt 0 ]]; then
  hdr "Streaming burst test ($STREAM_BURST concurrent SSE)"
  note "tip: requires --mock-delay-ms to actually hit the gate"
  T0=$(date +%s)
  for i in $(seq 1 "$STREAM_BURST"); do
    (
      resp=$(curl -sN -X POST "$BASE/v1/chat/completions" \
        -H "Authorization: Bearer $KEY" \
        -H "Content-Type: application/json" \
        -d "{\"model\":\"umans-coder\",\"stream\":true,\"messages\":[{\"role\":\"user\",\"content\":\"s-burst $i\"}]}" 2>&1)
      chunks=$(echo "$resp" | grep -c "^data:" || true)
      echo "[stream $i] chunks=$chunks"
    ) &
  done
  wait
  T1=$(date +%s)
  echo ""
  note "elapsed: $((T1 - T0))s"
  ok "$STREAM_BURST streaming requests completed"
fi

if [[ $RUN_BURST -eq 1 ]]; then
  hdr "Burst test ($BURST concurrent requests)"
  note "tip: run the proxy with --mock-delay-ms N to actually hit the gate"
  T0=$(date +%s)
  for i in $(seq 1 "$BURST"); do
    curl -s -o "/tmp/burst-$i.json" -w "%{http_code} " \
      -X POST "$BASE/v1/chat/completions" \
      -H "Authorization: Bearer $KEY" \
      -H "Content-Type: application/json" \
      -d "{\"model\":\"umans-coder\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":\"burst $i\"}]}" &
  done
  wait
  T1=$(date +%s)
  echo ""
  note "elapsed: $((T1 - T0))s"
  OK_COUNT=$(grep -l '"content"' /tmp/burst-*.json 2>/dev/null | wc -l | tr -d ' ')
  QUEUE_COUNT=$(grep -l 'queue_full' /tmp/burst-*.json 2>/dev/null | wc -l | tr -d ' ')
  ok "successful: $OK_COUNT / $BURST"
  [[ "$QUEUE_COUNT" -gt 0 ]] && note "queue-full (503): $QUEUE_COUNT"
fi

# ── Stats after activity ─────────────────────────────────────────────────────
hdr "Stats snapshot"
SUMMARY=$(curl -s "$BASE/api/stats?window=300&mode=summary")
note "$SUMMARY"
TOKENS=$(curl -s "$BASE/api/stats/tokens?window=300")
note "per-key: $TOKENS"

echo ""
if [[ $FAIL -eq 0 ]]; then
  printf "${G}${B}all checks passed${X}\n"
else
  printf "${R}${B}some checks failed${X}\n"
  exit 1
fi