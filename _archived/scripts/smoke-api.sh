#!/usr/bin/env bash
set -euo pipefail

API_BASE_URL="${API_BASE_URL:-http://localhost:3000}"
API_KEY="${API_KEY:-${BTCPC_RELAY_API_KEY:-}}"
MODEL="${MODEL:-qwen3:4b}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-30}"

if [[ -z "${API_KEY}" ]]; then
  echo "ERROR: API_KEY is required. Export API_KEY or BTCPC_RELAY_API_KEY first." >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

request() {
  local name="$1"
  local url="$2"
  local output="$3"
  shift 3

  local status
  status="$(curl -sS --max-time "${TIMEOUT_SECONDS}" -o "${output}" -w '%{http_code}' "$@" "${url}")"
  echo "${status}"
}

check_json_field() {
  local file="$1"
  local expr="$2"
  node -e "const fs=require('fs'); const data=JSON.parse(fs.readFileSync(process.argv[1],'utf8')); const ok=(${expr}); if(!ok){process.exit(1);}" "${file}"
}

print_json_excerpt() {
  local file="$1"
  node -e "const fs=require('fs'); const data=JSON.parse(fs.readFileSync(process.argv[1],'utf8')); console.log(JSON.stringify(data, null, 2).slice(0, 600));" "${file}"
}

echo "BTCPC smoke API"
echo "API_BASE_URL=${API_BASE_URL}"
echo "MODEL=${MODEL}"
echo

health_file="${tmpdir}/health.json"
health_status="$(request "health" "${API_BASE_URL}/health" "${health_file}")"
echo "health: HTTP ${health_status}"
if [[ "${health_status}" != "200" ]]; then
  cat "${health_file}"
  exit 1
fi
check_json_field "${health_file}" "data.status === 'OK'"

models_file="${tmpdir}/models.json"
models_status="$(request "models" "${API_BASE_URL}/v1/models" "${models_file}" -H "Authorization: Bearer ${API_KEY}")"
echo "models: HTTP ${models_status}"
if [[ "${models_status}" != "200" ]]; then
  cat "${models_file}"
  exit 1
fi
check_json_field "${models_file}" "Array.isArray(data.data) && data.data.length > 0"

pricing_file="${tmpdir}/pricing.json"
pricing_status="$(request "pricing" "${API_BASE_URL}/v1/pricing?model=${MODEL}" "${pricing_file}" -H "Authorization: Bearer ${API_KEY}")"
echo "pricing: HTTP ${pricing_status}"
if [[ "${pricing_status}" != "200" ]]; then
  cat "${pricing_file}"
  exit 1
fi
check_json_field "${pricing_file}" "typeof data.tokens_per_btcpc === 'number' && data.tokens_per_btcpc > 0"

network_file="${tmpdir}/network.json"
network_status="$(request "network" "${API_BASE_URL}/v1/network/models" "${network_file}" -H "Authorization: Bearer ${API_KEY}")"
echo "network: HTTP ${network_status}"
if [[ "${network_status}" != "200" ]]; then
  cat "${network_file}"
  exit 1
fi
check_json_field "${network_file}" "Array.isArray(data.available)"

body_file="${tmpdir}/completion_body.json"
cat > "${body_file}" <<JSON
{"model":"${MODEL}","messages":[{"role":"user","content":"Reply with exactly: BTCPC inference ok"}],"max_tokens":32,"temperature":0}
JSON

completion_file="${tmpdir}/completion.json"
completion_status="$(request "completion" "${API_BASE_URL}/v1/chat/completions" "${completion_file}" \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  --data "@${body_file}")"
echo "completion: HTTP ${completion_status}"
if [[ "${completion_status}" != "200" ]]; then
  cat "${completion_file}"
  exit 1
fi
check_json_field "${completion_file}" "Array.isArray(data.choices) && data.choices.length > 0 && typeof data.choices[0].message?.content === 'string' && data.choices[0].message.content.trim().length > 0"

echo
echo "smoke result: PASS"
echo "completion excerpt:"
print_json_excerpt "${completion_file}"
