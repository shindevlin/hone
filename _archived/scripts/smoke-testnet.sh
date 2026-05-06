#!/usr/bin/env bash
set -euo pipefail

TESTNET_BASE_URL="${TESTNET_BASE_URL:-https://btcpc.net}"
TESTNET_PAGE_PATH="${TESTNET_PAGE_PATH:-/testnet}"
API_BASE_URL="${API_BASE_URL:-${TESTNET_BASE_URL}}"
API_KEY="${API_KEY:-${BTCPC_RELAY_API_KEY:-${BTCPC_SMOKE_API_KEY:-}}}"
MODEL="${MODEL:-${BTCPC_MODEL:-qwen3:4b}}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-20}"
SMOKE_RETRIES="${SMOKE_RETRIES:-6}"
SMOKE_BACKOFF_MS="${SMOKE_BACKOFF_MS:-750}"
SMOKE_QUORUM="${SMOKE_QUORUM:-2}"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

request() {
  local url="$1"
  local output="$2"
  shift 2
  curl -sS --max-time "${TIMEOUT_SECONDS}" -o "${output}" -w '%{http_code}' "$@" "${url}"
}

json_ok() {
  local file="$1"
  local expr="$2"
  node -e "const fs=require('fs'); const data=JSON.parse(fs.readFileSync(process.argv[1],'utf8')); const ok=(${expr}); if(!ok) process.exit(1);" "${file}"
}

sleep_ms() {
  local ms="$1"
  python3 - <<PY
import time
time.sleep(${ms} / 1000.0)
PY
}

retry_json() {
  local name="$1"
  local url="$2"
  local output="$3"
  local expr="$4"
  shift 4
  local attempt status streak=0 last_status=0
  for attempt in $(seq 1 "${SMOKE_RETRIES}"); do
    status="$(request "${url}" "${output}" "$@")" || status="000"
    last_status="${status}"
    if [[ "${status}" == "200" ]]; then
      if json_ok "${output}" "${expr}"; then
        streak=$((streak + 1))
        if [[ "${streak}" -ge "${SMOKE_QUORUM}" ]]; then
          echo "${status}"
          return 0
        fi
      else
        streak=0
      fi
    else
      streak=0
    fi
    if [[ "${attempt}" -lt "${SMOKE_RETRIES}" ]]; then
      sleep_ms "${SMOKE_BACKOFF_MS}"
    fi
  done
  echo "${last_status}"
  return 1
}

retry_http() {
  local name="$1"
  local url="$2"
  local output="$3"
  shift 3
  local attempt status streak=0 last_status=0
  for attempt in $(seq 1 "${SMOKE_RETRIES}"); do
    status="$(request "${url}" "${output}" "$@")" || status="000"
    last_status="${status}"
    if [[ "${status}" == "200" ]]; then
      streak=$((streak + 1))
      if [[ "${streak}" -ge "${SMOKE_QUORUM}" ]]; then
        echo "${status}"
        return 0
      fi
    else
      streak=0
    fi
    if [[ "${attempt}" -lt "${SMOKE_RETRIES}" ]]; then
      sleep_ms "${SMOKE_BACKOFF_MS}"
    fi
  done
  echo "${last_status}"
  return 1
}

retry_body() {
  local name="$1"
  local url="$2"
  local output="$3"
  local pattern="$4"
  shift 4
  local attempt status streak=0 last_status=0
  for attempt in $(seq 1 "${SMOKE_RETRIES}"); do
    status="$(request "${url}" "${output}" "$@")" || status="000"
    last_status="${status}"
    if [[ "${status}" == "200" ]] && grep -Fqi -- "${pattern}" "${output}"; then
      streak=$((streak + 1))
      if [[ "${streak}" -ge "${SMOKE_QUORUM}" ]]; then
        echo "${status}"
        return 0
      fi
    else
      streak=0
    fi
    if [[ "${attempt}" -lt "${SMOKE_RETRIES}" ]]; then
      sleep_ms "${SMOKE_BACKOFF_MS}"
    fi
  done
  echo "${last_status}"
  return 1
}

echo "BTCPC testnet smoke"
echo "TESTNET_BASE_URL=${TESTNET_BASE_URL}"
echo "API_BASE_URL=${API_BASE_URL}"
echo "MODEL=${MODEL}"
echo

auth_args=()
if [[ -n "${API_KEY}" ]]; then
  auth_args=(-H "Authorization: Bearer ${API_KEY}")
fi

page_file="${tmpdir}/testnet.html"
page_status="$(retry_body "testnet page" "${TESTNET_BASE_URL}${TESTNET_PAGE_PATH}" "${page_file}" "btcpc testnet")"
echo "testnet page: HTTP ${page_status}"
[[ "${page_status}" == "200" ]]

health_file="${tmpdir}/health.json"
health_status="$(retry_json "health" "${API_BASE_URL}/health" "${health_file}" "data.status === 'OK' || data.status === 'ok' || data.ok === true")"
echo "health: HTTP ${health_status}"
[[ "${health_status}" == "200" ]]

status_file="${tmpdir}/machine-status.json"
status_status="$(retry_json "machine-status" "${API_BASE_URL}/public/machine-status" "${status_file}" "typeof data.truth_bearing === 'boolean' && typeof data.connected_peers === 'number'")"
echo "machine-status: HTTP ${status_status}"
[[ "${status_status}" == "200" ]]

network_file="${tmpdir}/network.json"
network_status="$(retry_json "public network" "${API_BASE_URL}/public/network" "${network_file}" "typeof data.truth_bearing === 'boolean' && typeof data.connected_peers === 'number'")"
echo "public network: HTTP ${network_status}"
[[ "${network_status}" == "200" ]]

nodes_file="${tmpdir}/nodes.json"
nodes_status="$(retry_json "node list" "${API_BASE_URL}/api/node/list" "${nodes_file}" "Array.isArray(data.nodes) && data.nodes.length > 0")"
echo "node list: HTTP ${nodes_status}"
[[ "${nodes_status}" == "200" ]]

models_file="${tmpdir}/models.json"
if [[ -n "${API_KEY}" ]]; then
  models_status="$(retry_json "models" "${API_BASE_URL}/v1/models" "${models_file}" "Array.isArray(data.data) && data.data.length > 0" "${auth_args[@]}")"
  echo "models: HTTP ${models_status}"
  [[ "${models_status}" == "200" ]]
else
  echo "models: skipped (API_KEY not set)"
fi

pricing_file="${tmpdir}/pricing.json"
if [[ -n "${API_KEY}" ]]; then
  pricing_status="$(retry_json "pricing" "${API_BASE_URL}/v1/pricing?model=${MODEL}" "${pricing_file}" "typeof data.tokens_per_btcpc === 'number' && data.tokens_per_btcpc > 0" "${auth_args[@]}")"
  echo "pricing: HTTP ${pricing_status}"
  [[ "${pricing_status}" == "200" ]]
else
  echo "pricing: skipped (API_KEY not set)"
fi

if [[ -n "${API_KEY}" ]]; then
  body_file="${tmpdir}/completion.json"
  cat > "${tmpdir}/completion-body.json" <<JSON
{"model":"${MODEL}","messages":[{"role":"user","content":"Reply with exactly: BTCPC testnet smoke ok"}],"max_tokens":32,"temperature":0}
JSON
  completion_status="$(retry_json "completion" "${API_BASE_URL}/v1/chat/completions" "${body_file}" "Array.isArray(data.choices) && data.choices.length > 0 && typeof data.choices[0].message?.content === 'string' && data.choices[0].message.content.trim().length > 0" -H "Authorization: Bearer ${API_KEY}" -H "Content-Type: application/json" --data "@${tmpdir}/completion-body.json")"
  echo "completion: HTTP ${completion_status}"
  [[ "${completion_status}" == "200" ]]
else
  echo "completion: skipped (API_KEY not set)"
fi

echo
echo "smoke result: PASS"
