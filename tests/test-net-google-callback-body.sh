#!/bin/bash

##############################################################################
# NET-GOOGLE-CALLBACK-BODY: Bridge-to-App Google Callback Body Contract
#
# Purpose: Verify that representative Google Play lifecycle webhooks deliver
#          the expected normalized callback JSON fields to the app callback URL.
#
# Usage: ./tests/test-net-google-callback-body.sh
#
# Prerequisites:
#   - Bridge Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars
#   - psql, curl, jq, and python installed and in PATH
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/gpbi/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

for cmd in psql curl jq python; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo -e "${RED}FAIL: $cmd is required${NC}"
        exit 1
    fi
done

WEBHOOK_INGRESS_TOKEN="${WEBHOOK_INGRESS_TOKEN:-${WEBHOOK_TOKEN:-}}"
if [[ -z "$WEBHOOK_INGRESS_TOKEN" ]]; then
    echo -e "${RED}FAIL: WEBHOOK_INGRESS_TOKEN or WEBHOOK_TOKEN is required${NC}"
    exit 1
fi

if [[ -z "${BRIDGE_API_KEY:-}" ]]; then
    echo -e "${RED}FAIL: BRIDGE_API_KEY is required${NC}"
    exit 1
fi

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="net-google-callback-body-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
TMP_DIR="${TMPDIR:-/tmp}/bridge-$TEST_RUN_ID"
CAPTURE_FILE="$TMP_DIR/callbacks.jsonl"
RECEIVER_SCRIPT="$TMP_DIR/callback_receiver.py"
CALLBACK_PORT="${CALLBACK_PORT:-$((18080 + ($$ % 1000)))}"
CALLBACK_URL="http://127.0.0.1:$CALLBACK_PORT/bridge-callback"
RECEIVER_PID=""
ORIGINAL_CALLBACK_URL=""
TEST_STATUS="fail"

mkdir -p "$TMP_DIR"
: > "$CAPTURE_FILE"

cleanup() {
    if [[ -n "$ORIGINAL_CALLBACK_URL" ]]; then
        local escaped_callback_url
        escaped_callback_url=$(printf '%s' "$ORIGINAL_CALLBACK_URL" | sed "s/'/''/g")
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
          -c "UPDATE pay.apps SET webhook_callback_url = '$escaped_callback_url' WHERE slug = 'hiha';" >/dev/null 2>&1 || true
    fi

    if [[ -n "$RECEIVER_PID" ]]; then
        kill "$RECEIVER_PID" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

cat > "$RECEIVER_SCRIPT" <<'PY'
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

capture_file = sys.argv[1]
port = int(sys.argv[2])

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw_body = self.rfile.read(length).decode("utf-8")
        try:
            body = json.loads(raw_body) if raw_body else None
        except json.JSONDecodeError:
            body = raw_body

        record = {
            "path": self.path,
            "headers": dict(self.headers),
            "body": body,
        }
        with open(capture_file, "a", encoding="utf-8") as f:
            f.write(json.dumps(record, separators=(",", ":")) + "\n")

        self.send_response(204)
        self.end_headers()

    def log_message(self, format, *args):
        return

server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
server.serve_forever()
PY

python "$RECEIVER_SCRIPT" "$CAPTURE_FILE" "$CALLBACK_PORT" &
RECEIVER_PID=$!

for i in {1..20}; do
    if curl -fsS "http://127.0.0.1:$CALLBACK_PORT/" >/dev/null 2>&1; then
        break
    fi
    if [[ "$i" -eq 20 ]]; then
        echo -e "${RED}FAIL: callback capture server did not start${NC}"
        exit 1
    fi
    sleep 1
done

ORIGINAL_CALLBACK_URL=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT webhook_callback_url FROM pay.apps WHERE slug = 'hiha' LIMIT 1;" -t | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

if [[ -z "$ORIGINAL_CALLBACK_URL" ]]; then
    echo -e "${RED}FAIL: could not load original hiha callback URL${NC}"
    exit 1
fi

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "UPDATE pay.apps SET webhook_callback_url = '$CALLBACK_URL' WHERE slug = 'hiha';" >/dev/null

echo -e "${YELLOW}========================================${NC}"
echo "NET-GOOGLE-CALLBACK-BODY: Bridge-to-App Google Callback Body Contract"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo "Callback capture URL: $CALLBACK_URL"
echo ""

register_and_verify_subscription() {
    local user_id="$1"
    local token="$2"
    local status_header="${3:-}"

    curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      -d "{
        \"external_user_id\": \"$user_id\",
        \"provider\": \"$PROVIDER\",
        \"subscription_id\": \"$PRODUCT_ID\",
        \"reason\": \"test-net-google-callback-body\",
        \"product_type\": \"subscription\",
        \"amount_cents\": 0,
        \"transaction_id\": \"$TEST_RUN_ID-$token\"
      }" >/dev/null

    local extra_args=()
    if [[ -n "$status_header" ]]; then
        extra_args=(-H "X-Test-Subscription-Status: $status_header")
    fi

    local verify_code
    verify_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      "${extra_args[@]}" \
      -d "{
        \"external_user_id\": \"$user_id\",
        \"provider\": \"$PROVIDER\",
        \"subscription_id\": \"$PRODUCT_ID\",
        \"purchase_token\": \"$token\",
        \"product_type\": \"subscription\"
      }")

    if [[ "$verify_code" != "200" ]]; then
        echo -e "${RED}FAIL: verify-purchase failed for $token with HTTP $verify_code${NC}"
        exit 1
    fi
}

send_google_webhook() {
    local message_id="$1"
    local notification_json="$2"
    local notification_b64

    notification_b64=$(echo -n "$notification_json" | base64 -w 0 2>/dev/null || echo -n "$notification_json" | base64)

    local http_code
    http_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
      -H "Content-Type: application/json" \
      -H "X-Webhook-Verification-Mode: off" \
      -d "{
        \"message\": {
          \"data\": \"$notification_b64\",
          \"message_id\": \"$message_id\",
          \"attributes\": {}
        }
      }")

    if [[ "$http_code" != "200" && "$http_code" != "204" ]]; then
        echo -e "${RED}FAIL: webhook $message_id returned HTTP $http_code${NC}"
        exit 1
    fi
}

wait_for_callback() {
    local token="$1"
    local jq_filter="$2"
    local label="$3"

    for i in {1..20}; do
        if jq -s -e --arg token "$token" \
          "map(select(.body.purchase_token == \$token and (.body | $jq_filter))) | length > 0" \
          "$CAPTURE_FILE" >/dev/null 2>&1; then
            echo -e "${GREEN}PASS: $label${NC}"
            return 0
        fi
        sleep 1
    done

    echo -e "${RED}FAIL: callback body not observed for $label${NC}"
    echo -e "${BLUE}Captured callbacks for token $token:${NC}"
    jq -s --arg token "$token" 'map(select(.body.purchase_token == $token) | .body)' "$CAPTURE_FILE" || true
    exit 1
}

echo -e "${YELLOW}[1/5] Revocation/refund callback body${NC}"
REV_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-net-callback-revoked-$TEST_RUN_ID"
REV_USER="test_net_callback_revoked_$TEST_RUN_ID"
register_and_verify_subscription "$REV_USER" "$REV_TOKEN"
REV_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "voidedPurchaseNotification": {
    "purchaseToken": "$REV_TOKEN",
    "orderId": "GPA.$TEST_RUN_ID.revoked",
    "productType": 1,
    "refundType": 0
  }
}
EOF
)
send_google_webhook "test-net-callback-revoked-$TEST_RUN_ID" "$REV_JSON"
wait_for_callback "$REV_TOKEN" '.event_type == "payment.refunded" and .revocation_reason == "REFUND"' "revocation_reason delivered"
echo ""

echo -e "${YELLOW}[2/5] Price step-up consent callback body${NC}"
PRICE_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-net-callback-price-$TEST_RUN_ID"
PRICE_USER="test_net_callback_price_$TEST_RUN_ID"
NEW_PRICE_MICROS=12990000
NEW_PRICE_CENTS=1299
CONSENT_DEADLINE_MS=$(($(date +%s) + 604800))000
register_and_verify_subscription "$PRICE_USER" "$PRICE_TOKEN" "trial"
PRICE_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 22,
    "purchaseToken": "$PRICE_TOKEN",
    "subscriptionId": "$PRODUCT_ID",
    "priceStepUpConsentDetails": {
      "priceMicros": $NEW_PRICE_MICROS,
      "consentDeadlineTimeMillis": $CONSENT_DEADLINE_MS
    }
  }
}
EOF
)
send_google_webhook "test-net-callback-price-$TEST_RUN_ID" "$PRICE_JSON"
wait_for_callback "$PRICE_TOKEN" ".event_type == \"subscription.price_step_up\" and .new_price_cents == $NEW_PRICE_CENTS and .google_price_step_up_consent_deadline != null" "price step-up fields delivered"
echo ""

echo -e "${YELLOW}[3/5] Scheduled pause callback body${NC}"
PAUSE_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-net-callback-pause-$TEST_RUN_ID"
PAUSE_USER="test_net_callback_pause_$TEST_RUN_ID"
PAUSE_SCHEDULE_TS=$(($(date +%s) + 604800))000
register_and_verify_subscription "$PAUSE_USER" "$PAUSE_TOKEN"
PAUSE_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 11,
    "purchaseToken": "$PAUSE_TOKEN",
    "subscriptionId": "$PRODUCT_ID",
    "pauseScheduleTimeMillis": "$PAUSE_SCHEDULE_TS"
  }
}
EOF
)
send_google_webhook "test-net-callback-pause-$TEST_RUN_ID" "$PAUSE_JSON"
wait_for_callback "$PAUSE_TOKEN" '.event_type == "subscription.pause_scheduled" and .google_pause_scheduled_at != null' "scheduled pause field delivered"
echo ""

echo -e "${YELLOW}[4/5] Deferral callback body${NC}"
DEFER_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-net-callback-defer-$TEST_RUN_ID"
DEFER_USER="test_net_callback_defer_$TEST_RUN_ID"
register_and_verify_subscription "$DEFER_USER" "$DEFER_TOKEN"
DEFER_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 9,
    "purchaseToken": "$DEFER_TOKEN",
    "subscriptionId": "$PRODUCT_ID",
    "deferredExpiryTimeMillis": 1900000000000
  }
}
EOF
)
send_google_webhook "test-net-callback-defer-$TEST_RUN_ID" "$DEFER_JSON"
wait_for_callback "$DEFER_TOKEN" '.event_type == "subscription.deferred" and .google_deferred_until != null' "deferral field delivered"
echo ""

echo -e "${YELLOW}[5/5] Scheduled cancellation callback body${NC}"
CANCEL_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-net-callback-cancel-$TEST_RUN_ID"
CANCEL_USER="test_net_callback_cancel_$TEST_RUN_ID"
register_and_verify_subscription "$CANCEL_USER" "$CANCEL_TOKEN"
CANCEL_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 18,
    "purchaseToken": "$CANCEL_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
send_google_webhook "test-net-callback-cancel-$TEST_RUN_ID" "$CANCEL_JSON"
wait_for_callback "$CANCEL_TOKEN" '.event_type == "subscription.cancelled" and .cancellation_mode == "scheduled"' "cancellation_mode delivered"
echo ""

TEST_STATUS="pass"
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo -e "${GREEN}NET-GOOGLE-CALLBACK-BODY Bridge Test PASSED${NC}"
echo "Test Run ID: $TEST_RUN_ID"
echo "Started: $TEST_STARTED_AT"
echo "Finished: $TEST_FINISHED_AT"
echo "Status: $TEST_STATUS"
echo "Verified callback fields: revocation_reason, new_price_cents, google_price_step_up_consent_deadline, google_pause_scheduled_at, google_deferred_until, cancellation_mode"
exit 0
